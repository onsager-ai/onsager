---
status: in-progress
created: 2026-03-23
priority: critical
tags:
- meta-testing
- harness
- ai-driven
- quality-validation
---

# AI Meta-Testing Framework

## Overview

Traditional software testing requires manually selecting frameworks, writing test harnesses, and validating that tests actually catch real bugs rather than producing false positives or false negatives. This is labor-intensive and brittle — each project requires different tools, different infrastructure, and different strategies.

Synodic's meta-testing framework inverts this: an AI agent acts as an expert testing consultant that analyzes the target project, reasons about what testing strategy is appropriate, implements the tests, executes them, and validates the results for reliability. Instead of hardcoded heuristics, the AI reasons about the project's nature, available tools, and real-world constraints.

The framework deliberately leverages Synodic's existing pipeline patterns (factory, fractal, adversarial) rather than reimplementing parallel infrastructure. This serves double duty: it validates the products being tested AND stress-tests Synodic's own patterns against a concrete, measurable problem.

### Why this matters

- **Every project is different.** A Python web app needs pytest + Docker + database fixtures. A Rust CLI needs `cargo test`. A React app needs Jest + browser testing. No single heuristic covers this.
- **E2E and integration tests require innovation.** Setting up multi-service test environments, mock APIs, seed data, and health checks is often harder than writing the tests themselves. The first attempt almost never works.
- **False positives/negatives undermine confidence.** A test suite that passes vacuously (tests that don't exercise changed code) or fails environmentally (missing deps, broken containers) is worse than no tests — it gives false confidence or wastes time investigating non-issues.
- **Synodic's patterns need real-world validation.** Using factory/fractal/adversarial for meta-testing dog-foods these patterns and produces governance data about their effectiveness.

## Design

### Architecture

The meta-testing module lives in `cli/synodic/src/meta/` (harness layer, not synodic-eval) because it tests products and features during development, not benchmark evaluations.

```
cli/synodic/src/meta/
├── mod.rs          # Types, orchestration loop, plan mutation
├── consult.rs      # AI consultant: project analysis → TestPlan
├── execute.rs      # Tiered execution with infrastructure lifecycle
└── validate.rs     # AI validator: result reliability assessment
```

### Core Concept: AI as Testing Consultant

Instead of pattern-matching test names or applying static rules, the AI receives full project context and reasons about:

1. **Project nature** — language, framework, architecture, existing test infrastructure
2. **Change scope** — what the diff or spec affects, what could break
3. **Tool selection** — which testing frameworks, infrastructure, and methodologies fit
4. **Strategy design** — tiered approach from cheap (smoke/unit) to expensive (integration/e2e)
5. **Implementation** — actual runnable test code, not abstract recommendations
6. **Pipeline recommendation** — which Synodic pattern fits (factory, fractal, adversarial)

### Tiered Test Plans

Tests are organized into tiers that mirror the testing pyramid:

```
TestPlan
├── tiers[]
│   ├── smoke    — does the system start? (seconds)
│   ├── unit     — do individual functions work? (seconds)
│   ├── integration — do components interact correctly? (minutes)
│   └── e2e      — does the full system work? (minutes-hours)
├── infrastructure[]
│   ├── InfraRequirement { setup, health_check, teardown }
│   └── ...
├── teardown_commands[]
└── risks[]
```

Each tier has independent setup/run/test definitions. Earlier tier failure short-circuits later tiers — no point running e2e if unit tests fail. Each tier declares `continue_on_failure` semantics.

### Infrastructure Lifecycle

Real testing often requires external services. The framework handles this explicitly:

```
InfraRequirement {
    name: "postgres",
    setup_command: "docker run -d -p 5432:5432 postgres:15",
    health_check: "pg_isready -h localhost -p 5432",
    teardown_command: "docker stop $(docker ps -q --filter ancestor=postgres:15)"
}
```

- Setup runs before any tiers
- Health checks retry up to 5 times with 2s backoff
- Teardown always runs, even on failure

### Iterative Rework Loop

The pipeline is NOT linear. It's an adaptive loop that mirrors the harness governance pattern:

```
consult → implement → execute ─┬→ validate → done
                                │
                          (infra/setup failed?)
                                │
                          diagnose → fix/reconsult → retry
                                │
                          (max rework?)
                                │
                          escalate with partial results
```

The `Diagnosis` system classifies infrastructure failures:

| FailureKind | Example | Action |
|---|---|---|
| `MissingDependency` | `pip install numpy` fails | Add setup command |
| `InfrastructureUnavailable` | Docker not running | Add infra requirement or switch approach |
| `VersionMismatch` | Python 3.8 vs 3.12 | Adjust commands |
| `TestCodeError` | Import path wrong | Replace test code |
| `EnvironmentConfig` | Missing env var | Add setup |
| `WrongApproach` | Framework doesn't exist | Full re-consultation |

For salvageable failures, the AI proposes incremental `PlanFix` actions (add command, replace test, add infra, remove broken test). For `WrongApproach`, a full re-consultation happens with the failure as context, forcing a different strategy.

### Synodic Pipeline Integration

The AI consultant recommends which Synodic pipeline pattern fits:

- **Factory** (BUILD → INSPECT): Straightforward testing — write tests, run them, review quality. Best for clear requirements, existing test infrastructure.
- **Fractal**: Complex multi-component testing. Decompose "test this feature" into sub-problems (set up database, write API tests, write UI tests, integration glue), solve each independently, reunify.
- **Adversarial**: Quality hardening. Generator writes tests, critic attacks them (finds vacuous passes, false negatives, flaky tests), escalating rounds until convergence.

This leverages existing infrastructure rather than reimplementing rework loops, decomposition, and critic patterns from scratch. Meta-testing runs produce governance logs that measure pattern effectiveness — dog-fooding Synodic's patterns against a concrete problem.

### CLI Interface

```bash
# Analyze project and propose + execute tests
synodic harness meta --spec path/to/spec.md

# With explicit diff
synodic harness meta --diff "$(git diff HEAD~1)"

# Dry run — show plan without executing
synodic harness meta --spec spec.md --dry-run

# JSON output for programmatic use
synodic harness meta --spec spec.md --json

# Custom agent and rework limit
synodic harness meta --agent claude --max-rework 3

# Target specific working directory
synodic harness meta --workdir /path/to/project --spec spec.md
```

### Governance Integration

All meta-testing runs are logged to `.harness/meta.governance.jsonl`:

```json
{
  "work_id": "meta-1711158000",
  "source": "meta-testing",
  "timestamp": "2026-03-23T...",
  "status": "passed|failed|unreliable",
  "strategy": "Tiered pytest with Docker postgres",
  "frameworks": ["pytest", "docker"],
  "tests_proposed": 8,
  "tests_passed": 7,
  "tests_failed": 1,
  "confidence": 0.85,
  "run_dir": ".harness/.runs/meta-1711158000"
}
```

Run artifacts saved to `.harness/.runs/{run_id}/`:
- `meta-consult-prompt.md` — what the AI saw
- `meta-consult-response.txt` — what it proposed
- `iteration-{n}/` — per-iteration tier outputs, infra output, teardown
- `meta-diagnose-{n}-prompt.md` — rework diagnosis prompts
- `meta-validate-prompt.md` — validation prompt
- Governance log entry

## Plan

- [x] Design meta-testing architecture in harness layer
- [x] Implement core types: TestPlan, TestTier, InfraRequirement, Diagnosis, PlanFix
- [x] Implement AI consultant (consult.rs): project analysis → tiered TestPlan
- [x] Implement tiered execution (execute.rs): infrastructure lifecycle, per-tier setup/run
- [x] Implement AI validator (validate.rs): result reliability assessment
- [x] Implement iterative rework loop: diagnose → fix/reconsult → retry
- [x] Implement plan mutation: apply_fixes(), execution_needs_rework()
- [x] Add CLI subcommand: `synodic harness meta`
- [x] Add governance logging to `.harness/meta.governance.jsonl`
- [ ] Integration test: run meta-testing against Synodic itself (dogfood)
- [ ] Integration test: run meta-testing against a Python project
- [ ] Connect pipeline recommendation to actual pipeline invocation
- [ ] Add `--pipeline` flag to force a specific Synodic pattern

## Test

- [x] TestPlan serialization roundtrip (tiers, infrastructure, risks)
- [x] Diagnosis serialization (FailureKind enum, PlanFix actions)
- [x] apply_fixes: SetupCommand adds to correct tier
- [x] apply_fixes: ReplaceTest modifies test code in place
- [x] apply_fixes: RemoveTest removes from tier
- [x] execution_needs_rework: infra failure triggers rework
- [x] execution_needs_rework: setup failure triggers rework
- [x] execution_needs_rework: zero tests ran triggers rework
- [x] execution_needs_rework: legitimate failure (1 passed, 2 failed) does NOT trigger rework
- [x] JSON extraction from markdown fences, bare objects, nested braces
- [x] Test result parsing: pytest, cargo test, mixed output formats
- [x] Validation report parsing: direct JSON, concerns, wrapped
- [x] Validation prompt contains strategy, tiers, per-tier results
- [ ] End-to-end: consult → execute → validate on a real project
- [ ] Rework loop: setup failure → diagnosis → fix → re-execute succeeds
- [ ] Adversarial: false positive detection on vacuous test suite

## Notes

### Why harness layer, not synodic-eval?

`synodic-eval` is a standalone eval framework for benchmarking (SWE-bench, FeatureBench). It has zero governance dependencies. Meta-testing is fundamentally about governance — it validates products/features during development, integrates with the harness rework loop, and produces governance logs. Putting it in eval would violate the separation boundary.

### Why AI reasoning, not heuristics?

An earlier iteration used heuristic pattern matching (classify test names as unit/integration/e2e, check for environment error strings). This was reverted because:
1. It can't reason about new frameworks or unconventional project structures
2. It can't design test harnesses for projects that don't have one
3. It can't decide when Docker is needed vs. when SQLite suffices
4. It can't adapt when its first approach fails

The AI-driven approach handles all of these because it reasons about the specific project rather than applying generic rules.

### Why tiered, not flat?

A flat test plan ("here are 10 tests, run them all") doesn't model real testing:
- You want fast feedback first (smoke tests in seconds, not e2e tests in minutes)
- Unit test failure makes integration tests meaningless
- Each tier has different infrastructure requirements (unit tests don't need Docker, e2e might)
- Different tiers have different false positive/negative profiles

### Dog-fooding value

Every meta-testing run is a data point about Synodic's patterns:
- If fractal decomposition works well for complex test setups, that validates the pattern
- If adversarial catches false positives that single-pass validation misses, that proves its value
- If the rework loop consistently stabilizes test infrastructure in 2-3 iterations, that quantifies its benefit
- Governance logs capture this data for analysis across runs
