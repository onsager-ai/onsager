---
status: in-progress
created: 2026-03-18
priority: high
tags:
- harness
- governance
- refactor
created_at: 2026-03-18T23:27:23.203961706Z
updated_at: 2026-03-18T23:27:23.203961706Z
---
# Refactor Governance to Post-Session Review Model

## Overview

Current governance is **intrusive** — `harness run` wraps agent execution in a synchronous rework loop, injecting feedback via stdin and blocking until the loop terminates (pass/escalate). This couples governance tightly to agent execution.

**New model:** Governance is **post-session**. After a session completes, `harness review` analyzes the output (git diff) and generates governance logs. No agent wrapping, no rework loops, no feedback injection.

**Why now:** The rework loop adds complexity without proportional value. Agents already self-correct during sessions. Governance should observe and report, not control execution flow.

## Design

### Remove intrusive governance loop
- Delete agent-wrapping code (`run_agent`, `run_agent_with_stdin`, rework loop)
- Remove `max_rework`, `agent_cmd` from config
- Remove `RunConfig` → replace with `ReviewConfig`

### New `harness review` subcommand
- Takes `--base-ref` (optional, auto-resolves via merge-base with main)
- Takes `--workdir`, `--no-l2`, `--judge`, `--dry-run`, `--quiet`, `--json`
- Analyzes git diff between base-ref and HEAD
- Runs Layer 1 (static gate + crystallized rules) on the diff
- Runs Layer 2 (AI judge) if enabled
- Generates governance log entry (`.harness/harness.governance.jsonl`)
- Exit code: 0 = clean, 1 = issues found

### Governance log schema change
- `source` field: `"review"` instead of `"harness"`
- `status`: `"clean"` or `"issues"` (no more `"escalated"` — no rework to exhaust)
- `issues` field replaces `rework_items` (same structure, clearer name)
- Remove `agent_command` and `attempt_count` from record

### HARNESS.md updates
- §2: Remove rework loop description, reframe as post-session analysis
- §4: Simplify checkpoint placement — post-session review replaces inline checkpoints
- §6: Manifest is per-review, not per-rework-attempt
- §7: Crystallization still works — governance logs feed the same pipeline

## Plan

- [ ] Rewrite `cli/src/harness/run.rs` — `ReviewConfig` + `execute()` as single-pass review
- [ ] Update `cli/src/cmd/harness.rs` — replace `Run` subcommand with `Review`
- [x] Update `HARNESS.md` — reflect non-intrusive model
- [ ] Build and run tests

## Test

- [ ] `cargo build` succeeds
- [ ] `cargo test` passes (29 tests)
- [ ] `synodic harness review --dry-run` shows expected plan without executing
- [ ] `synodic harness review --base-ref HEAD~1` analyzes last commit
- [ ] Governance log entry written with new schema

## Notes

- Layer 1 and Layer 2 evaluation logic is preserved — only the wrapping/looping is removed
- `harness log` and `harness rules` subcommands unchanged
- `harness eval` (evaluate_harness.py) unchanged — it already analyzes logs post-hoc
- Skills (factory, fractal) may still have their own inline governance — this spec only covers the `harness` CLI command