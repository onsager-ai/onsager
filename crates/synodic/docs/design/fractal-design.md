# Fractal Decomposition

Recursively split complex tasks into orthogonal sub-specs, solve each leaf
via independent subagents, reunify results bottom-up. The algorithmic spine
handles all structural decisions deterministically — AI is reserved for
semantic work only.

## At a Glance

```
                     ┌─────────────────┐
                     │   COMPLEX TASK   │
                     └────────┬────────┘
                              │
            ┌─────────────────▼─────────────────┐
            │        COMPLEXITY SCORE            │  algorithm
            │     (weighted feature scoring)     │
            └─────────────────┬─────────────────┘
                              │
                    ┌─────────▼─────────┐
                    │ score < threshold? │
                    └──┬─────────────┬──┘
                  YES  │             │  NO
                       ▼             ▼
                  ┌────────┐   ┌─────────┐
                  │  LEAF  │   │ AI SPLIT │  ← only AI call in decompose
                  │  (done)│   └────┬────┘
                  └────────┘        │
            ┌───────────────────────▼───────────────────────┐
            │              DECOMPOSE GATE                   │  algorithm
            │  TF-IDF cosine · Kahn's toposort · budget    │
            └───────────────────────┬───────────────────────┘
                                    │
            ┌───────────────────────▼───────────────────────┐
            │             SOLVE SCHEDULER                   │  algorithm
            │         (DAG wave scheduling)                 │
            └───────────────────────┬───────────────────────┘
                                    │
              wave 1: ┌─────┬─────┬─────┐
                      │ AI  │ AI  │ AI  │     ← real work happens here
              wave 2: └─────┴──┬──┴─────┘
                        ┌──────┘
                        │ AI  │               ← dependent leaves
                        └──┬──┘
                           │
            ┌──────────────▼──────────────┐
            │       REUNIFY MERGE         │  algorithm
            │  git merge-tree · conflict  │  (AI only if semantic conflict)
            └──────────────┬──────────────┘
                           │
            ┌──────────────▼──────────────┐
            │        PRUNE GATE           │  algorithm
            │   set cover · subset check  │
            └──────────────┬──────────────┘
                           │
                     ┌─────▼─────┐
                     │  output.md │
                     └───────────┘
```

## AI vs Algorithm Split

The key design principle: **AI for semantics, algorithms for structure.**

```
 ┌──────────────────────────────────────────────────────────────────┐
 │                                                                  │
 │  ┌──── ALGORITHMIC (deterministic, O(n) to O(n²)) ────────┐    │
 │  │                                                         │    │
 │  │  Complexity scoring ·············· decision tree        │    │
 │  │  Orthogonality check ············· TF-IDF cosine        │    │
 │  │  Cycle detection ················· Kahn's toposort      │    │
 │  │  Solve scheduling ··············· DAG BFS layers        │    │
 │  │  Budget allocation ·············· proportional          │    │
 │  │  Code merging ··················· git merge-tree        │    │
 │  │  Conflict detection ············· set intersection      │    │
 │  │  Redundancy pruning ············· greedy set cover      │    │
 │  │                                                         │    │
 │  └─────────────────────────────────────────────────────────┘    │
 │                                                                  │
 │  ┌──── AI SUBAGENT (semantic, context-dependent) ──────────┐    │
 │  │                                                         │    │
 │  │  Decompose ··· understand task → produce sub-specs      │    │
 │  │  Solve ······· understand sub-spec → write code         │    │
 │  │  Resolve ···· fix ambiguous interface mismatches        │    │
 │  │                                                         │    │
 │  └─────────────────────────────────────────────────────────┘    │
 │                                                                  │
 └──────────────────────────────────────────────────────────────────┘
```

For a typical 20-node tree with 12 leaves:

| Phase | Before (all AI) | After (algorithmic spine) |
|-------|:---:|:---:|
| Decompose (8 nodes) | 8 | 5-7 |
| Solve (12 leaves) | 12 | 12 |
| Reunify (8 nodes) | 8 | 0-2 |
| Reunify rework | 0-8 | 0-2 |
| Prune | 1 | 0 |
| **Total AI calls** | **29-37** | **17-23** |

## Usage

```bash
/fractal decompose specs/045-some-feature/README.md
/fractal decompose "Design a microservices platform with auth, data, and API"
```

## Directory Structure

Each fractal run creates a work directory on disk. The filesystem IS the
recursion stack — Claude Code subagents can't nest, so specs on disk
replace the call stack.

```
.fractal/fractal-1710600000/
│
├── manifest.json                    ← tree state + metrics
├── root.md                          ← original task
│
├── tree/
│   ├── 1-auth/
│   │   ├── spec.md                  ← "design auth system"
│   │   ├── result.md                ← filled after solve
│   │   ├── 1-oauth/
│   │   │   ├── spec.md              ← leaf: "OAuth flow"
│   │   │   └── result.md
│   │   └── 2-sessions/
│   │       ├── spec.md              ← leaf: "session mgmt"
│   │       └── result.md
│   ├── 2-data/
│   │   ├── spec.md
│   │   └── result.md
│   └── 3-api/
│       ├── spec.md                  ← leaf (no further split)
│       └── result.md
│
└── output.md                        ← final reunified result
```

## Configuration

Place `.fractal.yaml` in the repo root to override defaults:

```yaml
fractal:
  max_depth: 3              # 1-5, tree depth cap
  max_children: 5           # 2-10, children per node
  max_total_nodes: 20       # 2-50, hard cap on tree size
  split_strategy: orthogonal # orthogonal | aspect-based | temporal
  reunification: lossless-merge  # lossless-merge | best-child | summary-merge
  min_complexity: medium     # threshold for auto-LEAF
  solve_mode: parallel       # parallel | sequential | factory
  output_mode: code          # code | design | analysis
```

How `solve_mode` maps to execution:

```
  parallel     ─►  all leaves in one wave (if independent)
  sequential   ─►  one leaf per wave
  factory      ─►  each leaf goes through /factory run (full governance)

  (The DAG scheduler overrides both: it computes actual dependency waves)
```

## Orchestration Protocol

### Step 1 — Initialize

Parse input, generate work ID (`fractal-{unix-timestamp}`), create directory,
write `root.md` and `manifest.json`.

### Step 1.5 — Complexity Pre-Filter

```bash
echo "Build an auth system" | synodic fractal complexity
# → {"complexity_score": 0.088}     ← auto-LEAF, skip AI

echo "Build microservices with auth, DB, API, monitoring, caching" | synodic fractal complexity
# → {"complexity_score": 0.612}     ← proceed to AI decompose
```

Scoring weights:

```
  ┌──────────────────────────────────────────────────────┐
  │  cross_cutting ████████████████████████████░░  0.35  │  auth, caching, security...
  │  term_diversity ██████████████████░░░░░░░░░░  0.25  │  unique concept count
  │  enumeration    ██████████████████░░░░░░░░░░  0.25  │  bullet/numbered items
  │  line_count     ██████████░░░░░░░░░░░░░░░░░░  0.15  │  spec length
  └──────────────────────────────────────────────────────┘
```

### Step 2 — Decompose

AI subagent analyzes each node and produces a `DECOMPOSE VERDICT`:
either `LEAF` (stop) or `SPLIT` (decompose into children with slug,
scope, boundaries, inputs, outputs).

### Step 2.5 — Decompose Gate

```bash
echo '{"parent_spec": "...", "children": [...], ...}' | synodic fractal gate
```

Four structural checks, all deterministic:

```
  ┌─────────────────────────────────────────────────────────┐
  │                    DECOMPOSE GATE                       │
  │                                                         │
  │  ┌─ orthogonality ─┐  TF-IDF cosine similarity        │
  │  │  via rust-tfidf  │  between each pair of children.   │
  │  │  Jaccard pre-    │  Flag if Jaccard > 30%            │
  │  │  filter < 10%    │  OR cosine > 30%.                 │
  │  └─────────────────┘                                    │
  │                                                         │
  │  ┌─ coverage ──────┐  Parent spec terms covered         │
  │  │  set difference  │  by children? Flag if < 80%.      │
  │  └─────────────────┘                                    │
  │                                                         │
  │  ┌─ cycle ─────────┐  Kahn's topological sort on       │
  │  │  O(V+E)         │  input/output dependency graph.    │
  │  │                  │  Flag if not all nodes visited.    │
  │  └─────────────────┘                                    │
  │                                                         │
  │  ┌─ budget ────────┐  Flag if projected nodes > 80%    │
  │  │                  │  of max_total_nodes with depth    │
  │  │                  │  remaining.                        │
  │  └─────────────────┘                                    │
  │                                                         │
  │  Also returns:                                          │
  │    complexity_score ── 0.0-1.0 parent complexity        │
  │    budget_allocation ─ per-child node budget             │
  │    dependency_order ── solve waves (toposort)            │
  └─────────────────────────────────────────────────────────┘
```

If flags are raised, the decompose subagent is re-prompted **once** with
the flags as context. One retry only.

### Step 3 — Solve

```bash
cat .fractal/{work-id}/manifest.json | synodic fractal schedule
```

The scheduler computes parallel waves via topological sort:

```
  Example: 4 leaves with dependencies

  auth ──► api ──► monitoring
  data ──┘

  Schedule output:
  ┌─────────────────────────────────────────┐
  │ Wave 1:  [ auth, data ]   ← parallel   │
  │ Wave 2:  [ api ]          ← depends on wave 1
  │ Wave 3:  [ monitoring ]   ← depends on wave 2
  │                                         │
  │ Critical path: data → api → monitoring  │
  │ Max parallelism: 2                      │
  └─────────────────────────────────────────┘
```

Three behaviors from one algorithm:
- All independent → single wave (fully parallel)
- All dependent → one per wave (fully sequential)
- Mixed → multiple waves with maximal parallelism

Each leaf gets a SOLVE subagent that works in an isolated git worktree
(for `output_mode: code`), producing a `SOLVE REPORT` with status,
files changed, summary, and interfaces.

### Step 3.5 — Solve Gate

Static checks on changed files (no AI cost):

| Language | Checks |
|----------|--------|
| Rust | `cargo check`, `cargo clippy -- -D warnings` |
| TypeScript/JS | `tsc --noEmit`, `eslint` |
| Python | `pyright`, `ruff check` |

Plus any rules in `.harness/rules/`. On failure: re-solve **once**.

If `solve_mode: factory`, each leaf goes through the full Factory
BUILD → INSPECT pipeline instead.

### Step 4 — Reunify

```bash
echo '{"base_ref":"main","children":[...],...}' | synodic fractal reunify
```

For `output_mode: code`, algorithmic reunification runs first:

```
  ┌─────────────────────────────────────────────────────────┐
  │                    REUNIFY MERGE                        │
  │                                                         │
  │  1. Structural checks (pure set operations):            │
  │     ┌────────────┐                                      │
  │     │ [boundary]  │ child modified sibling's files?     │
  │     │ [redundancy]│ multiple children touched same file?│
  │     │ [gap]       │ declared input not produced?        │
  │     └────────────┘                                      │
  │                                                         │
  │  2. Git 3-way merge (per child, in dependency order):   │
  │     git merge-tree --write-tree <base> <ours> <theirs>  │
  │                                                         │
  │  3. Classify conflicts:                                 │
  │     ┌──────────────────┬───────────────────────┐        │
  │     │ Auto-resolvable  │ Needs AI              │        │
  │     ├──────────────────┼───────────────────────┤        │
  │     │ rename/rename    │ content conflict       │        │
  │     │ modify/delete    │ add/add conflict       │        │
  │     │                  │ boundary violation     │        │
  │     │                  │ interface gap           │        │
  │     └──────────────────┴───────────────────────┘        │
  │                                                         │
  │  4. If needs_ai=false → MERGED (no AI call)             │
  │     If needs_ai=true  → spawn AI for specific conflicts │
  └─────────────────────────────────────────────────────────┘
```

For `output_mode: design` or `analysis`, AI reunification is used
directly (no algorithmic shortcut for prose merging).

### Step 4.5 — Reunify Rework

If CONFLICT or PARTIAL: re-solve **only** the conflicting children,
re-run solve gate, retry reunify **once**. Max 1 rework per node.

### Step 5 — Prune & Finalize

```bash
echo '{"tree": ...}' | synodic fractal prune
```

```
  ┌─────────────────────────────────────────────────────────┐
  │                      PRUNE GATE                         │
  │                                                         │
  │  1. Subset detection:                                   │
  │     auth: {auth.rs, session.rs}                         │
  │     full: {auth.rs, session.rs, api.rs}                 │
  │     → auth ⊂ full → auth is prunable                   │
  │                                                         │
  │  2. Empty output detection:                             │
  │     empty: {} → prunable (no file changes)              │
  │                                                         │
  │  3. Identical output detection:                         │
  │     alpha: {a.rs, b.rs}                                 │
  │     beta:  {a.rs, b.rs}                                 │
  │     → identical pair → keep one, prune the other        │
  │                                                         │
  │  4. Greedy set cover (O(ln n) approximation):           │
  │     Find minimal set of nodes covering all files.       │
  │     Nodes not in covering set are prune candidates.     │
  └─────────────────────────────────────────────────────────┘
```

Write `output.md`, update manifest with final metrics, append to
`.harness/fractal.governance.jsonl`.

## CLI Reference

All subcommands accept JSON from stdin or `--input <file>`, output
pretty JSON to stdout.

```
synodic fractal gate         Validate decomposition structurally
synodic fractal schedule     Schedule solves into parallel waves
synodic fractal reunify      Analyze reunification conflicts
synodic fractal prune        Detect redundant nodes
synodic fractal complexity   Score a spec's complexity (0.0-1.0)
```

### gate

Input: `DecomposeInput`

```json
{
  "parent_spec": "Build a platform with auth and data",
  "children": [
    {"slug": "auth", "scope": "OAuth2 auth", "inputs": "none", "outputs": "tokens"},
    {"slug": "data", "scope": "PostgreSQL", "inputs": "none", "outputs": "models"}
  ],
  "current_depth": 1,
  "max_depth": 3,
  "total_nodes": 3,
  "max_total_nodes": 20
}
```

Output: `DecomposeOutput`

```json
{
  "flags": [],
  "complexity_score": 0.175,
  "budget_allocation": {"auth": 5, "data": 3},
  "dependency_order": [["auth", "data"]]
}
```

### schedule

Input: `Manifest` (with `tree` containing leaf nodes)

Output: `ScheduleOutput`

```json
{
  "waves": [["auth", "data"], ["api"], ["monitoring"]],
  "critical_path": ["data", "api", "monitoring"],
  "critical_path_length": 3,
  "max_parallelism": 2,
  "total_leaves": 4
}
```

### reunify

Input: `ReunifyInput`

```json
{
  "base_ref": "main",
  "children": [
    {"slug": "auth", "branch": "fractal/auth", "files": ["src/auth.rs"]},
    {"slug": "data", "branch": "fractal/data", "files": ["src/data.rs"]}
  ],
  "dependency_order": [["auth", "data"]],
  "node_slug": "root"
}
```

Output: `ReunifyOutput`

```json
{
  "status": "MERGED",
  "auto_resolved": [],
  "conflicts": [],
  "merge_order": ["auth", "data"],
  "needs_ai": false
}
```

### prune

Input: `{"tree": {<manifest tree nodes>}}`

Output: `PruneOutput`

```json
{
  "prunable": ["empty"],
  "reasons": {"empty": "produced no file changes"},
  "kept": ["auth", "data", "api"],
  "file_coverage": {"src/auth.rs": ["auth"]},
  "identical_pairs": [],
  "minimal_covering_set": ["auth", "data", "api"]
}
```

### complexity

Input: plain text (spec content)

Output: `{"complexity_score": 0.612}`

## Algorithms

| Algorithm | Module | Used For | Complexity |
|-----------|--------|----------|:----------:|
| TF-IDF cosine similarity | `decompose.rs` | Orthogonality (scope overlap) | O(n^2 m) |
| Jaccard similarity | `mod.rs` | Fast pre-filter for orthogonality | O(m) |
| Kahn's topological sort | `decompose.rs` | Cycle detection | O(V+E) |
| BFS layer decomposition | `schedule.rs` | Parallel wave scheduling | O(V+E) |
| Longest-path DP | `schedule.rs` | Critical path analysis | O(V+E) |
| Weighted feature scoring | `decompose.rs` | Complexity estimation | O(n) |
| Proportional allocation | `decompose.rs` | Budget distribution | O(n) |
| git merge-tree (3-way) | `reunify.rs` | Code reunification | O(n)/file |
| Set intersection | `reunify.rs` | Boundary/redundancy detection | O(n) |
| Greedy set cover | `prune.rs` | Minimal covering set | O(n^2 m) |

Where n=nodes, m=terms, V=vertices, E=edges.

All are polynomial. Most are linear. The entire algorithmic spine adds
negligible overhead compared to a single AI subagent call.

### Classical Algorithm Analogs

```
  ┌────────────────────┬────────────────────────────────────┐
  │ Classical          │ Fractal Equivalent                 │
  ├────────────────────┼────────────────────────────────────┤
  │ Decision Tree      │ LEAF/SPLIT verdict with complexity │
  │ (split criterion)  │ score as feature-weighted prior    │
  │                    │                                    │
  │ MapReduce          │ Solve phase: map = parallel leaf   │
  │ (map/shuffle/      │ solves, shuffle = DAG scheduling,  │
  │  reduce)           │ reduce = bottom-up reunification   │
  │                    │                                    │
  │ MergeSort          │ Bottom-up reunify: children must   │
  │ (bottom-up merge)  │ all be solved before parent merges │
  │                    │                                    │
  │ QuickSort          │ Recursive decomposition with       │
  │ (partition+recurse)│ forced termination (budget cap)    │
  │                    │                                    │
  │ Random Forest      │ Multiple split strategies +        │
  │ (ensemble)         │ composability with speculative     │
  │                    │ swarm primitive                    │
  │                    │                                    │
  │ Set Cover          │ Prune gate: find minimal set of    │
  │ (NP-hard approx)   │ nodes covering all output files    │
  └────────────────────┴────────────────────────────────────┘
```

## Governance Integration

```
  Step 1.5  COMPLEXITY PRE-FILTER ──────► (no governance layer)
  Step 2.5  DECOMPOSE GATE ─────────────► Layer 1 (deterministic)
  Step 3.5  SOLVE GATE ─────────────────► Layer 1 (static checks)
            SOLVE GATE (factory mode) ──► Layer 2 (AI review)
  Step 4.5  REUNIFY REWORK ─────────────► Layer 2 (bounded rework)
  Step 5    ESCALATE (exhausted) ───────► Layer 3 (human review)
```

All checkpoints are **bounded**: 1 retry at DECOMPOSE, 1 retry per
leaf at SOLVE, 1 retry per node at REUNIFY. Unbounded rework in a
tree would be exponential.

## Composability

```
  ┌──────────────────────────────────────────┐
  │           Fractal → Factory              │
  │                                          │
  │  Set solve_mode: factory in .fractal.yaml│
  │  Each leaf sub-spec goes through the     │
  │  full Factory BUILD → INSPECT pipeline   │
  │  instead of a bare SOLVE subagent.       │
  └──────────────────────────────────────────┘

  ┌──────────────────────────────────────────┐
  │           Factory → Fractal              │
  │                                          │
  │  A Factory BUILD station can invoke      │
  │  /fractal decompose if it determines     │
  │  the spec is too complex for one pass.   │
  └──────────────────────────────────────────┘
```

## Manifest Schema

Node status lifecycle:

```
  pending ──► decomposed ──► solving ──► solved ──► reunifying ──► reunified
     │                          │                        │
     ▼                          ▼                        ▼
  forced-leaf                failed                   pruned
     │
     ▼
   leaf ──► solving ──► solved
```

Key manifest fields:

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | `fractal-{unix-timestamp}` |
| `status` | enum | decomposing, solving, reunifying, complete, failed |
| `config.max_depth` | int | 1-5, default 3 |
| `config.max_children` | int | 2-10, default 5 |
| `config.max_total_nodes` | int | 2-50, default 20 |
| `config.split_strategy` | enum | orthogonal, aspect-based, temporal |
| `config.reunification` | enum | lossless-merge, best-child, summary-merge |
| `config.solve_mode` | enum | parallel, sequential, factory |
| `config.output_mode` | enum | code, design, analysis |
| `tree.*` | TreeNode | slug, depth, status, scope, boundaries, inputs, outputs, children, files |
| `metrics.cycle_time_seconds` | number | Wall-clock time for full run |
| `metrics.tree_depth` | int | Actual depth achieved |
| `metrics.total_nodes` | int | Nodes in final tree |
| `metrics.leaf_nodes` | int | Leaves solved |
| `metrics.pruned_nodes` | int | Nodes removed by prune gate |
| `metrics.budget_enforced` | bool | True if max_total_nodes was hit |

## Test Coverage

| Module | Tests | Covers |
|--------|:-----:|--------|
| `fractal/mod.rs` | 3 | Term extraction, Jaccard similarity |
| `fractal/decompose.rs` | 9 | TF-IDF overlap, no-overlap, cycles, no-cycles, waves (linear/parallel), complexity scoring, budget allocation, coverage gaps |
| `fractal/schedule.rs` | 4 | All-independent, linear chain, diamond dependency, empty manifest |
| `fractal/reunify.rs` | 4 | No conflicts, file overlap, interface gap, merge ordering |
| `fractal/prune.rs` | 5 | Empty nodes, subset, identical pairs, no redundancy, covering set |
| **Total** | **25** | |

Run with:

```bash
cd cli && cargo test fractal
```

## Source Files

```
cli/src/
├── fractal/
│   ├── mod.rs           # shared types, NLP utilities (extract_terms, jaccard)
│   ├── decompose.rs     # TF-IDF gate, cycle detection, complexity, budget
│   ├── schedule.rs      # DAG wave scheduling, critical path
│   ├── reunify.rs       # git merge-tree, structural conflict detection
│   └── prune.rs         # set cover, subset/identity detection
├── cmd/
│   └── fractal.rs       # CLI subcommand dispatch (gate, schedule, reunify, prune, complexity)
│
skills/fractal/
├── SKILL.md             # orchestration protocol (what the AI agent follows)
├── evals/               # 7 behavioral evals
└── references/
    └── manifest.schema.json

specs/047-fractal-algorithmic-spine/
└── README.md            # design spec for the algorithmic spine
```
