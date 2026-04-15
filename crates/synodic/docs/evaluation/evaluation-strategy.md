# Synodic Benchmark Evaluation Strategy

## Primary Benchmark: SWE-bench Pro

**SWE-bench Pro** (Scale AI, 2025) is our primary evaluation benchmark. It measures
real-world software engineering ability on tasks that current agents consistently fail —
exactly the gap that Synodic's orchestration (Factory, Fractal) targets.

### Why SWE-bench Pro over alternatives

| Property | SWE-bench Verified | FeatureBench | **SWE-bench Pro** |
|----------|-------------------|--------------|-------------------|
| SOTA | ~80% (saturating) | ~45% | **~23%** |
| Contamination resistance | None | Low (public repos) | **High (GPL + private set)** |
| Task scope | 1-2 files, localized | 5-15 files, features | **4+ files, diverse** |
| Languages | Python only | Python only | **Python, Go, TS, JS** |
| Task diversity | Bug fixes only | Feature additions | **Bugs, features, security, perf** |
| Tasks | 500 | ~500 | **1,865** |
| Test rigor | Modified tests only | F2P + P2P | **Full suite, 3x rerun** |
| Status | Deprecated | Active | **Active (primary)** |

### What makes SWE-bench Pro hard

1. **Multi-file edits**: average solution touches 4.1 files and changes 107 lines
2. **Enterprise codebases**: Django, Flask, scikit-learn, Go standard libraries, etc.
3. **Contamination-resistant**: public set uses GPL repos (legal deterrent), private set
   uses proprietary codebases (physically unseen by models)
4. **Full-suite testing**: all tests must pass (not just modified ones), with 3 reruns
   to eliminate flakiness

### The contamination problem

The 57-point drop from SWE-bench Verified (~80%) to SWE-bench Pro (~23%) is the
clearest evidence that contamination inflates benchmark scores. SWE-bench Pro addresses
this with:

- **Public set** (731 instances): repos with strong copyleft licenses (GPL) — legal
  deterrent against inclusion in training data
- **Private set** (276 instances): proprietary codebases from startups — models have
  literally never seen this code

Performance drops further on the private set (Claude Opus 4.1: 22.7% → 17.8%;
GPT-5: 23.1% → 14.9%), confirming that even the GPL public set has some leakage.

### Why orchestration should help here

SWE-bench Pro tasks fail because agents lose coherence across files. This is precisely
what Synodic's orchestration addresses:

- **Fractal decomposition** splits multi-file fixes into per-concern sub-problems
- **Worktree isolation** gives each solver a clean scope
- **Reunification** merges fixes with cross-file consistency checks
- For simple tasks, decomposition correctly detects them as LEAFs (no overhead)

The hypothesis: orchestrated multi-agent solving should improve resolve rates on the
hard tail (3+ file tasks) without regressing on simple tasks.

## Secondary Benchmarks

### FeatureBench (ICLR 2026)

- **Role**: Complementary eval for feature addition tasks
- **Dataset**: `LiberCoders/FeatureBench` (~500 tasks)
- **Strength**: Tests feature implementation (additive code, not just bug fixes)
- **Weakness**: Python-only, public repos (contamination risk), lower task diversity
- **Use**: Validates Factory skill's BUILD → INSPECT pipeline on feature tasks

We keep FeatureBench evals for Factory skill validation where the task is explicitly
"implement this new feature." But it is not the primary signal.

### DevBench (2024)

- **Role**: Complementary eval for greenfield project construction
- **Dataset**: `open-compass/DevBench` (22 projects, 4 languages)
- **Strength**: Most natural fit for fractal decomposition — PRDs decompose into modules
- **Weakness**: Small dataset, subjective scoring, <40% SOTA
- **Use**: Tests deep decomposition (depth 3+) and cross-module integration

DevBench remains the best eval for Fractal skill's deepest capability — building entire
projects from specs — but the dataset is too small for reliable metrics.

### SWE-bench Verified (deprecated)

- **Status**: Deprecated — do not use for new evaluations
- **Reason**: Saturated (~80% SOTA), contamination-prone, Python-only
- **Use**: Control group only — verify that orchestration does not regress on easy tasks

## Benchmark-to-Skill Mapping

| Benchmark | Primary skill | What it tests |
|-----------|--------------|---------------|
| SWE-bench Pro | Fractal | Multi-file decomposition on hard bug fixes |
| SWE-bench Pro | Factory | Single-file BUILD → INSPECT on localized fixes |
| FeatureBench | Factory | Feature implementation quality and review rigor |
| DevBench | Fractal | Deep decomposition and cross-module integration |
| SWE-bench Verified | (control) | Overhead tolerance — no regression on easy tasks |

## Evaluation Infrastructure

### Running SWE-bench Pro evals

```bash
# Run with fractal decomposition
./evals/run.sh swe:<instance-id> --split pro --skill fractal

# Same task with factory
./evals/run.sh swe:<instance-id> --split pro --skill factory

# Baseline (no skill)
./evals/run.sh swe:<instance-id> --split pro --skill baseline

# Score only
./evals/score.sh <instance-id> --testbed-dir /tmp/swebench-testbed/<instance-id>

# Dry run
./evals/run.sh swe:<instance-id> --split pro --dry-run
```

### Running FeatureBench evals

```bash
# Run with any skill
./evals/run.sh fb:<alias> --skill fractal
./evals/run.sh fb:<alias> --skill factory

# Score only
./evals/score.sh <instance-id>
```

### Running DevBench evals

```bash
# Run DevBench project
./evals/run.sh dev:<project-name> --skill fractal

# Score only
./evals/score-devbench.sh <project-name>
```

### Scoring protocol

All benchmarks use the same pass criteria:

| Test type | Description | Required |
|-----------|-------------|----------|
| **F2P** (Fail-to-Pass) | Tests that should pass after the fix/feature | Yes |
| **P2P** (Pass-to-Pass) | Tests that already passed — no regressions | Yes |

`resolved = F2P_all_pass AND P2P_all_pass`

SWE-bench Pro adds: full test suite must pass (not just F2P/P2P), with 3 reruns per
instance to eliminate flaky tests.

## Evaluation Priorities

When allocating eval compute, prioritize in this order:

1. **SWE-bench Pro (public)** — primary signal, largest dataset, contamination-resistant
2. **SWE-bench Pro (private)** — gold standard for generalization (requires Scale API access)
3. **FeatureBench** — validates feature addition capability
4. **DevBench** — validates deep decomposition
5. **SWE-bench Verified** — control group only, run sparingly

## Key Metrics to Track

| Metric | Definition | Target |
|--------|-----------|--------|
| **Resolve rate** | % of tasks where F2P + P2P all pass | > SOTA baseline |
| **Decomposition accuracy** | LEAFs for simple tasks, splits for complex | > 90% correct classification |
| **Overhead tolerance** | Resolve rate on simple tasks vs. no-decomposition baseline | No regression |
| **Private set delta** | Public resolve rate − private resolve rate | Minimize gap |
| **Cost per resolve** | Total API spend / resolved tasks | Track trend |

## References

- [SWE-bench Pro Leaderboard](https://labs.scale.com/leaderboard/swe_bench_pro_public) (Scale AI)
- [SWE-bench Pro Paper](https://openreview.net/forum?id=9R2iUHhVfr) (OpenReview)
- [FeatureBench](https://huggingface.co/datasets/LiberCoders/FeatureBench) (ICLR 2026)
- [DevBench](https://github.com/open-compass/DevBench) (2024)
- [SWE-bench Original](https://www.swebench.com/) (Princeton NLP)
- [SWE-bench Contamination Analysis](https://metr.org/notes/2026-03-10-many-swe-bench-passing-prs-would-not-be-merged-into-main/) (METR)
