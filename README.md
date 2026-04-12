# Onsager

Client library for the Onsager event spine — shared PostgreSQL event stream coordination for the [onsager-ai](https://github.com/onsager-ai) polyrepo.

Onsager is a **library**, not a service. It publishes a single Rust crate that sibling repos (`stiglab`, `synodic`, `ising`, `telegramable`) depend on to coordinate via a shared PostgreSQL `events` / `events_ext` table and the `onsager_events` pg_notify channel.

## Installation

Pre-publication — install via git dependency:

```toml
onsager = { git = "https://github.com/onsager-ai/onsager", branch = "main" }
```

## Concepts

| Type | Role | Description |
|------|------|-------------|
| `EventStore` | Producer + Consumer | Read/write access to `events` and `events_ext` tables, plus real-time `pg_notify` subscription. |
| `Listener` | Consumer | High-level consumer that filters notifications by `Namespace` and dispatches them to an `EventHandler`. |
| `EventHandler` | Consumer | Trait implemented by code that reacts to events. |
| `Namespace` | Both | Validated newtype that partitions the `events_ext` table between components. |

## Schema

This library does **not** create or migrate database tables. The schema contract lives in [`migrations/001_initial.sql`](migrations/001_initial.sql) and downstream services apply it themselves (e.g. via `sqlx migrate`, a CI step, or manual execution).

Schema changes are coordinated by adding a new `00X_*.sql` file and bumping the crate version.

### Why two tables?

- **`events`** — strong-schema core spine. Every event has a `stream_id`, `stream_type`, `event_type`, typed `data` JSONB, `sequence` number, and `metadata`. This is the append-only event log that all components share.
- **`events_ext`** — wide JSON extension table namespaced by component. Each component owns a `namespace` (e.g. `"stiglab"`, `"synodic"`) and can publish arbitrary extension events without changing the core schema.

Both tables fire `pg_notify` on insert via the `onsager_events` channel, enabling real-time subscription.

## Usage

### Producer

```rust
use onsager::{CoreEvent, EventMetadata, EventStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = EventStore::connect(&std::env::var("DATABASE_URL")?).await?;

    let event = CoreEvent::SessionCreated {
        session_id: "stiglab:session:demo-1".into(),
        task_id: "stiglab:task:1".into(),
        node_id: "node-1".into(),
    };
    let metadata = EventMetadata {
        actor: "my-service".into(),
        ..Default::default()
    };

    let id = store.append(&event, &metadata).await?;
    println!("appended event {id}");
    Ok(())
}
```

### Consumer

```rust
use onsager::{EventHandler, EventNotification, Listener, Namespace, EventStore};
use async_trait::async_trait;

struct MyHandler;

#[async_trait]
impl EventHandler for MyHandler {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        println!("received: {} ({})", event.stream_id, event.event_type);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = EventStore::connect(&std::env::var("DATABASE_URL")?).await?;

    Listener::new(store)
        .subscribe(Namespace::stiglab())
        .run(MyHandler)
        .await
}
```

## License

AGPL-3.0
