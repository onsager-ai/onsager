---
status: complete
created: 2026-03-18
priority: high
tags:
- dogfood
- eval
- synodic
- benchmark
- self
depends_on:
- 044-factory-skill-mvp
- 045-rust-consolidation
parent: 037-coding-factory-vision
created_at: 2026-03-18T11:00:00Z
updated_at: 2026-03-19T05:25:28.874692208Z
completed_at: 2026-03-19T05:25:28.874692208Z
transitions:
- status: complete
  at: 2026-03-19T05:25:28.874692208Z
---

# 046 — Synodic Dogfood: Self-Referential Eval

> **Status**: in-progress · **Priority**: high · **Created**: 2026-03-18

## Overview

The Synodic eval system currently supports three external benchmarks: SWE-bench,
FeatureBench, and DevBench. This spec adds a fourth: **synodic itself**.

"Dogfooding" means using Synodic's factory and fractal skills to implement
Synodic's own specs — evaluated by running `cargo test` in the Synodic CLI.
The self-referential loop is intentional: it validates both the skill quality
and the eval infrastructure simultaneously.

## Design

### New Benchmark Type: `synodic`

A synodic eval instance is:
- A Synodic spec (one of `specs/<nnn>-*/README.md`)
- A base commit in the Synodic repo (before the spec was implemented)
- A score command (`cargo test` in `cli/`)

The eval runner:
1. Clones `codervisor/synodic` at the base commit
2. Reads the spec from the *current* Synodic install (not the testbed)
3. Writes the spec as the agent's task
4. Agent implements the spec in the testbed repo
5. Scoring: `cargo test` — all tests pass = resolved

### Instance Alias

Synodic instances use the `syn:` prefix:
```
syn:dogfood-syn-support    → benchmark=synodic, instance=dogfood-syn-support
```

Instance metadata lives in `evals/tasks/synodic/<alias>.meta.json`:
```json
{
  "id": "synodic-dogfood-syn-support",
  "alias": "dogfood-syn-support",
  "repo": "codervisor/synodic",
  "base_commit": "<sha>",
  "spec_path": "specs/046-synodic-dogfood/README.md",
  "score_dir": "cli",
  "score_command": ["cargo", "test"]
}
```

### Directory Layout

```
evals/
├── setup/synodic.sh         # Testbed setup for synodic instances
├── score-synodic.sh         # Cargo-based scoring
├── tasks/synodic/           # Per-instance metadata
│   └── dogfood-syn-support.meta.json
└── evals.json               # Synodic entry added
```

### Rust Changes

`cli/src/eval/`:
- `setup/synodic.rs` — new setup module (replaces setup/synodic.sh)
- `setup/mod.rs` — dispatch for `synodic` benchmark
- `run.rs` — `syn:` prefix in `resolve_target`, synodic scoring in `execute`
- `score/verdict.rs` — `score_synodic()` using `cargo test`

### Scoring Logic

Unlike SWE-bench/FeatureBench (which use F2P/P2P pytest lists), synodic scoring
is simpler:
1. Run `cargo test` in `repo/cli/`
2. If exit code 0: all tests pass → `resolved = true`
3. If exit code non-zero: some tests fail → `resolved = false`

The score report uses the same JSON schema as other benchmarks:
```json
{
  "instance_id": "synodic-dogfood-syn-support",
  "resolved": true,
  "score": { "passed": 32, "failed": 0, "errors": 0, "skipped": 0 },
  "benchmark": "synodic"
}
```

## Plan

- [x] Create spec `specs/046-synodic-dogfood/README.md`
- [x] Create `evals/tasks/synodic/dogfood-syn-support.meta.json`
- [x] Create `evals/setup/synodic.sh`
- [x] Create `evals/score-synodic.sh`
- [x] Update `evals/run.sh` — add `syn:` prefix and synodic benchmark type
- [x] Update `evals/evals.json` — add synodic entry
- [x] Create `cli/src/eval/setup/synodic.rs`
- [x] Update `cli/src/eval/setup/mod.rs`
- [x] Update `cli/src/eval/run.rs` — `syn:` alias + synodic scoring
- [x] Update `cli/src/eval/score/verdict.rs` — `score_synodic()`
- [x] Add unit tests for `syn:` alias resolution

## Test

- [ ] `syn:dogfood-syn-support` resolves to `benchmark=synodic, instance=dogfood-syn-support`
- [ ] `./run.sh syn:dogfood-syn-support --dry-run` prints the agent prompt
- [ ] `synodic eval run syn:dogfood-syn-support --dry-run` prints the agent prompt
- [ ] `cargo test` passes all 29+ tests (including new alias resolution tests)
- [ ] Score report shows `resolved: true` when `cargo test` passes

## Notes

The initial dogfood instance (`dogfood-syn-support`) is self-referential: the
task is to implement this very feature (spec 046). When an agent solves it, the
`test_resolve_synodic_alias` tests pass, and `cargo test` exits 0.

This validates the entire dogfood pipeline end-to-end.
