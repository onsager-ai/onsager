//! Producer/consumer example using the Onsager event spine.
//!
//! Requires a running PostgreSQL with the schema from `migrations/001_initial.sql` applied.
//!
//! ```bash
//! export DATABASE_URL=postgres://onsager:onsager@localhost:5432/onsager
//! cargo run --example producer_consumer
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use onsager::{
    CoreEvent, EventHandler, EventMetadata, EventNotification, EventStore, Listener, Namespace,
};

/// A simple handler that prints events and counts them.
struct PrintHandler {
    count: Arc<AtomicUsize>,
    expected: usize,
    notify: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl EventHandler for PrintHandler {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        println!(
            "  received event #{}: stream_id={}, type={}",
            event.id, event.stream_id, event.event_type
        );
        let prev = self.count.fetch_add(1, Ordering::SeqCst);
        if prev + 1 >= self.expected {
            self.notify.notify_one();
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let store = EventStore::connect(&database_url).await?;
    println!("connected to event store");

    // Shared state to know when we've received all events.
    let expected = 3;
    let count = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(tokio::sync::Notify::new());

    let handler = PrintHandler {
        count: Arc::clone(&count),
        expected,
        notify: Arc::clone(&notify),
    };

    // Spawn the listener in the background, subscribed to stiglab events.
    let listener_store = store.clone();
    let listener_handle = tokio::spawn(async move {
        Listener::new(listener_store)
            .subscribe(Namespace::stiglab())
            .run(handler)
            .await
    });

    // Give the listener a moment to connect to pg_notify.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Produce a few events with stream_ids prefixed by the stiglab namespace.
    let metadata = EventMetadata {
        actor: "example-producer".into(),
        ..Default::default()
    };

    println!("producing {expected} events...");
    for i in 1..=expected {
        let event = CoreEvent::SessionCreated {
            session_id: format!("stiglab:session:demo-{i}"),
            task_id: format!("stiglab:task:{i}"),
            node_id: "example-node".into(),
        };
        let id = store.append(&event, &metadata).await?;
        println!("  appended event {id}");
    }

    // Wait for the handler to receive all events.
    notify.notified().await;
    println!("all {expected} events received, shutting down");

    // Cancel the listener.
    listener_handle.abort();
    Ok(())
}
