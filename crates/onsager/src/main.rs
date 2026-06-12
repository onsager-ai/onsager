//! Onsager — the factory, one process (ADR 0029).
//!
//! Serves the HTTP edge (dashboard API, auth, credentials; later
//! webhooks + MCP) and hosts the scheduler as an in-process task (M1).
//! Configuration is environment-only; see [`onsager::config::Config`].

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let config = match onsager::config::Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("onsager: bad configuration: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = onsager::serve(config).await {
        tracing::error!("onsager: fatal: {e:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
