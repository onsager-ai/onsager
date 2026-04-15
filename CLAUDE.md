# Onsager

AI factory stack — monorepo for the Onsager event bus and its subsystems.

## Architecture

Onsager is a **factory event bus** architecture. Subsystems are runtime-decoupled
via a shared PostgreSQL `events` / `events_ext` table + `pg_notify` channel.
They coordinate through stigmergy (indirect signals via shared medium), not
direct calls.

```
         onsager-spine (event bus lib)
        /              \
   stiglab           synodic        <- do NOT depend on each other
```

**Architectural invariant**: `stiglab` and `synodic` must NOT import each other,
and must NOT be statically linked into the same binary. The `onsager` dispatcher
has zero business dependencies -- it discovers subsystem binaries on PATH.

## Workspace layout

```
crates/
  onsager-spine/   <- event bus client library
  onsager/         <- dispatcher CLI (~100 LOC, no business deps)
  stiglab/         <- distributed AI agent session orchestration (lib + bin)
  synodic/         <- AI agent governance (lib + bin)
apps/
  dashboard/       <- React UI (sessions, nodes, governance, factory views)
```

## Build & Test

```bash
just build           # Rust workspace + dashboard
just test            # All tests
just lint            # fmt + clippy + eslint
```

Or directly:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Conventions

- Rust edition 2021, rustfmt formatting, clippy with warnings-as-errors
- thiserror for library errors, anyhow for application errors
- Small focused commits, imperative mood, under 72 characters
- Unit tests co-located in `#[cfg(test)]` modules
- All internal deps use `path = "../..."` -- no git deps, no crates.io

## Per-crate context

Each subsystem has its own CLAUDE.md or `.claude/` directory with
subsystem-specific instructions:

- `crates/onsager-spine/CLAUDE.md`
- `crates/stiglab/.claude/`
- `crates/synodic/.claude/`
