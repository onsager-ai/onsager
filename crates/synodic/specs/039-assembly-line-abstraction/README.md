---
status: archived
created: 2026-03-11
priority: high
tags:
- factory
- assembly-line
- station
- pipeline
- conveyor
- backpressure
parent: 037-coding-factory-vision
depends_on:
- 038-factory-mvp-first-car
- 005-agent-message-bus-task-orchestration
created_at: 2026-03-11T00:00:00Z
updated_at: 2026-03-11T00:00:00Z
---

# Assembly Line Abstraction — Station, Line & Conveyor

## Overview

Coordination primitives (swarm, mesh, adversarial) are defined but there's no "Station" or "Pipeline" as first-class concepts. This is like having designed power tools and machines but not the conveyor belt system that connects them into a production line.

This spec introduces three core abstractions that upgrade the MVP's ad-hoc pipeline (spec 038) into a proper assembly line:

- **Station** — a processing unit with typed input, typed output, quality gate, staffing policy, and SLA
- **Line** — an ordered sequence of stations with routing rules (pass, rework, reject)
- **Conveyor** — the transport mechanism that moves work items between stations with WIP limits and back-pressure

These abstractions are the factory's equivalent of Ford's moving assembly line.

## Design

### Station

A Station is the atomic unit of the production line. Each station defines:

```yaml
station:
  id: build
  name: "BUILD — Blueprint → Code"
  input_type: blueprint         # typed artifact accepted
  output_type: implementation   # typed artifact produced
  quality_gate:
    required_checks:
      - tests_pass
      - lint_clean
      - coverage_threshold
    threshold: all              # all | majority | any
  staffing:
    agent_types: [implementer]  # which agent roles can work here
    min_agents: 1
    max_agents: 4               # horizontal scaling
    model_tiers: [frontier, mid]  # Nemosis-compatible tier preferences
  sla:
    target_duration: 300s       # 5 minutes per work item
    max_duration: 900s          # hard timeout
  coordination:
    primitive: hierarchical     # which coordination primitive governs this station
    config: {}                  # primitive-specific configuration
```

Stations are composable: any Station can be plugged into any Line as long as its input type matches the upstream station's output type.

### Line

A Line is an ordered sequence of stations with routing logic:

```yaml
line:
  id: coding-factory
  name: "Software Production Line"
  stations: [intake, design, build, inspect, harden, deploy, maintain]
  routing:
    pass: next_station          # on quality gate pass, move forward
    rework:
      target: previous_station  # default: send back one station
      max_cycles: 3             # cap rework before escalation
    reject:
      target: intake            # rejected work goes back to intake for re-scoping
    escalate:
      target: human_queue       # unresolvable items escalate to human
  wip_limits:
    global: 20                  # max work items across entire line
    per_station:
      intake: 10
      design: 5
      build: 5
      inspect: 3
      harden: 2
      deploy: 1
      maintain: 5
```

Lines support multiple configurations for different work types (e.g., bug-fix line skips DESIGN, hotfix line skips HARDEN).

### Conveyor

The Conveyor handles movement and flow control:

- **Pull-based:** Each station pulls the next work item when it has capacity (not push-based). This naturally prevents overloading.
- **WIP enforcement:** The conveyor refuses to deliver more work to a station at its WIP limit.
- **Back-pressure propagation:** When a downstream station is full, upstream stations' pull requests are queued. This automatically throttles the whole line.
- **Priority queuing:** Work items carry priority levels; high-priority items skip to the front of inter-station queues.
- **Rework routing:** When a station fails quality gate, the conveyor routes the work item to the appropriate upstream station with the failure context attached.
- **Dead letter:** Work items that exceed max rework cycles go to a dead letter queue for human review.

### Work Item Lifecycle

```
QUEUED → IN_PROGRESS → COMPLETED → QUEUED (next station)
                     → REWORK → QUEUED (previous station)
                     → REJECTED → QUEUED (intake or dead letter)
                     → ESCALATED → human queue
```

Each transition is recorded in the work item's history for traceability.

### Type System for Artifacts

Stations communicate through typed artifacts. The type system ensures stations only receive artifacts they understand:

| Artifact Type | Produced By | Consumed By |
|---|---|---|
| `requirement` | External / MAINTAIN | INTAKE |
| `spec` | INTAKE | DESIGN |
| `blueprint` | DESIGN | BUILD |
| `implementation` | BUILD | INSPECT |
| `reviewed_code` | INSPECT | HARDEN |
| `hardened_code` | HARDEN | DEPLOY |
| `deployment` | DEPLOY | MAINTAIN |
| `maintenance_task` | MAINTAIN | INTAKE (loop) |

## Plan

- [ ] Define Station trait/interface in Rust with input type, output type, quality gate, staffing, and SLA
- [ ] Define Line configuration schema (YAML) with station ordering and routing rules
- [ ] Implement Conveyor with pull-based work item delivery and WIP limit enforcement
- [ ] Implement back-pressure propagation: upstream stations block when downstream is full
- [ ] Implement rework routing with attempt tracking and max cycle enforcement
- [ ] Implement dead letter queue for work items that exceed rework limits
- [ ] Define artifact type system with compile-time station compatibility checks
- [ ] Implement priority queuing in inter-station queues
- [ ] Migrate spec 038 MVP's file-based pipeline to Station/Line/Conveyor abstractions
- [ ] Support line variants (e.g., bug-fix line, hotfix line) via station subset configuration

## Test

- [ ] A 3-station line (BUILD → INSPECT → DEPLOY) routes a work item correctly through all stations
- [ ] WIP limit of 1 on INSPECT causes BUILD to block when INSPECT is occupied
- [ ] Back-pressure propagates: filling INSPECT blocks BUILD which blocks DESIGN
- [ ] Rework routing sends failed INSPECT items back to BUILD with review context attached
- [ ] Dead letter queue catches work items after 3 failed rework cycles
- [ ] Priority work items skip to front of inter-station queues
- [ ] Station type mismatch (e.g., connecting BUILD output to INTAKE input) is caught at config time
- [ ] Line variant with 2 stations (BUILD → INSPECT) works alongside full 7-station line

## Notes

The Station abstraction is designed to be coordination-primitive-agnostic. A station can use any primitive (hierarchical, swarm, adversarial) internally. The Line cares only about typed inputs/outputs and quality gate pass/fail.

The Conveyor will initially be backed by the message bus (spec 005). The pull-based model maps naturally to message bus consumption patterns.