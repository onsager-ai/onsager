---
status: archived
created: 2026-03-11
priority: high
tags:
- factory
- quality
- inspection
- rework
- defect-tracking
- andon
parent: 037-coding-factory-vision
depends_on:
- 039-assembly-line-abstraction
- 028-generative-adversarial-primitive
created_at: 2026-03-11T00:00:00Z
updated_at: 2026-03-11T00:00:00Z
---

# Factory Quality System — Gates, Rework Routing & Defect Tracking

## Overview

The generative-adversarial coordination primitive (spec 028) provides a single mechanism for generator/critic loops. But a factory needs a full quality department, not just one measuring tool. This spec builds the integrated quality system that wraps around the assembly line.

Quality in the factory model is not an afterthought bolted onto INSPECT — it's a cross-cutting system that touches every station. Every station has a quality gate. Every gate produces structured verdicts. Every defect is tracked, classified, and traced back to its origin. And any agent can pull the Andon cord to halt the line when it detects a systemic issue.

## Design

### Quality Gates

Every station (spec 039) has a quality gate, but this spec defines the gate framework itself:

```yaml
quality_gate:
  id: inspect-gate
  station: inspect
  checks:
    - id: correctness
      type: adversarial_review    # uses spec 028 generative-adversarial
      weight: critical            # critical | major | minor
      config:
        max_iterations: 3
        convergence_threshold: 0.9
    - id: security
      type: static_analysis
      weight: critical
      config:
        tools: [semgrep, bandit]
    - id: style
      type: lint
      weight: minor
      config:
        tools: [eslint, rustfmt]
    - id: coverage
      type: threshold
      weight: major
      config:
        minimum: 80
  verdict_policy:
    pass: all_critical_pass AND majority_major_pass
    rework: any_critical_fail
    reject: repeated_critical_fail AND attempt > 2
```

Gate checks run in parallel. Each produces a structured verdict:

```json
{
  "check_id": "correctness",
  "verdict": "fail",
  "weight": "critical",
  "findings": [
    {
      "id": "F-001",
      "severity": "P1",
      "category": "logic_error",
      "location": "src/pipeline.rs:142",
      "description": "Off-by-one in batch size calculation",
      "suggested_fix": "Change `< len` to `<= len`"
    }
  ]
}
```

### Rework Routing

When a quality gate fails, rework routing ensures defective items go back to the right station — not to the end of the queue:

- **Same-station rework:** Minor issues (style, formatting) are fixed in-place by the current station's agent
- **Previous-station rework:** Logic errors, missing functionality are routed back to BUILD
- **Multi-station rework:** Fundamental design issues route back to DESIGN with the full finding context
- **Targeted rework:** The rework item includes only the specific findings to address, not a general "redo everything"

Rework items carry a **rework context** — the original work, the review findings, and specific instructions for what to fix. This prevents agents from starting from scratch.

### Defect Tracking

Every quality gate finding is recorded in a defect ledger:

```json
{
  "defect_id": "D-2026-0142",
  "work_item_id": "work-001",
  "detected_at_station": "inspect",
  "origin_station": "build",         // root cause attribution
  "category": "logic_error",
  "severity": "P1",
  "detected_at": "2026-03-11T14:23:00Z",
  "resolved_at": "2026-03-11T14:28:00Z",
  "resolution": "fixed_in_rework",
  "rework_cycles": 1,
  "escaped": false                    // did it reach production?
}
```

Defect categories: `logic_error`, `security_vulnerability`, `performance_regression`, `missing_tests`, `style_violation`, `spec_mismatch`, `integration_failure`.

### Escape Analysis

When defects reach production (detected by MAINTAIN station), trace back to find which gate should have caught them:

1. **Gate gap:** No check existed for this defect category at any station
2. **Gate miss:** A check existed but failed to detect the defect (false negative)
3. **Gate override:** A check flagged the defect but it was overridden
4. **New category:** A defect category that wasn't anticipated in any gate configuration

Escape analysis feeds back into gate configuration: gate gaps trigger new check creation, gate misses trigger check tuning.

### Andon Cord

Borrowed from Toyota's production system (a refinement of Ford's model): any agent at any station can pull the Andon cord to halt the production line when it detects a systemic issue.

Triggers:
- **Defect rate spike:** More than N failures at a station within a time window
- **Cascade failure:** The same defect category appearing at multiple stations
- **Infrastructure issue:** Agent health degradation, API rate limits, resource exhaustion
- **Spec ambiguity:** Agent detects that the input spec is contradictory or untestable

When the Andon cord is pulled:
1. All in-progress work items at the affected station(s) are paused
2. New work items are held in the conveyor queue
3. A diagnostic report is generated (defect pattern, station metrics, recent history)
4. Human notification is sent with the diagnostic report
5. Line resumes only after explicit human approval or automated resolution

## Plan

- [ ] Define quality gate schema with check types, weights, and verdict policies
- [ ] Implement parallel gate check execution with structured verdict output
- [ ] Implement rework routing logic: same-station, previous-station, and multi-station rework
- [ ] Build rework context packaging: findings, specific fix instructions, original work reference
- [ ] Implement defect ledger with full lifecycle tracking (detected → attributed → resolved)
- [ ] Implement defect categorization and severity classification
- [ ] Build escape analysis: trace production defects back to gate gaps/misses
- [ ] Implement Andon cord: automatic line halt on defect rate spikes and cascade failures
- [ ] Wire escape analysis feedback loop into gate configuration updates
- [ ] Integrate quality gates with the Station abstraction from spec 039

## Test

- [ ] A quality gate with 3 checks runs all in parallel and produces a combined verdict
- [ ] Critical check failure triggers rework; minor-only failures pass with warnings
- [ ] Rework routing sends a logic error back to BUILD with specific findings attached
- [ ] Rework context includes the original work item, review findings, and fix instructions
- [ ] Defect ledger records full lifecycle from detection to resolution
- [ ] Escape analysis correctly identifies a gate gap when a new defect category reaches production
- [ ] Andon cord fires when 3 consecutive work items fail at the same station
- [ ] Andon cord pauses all in-progress work and queues new items until resolution
- [ ] Gate miss feedback: after a false negative escape, the check configuration is flagged for tuning

## Notes

The quality system deliberately separates the mechanism (quality gates, rework routing) from the strategy (which checks to run, how to configure them). This allows different lines to have different quality profiles — a hotfix line might relax style checks while tightening security checks.

The generative-adversarial primitive (spec 028) is one check type among many. Static analysis, threshold checks, and integration tests are equally first-class.