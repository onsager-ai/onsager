//! `forge serve` — start the Forge scheduling loop.

/// Start the Forge scheduling loop.
///
/// In production, this connects to the event spine, instantiates the scheduling
/// kernel, and runs the core tick loop. For v0.1, this is a skeleton that
/// demonstrates the structure.
pub fn run(database_url: &str, tick_ms: u64) {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async move {
        tracing_subscriber::fmt()
            .with_env_filter("forge=info")
            .init();

        tracing::info!(
            database_url = database_url,
            tick_ms = tick_ms,
            "forge: starting scheduling loop"
        );

        // In production this would:
        // 1. Connect to the event spine (EventStore)
        // 2. Load artifact state from the spine
        // 3. Instantiate the scheduling kernel
        // 4. Run the tick loop with Stiglab dispatch + Synodic gate calls
        //
        // For now, log and exit.
        tracing::info!("forge: v0.1 scaffold — scheduling loop not yet connected to spine");
    });
}
