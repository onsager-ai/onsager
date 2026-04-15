# 000 — Architecture Overview

This document references the full architecture specification defined in `BOOTSTRAP_PROMPT.md` at the repository root.

## Summary

Stiglab is a distributed AI agent session orchestration platform. It manages multiple AI coding agent sessions (Claude Code, Codex, Gemini CLI, etc.) across distributed machines from a unified control plane.

## Architecture

- **Control Plane** (`stiglab-server`): Rust + Axum HTTP/WS server handling task dispatch, session lifecycle, node routing, and state persistence.
- **Node Agent** (`stiglab-agent`): Rust binary running on each machine, managing agent subprocesses and reporting state to the control plane via WebSocket.
- **Dashboard** (`stiglab-ui`): React + shadcn/ui frontend for monitoring nodes, sessions, and streaming logs.
- **Core** (`stiglab-core`): Shared types, state machine, protocol definitions, and error handling.

## Key Design Insight

AI coding agent sessions have a **WAITING_INPUT** state that traditional task runners lack. The system supports bidirectional communication, not just one-way log streaming.

## Session State Machine

```
PENDING -> DISPATCHED -> RUNNING <-> WAITING_INPUT
                          |
                     +----+----+
                     v         v
                   DONE      FAILED
```
