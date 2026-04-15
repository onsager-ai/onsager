//! `ising serve` — start the Ising observation loop.

/// Start the Ising observation loop.
///
/// In production, this connects to the event spine, instantiates the analyzer
/// registry, and runs the core observation loop. For v0.1, this is a skeleton.
pub fn run(_database_url: &str, tick_ms: u64) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async move {
        tracing_subscriber::fmt()
            .with_env_filter("ising=info")
            .init();

        tracing::info!(tick_ms = tick_ms, "ising: starting observation loop");

        // In production this would:
        // 1. Connect to the event spine (EventStore)
        // 2. Register all analyzers (register_defaults)
        // 3. Subscribe to pg_notify and consume events
        // 4. Run analyzers on each tick
        // 5. Emit insights back to the spine
        //
        // For now, log and exit.
        tracing::info!("ising: v0.1 scaffold — observation loop not yet connected to spine");
    });
}
