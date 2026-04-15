# Synodic Specs

Spec-driven development for Synodic. Each spec is a numbered directory containing a LeanSpec `README.md` with YAML frontmatter.

## Status Legend

| Status | Meaning |
|--------|---------|
| `in-progress` | Currently being implemented |
| `planned` | Designed, ready for implementation |
| `archived` | Superseded or abandoned |

## Active Specs

| # | Spec | Status |
|---|------|--------|
| 037 | [Coding Factory Vision](./037-coding-factory-vision/) | planned |
| 044 | [Factory Skill MVP](./044-factory-skill-mvp/) | in-progress |

## Archived Specs

Specs 001–043 (excluding 037) are archived. They were designed for a platform architecture
(Rust binary, fleet execution, coordination theory) that was superseded by the skill-first
approach. See [scope-review.md](../docs/architecture/scope-review.md) for the rationale.

| Range | Category | Count |
|-------|----------|-------|
| 001–010 | Fleet Execution & Auth | 10 |
| 011–035 | Coordination Theory & Primitives | 25 |
| 036 | Competitive Analysis | 1 |
| 038–043 | Factory (pre-skill versions) | 6 |

## Related Documentation

See [`docs/`](../docs/) for project documentation (architectural context, evaluation strategy, etc.).
