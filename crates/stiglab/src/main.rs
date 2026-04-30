mod runner;

// cache-bust: 2026-04-08b
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use stiglab::agent::config::AgentConfig;
use stiglab::server::config::ServerConfig;
use stiglab::server::spine::SpineEmitter;
use stiglab::server::{db, state::AppState};

#[derive(Parser)]
#[command(
    name = "stiglab",
    about = "Stiglab – distributed AI agent orchestration"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run as server: serves UI, API, and optionally executes tasks locally
    Server {
        /// Disable the built-in task runner (server becomes API-only, requires external agents)
        #[arg(long, env = "STIGLAB_NO_RUNNER")]
        no_runner: bool,

        /// Maximum concurrent sessions for the built-in runner
        #[arg(long, env = "STIGLAB_MAX_SESSIONS", default_value = "4")]
        max_sessions: u32,

        /// Command to execute for agent sessions
        #[arg(long, env = "STIGLAB_AGENT_COMMAND", default_value = "claude")]
        agent_command: String,

        /// Name for the built-in runner node
        #[arg(long, env = "STIGLAB_NODE_NAME")]
        node_name: Option<String>,
    },

    /// Run as agent: connects to a server and executes tasks
    Agent(AgentArgs),
}

#[derive(Parser)]
struct AgentArgs {
    /// WebSocket URL of the server
    #[arg(
        long,
        short,
        env = "STIGLAB_SERVER_URL",
        default_value = "ws://localhost:3000/agent/ws"
    )]
    server: String,

    /// Name of this node
    #[arg(long, short, env = "STIGLAB_NODE_NAME")]
    name: Option<String>,

    /// Maximum concurrent sessions
    #[arg(long, short, env = "STIGLAB_MAX_SESSIONS", default_value = "4")]
    max_sessions: u32,

    /// Command to execute for agent sessions
    #[arg(long, env = "STIGLAB_AGENT_COMMAND", default_value = "claude")]
    agent_command: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Server {
            no_runner,
            max_sessions,
            agent_command,
            node_name,
        } => run_server(no_runner, max_sessions, agent_command, node_name).await,
        Command::Agent(args) => run_agent(args).await,
    }
}

async fn run_server(
    no_runner: bool,
    max_sessions: u32,
    agent_command: String,
    node_name: Option<String>,
) -> anyhow::Result<()> {
    let config = ServerConfig::from_env();
    tracing::info!("starting stiglab server on {}:{}", config.host, config.port);

    tracing::info!("connecting to database...");
    let pool = db::init_pool(&config.database_url).await?;
    tracing::info!("database connected");

    // Register the GitHub adapter into the spine `artifact_adapters`
    // catalog (closes the empty-catalog drift from migration 004).
    // Best-effort: a missing table on older schemas shouldn't block
    // boot. See `onsager_github::adapter::register_any`.
    if let Err(e) = onsager_github::adapter::register_any(&pool, "default").await {
        tracing::warn!("github adapter registration skipped: {e}");
    }

    // Dev-login seeding (issue #193). Debug builds always materialize a
    // `${USER}@local` user + `dev` workspace + membership, idempotently,
    // so the LoginPage's "Dev Login" button always has a real user to
    // mint a session for. Release builds skip this entirely — the
    // symbol is `cfg(debug_assertions)`-gated, not just no-op'd.
    #[cfg(debug_assertions)]
    if let Err(e) = stiglab::server::dev_auth::seed_dev_user_and_workspace(&pool).await {
        tracing::warn!("dev-login: seeder failed (non-fatal): {e}");
    }

    // Connect to Onsager event spine if configured
    let spine = if let Ok(url) = std::env::var("ONSAGER_DATABASE_URL") {
        tracing::info!("connecting to onsager event spine...");
        match SpineEmitter::connect(&url).await {
            Ok(emitter) => {
                tracing::info!("onsager event spine connected");
                Some(emitter)
            }
            Err(e) => {
                tracing::warn!("failed to connect to onsager event spine: {e}");
                None
            }
        }
    } else {
        tracing::info!("ONSAGER_DATABASE_URL not set, spine events disabled");
        None
    };

    let state = AppState::new(pool.clone(), config.clone(), spine);

    // Backfill the spine `workflows` schema from `workspace_workflows` so
    // forge can resolve workflows created before the mirror landed (and
    // self-heal any drift caused by best-effort CRUD-time mirroring).
    if let Some(spine) = state.spine.as_ref() {
        match stiglab::server::workflow_spine_mirror::backfill(&state.db, spine.pool()).await {
            Ok(n) => tracing::info!("workflow spine backfill: synced {n} workflow(s)"),
            Err(e) => tracing::warn!("workflow spine backfill failed: {e}"),
        }
    }

    // Spawn the forge.shaping_dispatched listener (spec #131 / ADR 0004
    // Lever C, phase 3). Replaces the legacy `forge → POST /api/shaping`
    // HTTP path with an event-driven flow: the listener consumes each
    // request, calls the same dispatch core the HTTP route uses, and
    // reuses idempotency via `request_id` so spine redelivery never
    // produces a duplicate session. Result correlation flows back via
    // `stiglab.shaping_result_ready` from the agent message handler.
    //
    // Warm-start at `max_event_id` so a fresh boot doesn't replay every
    // historical request. Phase 6 will persist a per-process cursor.
    if let Some(spine) = state.spine.as_ref() {
        let listener_store = spine.store_clone();
        let listener_state = state.clone();
        tokio::spawn(async move {
            let since = match listener_store.max_event_id().await {
                Ok(cursor) => cursor,
                Err(e) => {
                    tracing::warn!(
                        "stiglab: max_event_id lookup failed ({e}); starting \
                         shaping_dispatched listener from the beginning"
                    );
                    None
                }
            };
            if let Err(e) =
                stiglab::server::shaping_listener::run(listener_store, listener_state, since).await
            {
                tracing::error!("stiglab: shaping_dispatched listener exited: {e}");
            }
        });
    }

    // Start built-in runner if enabled
    if !no_runner {
        let runner_node_name = node_name.unwrap_or_else(|| "built-in-runner".to_string());

        tracing::info!(
            "built-in runner enabled: node={runner_node_name}, max_sessions={max_sessions}, command={agent_command}"
        );

        runner::start_built_in_runner(
            &state,
            &pool,
            &runner_node_name,
            max_sessions,
            &agent_command,
        )
        .await?;
    } else {
        tracing::info!("built-in runner disabled, external agents required");
    }

    let app = stiglab::server::build_router(state, &config);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_agent(args: AgentArgs) -> anyhow::Result<()> {
    let config = AgentConfig {
        server: args.server,
        name: args.name,
        max_sessions: args.max_sessions,
        agent_command: args.agent_command,
    };

    tracing::info!("stiglab agent starting");
    tracing::info!("  node name: {}", config.node_name());
    tracing::info!("  server: {}", config.server);
    tracing::info!("  max sessions: {}", config.max_sessions);

    loop {
        match stiglab::agent::connection::connect_and_run(config.clone()).await {
            Ok(()) => {
                tracing::info!("connection closed, reconnecting in 5s...");
            }
            Err(e) => {
                tracing::error!("connection error: {e}, reconnecting in 5s...");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
