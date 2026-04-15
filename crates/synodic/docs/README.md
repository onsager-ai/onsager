# Documentation

Project documentation for understanding Synodic — its architecture, decisions, and evaluation strategy.

**Docs vs Specs:** Documentation (`docs/`) is guidance for understanding the project. Specs (`specs/`) are actionable requirements for developers and AI agents to implement.

## Architecture

| Document | Description |
|----------|-------------|
| [scope-review.md](./architecture/scope-review.md) | March 2026 scope review — rationale for pivoting from a Rust platform to a Claude Code skill package, spec disposition decisions, and the skill-first roadmap |

## Design

| Document | Description |
|----------|-------------|
| [factory-design.md](./design/factory-design.md) | Factory skill design — BUILD → INSPECT pipeline, adversarial review, static gates, rework loop, metrics and crystallization |
| [fractal-design.md](./design/fractal-design.md) | Fractal decomposition design — algorithmic spine philosophy, 8-step protocol with visual diagrams, complexity scoring, orthogonality checking, DAG scheduling, and CLI reference |

## Evaluation

| Document | Description |
|----------|-------------|
| [evaluation-strategy.md](./evaluation/evaluation-strategy.md) | Evaluation strategy — SWE-bench Pro as primary benchmark (1,865 tasks), complementary benchmarks (FeatureBench, DevBench), scoring protocol (F2P/P2P), and infrastructure |

## Related

| Location | What it covers |
|----------|---------------|
| [skills/factory/SKILL.md](../skills/factory/SKILL.md) | Factory skill definition — 7-step orchestration protocol, BUILD/INSPECT prompts, static gate, rework loop |
| [skills/fractal/SKILL.md](../skills/fractal/SKILL.md) | Fractal skill definition — 6-step protocol, complexity pre-filter, decompose/solve/reunify gates, DAG scheduler |
| [specs/](../specs/) | Actionable specifications in LeanSpec format — the requirements that skills implement |
