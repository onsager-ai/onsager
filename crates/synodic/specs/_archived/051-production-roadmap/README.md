---
status: in-progress
created: 2026-03-19
priority: critical
tags:
- roadmap
- factory
- harness
- orchestration
- strategy
parent: 037-coding-factory-vision
created_at: 2026-03-19T05:26:12.586891939Z
updated_at: 2026-03-19T05:26:12.586891939Z
---

# Synodic Production Roadmap — Close Loop, Harden, Scale

## Overview

Three-phase execution roadmap to turn the Synodic coding factory from a designed system into a running, measured, self-improving one.

**North star:** A working feedback loop where agents build code, governance observes the output, findings crystallize into rules, and rules improve the next run.

**Current state (2026-03-19):**
- Factory skill (`/factory run`) is built but has never been run end-to-end
- Governance model is designed (HARNESS.md v0.2.0) but `harness review` is not yet implemented
- Fractal algorithmic spine is built but unvalidated in production
- No production metrics collected yet

## Phase 1 — Close the Loop

**Goal:** One complete cycle: spec → factory run → governance review → log entry.

| Step | Work | Spec |
|------|------|------|
| 1 | Run `/factory run` on a trivial spec — first PR produced | 044 |
| 2 | Implement `synodic harness review` (rewrite `harness/run.rs` → `ReviewConfig`) | 048 |
| 3 | Run factory + `harness review` together → first governance log entry | — |
| 4 | Verify the loop: factory output → review → JSONL entry in `.harness/` | — |

**Exit criteria:** A governance log entry exists that was produced from a real factory run.

## Phase 2 — Harden and Measure

**Goal:** Reliable, repeatable runs with visible quality metrics.

| Step | Work | Spec |
|------|------|------|
| 5 | Factory test harness: 3 fixtures, `run-factory-tests.sh` | 049 |
| 6 | First-pass yield, cycle time, rework count visible per run | 044 metrics |
| 7 | First rule crystallization from accumulated governance logs | HARNESS.md §7 |

**Exit criteria:** `./tests/run-factory-tests.sh` passes all 3 fixtures; metrics table printed.

## Phase 3 — Scale Patterns

**Goal:** Prove two orchestration patterns work end-to-end and compose.

| Step | Work | Spec |
|------|------|------|
| 8 | Fractal + factory composition: decompose a large spec, run factory on leaves | 051 |
| 9 | Parallel factory runs: batch mode across multiple specs | — |
| 10 | Measure and compare: fractal+factory vs. factory-direct on same problem | — |

**Exit criteria:** A non-trivial spec (≥3 sub-problems) is implemented via fractal decomposition with factory at the leaves, producing a PR.

## Plan

- [x] Renumber and fix spec housekeeping (044 dupe, 047 dupe, 045 frontmatter)
- [ ] Run `/factory run` on a trivial spec (Phase 1, Step 1)
- [ ] Implement `synodic harness review` (spec 048)
- [ ] Close the feedback loop end-to-end (Phase 1, Steps 3–4)
- [ ] Build factory test harness (spec 049)
- [ ] Collect first metrics batch (Phase 2)
- [ ] Fractal + factory composition (spec 051)

## Test

- [ ] Governance log contains at least one entry from a real factory run
- [ ] `run-factory-tests.sh` passes all 3 fixtures
- [ ] A non-trivial spec implemented via fractal+factory produces a merged PR

## Notes

- Phase 1 is the critical path — nothing in Phase 2 or 3 is meaningful without a working feedback loop
- The Rust CLI (`synodic`) role: operator tool for governance reviews and benchmarks; not a runtime dependency of skills
- Factory and fractal are the product; the CLI is the measurement and governance layer
