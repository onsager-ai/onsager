# Examples

## producer_consumer

A single-process example that spawns an event listener and a producer to demonstrate the core Onsager workflow.

Requires a running PostgreSQL instance with the schema from `migrations/001_initial.sql` applied.

```bash
export DATABASE_URL=postgres://onsager:onsager@localhost:5432/onsager
cargo run --example producer_consumer
```
