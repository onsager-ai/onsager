---
status: complete
created: 2026-03-22
priority: critical
tags:
- testing
- harness
- assessment
- lean-spec
created_at: 2026-03-22T11:28:00.496848906Z
updated_at: 2026-03-22T11:28:00.496848906Z
---

# Harness Real-World Assessment: lean-spec Project

## Overview

Honest assessment of whether synodic's harness governance methodology works on a real external project (codervisor/lean-spec — polyglot Rust+TypeScript monorepo, 5 Rust crates, 74+ Rust tests, Vitest, ESLint, Clippy).

**Bottom line: The methodology is sound but the implementation has critical gaps that make it unusable on real projects today.**

## Test Methodology

- Target: lean-spec @ commit c55cf343 (main branch)
- Agent: `claude` (real Claude CLI, not a mock script)
- Task: Add `validate_spec_name()` function + tests to leanspec-core
- Static gate: Real tooling — cargo fmt, cargo clippy -D warnings, cargo test
- L2 judge: claude --print (AI judge subprocess)
- Max rework: 2 cycles

## Results

| Metric | Value |
|--------|-------|
| Outcome | ESCALATED (false negative) |
| Duration | 1115s (~18 minutes) |
| Attempts | 3 (1 initial + 2 rework) |
| Agent code quality | Good — correct function, 7 tests, all passing |
| L1 gate result | FAIL (preexisting test failures, not agent's fault) |
| L2 judge result | Could not run (subprocess killed in nested Claude env) |
| Actionable feedback to agent | Zero — empty feedback on all 3 attempts |
| Wasted compute | ~18 min of agent time with no useful rework signal |

## Critical Findings

### F1: Feedback loop is completely broken

The harness rejected correct agent code 3 times but provided ZERO actionable feedback. The rework feedback file contained only:

```
## Layer 1 (Static Rules) Failures
Fix these issues before your changes can be accepted.
```

**Root cause chain:**
1. Static gate outputs mixed text + JSON to stdout (log lines before JSON)
2. Harness parses ALL stdout as JSON (`serde_json::from_str(&gate_out)`) — fails silently
3. Even if JSON parsed, harness looks for `"failures"` key but no contract specifies this
4. Result: `l1_failures: []` — empty array, zero feedback to agent
5. Agent blindly retries with no idea what to fix

**Impact:** Rework loop is theater. Agent wastes 3 cycles doing the same thing.

### F2: No baseline test isolation

The static gate runs ALL tests, not just tests affected by the agent's changes. Lean-spec has 3 preexisting failing worktree session tests (environment-dependent). The agent's 7 new tests all pass. But the gate sees `cargo test` exit code 1 and rejects.

**The harness cannot distinguish "agent broke tests" from "tests were already broken."**

**Fix needed:** Run tests at base_ref first, record failures, then only fail on NEW test failures introduced by the agent's diff.

### F3: Static gate has no execution contract

The static gate script interface is undocumented:
- No specified stdout format (text? JSON? what keys?)
- No specified environment variables passed
- No `current_dir` set (line 132: `Command::new(&static_gate)` with no `.current_dir()`)
- No timeout
- Args are `[base_ref, "HEAD"]` but this is implicit — not documented anywhere

### F4: L2 judge verdict parsing is inverted safety

When the judge subprocess fails, times out, or produces unparseable output, the harness **auto-approves** (run.rs:378):

```rust
"could not parse verdict. Accepting by default."
```

This means: broken judge = everything passes governance. The safety gate fails open.

### F5: L2 judge cannot run in nested Claude environments

Running `claude --print -` as judge subprocess inside an existing Claude session causes the subprocess to be killed (OOM/resource limits). The harness has no fallback, no timeout detection, and no error handling for this case — it just auto-approves per F4.

### F6: stdin feedback delivery is unreliable

Rework feedback is written to agent's stdin (run.rs:544-546) but:
- Write errors are silently discarded (`let _ = stdin.write_all(...)`)
- stdin is never closed before `child.wait()` — agent may hang waiting for EOF
- Agent's original task context is not re-provided — only raw feedback text
- No mechanism to verify agent actually received and processed feedback

### F7: No agent timeout

`Command::new(&cmd[0]).status()` (run.rs:517) has no timeout. A hanging agent blocks the harness forever. No kill signal, no cleanup.

### F8: Diff observation includes harness artifacts

The diff between base and HEAD includes `.harness/.runs/` artifacts from previous attempts. By attempt 3, the diff was 915 insertions — most of it harness-generated patch files and JSON reports, not agent code. This pollutes both L1 and L2 review.

## What Works

- **Core governance loop structure** — attempt → observe → gate → feedback → retry is the right pattern
- **Governance log persistence** — JSONL append-only log recorded correctly
- **Run manifest** — per-run artifacts (.runs/{id}/) with diffs, reports, manifests
- **Diff observation** — git diff stat display is clear and useful
- **Agent invocation** — workdir, env vars, stdout/stderr capture all work
- **Exit code semantics** — 0/1/2 (passed/error/escalated) are correct and useful
- **Dry-run mode** — works perfectly for planning

## Verdict

The harness methodology (observe → L1 static → L2 AI → rework loop) is architecturally sound. But the implementation has **5 critical bugs** that make it produce wrong results on real projects:

1. Feedback is empty (F1) — rework is pointless
2. No test baseline (F2) — preexisting failures block correct code  
3. Judge fails open (F4) — broken safety = no safety
4. No timeouts (F7) — can hang forever
5. Artifacts in diff (F8) — review sees garbage

**These are not edge cases. They fire on the first real-world test.**

## Plan

- [x] Run harness against real external project (lean-spec)
- [x] Test L1 static gate with real tooling (cargo fmt/clippy/test)
- [x] Test L2 AI judge with real code diffs
- [x] Test rework loop with real agent (claude)
- [x] Identify implementation gaps blocking real-world use

## Notes

- Agent produced correct code on attempt 1 — the function and all 7 tests pass
- The agent also placed code in two locations (types/validation.rs AND validators/mod.rs) — L2 judge should catch this duplication but never got the chance
- Lean-spec's own CI (ci.yml) runs tests with `--test-threads=1` — the harness static gate should match this
- The 3 failing worktree tests in lean-spec are environment-dependent (need specific git config) — common in real projects
