---
status: archived
created: 2026-03-11
priority: critical
tags:
- factory
- mvp
- build
- inspect
- pipeline
parent: 037-coding-factory-vision
depends_on:
- 004-fleet-process-supervisor
- 005-agent-message-bus-task-orchestration
created_at: 2026-03-11T00:00:00Z
updated_at: 2026-03-11T00:00:00Z
---

# Factory MVP — First Car Off the Line

## Overview

36 specs, 0 lines of shipped code. This is the critical gap. A factory that produces one car proves the concept — a factory with no cars is a museum of unbuilt machines.

This spec delivers the minimum viable production line: a two-station pipeline (BUILD → INSPECT) that takes a spec as input and produces a reviewed PR as output, with zero human intervention between stations. One agent builds, one agent reviews, and the work flows forward automatically.

The goal is not completeness — it's proof of flow. If two agents can coordinate to ship one PR with measurable cycle time and pass/fail outcome, we have a factory. Everything else is scaling.

## Design

### Two-Station Pipeline

```
                ┌─────────────┐         ┌─────────────┐
  LeanSpec ───→ │  STATION 3  │ ──PR──→ │  STATION 4  │ ───→ Approved PR
   (input)      │    BUILD    │         │   INSPECT   │      (or rework)
                │             │         │             │
                │ Claude Code │         │ Claude Code │
                │ Implementer │         │  Reviewer   │
                └─────────────┘         └─────────────┘
                       ↑                       │
                       └───── rework ──────────┘
```

### Station 3 — BUILD

- **Agent:** Claude Code in implementation mode
- **Input:** LeanSpec README.md (the spec to implement)
- **Process:**
  1. Read the spec's Plan section for implementation steps
  2. Implement each step (code changes, new files)
  3. Run tests from the spec's Test section
  4. Create a git branch and commit
  5. Produce a structured build report (files changed, tests passed/failed, open questions)
- **Output:** Git branch with implementation + build report artifact
- **Quality gate:** All spec-defined tests pass; no syntax errors; code compiles

### Station 4 — INSPECT

- **Agent:** Claude Code in review mode
- **Input:** Build report + git diff from Station 3
- **Process:**
  1. Review against the original spec's acceptance criteria
  2. Check correctness, security, style, and completeness
  3. Produce a review report with pass/fail per dimension
  4. If pass: mark approved
  5. If fail: produce specific rework items and route back to Station 3
- **Output:** Approved PR or rework items sent back to BUILD
- **Quality gate:** All review dimensions pass; no blocker findings
- **Rework limit:** Maximum 3 rework cycles before escalation to human

### Conveyor (Minimal)

The conveyor is a simple file-based protocol for the MVP:

1. **Work item:** A JSON manifest describing the spec path, current station, attempt count, and artifact paths
2. **Handoff:** BUILD writes its artifacts to a staging directory; the conveyor moves the work item to INSPECT
3. **Rework:** INSPECT writes rework items; the conveyor routes the work item back to BUILD with the review feedback attached
4. **Completion:** INSPECT marks the work item as approved; the conveyor creates the PR

```json
{
  "id": "work-001",
  "spec": "specs/004-fleet-process-supervisor",
  "station": "build",
  "attempt": 1,
  "artifacts": {
    "branch": "factory/work-001",
    "build_report": ".factory/work-001/build-report.json",
    "review_report": null
  },
  "history": []
}
```

### CLI Interface

```bash
# Run a spec through the two-station line
synodic run <spec-path>

# Example
synodic run specs/004-fleet-process-supervisor

# Watch progress
synodic status <work-id>
```

### Metrics (Minimal)

Even the MVP measures:
- **Cycle time:** Wall-clock seconds from `synodic run` to approved PR (or escalation)
- **First-pass yield:** Did INSPECT approve on the first attempt? (boolean)
- **Rework count:** How many BUILD↔INSPECT loops before approval
- **Token cost:** Total tokens consumed across both stations

## Plan

- [ ] Define work item manifest schema (JSON) and artifact directory structure (`.factory/`)
- [ ] Implement BUILD station: read spec, spawn Claude Code agent, produce implementation + build report
- [ ] Implement INSPECT station: read build output + spec, spawn Claude Code agent, produce review report
- [ ] Implement minimal conveyor: route work items between BUILD and INSPECT, handle rework loops
- [ ] Implement `synodic run <spec>` CLI command that orchestrates the full pipeline
- [ ] Implement `synodic status <work-id>` to show progress and metrics
- [ ] Add cycle time, first-pass yield, rework count, and token cost tracking
- [ ] Cap rework loops at 3 with human escalation

## Test

- [ ] `synodic run` on a trivial spec (e.g., "add a hello-world function") produces a git branch with implementation
- [ ] INSPECT station catches a deliberate bug injected by BUILD; rework loop fires and BUILD fixes it
- [ ] Rework limit of 3 is enforced — after 3 failed cycles, escalation occurs
- [ ] Cycle time and token cost are recorded and reported on completion
- [ ] Work item manifest tracks full history of station transitions
- [ ] Two concurrent `synodic run` invocations don't interfere with each other

## Notes

This is Phase 0 from the spec 037 roadmap. The conveyor is intentionally simple (file-based) — it will be replaced by the message bus (spec 005) and the assembly line abstraction (spec 039) in later phases. The point is to ship something that works, then improve it.

The BUILD and INSPECT stations will initially be the same Claude Code model in different prompt configurations. Heterogeneous agent support comes in Phase 1.