use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};

use onsager_core::executor::SessionExecutor;
use onsager_core::process::ProcessConfig;
use onsager_core::replay::ReplayEngine;
use onsager_core::task::TaskRequest;
use onsager_core::SessionState;
use onsager_events::{CoreEvent, EventStore};
use onsager_synodic::intercept::InterceptEngine;
use onsager_synodic::processor::PolicyProcessor;

#[derive(Parser)]
#[command(name = "onsager", about = "The AI Factory CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the database schema
    Init,

    /// Run an agent session end-to-end
    Run {
        /// The prompt for the agent
        prompt: String,

        /// Working directory
        #[arg(short = 'w', long, default_value = ".")]
        working_dir: String,

        /// Model to use
        #[arg(short = 'm', long)]
        model: Option<String>,

        /// Maximum conversation turns
        #[arg(long)]
        max_turns: Option<u32>,

        /// System prompt
        #[arg(long)]
        system_prompt: Option<String>,

        /// Permission mode for the agent
        #[arg(long, default_value = "auto")]
        permission_mode: String,

        /// Agent binary to invoke
        #[arg(long, default_value = "claude")]
        agent_command: String,

        /// Disable policy enforcement
        #[arg(long)]
        no_policy: bool,
    },

    /// Replay the event stream for a session
    Replay {
        /// Session ID to replay
        session_id: String,

        /// Follow mode — wait for new events
        #[arg(short = 'f', long)]
        follow: bool,

        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Include extension events
        #[arg(long)]
        include_ext: bool,
    },

    /// Manage sessions
    Sessions {
        #[command(subcommand)]
        command: SessionsCommands,
    },

    /// Browse raw events
    Events {
        /// Filter by stream ID
        #[arg(long)]
        stream: Option<String>,

        /// Filter by event type
        #[arg(long, name = "type")]
        event_type: Option<String>,

        /// Events after this timestamp (ISO 8601)
        #[arg(long)]
        since: Option<String>,

        /// Include extension events
        #[arg(long)]
        include_ext: bool,

        /// Maximum number of events to show
        #[arg(long, default_value = "50")]
        limit: i64,
    },

    /// Manage governance policies
    Policies {
        #[command(subcommand)]
        command: PoliciesCommands,
    },
}

#[derive(Subcommand)]
enum SessionsCommands {
    /// List all sessions
    List {
        /// Filter by state
        #[arg(long)]
        state: Option<String>,
    },
    /// Show session details
    Show {
        /// Session ID
        session_id: String,
    },
}

#[derive(Subcommand)]
enum PoliciesCommands {
    /// List active intercept rules
    List,
    /// Test a tool call against policies
    Test {
        /// Tool name
        tool: String,
        /// Tool input as JSON string
        input: String,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => cmd_init().await,
        Commands::Run {
            prompt,
            working_dir,
            model,
            max_turns,
            system_prompt,
            permission_mode,
            agent_command,
            no_policy,
        } => {
            cmd_run(
                prompt,
                working_dir,
                model,
                max_turns,
                system_prompt,
                permission_mode,
                agent_command,
                no_policy,
            )
            .await
        }
        Commands::Replay {
            session_id,
            follow,
            format,
            include_ext,
        } => cmd_replay(session_id, follow, format, include_ext).await,
        Commands::Sessions { command } => match command {
            SessionsCommands::List { state } => cmd_sessions_list(state).await,
            SessionsCommands::Show { session_id } => cmd_sessions_show(session_id).await,
        },
        Commands::Events {
            stream,
            event_type,
            since,
            include_ext,
            limit,
        } => cmd_events(stream, event_type, since, include_ext, limit).await,
        Commands::Policies { command } => match command {
            PoliciesCommands::List => cmd_policies_list(),
            PoliciesCommands::Test { tool, input } => cmd_policies_test(tool, input),
        },
    }
}

fn get_database_url() -> Result<String> {
    std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))
}

async fn connect_store() -> Result<EventStore> {
    let url = get_database_url()?;
    let store = EventStore::connect(&url).await?;
    Ok(store)
}

// --- Command implementations ---

async fn cmd_init() -> Result<()> {
    let store = connect_store().await?;
    store.migrate().await?;
    println!("Database schema initialized successfully.");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    prompt: String,
    working_dir: String,
    model: Option<String>,
    max_turns: Option<u32>,
    system_prompt: Option<String>,
    permission_mode: String,
    agent_command: String,
    no_policy: bool,
) -> Result<()> {
    let store = connect_store().await?;

    let working_dir = if working_dir == "." {
        std::env::current_dir()?.to_string_lossy().to_string()
    } else {
        std::fs::canonicalize(&working_dir)?
            .to_string_lossy()
            .to_string()
    };

    let config = ProcessConfig {
        agent_command,
        permission_mode: permission_mode.clone(),
        model: model.clone(),
        max_turns,
        system_prompt: system_prompt.clone(),
    };

    let executor = SessionExecutor::new(store.clone(), config);

    let request = TaskRequest {
        prompt: prompt.clone(),
        working_dir,
        model,
        max_turns,
        system_prompt,
        permission_mode: Some(permission_mode),
    };

    let policy_processor = if no_policy {
        None
    } else {
        Some(PolicyProcessor::new(InterceptEngine::with_defaults()))
    };

    // Callback for real-time terminal output
    let on_event: Option<onsager_core::executor::EventCallback> =
        Some(Box::new(|event| match event {
            CoreEvent::TaskCreated { task_id, .. } => {
                eprintln!("[task:{task_id}] Created task");
            }
            CoreEvent::SessionCreated { session_id, .. } => {
                eprintln!("[session:{session_id}] Session created");
            }
            CoreEvent::SessionDispatched { session_id } => {
                eprintln!("[session:{session_id}] Dispatched");
            }
            CoreEvent::SessionRunning { session_id } => {
                eprintln!("[session:{session_id}] Running...");
            }
            CoreEvent::SessionOutput { chunk, .. } => {
                print!("{chunk}");
            }
            CoreEvent::SessionToolUse {
                tool_name,
                tool_input,
                ..
            } => {
                let input_preview = tool_input
                    .as_object()
                    .and_then(|o| {
                        o.get("command")
                            .or_else(|| o.get("file_path"))
                            .or_else(|| o.get("pattern"))
                    })
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                eprintln!("\n[tool_use] {tool_name} {input_preview}");
            }
            CoreEvent::SessionCompleted { session_id, .. } => {
                eprintln!("\n[session:{session_id}] Completed");
            }
            CoreEvent::SessionFailed {
                session_id, error, ..
            } => {
                eprintln!("\n[session:{session_id}] Failed: {error}");
            }
            _ => {}
        }));

    let result = executor
        .run(
            &request,
            on_event,
            policy_processor
                .as_ref()
                .map(|p| p as &dyn onsager_core::executor::PolicyEvaluator),
        )
        .await?;

    eprintln!();
    if result.success {
        eprintln!("Session {} completed successfully.", result.session_id);
    } else {
        eprintln!(
            "Session {} failed: {}",
            result.session_id,
            result.error.as_deref().unwrap_or("unknown error")
        );
    }
    eprintln!("Replay: onsager replay {}", result.session_id);

    Ok(())
}

async fn cmd_replay(
    session_id: String,
    follow: bool,
    format: OutputFormat,
    include_ext: bool,
) -> Result<()> {
    let store = connect_store().await?;
    let engine = ReplayEngine::new(store.clone());

    let entries = engine.replay_session(&session_id, 0, include_ext).await?;

    if entries.is_empty() {
        println!("No events found for session {session_id}");
        return Ok(());
    }

    for entry in &entries {
        match &format {
            OutputFormat::Json => match entry {
                onsager_core::replay::ReplayEntry::Core(r) => {
                    println!("{}", serde_json::to_string(&r.data)?);
                }
                onsager_core::replay::ReplayEntry::Extension(r) => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ext": true,
                            "namespace": r.namespace,
                            "event_type": r.event_type,
                            "data": r.data,
                        })
                    );
                }
            },
            OutputFormat::Text => match entry {
                onsager_core::replay::ReplayEntry::Core(r) => {
                    let ts = r.created_at.format("%H:%M:%S%.3f");
                    println!("[{ts}] #{} {}", r.sequence, r.event_type);
                    // Show relevant data fields
                    if let Some(chunk) = r.data.get("chunk").and_then(|v| v.as_str()) {
                        print!("  {chunk}");
                    } else if let Some(tool) = r.data.get("tool_name").and_then(|v| v.as_str()) {
                        let input = r.data.get("tool_input").unwrap_or(&serde_json::Value::Null);
                        println!("  {tool} {input}");
                    } else if let Some(error) = r.data.get("error").and_then(|v| v.as_str()) {
                        println!("  ERROR: {error}");
                    }
                }
                onsager_core::replay::ReplayEntry::Extension(r) => {
                    let ts = r.created_at.format("%H:%M:%S%.3f");
                    println!("[{ts}] [ext] {}.{}", r.namespace, r.event_type);
                    if let Some(reason) = r.data.get("reason").and_then(|v| v.as_str()) {
                        println!("  {reason}");
                    }
                }
            },
        }
    }

    if follow {
        eprintln!("--- following (Ctrl+C to stop) ---");
        let mut rx = store.subscribe().await?;
        while let Some(notification) = rx.recv().await {
            if notification.stream_id == session_id {
                let ts = chrono::Utc::now().format("%H:%M:%S%.3f");
                println!("[{ts}] {}", notification.event_type);
            }
        }
    }

    Ok(())
}

async fn cmd_sessions_list(state_filter: Option<String>) -> Result<()> {
    let store = connect_store().await?;
    let engine = ReplayEngine::new(store);

    let filter = state_filter
        .as_deref()
        .map(|s| s.parse::<SessionState>())
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid state filter: {e}"))?;

    let sessions = engine.list_sessions(filter).await?;

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!(
        "{:<38} {:<12} {:<20} {:<20}",
        "SESSION ID", "STATE", "CREATED", "UPDATED"
    );
    println!("{}", "-".repeat(90));

    for session in &sessions {
        println!(
            "{:<38} {:<12} {:<20} {:<20}",
            session.id,
            session.state.to_string(),
            session.created_at.format("%Y-%m-%d %H:%M:%S"),
            session.updated_at.format("%Y-%m-%d %H:%M:%S"),
        );
    }

    println!("\n{} session(s)", sessions.len());
    Ok(())
}

async fn cmd_sessions_show(session_id: String) -> Result<()> {
    let store = connect_store().await?;
    let engine = ReplayEngine::new(store);

    match engine.materialize_session(&session_id).await? {
        Some(session) => {
            println!("Session ID:  {}", session.id);
            println!("Task ID:     {}", session.task_id);
            println!("Node ID:     {}", session.node_id);
            println!("State:       {}", session.state);
            println!("Created:     {}", session.created_at);
            println!("Updated:     {}", session.updated_at);
        }
        None => {
            println!("Session {session_id} not found.");
        }
    }
    Ok(())
}

async fn cmd_events(
    stream: Option<String>,
    event_type: Option<String>,
    since: Option<String>,
    include_ext: bool,
    limit: i64,
) -> Result<()> {
    let store = connect_store().await?;

    let since_dt: Option<DateTime<Utc>> = since
        .as_deref()
        .map(|s| {
            s.parse::<DateTime<Utc>>()
                .map_err(|e| anyhow::anyhow!("invalid timestamp: {e}"))
        })
        .transpose()?;

    let events = store
        .query_events(stream.as_deref(), event_type.as_deref(), since_dt, limit)
        .await?;

    if events.is_empty() && !include_ext {
        println!("No events found.");
        return Ok(());
    }

    println!("{:<6} {:<38} {:<24} {:<4}", "ID", "STREAM", "TYPE", "SEQ");
    println!("{}", "-".repeat(75));

    for event in &events {
        println!(
            "{:<6} {:<38} {:<24} {:<4}",
            event.id, event.stream_id, event.event_type, event.sequence
        );
    }

    if include_ext {
        let ext_events = store
            .query_ext_events(stream.as_deref(), None, limit)
            .await?;
        if !ext_events.is_empty() {
            println!("\n--- Extension Events ---");
            for event in &ext_events {
                println!(
                    "{:<6} {:<38} {}.{}",
                    event.id, event.stream_id, event.namespace, event.event_type
                );
            }
        }
    }

    Ok(())
}

fn cmd_policies_list() -> Result<()> {
    let engine = InterceptEngine::with_defaults();
    let rules = engine.rules();

    println!("{:<20} DESCRIPTION", "RULE");
    println!("{}", "-".repeat(70));

    for rule in rules {
        println!("{:<20} {}", rule.id, rule.description);
        if let Some(tools) = &rule.tools {
            println!("                     tools: {}", tools.join(", "));
        }
    }

    println!("\n{} rule(s)", rules.len());
    Ok(())
}

fn cmd_policies_test(tool: String, input: String) -> Result<()> {
    let engine = InterceptEngine::with_defaults();

    let tool_input: serde_json::Value =
        serde_json::from_str(&input).map_err(|e| anyhow::anyhow!("invalid JSON input: {e}"))?;

    let request = onsager_synodic::InterceptRequest {
        tool_name: tool.clone(),
        tool_input,
    };

    let response = engine.evaluate(&request);

    match response.decision {
        onsager_synodic::Decision::Allow => {
            println!("ALLOWED: {tool} call passed all policy checks.");
        }
        onsager_synodic::Decision::Block => {
            println!(
                "BLOCKED by rule '{}': {}",
                response.rule.as_deref().unwrap_or("unknown"),
                response.reason.as_deref().unwrap_or("no reason")
            );
        }
    }

    Ok(())
}
