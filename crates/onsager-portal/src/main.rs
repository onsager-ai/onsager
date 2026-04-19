//! `onsager-portal` binary entry point — see crate docs for the full
//! ingestion pipeline.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "onsager-portal",
    about = "GitHub webhook ingress for the Onsager factory"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the webhook server.
    Serve {
        /// Bind address (default `0.0.0.0:8080` or `PORTAL_BIND`).
        #[arg(long, env = "PORTAL_BIND", default_value = "0.0.0.0:8080")]
        bind: String,
        /// Postgres URL (`DATABASE_URL`).
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        /// AES-256-GCM credential key (hex), shared with stiglab so the
        /// portal can decrypt `webhook_secret_cipher` rows.
        #[arg(long, env = "ONSAGER_CREDENTIAL_KEY")]
        credential_key: Option<String>,
        /// Synodic gate URL (e.g. `http://synodic:3001`).
        #[arg(long, env = "SYNODIC_URL")]
        synodic_url: Option<String>,
        /// Optional GitHub PAT used for posting check runs / comments when
        /// installation-token signing isn't wired up. Prefer per-installation
        /// tokens when available.
        #[arg(long, env = "GITHUB_TOKEN")]
        github_token: Option<String>,
    },
    /// Backfill issues + PRs for an existing project.
    Backfill {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long)]
        project: String,
        #[arg(long, default_value = "recent")]
        strategy: String,
        #[arg(long, default_value_t = 100)]
        cap: usize,
        #[arg(long, env = "GITHUB_TOKEN")]
        github_token: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "onsager_portal=info".into()),
        )
        .init();

    match cli.command {
        Command::Serve {
            bind,
            database_url,
            credential_key,
            synodic_url,
            github_token,
        } => {
            let cfg = onsager_portal::config::Config {
                bind: bind.clone(),
                database_url,
                credential_key,
                synodic_url,
                github_token,
            };
            tracing::info!(%bind, "onsager-portal: starting webhook server");
            onsager_portal::server::run(cfg).await
        }
        Command::Backfill {
            database_url,
            project,
            strategy,
            cap,
            github_token,
        } => {
            let strategy: onsager_portal::backfill::Strategy = strategy.parse()?;
            let pool = onsager_portal::db::connect(&database_url).await?;
            onsager_portal::migrate::run(&pool).await?;
            let store = onsager_spine::EventStore::connect(&database_url).await?;
            let report =
                onsager_portal::backfill::run(&pool, &store, &project, strategy, cap, github_token)
                    .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
    }
}
