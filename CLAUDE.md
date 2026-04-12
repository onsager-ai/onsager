# Onsager

AI Factory — event-stream-based orchestration for AI agent sessions with quality control, traceability, and continuous improvement.

## Architecture

- **onsager-events**: PostgreSQL event store (append-only events + extension events + pg_notify)
- **onsager-core**: Domain types (Session, Task, Node), session executor, replay engine
- **onsager-synodic**: Policy enforcement layer (InterceptEngine with pattern-based rules)
- **onsager-cli**: CLI binary tying everything together

## Build & Test

```bash
cargo build              # Build all crates
cargo test               # Run all tests
cargo clippy -- -D warnings  # Lint
cargo fmt --check        # Format check
```

## Local Development

```bash
docker compose up -d     # Start PostgreSQL
export DATABASE_URL=postgres://onsager:onsager@localhost:5432/onsager
cargo run -- init        # Initialize database schema
cargo run -- run "your prompt here"  # Run a session
```

## Conventions

- Rust edition 2021, rustfmt formatting, clippy with warnings-as-errors
- thiserror for library errors, anyhow for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
