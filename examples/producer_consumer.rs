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
use chrono::Utc;
use onsager::{
    EventHandler, EventMetadata, EventNotification, EventStore, FactoryEvent, FactoryEventKind,
    Listener, Namespace,
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

    let metadata = EventMetadata {
        actor: "example-producer".into(),
        ..Default::default()
    };

    // Produce events BEFORE the listener starts so that they are recovered
    // via backfill rather than pg_notify.
    println!("producing {expected} events before listener starts (will be backfilled)...");
    let mut first_id: Option<i64> = None;
    for i in 1..=expected {
        let event = FactoryEvent {
            event: FactoryEventKind::StiglabSessionCreated {
                session_id: format!("stiglab:session:demo-{i}"),
                request_id: format!("stiglab:request:{i}"),
                node_id: "example-node".into(),
            },
            correlation_id: None,
            causation_id: None,
            actor: "example-producer".into(),
            timestamp: Utc::now(),
        };
        let id = store.append_factory_event(&event, &metadata).await?;
        println!("  appended event {id}");
        if first_id.is_none() {
            first_id = Some(id);
        }
    }

    // Start the listener with a backfill cursor that picks up all pre-existing
    // events. The listener matches events whose stream_id starts with "stiglab:".
    let cursor = first_id.map(|id| id - 1); // replay from just before the first event
    let listener_store = store.clone();
    let listener_handle = tokio::spawn(async move {
        Listener::new(listener_store)
            .subscribe(Namespace::stiglab())
            .with_since(cursor)
            .run(handler)
            .await
    });

    // Wait for the handler to receive all events.
    notify.notified().await;
    println!("all {expected} events received, shutting down");

    // Cancel the listener.
    listener_handle.abort();
    Ok(())
}
