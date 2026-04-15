# Onsager

AI factory stack — unified monorepo.

## Subsystems

| Crate              | Role                                                  |
|--------------------|-------------------------------------------------------|
| `onsager-spine`    | Shared event bus library (PostgreSQL + pg_notify)     |
| `onsager`          | Unified CLI dispatcher (`onsager <subsystem> ...`)    |
| `stiglab`          | Distributed AI agent session orchestration            |
| `synodic`          | AI agent governance (hooks + spine integration)       |

All subsystems coordinate at runtime through the `onsager-spine` event bus.
They are **not** statically linked into a shared binary — loose coupling is
preserved at the build dependency graph level.

## Dashboard

A single React app at `apps/dashboard/` surfaces sessions (stiglab), nodes
(stiglab), governance (synodic), and factory events (onsager-spine) views.

## Build

    just build         # Rust workspace + dashboard
    just test
    just lint

## Run locally

    just dev-stiglab   # cargo run -p stiglab -- serve
    just dev-synodic   # cargo run -p synodic -- serve
    just dev-dashboard # pnpm --filter dashboard dev

## Install

    just install       # installs onsager, stiglab, synodic binaries

After install, both forms work:

    onsager stiglab serve
    stiglab serve
