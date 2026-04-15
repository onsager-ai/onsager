---
status: complete
created: 2026-03-18
priority: high
tags:
- architecture
- eval
- refactor
created_at: 2026-03-18T22:30:33.987624411Z
updated_at: 2026-03-19T05:25:29.598287448Z
completed_at: 2026-03-19T05:25:29.598287448Z
transitions:
- status: in-progress
  at: 2026-03-18T23:37:22.070246603Z
- status: complete
  at: 2026-03-19T05:25:29.598287448Z
---

# Decouple Eval as Standalone Testing Framework

## Overview

The eval framework (setup → agent → score pipeline) is a general-purpose AI coding evaluation tool that should work independently of synodic's governance harness. Today eval directly writes to `.harness/eval.governance.jsonl` and reads `SYNODIC_ROOT` — it has no business knowing about governance at all.

**Why now:** Eval is mature enough (29 tests, 3 benchmarks, batch mode) to stand alone. Complete separation enables:
- Eval as a zero-dependency testing framework — no governance concepts leak in
- Independent versioning and release cycles
- Use by external teams without adopting synodic governance
- Synodic consumes eval output (JSON on stdout/files) and writes its own governance logs

## Design

### Separation principle: eval knows nothing about governance

Eval produces **structured output** (JSON verdict + score reports). Period. It does not:
- Write to `.harness/` or any governance directory
- Read `SYNODIC_ROOT` or any synodic-specific env var
- Reference HARNESS.md, cross-run learning, or governance concepts
- Know it's being orchestrated by anything

Synodic's harness is the **consumer** — it invokes eval, reads its stdout/exit code, and writes governance logs itself.

### Current coupling to remove

| What | Where | Action |
|------|-------|--------|
| `append_governance_log()` | eval/run.rs:486-534 | **Delete entirely** — eval doesn't write gov logs |
| `extract_findings()` | eval/run.rs (helper for gov log) | **Delete** — governance categorization is harness's job |
| `.harness/` directory creation | eval/run.rs:494-497 | **Delete** — eval never touches .harness/ |
| `SYNODIC_ROOT` env var read | util.rs:9-13 | **Remove from eval** — eval uses its own project root discovery |
| Gov log comments | eval/run.rs:321 referencing "HARNESS.md §6-7" | **Remove** — no harness references in eval |
| `find_repo_root()` looking for `.harness/` | util.rs | **Split** — eval version looks for `evals/` or `.git`, not `.harness/` |

### Target architecture: Cargo workspace

```
cli/
├── Cargo.toml                  # [workspace] members = ["synodic", "synodic-eval"]
├── synodic/                    # Governance binary
│   ├── Cargo.toml              # depends on synodic-eval as library
│   └── src/
│       ├── main.rs             # Cli { Harness, Eval }
│       ├── cmd/harness.rs
│       ├── harness/
│       │   ├── run.rs          # Invokes eval, reads output, writes gov logs
│       │   ├── log.rs          # Reads .harness/*.governance.jsonl
│       │   └── rules.rs
│       ├── governance.rs       # NEW: writes eval results → .harness/eval.governance.jsonl
│       └── util.rs             # find_repo_root() — looks for .harness/
├── synodic-eval/               # Standalone eval framework
│   ├── Cargo.toml              # Zero synodic dependencies
│   └── src/
│       ├── lib.rs              # Public API: run(), score(), list(), batch()
│       ├── main.rs             # Binary: `synodic-eval run|score|list|batch|report`
│       ├── run.rs              # Orchestrate: setup → agent → score → JSON output
│       ├── batch.rs
│       ├── list.rs
│       ├── report.rs
│       ├── score/              # parser, runner, verdict, report — unchanged
│       ├── setup/              # swebench, featurebench, devbench — unchanged
│       └── util.rs             # find_project_root() — looks for evals/ or .git
```

### Eval output contract

Eval communicates results through two channels only:

**1. Exit code** — `0` = resolved, `1` = not resolved, `2` = error
**2. Structured JSON output** — written to `--output <path>` or stdout:

```json
{
  "instance_id": "django__django-16379",
  "benchmark": "swebench",
  "skill": "factory",
  "resolved": true,
  "duration_s": 142,
  "f2p": { "group": "FAIL_TO_PASS", "expected": 3, "passed": 3 },
  "p2p": { "group": "PASS_TO_PASS", "expected": 47, "passed": 47 },
  "score_report": "path/to/score_report.json"
}
```

**Harness side (governance.rs — NEW file in synodic crate):**
```rust
// synodic reads eval's JSON output and writes governance log
fn record_eval_result(harness_dir: &Path, eval_output: &EvalOutput) {
    let record = json!({
        "work_id": format!("eval-{}-{}-{}", eval_output.instance_id, eval_output.skill, timestamp),
        "source": "eval",
        "timestamp": Utc::now().to_rfc3339(),
        "status": if eval_output.resolved { "resolved" } else { categorize(&eval_output) },
        "findings": extract_findings(&eval_output),
        // ... same schema as today, but owned by harness
    });
    append_jsonl(harness_dir.join("eval.governance.jsonl"), &record);
}
```

### Project root discovery

Eval needs to find the project root (for `evals/evals.json`). Currently it piggybacks on `find_repo_root()` which looks for `.harness/`. After decoupling:

- **synodic-eval**: `find_project_root()` walks up looking for `evals/evals.json` or `.git`. Overridable via `EVAL_ROOT` env var.
- **synodic**: `find_repo_root()` walks up looking for `.harness/` or `.git`. Sets `EVAL_ROOT` when spawning eval subprocess.

## Plan

- [x] Create Cargo workspace with `synodic` and `synodic-eval` member crates
- [x] Move eval modules to synodic-eval crate (run, batch, list, report, score/, setup/)
- [x] Delete `append_governance_log()` and `extract_findings()` from eval/run.rs entirely
- [x] Remove all `.harness/` references from eval code
- [x] Remove `SYNODIC_ROOT` reads from eval; add `EVAL_ROOT` env var for project root
- [x] Create `synodic/src/governance.rs` — reads eval JSON output, writes gov JSONL
- [x] Update harness/run.rs to capture eval stdout, pass to governance.rs
- [x] Add `--output <path>` flag to eval for structured JSON output
- [x] Ensure eval works standalone: no .harness/ needed, no synodic env vars
- [x] Verify all 29 existing tests pass in synodic-eval crate
- [x] Update CLAUDE.md to reflect two-crate architecture

## Test

- [x] `cd cli/synodic-eval && cargo test` — all 29 tests pass standalone
- [x] `cd cli/synodic-eval && cargo build` — standalone binary, no synodic deps
- [x] `synodic-eval run` works in a directory with no `.harness/`
- [x] `synodic eval run` still works (synodic invokes eval, writes gov log itself)
- [x] Governance log schema unchanged (harness consumers unaffected)
- [x] `grep -r "harness\|governance\|SYNODIC" synodic-eval/src/` returns zero matches

## Notes

**What moves where:**
- `append_governance_log()` → deleted from eval, rewritten in `synodic/src/governance.rs`
- `extract_findings()` → deleted from eval, rewritten in governance.rs
- `find_repo_root()` → split: eval gets `find_project_root()` (no .harness), harness keeps original
- `SYNODIC_ROOT` → harness-only; eval uses `EVAL_ROOT`

**Alternatives considered:**
- **EvalReporter trait (previous design):** Still couples eval to the concept of "reporting to something". Rejected — eval should just produce output, not call callbacks.
- **Separate repo:** Too aggressive. Workspace keeps co-development while allowing independent builds.