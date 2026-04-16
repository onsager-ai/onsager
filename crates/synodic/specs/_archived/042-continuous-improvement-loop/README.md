---
status: archived
created: 2026-03-11
priority: medium
tags:
- factory
- kaizen
- continuous-improvement
- ab-testing
- retrospective
- pattern-library
parent: 037-coding-factory-vision
depends_on:
- 041-production-metrics-dashboard
- 011-fleet-coordination-optimization
created_at: 2026-03-11T00:00:00Z
updated_at: 2026-03-11T00:00:00Z
---

# Continuous Improvement Loop — Self-Optimizing Factory

## Overview

Nemosis (spec 016) handles cost optimization by routing repetitive tasks to cheaper models. But there's no mechanism for the factory to improve its own process — no A/B testing of coordination strategies, no automated retrospectives, no pattern library, no skill accumulation.

This is like having a plan to buy cheaper steel but no system for workers to suggest line improvements and no process engineering department.

This spec builds the Kaizen system: the factory's ability to observe its own production data, experiment with process changes, learn from successes and failures, and evolve its configuration over time — with human oversight on all changes.

## Design

### A/B Testing of Coordination Strategies

Different coordination primitives may perform differently at the same station. The A/B test framework lets the factory experiment:

```yaml
experiment:
  id: exp-build-coordination
  station: build
  hypothesis: "Speculative swarm (2 forks) produces higher first-pass yield than hierarchical at BUILD"
  variants:
    control:
      coordination: hierarchical
      traffic: 50%
    treatment:
      coordination: speculative_swarm
      config: { forks: 2 }
      traffic: 50%
  metrics:
    primary: first_pass_yield
    secondary: [cycle_time, cost_per_unit]
  duration: 100_work_items  # or time-based
  significance: 0.95
  auto_promote: false       # require human approval to adopt winner
```

Traffic splitting is handled by the conveyor: for work items entering the test station, alternate between control and treatment configurations. Results are stored in the metrics layer (spec 041) with experiment tags.

### Automated Retrospectives

After each completed work item (or batch), agents analyze what happened:

1. **Performance retrospective:** Was cycle time above/below target? Which station was slowest? Why?
2. **Quality retrospective:** Were there rework cycles? What was the root cause? Was the defect preventable?
3. **Cost retrospective:** Was any station over-spending tokens? Could a cheaper model tier have worked?

Retrospective output is a structured report:

```json
{
  "work_item_id": "work-001",
  "retrospective": {
    "bottleneck": "inspect",
    "bottleneck_cause": "adversarial review required 3 iterations due to ambiguous spec",
    "improvement_suggestion": "Add spec clarity check at INTAKE quality gate",
    "category": "process_gap",
    "confidence": 0.8
  }
}
```

Improvement suggestions are accumulated and surfaced to humans in a weekly digest. Patterns that appear 3+ times are flagged for automatic experiment creation.

### Pattern Library

Successful coordination configurations are captured as reusable patterns:

```yaml
pattern:
  id: pat-frontend-build
  name: "Frontend Feature Build"
  description: "Optimal configuration discovered for React component implementation"
  applicable_when:
    work_type: feature
    domain: frontend
    complexity: [small, medium]
  station_configs:
    design:
      coordination: speculative_swarm
      config: { forks: 2, time_limit: 120s }
    build:
      coordination: hierarchical
      config: { specialists: [component, style, test] }
    inspect:
      coordination: generative_adversarial
      config: { max_iterations: 2 }
  performance:
    first_pass_yield: 0.88
    avg_cycle_time: 420s
    avg_cost: 0.35
    sample_size: 47
  discovered_via: exp-frontend-swarm-vs-hier
  created_at: 2026-04-15
```

When a new work item arrives, the factory matches it against the pattern library to select the best-known configuration. Unknown work types fall back to default configuration and create learning opportunities.

### Skill Accumulation

Beyond pattern library (which captures station configurations), skill accumulation captures agent-level learning:

- **Prompt refinements:** When an agent's custom instructions improve performance at a station, the refinement is stored
- **Domain knowledge:** Codebase-specific patterns, common pitfalls, preferred approaches discovered during production
- **Tool effectiveness:** Which tools work best for which check types at quality gates

Skill accumulation feeds into agent configuration: when an agent is assigned to a station, it receives the accumulated skills for that station+domain combination. This is complementary to Nemosis (spec 016): Nemosis optimizes which model to use; skill accumulation optimizes how the model is prompted.

### Process Evolution

The highest-order loop: the line layout itself can change based on production data.

Examples of process evolution (all require human approval):
- **Station insertion:** Metrics show that INSPECT catches many spec-ambiguity issues → suggest adding a SPEC_REVIEW station between INTAKE and DESIGN
- **Station removal:** A station consistently passes 99%+ of items → suggest removing or merging it
- **Station reordering:** Static analysis at INSPECT catches issues that could be caught cheaper at BUILD → suggest moving the check upstream
- **Gate tuning:** A quality gate check has high false-positive rate → suggest adjusting threshold

Process evolution proposals are generated automatically but always require human review and approval before deployment.

## Plan

- [ ] Define A/B experiment schema with variants, traffic splitting, metrics, and significance testing
- [ ] Implement traffic splitting in the conveyor: route work items to variant configurations
- [ ] Implement statistical significance testing for experiment results (at minimum: two-proportion z-test)
- [ ] Build automated retrospective agent: post-item analysis of bottlenecks, quality, and cost
- [ ] Implement improvement suggestion accumulation with pattern detection (3+ occurrences = pattern)
- [ ] Define pattern library schema and storage (extends SQLite from spec 006)
- [ ] Implement pattern matching: classify incoming work items and select best-known configuration
- [ ] Build skill accumulation store: prompt refinements, domain knowledge, tool effectiveness per station
- [ ] Implement process evolution proposal generation based on sustained metric anomalies
- [ ] Build human review interface for experiment results, patterns, and process evolution proposals

## Test

- [ ] A/B experiment correctly splits traffic 50/50 between control and treatment
- [ ] After 100 work items, experiment declares a statistically significant winner (using seeded test data)
- [ ] Automated retrospective identifies correct bottleneck station and generates improvement suggestion
- [ ] Pattern with 3+ matching retrospective suggestions is flagged for experiment creation
- [ ] Pattern library matches a "frontend feature" work item to the correct stored configuration
- [ ] Skill accumulation: a prompt refinement stored for BUILD is included when a new agent starts at BUILD
- [ ] Process evolution correctly proposes adding a station when defect-at-INSPECT rate exceeds threshold
- [ ] All process evolution proposals are held for human approval (never auto-applied)

## Notes

The improvement loop has three time horizons:
1. **Per-item:** Automated retrospectives (immediate)
2. **Per-experiment:** A/B test results (days to weeks)
3. **Per-quarter:** Process evolution proposals (weeks to months)

The system is designed to be conservative: it suggests changes with supporting data and confidence levels, but humans make the final call. The factory improves, but it doesn't redesign itself without oversight.