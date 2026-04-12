# Onsager

The AI Factory — event-stream-based orchestration for AI agent sessions with quality control, traceability, and continuous improvement.

## Overview

Onsager is a factory for AI production. It organizes AI agent work through a shared PostgreSQL event stream, providing standardized production units, quality control, traceability, and continuous improvement.

### Core Level 1

This is the Core Level 1 implementation: a single-binary CLI that can:

- **Run** an AI agent session end-to-end with full event recording
- **Enforce** governance policies (observational mode — logs violations as audit events)
- **Replay** the complete event stream for any session
- **Browse** sessions, events, and policy rules

## Architecture

```
┌────────────────────────────────────────────┐
│              PostgreSQL                    │
│  ┌──────────┐  ┌────────────────────────┐  │
│  │ events   │  │ events_ext             │  │
│  │ (core)   │  │ (extension, namespaced)│  │
│  └──────────┘  └────────────────────────┘  │
└──────────┬──────────────┬──────────────────┘
           │  pg_notify   │
    ┌──────┼──────────────┼──────┐
    ▼      ▼              ▼      ▼
┌────────┐ ┌──────────┐ ┌──────────────────┐
│Executor│ │ Synodic  │ │  Replay Engine   │
│(Claude)│ │ (Policy) │ │  (Materialize)   │
└────────┘ └──────────┘ └──────────────────┘
```

**Four crates:**

- `onsager-events` — PostgreSQL event store (append-only events + extension events + pg_notify)
- `onsager-core` — Domain types (Session, Task, Node), session executor, replay engine
- `onsager-synodic` — Policy enforcement layer (5 default intercept rules)
- `onsager-cli` — CLI binary

## Quick Start

```bash
# Start PostgreSQL
docker compose up -d

# Initialize the database
export DATABASE_URL=postgres://onsager:onsager@localhost:5432/onsager
cargo run -- init

# Run an agent session
cargo run -- run "Create a hello world Python script" -w /tmp/test

# List sessions
cargo run -- sessions list

# Replay a session's event stream
cargo run -- replay <session-id> --include-ext

# Test a policy rule
cargo run -- policies test Bash '{"command": "git push --force origin main"}'
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `onsager init` | Initialize database schema |
| `onsager run <prompt>` | Run an agent session end-to-end |
| `onsager replay <session-id>` | Replay the event stream for a session |
| `onsager sessions list` | List all sessions |
| `onsager sessions show <id>` | Show session details |
| `onsager events` | Browse raw events |
| `onsager policies list` | Show active governance rules |
| `onsager policies test <tool> <json>` | Test a tool call against policies |

## Governance Rules

Five default rules ported from [Synodic](https://github.com/onsager-ai/synodic):

| Rule | Blocks |
|------|--------|
| `destructive_git` | `git push --force`, `git reset --hard`, `git clean -fd` |
| `secrets_in_args` | Tool calls containing `password=`, `secret=`, `token=`, `api_key=` |
| `etc_writes` | File writes to `/etc/**` |
| `usr_writes` | File writes to `/usr/**` |
| `dangerous_rm` | `rm -rf /`, `rm -rf ~`, `rm -rf $HOME` |

## Development

```bash
cargo build              # Build all crates
cargo test               # Run all tests (21 unit tests)
cargo clippy -- -D warnings  # Lint
cargo fmt --check        # Format check
```

## License

AGPL-3.0
