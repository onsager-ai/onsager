# Orchestration Patterns — Concept Reference

Synodic governs multi-agent pipelines. These four coordination topologies define the patterns that agents use and the governance layer observes. The Rust implementation lives in [`codervisor/orchestra`](https://github.com/codervisor/orchestra); this document preserves the concepts for governance context.

## Factory (Linear)

**Pattern**: BUILD -> INSPECT -> route -> PR

**Best for**: Clear, spec-driven tasks with a single implementation path.

**Flow**:
1. **Build agent** implements the spec in an isolated worktree
2. **Static gate** runs preflight checks (fmt, lint, test)
3. **Inspect agent** reviews the diff against the spec
4. **Route**: approve -> PR, rework -> loop back to build (max 3 iterations)

**Governance checkpoints**: L2 review after build step, before PR creation.

## Adversarial (Generate-Attack Loop)

**Pattern**: GENERATE -> static gate -> ATTACK -> loop

**Best for**: Security hardening, robustness testing, quality escalation.

**Flow**:
1. **Generate agent** produces implementation
2. **Static gate** runs preflight checks
3. **Attack agent** tries to find flaws (escalating critic modes: syntax -> edge cases -> concurrency -> adversarial inputs -> semantic)
4. Loop terminates after 2 consecutive clean rounds or 3 plateau rounds

**Governance checkpoints**: L2 review validates attack/defense rounds.

## Fractal (Recursive Decompose-Solve-Reunify)

**Pattern**: DECOMPOSE -> schedule -> parallel SOLVE -> REUNIFY -> prune

**Best for**: Large, complex tasks that benefit from divide-and-conquer.

**Flow**:
1. **Complexity check** scores the task
2. **Decompose** recursively splits into orthogonal subtasks (max depth 3, max 20 nodes)
3. **Schedule** topological-sorts the DAG into parallel waves
4. **Solve** each leaf node in parallel isolated worktrees
5. **Reunify** merges results, resolving conflicts
6. **Prune** removes redundant branches (greedy set cover)

**Governance checkpoints**: L2 review after reunification.

**Algorithms** (in orchestra-core):
- TF-IDF orthogonality scoring for decomposition quality
- Cycle detection in dependency graphs
- DAG topological sort into parallel execution waves
- Structural + git merge-tree conflict analysis
- Greedy set cover for redundancy pruning

## Swarm (Speculative Parallel Exploration)

**Pattern**: STRATEGIZE -> parallel EXPLORE -> checkpoint/prune loop -> MERGE

**Best for**: Ambiguous tasks where the best approach is unknown upfront.

**Flow**:
1. **Strategize** generates N divergent strategies
2. **Explore** each strategy in parallel isolated worktrees
3. **Checkpoint loop** periodically measures pairwise Jaccard similarity, cross-pollinates discoveries, prunes converging branches (min 2 survivors)
4. **Merge** combines the best branch results
5. **Gate** runs preflight checks on merged output

**Governance checkpoints**: L2 review before final merge.

**Algorithms** (in orchestra-core):
- Pairwise Jaccard similarity for convergence detection
- Cross-pollination of discoveries between branches
- Convergence-based pruning with minimum survivor guarantee

## Pipeline YAML Reference

The original pipeline definitions are preserved in this directory:
- [`factory.yml`](./factory.yml)
- [`adversarial.yml`](./adversarial.yml)
- [`fractal.yml`](./fractal.yml)
- [`swarm.yml`](./swarm.yml)
