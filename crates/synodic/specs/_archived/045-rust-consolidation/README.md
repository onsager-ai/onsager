---
status: complete
created: 2026-03-17
priority: high
tags:
- eval
- rust
- refactor
- architecture
created_at: 2026-03-17T00:00:00Z
updated_at: 2026-03-19T05:25:31.214209964Z
completed_at: 2026-03-19T05:25:31.214209964Z
---

# 045 — Rust Consolidation & Modularization

**Status:** Proposed
**Scope:** Consolidate eval harness from Shell+Python+Rust to Rust-primary
**Priority:** High (scoring bugs traced to untyped Python/shell seams)

## Problem

The eval harness has three orchestration layers that don't compose well:

1. **Rust CLI** (`cmd/eval.rs`) — Parses args, then shells out to bash scripts via `util::exec_script()`
2. **Shell scripts** (`run.sh`, `score.sh`, `swebench.sh`) — Real orchestration lives here
3. **Python** (`score_runner.py`) — Test execution, output parsing, verdict computation

This creates two problems:
- **Bug surface:** The scoring path (Python) is stringly-typed. All 4 scoring bugs (f2p/p2p inversion, Django format misparse, passed>total, missing error count) originated in untyped string parsing.
- **Leaky abstraction:** The Rust CLI is a thin wrapper around shell scripts. `synodic eval run` just builds an args array and calls bash. Two arg-parsing layers, zero type safety between them.

## Design

### Target Module Structure

```
cli/src/
├── main.rs
├── cmd/
│   ├── mod.rs
│   ├── eval.rs              # Arg parsing only, delegates to eval::run
│   └── harness.rs
├── eval/
│   ├── mod.rs
│   ├── run.rs               # Phase 1→2→3 orchestration (replaces run.sh)
│   ├── setup/
│   │   ├── mod.rs
│   │   ├── swebench.rs      # Replaces setup/swebench.sh
│   │   ├── featurebench.rs  # Replaces setup/featurebench.sh
│   │   └── devbench.rs      # Replaces setup/devbench.sh
│   ├── score/
│   │   ├── mod.rs
│   │   ├── runner.rs         # Subprocess execution (pytest/django-test)
│   │   ├── parser.rs         # Output parsing — pure functions, heavily tested
│   │   ├── verdict.rs        # Counting + invariant enforcement
│   │   └── report.rs         # JSON score report generation
│   ├── batch.rs              # Existing, unchanged
│   ├── report.rs             # Existing, unchanged
│   └── list.rs               # Existing, unchanged
├── harness/
│   └── ...                   # Existing, unchanged
└── util.rs
```

### Language Boundaries

| Layer | Current | Target |
|-------|---------|--------|
| CLI + Orchestration | Rust → bash → Python | Rust directly |
| Testbed setup (git, pip, venv) | bash | Rust `std::process::Command` |
| HuggingFace download | Inline Python in bash | Rust calls `evals/setup/download_task.py` (~40 lines) |
| Test execution | Python `subprocess.run` | Rust `Command` (pytest/django are just subprocesses) |
| Output parsing | Python regex on stdout | Rust regex with typed `TestResult` |
| Scoring/verdict | Python arithmetic | Rust with structural invariants |
| Batch + reporting | Rust (done) | Rust (unchanged) |

**Python stays only for:** HuggingFace `datasets` library (no viable Rust equivalent). One small script, called via `Command`.

### Key Types

```rust
/// Individual test outcome — no stringly-typed status
#[derive(Debug, Clone, PartialEq)]
enum TestStatus {
    Passed,
    Failed,
    Error,
    Skipped,
}

/// Single test result from parser
#[derive(Debug, Clone)]
struct TestResult {
    name: String,
    status: TestStatus,
    duration_ms: Option<u64>,
    output: Option<String>,
}

/// Aggregate score — the invariant is structural
#[derive(Debug)]
struct ScoreResult {
    passed: usize,
    failed: usize,
    errors: usize,
    skipped: usize,
}

impl ScoreResult {
    /// Total is always passed+failed+errors+skipped.
    /// There is no way to have passed > total.
    fn total(&self) -> usize {
        self.passed + self.failed + self.errors + self.skipped
    }

    fn pass_rate(&self) -> f64 {
        let t = self.total();
        if t == 0 { 0.0 } else { self.passed as f64 / t as f64 }
    }
}

/// Verdict for a test group (f2p or p2p)
#[derive(Debug)]
struct GroupVerdict {
    group: TestGroup,         // F2P or P2P — not a string
    expected: Vec<String>,    // Test IDs from task metadata
    results: Vec<TestResult>, // Actual outcomes
    score: ScoreResult,
}

/// Overall evaluation verdict
#[derive(Debug)]
struct EvalVerdict {
    instance_id: String,
    f2p: GroupVerdict,
    p2p: GroupVerdict,
    resolved: bool,           // f2p.score.passed == f2p.expected.len()
}
```

### Parser Design

`parser.rs` contains pure functions with no side effects:

```rust
/// Parse Django test runner output
fn parse_django_output(stdout: &str) -> Vec<TestResult>;

/// Parse pytest output (verbose mode)
fn parse_pytest_output(stdout: &str) -> Vec<TestResult>;

/// Parse pytest JUnit XML (more reliable than stdout)
fn parse_pytest_junit_xml(xml: &str) -> Vec<TestResult>;

/// Auto-detect format and parse
fn parse_test_output(stdout: &str, framework: TestFramework) -> Vec<TestResult>;
```

Each function is unit-tested with canned output strings covering:
- Normal pass/fail
- Error/exception during test
- Unicode in test names
- Empty output
- Truncated output
- Mixed pass/fail/error in same run

### Orchestration Flow (replaces run.sh)

`eval/run.rs`:

```rust
pub fn execute(opts: RunOptions) -> Result<EvalVerdict> {
    let target = resolve_target(&opts.alias)?;         // Alias resolution (was bash case statement)
    let testbed = setup_testbed(&target, &opts)?;      // Phase 1: setup (was swebench.sh)
    let _agent_output = invoke_agent(&testbed, &opts)?; // Phase 2: agent (was cd + pipe)
    let verdict = score(&testbed, &target)?;            // Phase 3: score (was score.sh + score_runner.py)
    Ok(verdict)
}
```

No shell scripts in the loop. Each phase returns a typed result or an error.

### Setup Modules (replace setup/*.sh)

`eval/setup/swebench.rs`:

```rust
pub fn setup(instance_id: &str, opts: &SetupOptions) -> Result<Testbed> {
    let task = download_task(instance_id, &opts.split)?;  // Calls download_task.py
    let repo = clone_repo(&task)?;                         // git clone + checkout via Command
    apply_test_patch(&repo, &task)?;                       // git apply via Command
    install_deps(&repo)?;                                  // pip install via Command (uses venv python path)
    let prompt = generate_prompt(&task, &opts.skill)?;     // String building (was heredoc in bash)
    Ok(Testbed { repo, task, prompt })
}
```

Venv handling: instead of `source venv/bin/activate`, we pass the full venv python path directly:
```rust
Command::new(format!("{}/venv/bin/python", testbed_dir))
    .args(["-m", "pytest", ...])
```

## Implementation Phases

### Phase 1: Port Scorer (Highest ROI)

Move `score_runner.py` → `eval/score/{parser,runner,verdict}.rs`.

- Port Django output parser with tests (canned strings from real runs)
- Port pytest parser with tests
- Port verdict computation with structural invariants
- Wire into existing `cmd/eval.rs` Score subcommand
- Keep shell scripts for setup/run phases (unchanged)

**Removes:** `score_runner.py`, `score.sh`
**Tests:** `cargo test` covers all parsing paths
**Risk:** Low — scoring is self-contained, no setup dependencies

### Phase 2: Port Orchestration

Move `run.sh` logic → `eval/run.rs`.

- Phase coordinator: setup → agent → score
- Alias resolution (the big `case` statement → a `HashMap` or match)
- Agent invocation via `Command`
- Wire `cmd/eval.rs` Run subcommand to call `eval::run::execute()` directly

**Removes:** `run.sh`
**Risk:** Medium — touches the main eval entry point

### Phase 3: Port Setup

Move `setup/swebench.sh` → `eval/setup/swebench.rs` (and featurebench, devbench).

- Git operations via `Command`
- Dependency installation via `Command` (with venv python path)
- Extract `download_task.py` as the minimal Python remnant
- Prompt generation as Rust string formatting

**Removes:** `setup/swebench.sh`, `setup/featurebench.sh`, `setup/devbench.sh`
**Keeps:** `evals/setup/download_task.py` (~40 lines, HuggingFace only)
**Risk:** Medium — environment setup is fragile by nature

### Phase 4: Cleanup

- Remove all `.sh` scripts from `evals/`
- Remove `score_runner.py`
- Update CI to just `cargo build && cargo test`
- Update any docs referencing shell scripts

## What We Gain

1. **Type safety on the bug-prone path.** Parser returns `Vec<TestResult>` with `enum TestStatus`, not strings. Verdict uses structural invariants — `passed > total` is impossible by construction.
2. **One orchestration layer.** `synodic eval run` does everything directly. No arg serialization to bash to Python.
3. **Testable parsing.** `#[test]` with canned Django/pytest output. No infrastructure needed.
4. **Single build.** `cargo build` produces everything. No "did you pip install?" failures.
5. **Unified error handling.** `anyhow::Result` propagates through all phases. No `|| true` hiding failures.

## What We Lose (Acceptable)

- **Shell prototyping speed** — but these scripts are past prototype stage; they have production bugs.
- **Python string convenience** — Rust regex is slightly more verbose but equally capable, and the compiler catches mistakes Python doesn't.
- **HuggingFace in-process** — we keep a 40-line Python script for this. Acceptable.

## Dependencies

- `regex` — output parsing
- `serde` / `serde_json` — task metadata, score reports
- `quick-xml` (optional) — JUnit XML parsing for pytest
- `anyhow` — error handling (already used)
- `clap` — CLI args (already used)

## Non-Goals

- Rewriting the HuggingFace download in Rust
- Changing the batch/report modules (already well-structured Rust)
- Changing the harness modules
- Adding async (subprocess orchestration is sequential by nature)
