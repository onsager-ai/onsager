# Synodic Specs

Spec-driven development for Synodic. Each spec is a numbered directory
containing a LeanSpec `README.md` with YAML frontmatter.

## Status Legend

| Status | Meaning |
|--------|---------|
| `draft` | Design in progress |
| `planned` | Designed, ready for implementation |
| `complete` | Implemented |
| `archived` | Superseded or abandoned (in `_archived/`) |

## Active Specs

| # | Spec | Status |
|---|------|--------|
| 068 | [Governance Architecture v2](./068-governance-architecture-v2/) | draft |
| 069 | [Governance Capability Matrix](./069-governance-capability-matrix/) | draft |
| 072 | [Governance Data Model](./072-governance-data-model/) | complete |
| 073 | [Feedback Ingestion Override](./073-feedback-ingestion-override/) | complete |
| 074 | [Governance Scoring Engine](./074-governance-scoring-engine/) | complete |
| 075 | [Adversarial Probing](./075-adversarial-probing/) | complete |
| 076 | [Rule Lifecycle Convergence](./076-rule-lifecycle-convergence/) | complete |
| 077 | [Pipeline Runtime](./077-pipeline-runtime/) | draft |
| 078 | [Pipeline Telemetry](./078-pipeline-telemetry/) | complete |
| 079 | [Semantic Inspect](./079-semantic-inspect/) | complete |

The [coordination-model skill](../.claude/skills/coordination-model/)
remains active for agent coordination playbook validation.

## Archived Specs

All archived specs live in [`_archived/`](./_archived/). They were designed for
earlier architectures (fleet execution, coordination theory, harness-first
governance) that were superseded. See
[scope-review.md](../docs/architecture/scope-review.md) for the rationale.

| Range | Category | Count |
|-------|----------|-------|
| 001–016, 018–035 | Fleet Execution, Coordination Theory & Primitives | 33 |
| 017 (umbrella) | AI-Native Coordination Model (umbrella spec + evals) | 1 |
| 036 | Competitive Analysis | 1 |
| 038–043 | Factory (pre-skill versions) | 6 |
| 045–056 | Harness, Eval, Dogfood | 9 |
| 067, 070–071 | Harness Repositioning, Cleanup, Feedback Umbrella | 3 |

## Related Documentation

See [`docs/`](../docs/) for project documentation (architectural context,
evaluation strategy, etc.).
