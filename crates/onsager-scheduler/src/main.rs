//! `onsager-scheduler` — substrate scheduler host binary (RUN-03,
//! #386). Listens on the spine for `trigger.fired`, compiles each
//! fire's workflow into an [`ExecutionPlan`], and runs it through
//! [`onsager_nodes::Scheduler`].
//!
//! Entrypoint shape mirrors the other subsystem binaries: a single
//! `serve` subcommand under clap. The dispatcher (`crates/onsager/`)
//! resolves this binary on PATH like every other subsystem.

use anyhow::Result;
use clap::{Parser, Subcommand};
use onsager_scheduler::{SchedulerService, ServiceConfig};

#[derive(Parser)]
#[command(
    name = "onsager-scheduler",
    about = "Substrate scheduler host (RUN-03, #386)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the scheduler service: subscribe to the spine, drive
    /// trigger.fired through the bridge.
    Serve(ServeArgs),
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Postgres connection URL.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    /// Actor stamped on spine emits. Defaults to "substrate-scheduler".
    #[arg(long, env = "SCHEDULER_ACTOR", default_value = "substrate-scheduler")]
    actor: String,
    /// Replay every historical `trigger.fired` on startup. Off by
    /// default — surgical re-fires use `onsager trigger replay`.
    #[arg(long, env = "SCHEDULER_REPLAY_HISTORY", default_value_t = false)]
    replay_history: bool,
    /// AES-256-GCM credential key (hex), shared with portal / stiglab.
    /// Decrypts the plan-scoped session token (#536) off
    /// `plan.run_requested` so agent nodes get `ONSAGER_SESSION_TOKEN`.
    /// When unset, agent nodes run without MCP auth (gate still applies).
    #[arg(long, env = "ONSAGER_CREDENTIAL_KEY")]
    credential_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("onsager_scheduler=info")),
        )
        .compact()
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve(args) => {
            let config = ServiceConfig {
                database_url: args.database_url,
                actor: args.actor,
                replay_history: args.replay_history,
                credential_key: args.credential_key.filter(|s| !s.is_empty()),
            };
            SchedulerService::new(config).run().await
        }
    }
}
