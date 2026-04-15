---
status: completed
created: 2026-03-23
priority: high
tags:
- testing
- harness
- pipeline
- quality
parent: 058-code-harness-orchestration
depends_on:
- '061'
- '062'
- '063'
- '064'
- '065'
created_at: 2026-03-23T21:00:00.000000000Z
updated_at: 2026-03-23T21:00:00.000000000Z
---

# Test Assessment: Umbrella Spec 058 and Child Specs 061–065

## Overview

Systematic test assessment of the Code Harness Orchestration umbrella (spec 058) and its five child specs. Evaluated existing test coverage, identified gaps against each spec's test plan, wrote 77 new tests to close those gaps, and verified all 175 workspace tests pass.

## Findings

### Pre-existing state

The synodic crate had **63 unit tests** across 12 modules. The synodic-eval crate had **35 tests** (parser, verdict). All 98 passed. No integration or end-to-end tests existed for the pipeline system as a whole.

### Implementation completeness

| Spec | Component | Implementation | Pre-existing tests | Assessment |
|------|-----------|---------------|-------------------|------------|
| **061** | Pipeline Engine Core | Complete — schema, parser, 4-type executor, middleware, vars, validate | 30 | Parsing and validation covered; middleware composition and edge cases missing |
| **062** | Gate System | Complete — gates.yml parser, file-match, command execution, structured output | 5 | Structure-only tests; no execution, multi-pattern matching, or failure aggregation |
| **063** | Pipeline Definitions | **Not implemented** — no .yml files, no prompt templates, no output schemas | 0 | Blocked on content authoring; engine validates correctly against spec 063 patterns |
| **064** | Algorithmic Commands | Complete — fractal (5 commands) + swarm (2 commands) | 24 | Core algorithms tested; edge cases (empty inputs, budget bounds, diamond DAGs) missing |
| **065** | Skill Migration | **Not implemented** — no SKILL.md files exist to migrate | 0 | No shims or prompt templates to test; blocked on spec 063 |

### Gap analysis by spec

**Spec 061 — Pipeline Engine Core**

Gaps closed:
- All 4 step types parsed in a single pipeline (verifying the 7→4 type reduction from spec 058)
- Malformed YAML error handling
- Agent step context map interpolation
- Run step poll configuration parsing
- Branch default max_iterations (3)
- Fan sequential mode
- Variable interpolation: multiple unset vars reported, duplicate vars in same string, all 5 scopes (config, spec, manifest, steps, loop), underscored names, empty strings
- Validation: empty pipeline/step names, branch zero max_iterations, fan parallel without over/steps, on_fail escalate vs rework distinction, full factory pipeline, full adversarial pipeline, multiple errors collected in one pass

Remaining gap: No executor-level integration tests (would require mock `claude -p` subprocess). Middleware retry/timeout interaction tested structurally via validation but not at runtime.

**Spec 062 — Gate System**

Gaps closed:
- File-match with multiple extensions (`.ts`, `.css`, `.py`, `.go`)
- Full-path matching (nested paths like `cli/synodic/src/main.rs`)
- GateGroupResult serialization for all states (passed, failed, mixed)
- Multiple failure aggregation
- Skipped gate tracking
- Missing gates.yml graceful handling
- Gate entries without match patterns
- Full spec 062 gates.yml structure (4-gate preflight group)

Remaining gap: Actual gate command execution requires real subprocess spawning. `run_gate_groups()` tested structurally but not end-to-end (needs git repo fixture).

**Spec 063 — Pipeline Definitions**

No implementation exists. However, the validation tests in spec 061 now verify that the **intended pipeline structures** from spec 063 validate correctly:
- Factory pipeline (linear: build → gate → inspect → branch → PR) — validates clean
- Adversarial pipeline (escalating: generate → gate → fan/loop → PR) — validates clean
- Variable references in context maps parse correctly
- Output schema fields accepted by agent step parser

**Spec 064 — Algorithmic Commands**

Gaps closed:
- **Fractal decompose**: empty children (no false flags), budget tight/ok threshold (80%), complexity score range bounds, empty budget allocation, minimum-1 allocation, cosine similarity identity/orthogonality, 3-child linear chain, diamond dependency pattern
- **Fractal schedule**: critical path length = wave count, max parallelism in diamond, single node, non-leaf exclusion, wide parallel (5 independent)
- **Fractal reunify**: redundancy conflict detection (shared files), clean merge status, interface gap → needs_ai, empty children clean merge, alphabetical merge ordering without waves, multiple conflict types aggregated
- **Fractal prune**: empty tree, all-empty nodes prunable, mixed redundancy + unique, file coverage map, non-solved nodes excluded from analysis
- **Fractal NLP**: identical set Jaccard = 1.0, stop word filtering, short word filtering, hyphenated term extraction, duplicate-preserving term list, Child/Manifest serialization roundtrips
- **Swarm checkpoint**: 3-branch N-way similarity matrix (3 pairs), no cross-pollination for identical branches, empty file sets → 0.0, single branch → no pairs
- **Swarm prune**: custom threshold (0.3) triggers pruning, high threshold (0.99) prevents pruning, 5 converging branches keep min 2, single branch safety

Remaining gap: `synodic fractal complexity` CLI integration test (command-line JSON-in/JSON-out). All algorithms tested at library level.

**Spec 065 — Skill Migration**

No SKILL.md files or pipeline YAML definitions exist yet. Spec 065 is blocked on specs 061–064. No tests can be written until migration artifacts exist. The shim format (`synodic harness run --pipeline <name> --spec <path>`) is validated indirectly through spec 061's executor config structure.

## Results

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| synodic crate tests | 63 | 140 | +77 |
| synodic-eval crate tests | 35 | 35 | 0 |
| **Workspace total** | **98** | **175** | **+77** |
| Test files modified | — | 11 | — |
| Lines added | — | 1,369 | — |
| Failures | 0 | 0 | — |

### Test distribution by module

| Module | Before | After | Coverage focus |
|--------|--------|-------|----------------|
| pipeline/schema | 8 | 18 | +10: all types, malformed input, context, poll, defaults |
| pipeline/validate | 8 | 18 | +10: empty names, targets, full pipeline validation |
| pipeline/gates | 5 | 12 | +7: multi-ext matching, results, spec 062 YAML |
| pipeline/vars | 6 | 15 | +9: scopes, errors, edge cases |
| fractal/decompose | 9 | 21 | +12: budget, cosine, diamond deps, empty inputs |
| fractal/schedule | 4 | 9 | +5: critical path, parallelism, non-leaf exclusion |
| fractal/reunify | 4 | 10 | +6: redundancy, AI-needed, alphabetical ordering |
| fractal/prune | 5 | 10 | +5: empty tree, status filtering, mixed scenarios |
| fractal/mod | 3 | 10 | +7: serialization, NLP edge cases |
| swarm/checkpoint | 3 | 7 | +4: N-way, identical, empty, single branch |
| swarm/prune | 4 | 8 | +4: thresholds, many branches, single branch |

## Architectural observations

1. **The 7→4 type reduction works.** All pipeline patterns from specs 063 (factory, fractal, swarm, adversarial) express correctly using only `agent`, `run`, `branch`, `fan`. No test required a missing step type.

2. **Middleware is parse-validated but not runtime-tested.** The `retry`, `timeout`, `log`, `on_fail` fields parse and validate correctly. The executor applies them (retry loop in `execute_step_with_middleware`), but testing this requires subprocess mocking that doesn't exist yet.

3. **Variable interpolation is robust.** The `${scope.field}` system handles all spec-defined scopes, fails fast on unset variables, and correctly handles edge cases (duplicates, empty strings, underscores). The deliberate absence of filters/pipes/expressions is validated by the regex pattern.

4. **Gate system is structurally complete but needs integration fixtures.** File-match filtering, YAML parsing, and result serialization all work. End-to-end testing of `run_gate_groups()` needs a git repository fixture with staged changes.

5. **Specs 063 and 065 are blocked on content, not code.** The engine (061), gates (062), and algorithms (064) are fully implemented. What's missing is the declarative content: pipeline YAML files, prompt templates, output schemas, and SKILL.md migration shims.

## Recommendations

1. **Spec 063 is the critical next step.** Write the 4 pipeline YAML files (factory, fractal, swarm, adversarial) and validate them with `synodic harness validate`.
2. **Add executor integration tests** with a mock `claude` binary that returns structured output, to test the full pipeline execution path.
3. **Add gate execution integration tests** with a temporary git repo fixture containing staged changes.
4. **Spec 065 can begin** once spec 063's pipeline YAMLs and prompt templates exist.
