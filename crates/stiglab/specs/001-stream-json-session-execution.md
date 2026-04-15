# 001 — Stream-JSON Session Execution

## Status

`implemented`

## Problem

`SessionProcess` currently spawns agent sessions with:

```
claude --print <prompt>
```

This is fundamentally wrong for an orchestration platform:

- **No streaming** — output arrives all at once when the process exits
- **No tool use** — `--print` runs Claude in single-turn print mode, bypassing the agentic loop
- **No multi-turn** — one prompt in, one response out, then done
- **Naive WaitingInput detection** — pattern-matching on `"? "` / `"[Y/n]"` in stdout will never trigger reliably against Claude's actual output

The result is a system that can dispatch tasks and track session state, but cannot actually run real agent sessions.

## Decision

Stiglab's `SessionProcess` must run Claude Code the same way Telegramable's `CliRuntime` does.

**Telegramable is the reference implementation.** It has solved:
- Streaming NDJSON output parsing
- Tool-use event handling
- Activity-based timeout
- Multi-turn interaction
- Permission request interception

Stiglab is the scalability layer. Telegramable is the communication layer. Both should run Claude Code sessions identically — Stiglab just runs more of them, distributed across nodes.

## Target CLI Invocation

```
claude \
  --output-format stream-json \
  --verbose \
  --include-partial-messages \
  --permission-mode bypassPermissions \
  [--model <model>] \
  [--max-turns <n>] \
  [--system-prompt <prompt>] \
  -- <user_prompt>
```

Key flags:
- `--output-format stream-json` — emit NDJSON events line by line (streaming)
- `--verbose` — required for stream-json to emit tool-use and lifecycle events
- `--include-partial-messages` — emit text delta events for real-time token streaming
- `--permission-mode bypassPermissions` — non-interactive; no prompts during tool use

## NDJSON Event Model

Each line of stdout is a JSON object. Relevant event types:

| Event type | Field | Meaning |
|---|---|---|
| `stream_event` | `event.type = "content_block_delta"`, `delta.type = "text_delta"` | Streaming text token |
| `stream_event` | `event.type = "content_block_start"`, `content_block.type = "tool_use"` | Tool call started |
| `stream_event` | `event.type = "content_block_delta"`, `delta.type = "input_json_delta"` | Tool input accumulating |
| `stream_event` | `event.type = "content_block_stop"` | Tool call input complete |
| `system` | `subtype = "session_id"` | Claude session ID assigned |
| `result` | `subtype = "success"` | Session completed successfully |
| `result` | `subtype = "error_during_execution"` | Session failed |

## Changes Required

### `crates/stiglab-agent/src/session/process.rs`

Rewrite `SessionProcess::spawn()`:

1. Build CLI args with `--output-format stream-json --verbose --include-partial-messages --permission-mode bypassPermissions`
2. Parse stdout line-by-line as NDJSON
3. On `text_delta` events → emit `AgentMessage::SessionOutput` (streaming chunk)
4. On `tool_use` start → emit `AgentMessage::SessionOutput` with tool activity description
5. On `result` event:
   - `success` → emit `AgentMessage::SessionCompleted`
   - `error_during_execution` → emit `AgentMessage::SessionFailed`
6. Remove naive `WaitingInput` pattern matching — replace with proper `system` event detection if needed

### `stiglab-core` — `Task` struct

Add optional execution config fields:

```rust
pub struct Task {
    // existing fields ...
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub system_prompt: Option<String>,
    pub permission_mode: Option<String>, // defaults to "bypassPermissions"
}
```

### `POST /api/tasks` — `TaskRequest`

Expose the new fields so callers can configure per-task execution:

```json
{
  "prompt": "Add error handling to the auth module",
  "working_dir": "/workspace/my-project",
  "model": "claude-opus-4-5",
  "max_turns": 20,
  "system_prompt": "You are working in a TypeScript monorepo."
}
```

## Working Directory

`task.working_dir` must be set for sessions to have project context. When `None`, the agent runs with no codebase context — Claude will have nothing to work with.

Callers (Telegramable, CI pipelines, etc.) are responsible for setting this to a meaningful path on the agent node.

## Out of Scope

- Permission relay (interactive approval of tool calls across the network) — deferred
- Multi-turn input injection (`SendInput`) — deferred until `WaitingInput` is properly detected
- MCP server configuration — deferred

## Acceptance Criteria

- [ ] `POST /api/tasks` with `{"prompt": "say hello", "working_dir": "/tmp"}` creates a session
- [ ] Session transitions: `Pending → Dispatched → Running → Done`
- [ ] `GET /api/sessions/{id}/logs` SSE stream delivers streaming text chunks in real time
- [ ] `GET /api/sessions/{id}` returns aggregated output after completion
- [ ] Tool-use events appear in the log stream (e.g. `[tool] Read: src/main.rs`)
- [ ] Session fails cleanly with error message if `claude` is not in PATH
