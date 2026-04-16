# 000 — Architecture Overview

## Summary

Stiglab is a distributed AI agent session orchestration platform. It manages
multiple AI coding agent sessions (Claude Code, Codex, Gemini CLI, etc.) across
distributed machines from a unified control plane.

## Architecture

Single `stiglab` binary with two subcommands: `serve` (control plane) and
`agent` (node agent). Shipped as one crate (`crates/stiglab/`).

- **Server** (`src/server/`): Axum HTTP/WS control plane — task dispatch, session lifecycle, node routing, state persistence (PostgreSQL), spine event emission.
- **Agent** (`src/agent/`): Node agent that connects to the control plane via WebSocket, manages agent subprocesses, and reports session state.
- **Core** (`src/core/`): Shared types — session state machine, node model, task model, protocol definitions, error handling.
- **Dashboard** (`apps/dashboard/`): React + shadcn/ui frontend for monitoring nodes, sessions, and streaming logs.

## Key Design Insight

AI coding agent sessions have a **WAITING_INPUT** state that traditional task
runners lack. The system supports bidirectional communication, not just one-way
log streaming.

## Session State Machine

```
PENDING -> DISPATCHED -> RUNNING <-> WAITING_INPUT
                          |
                     +----+----+
                     v         v
                   DONE      FAILED
```
