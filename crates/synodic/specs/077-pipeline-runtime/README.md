---
status: draft
created: 2026-04-02
priority: critical
tags:
- orchestration
- pipeline
- runtime
- strategy
- governance
depends_on:
- "067"
- "072"
- "076"
created_at: 2026-04-02T00:00:00Z
updated_at: 2026-04-02T00:00:00Z
---

# Synodic Pipeline Runtime & Strategic Positioning

> **Status**: draft · **Priority**: critical · **Created**: 2026-04-02

## Overview

Synodic needs to evolve from a governance-only tool into a governed pipeline runtime that works for individual developers and corporate teams, while carving defensible positioning against major players (Claude Code, Copilot, Cursor).

This spec addresses three concerns:

1. **Technical** — `synodic run` as the pipeline executor, `pipeline.yml` as single source of truth
2. **Corporate** — How Synodic adapts to multi-repo, multi-team, compliance-driven environments
3. **Competitive** — Where Synodic sits relative to Claude Code, Copilot, Cursor, and what the moat is

## Strategic Analysis

### What major players have

| Capability | Claude Code | GitHub Copilot | Cursor/Windsurf |
|---|---|---|---|
| Agent loop | Hooks, sub-agents, Agent SDK | Copilot Workspace, coding agent | IDE-native agent loops |
| PR automation | Via hooks + workflow | Creates PRs from issues | IDE-driven commits |
| Quality checks | User-configured hooks | GitHub Actions | IDE linters |
| Governance/audit | PreToolUse hooks (stateless) | None | None |
| Learning from history | None (each session starts blank) | None | None |
| Rule evolution | None | None | None |
| Cross-session memory | None | None | None |

### What none of them have

**Persistent governance intelligence.** Every session starts from zero. No tool tracks:
- "Rule X blocked 47 dangerous commands this month with 2% false positive rate"
- "Your agent has a pattern of writing to /etc — here's a candidate rule"
- "This check has 100% pass rate across 30 runs — crystallize it to a git hook"
- "Override cluster analysis: 80% of overrides on rule Y are because of non-production environments"

### Synodic's positioning

```
Don't compete on orchestration (commodity).
Compete on governance that learns.
```

Synodic is to AI agents what **audit logging + adaptive RBAC** is to cloud infrastructure. You need it, nobody else provides it, and it works with whatever agent tools you already use.

The pipeline executor (`synodic run`) is not the moat — it's the **distribution mechanism** for governance. Making governance the default requires owning the loop. The moat is what happens inside:

1. **Persistent rule learning** — Rules accumulate evidence (Bayesian alpha/beta), converge, get promoted or deprecated
2. **Adversarial probing** — Testing rules for bypass vulnerabilities (5 strategies implemented)
3. **Crystallization** — L2 pattern-based rules graduating to L1 deterministic git hooks
4. **Override forensics** — Clustering override reasons, detecting when rules are wrong vs. context-specific
5. **Pipeline telemetry** — Every check failure becomes a governance event, building institutional knowledge

### Competitive strategy

**Phase 1 (now):** Synodic wraps Claude Code. `synodic run` invokes `claude --print`. Users get governance for free by using Synodic's pipeline instead of raw Claude Code.

**Phase 2 (6mo):** Agent-agnostic ingestion. Accept governance events from Claude Code hooks, GitHub Actions, GitLab CI, Copilot sessions. Synodic becomes the governance layer for any AI coding tool.

**Phase 3 (12mo):** Corporate platform. Shared governance DB, org-level policies, central dashboard, compliance reporting. The data moat — Synodic has months of governance intelligence that no competitor can replicate.

## Corporate Adaptation

### The enterprise problem

Individual developers want: "set it up, run pipelines, get PRs."

Corporate teams need:
- **Multi-repo governance** — Shared rules across 50 repos
- **Policy hierarchy** — Org policies that individual repos can't override
- **Compliance audit trail** — Immutable log of every agent action, block, override
- **Approval gates** — No auto-merge without human review
- **Central visibility** — Dashboard showing all agent activity across the org
- **Role separation** — Admins set rules, developers work under them

### Design: hierarchical `pipeline.yml`

```yaml
# .harness/pipeline.yml (repo-level)
extends: "https://github.com/myorg/.synodic-policies/main/pipeline.yml"
# or: extends: "s3://myorg-governance/base-policy.yml"

language: rust

checks:
  - name: format
    run: "cargo fmt --all -- --check"
    fix: "cargo fmt --all"
    stage: commit
  - name: test
    run: "cargo test"
    stage: push

# Org policy may add checks that repos cannot remove
# Repo can add checks but not weaken org requirements
```

The org-level policy:

```yaml
# Org: .synodic-policies/pipeline.yml
governance:
  # Rules that all repos inherit (cannot be disabled locally)
  required_rules:
    - secrets-in-args
    - destructive-git
    - writes-outside-project

  # Minimum check requirements
  required_stages:
    - lint
    - test

pipeline:
  auto_merge: false              # Org policy: never auto-merge
  max_rework: 5                  # Org default, repos can lower

audit:
  storage: "postgresql://governance.internal/synodic"
  immutable: true                # Events cannot be deleted
  retention_days: 365
```

**Resolution order:** Org policies are **additive and floor-setting**. A repo can add checks or make settings stricter, but cannot weaken org requirements. `synodic run` merges configs and enforces the union.

### Design: shared governance backend

```
Repo A (SQLite local)  ──┐
Repo B (SQLite local)  ──┼──▶  Central PostgreSQL
Repo C (SQLite local)  ──┘       (aggregated events)
                                       │
                                       ▼
                              ┌─────────────────┐
                              │  Dashboard       │
                              │  - Org overview  │
                              │  - Rule health   │
                              │  - Compliance    │
                              └─────────────────┘
```

Each repo keeps a local SQLite for fast interception (<100ms). Pipeline events are also forwarded to a central PostgreSQL for aggregation. This is configured via:

```yaml
# .harness/pipeline.yml
audit:
  forward_to: "postgresql://governance.internal/synodic"
```

Or via environment variable: `SYNODIC_CENTRAL_DB=postgresql://...`

### Design: compliance & audit trail

For regulated industries (finance, healthcare, government), Synodic provides:

1. **Immutable event log** — Every agent action, block, override with timestamp, actor, session ID
2. **Override justification** — Overrides require a reason (already implemented in intercept.sh)
3. **Approval gates** — `auto_merge: false` enforced at org level
4. **Export** — `synodic export --since 30d --format csv` for compliance reporting
5. **Retention** — Configurable retention period, no silent deletion

This is a natural extension of the existing governance DB. The data model (rules, feedback_events, probe_reports) already captures what compliance needs.

### Corporate adoption path

```
Phase 1: Bottom-up (individual developer)
  Developer finds Synodic, runs `synodic init`, likes it
  → No org involvement needed

Phase 2: Team adoption
  Developer shares pipeline.yml with team
  Team uses shared PostgreSQL for governance DB
  → `synodic status` shows team-wide metrics

Phase 3: Org rollout
  Admin creates org policy repo (.synodic-policies)
  Repos use `extends:` to inherit org rules
  Central dashboard aggregates all repos
  → Compliance team gets audit trail + reports
```

**Critical:** Phase 1 must be excellent before Phase 2 is possible. Nobody brings a tool to their team if they don't love it individually.

## Technical Design

### `pipeline.yml` as single source of truth

```yaml
# .harness/pipeline.yml
language: rust

checks:
  - name: format
    run: "cargo fmt --all -- --check"
    fix: "cargo fmt --all"
    stage: commit                 # → pre-commit hook
  - name: lint
    run: "cargo clippy --all-targets -- -D warnings"
    stage: push                   # → pre-push hook
  - name: test
    run: "cargo test"
    stage: push                   # → pre-push hook

pipeline:
  max_rework: 3
  auto_merge: false
  branch_prefix: "synodic"
```

**`stage` field** controls git hook generation:
- `commit` → check runs in `.githooks/pre-commit`
- `push` → check runs in `.githooks/pre-push`
- omitted → check runs only during `synodic run` INSPECT phase

Everything derives from this file:
- `synodic run` reads checks + pipeline settings
- `synodic init` generates git hooks from checks with `stage: commit|push`
- `synodic init` generates GHA workflow that calls `synodic run`
- `synodic run --dry-run` replaces custom CI check scripts

### `synodic run` — the pipeline executor

```
synodic run --prompt "add rate limiting"
```

**State machine:**

```
          ┌──────────────────────────────────────┐
          │                                      │
   ┌──────▼──────┐         ┌──────────┐   ┌─────┴──────┐
   │    BUILD    │────────▶│  INSPECT  │──▶│   ROUTE    │
   │ (claude)    │         │ (checks)  │   │            │
   └─────────────┘         └──────────┘   └─────┬──────┘
                                                 │
                                 ┌───────────────┼──────────────┐
                                 │               │              │
                           all pass         any fail       exhausted
                                 │               │              │
                           ┌─────▼────┐   ┌─────▼────┐  ┌─────▼────┐
                           │CREATE PR │   │ REWORK   │  │  FAIL    │
                           └──────────┘   └──────────┘  └──────────┘
```

**Phases:**

1. **INIT** — Read pipeline.yml. Create branch. Record run start.
2. **BUILD** — Invoke `claude --print` with prompt (+ rework feedback if looping). L2 hooks active.
3. **INSPECT** — Run each check. Record results as governance events.
4. **ROUTE** — All pass → PR. Any fail + budget remaining → rework. Exhausted → fail.
5. **PR** — Push branch, create PR via `gh pr create`. Optionally auto-merge.

**Feedback wiring (automatic):**

```
INSPECT: "cargo test" fails
  → internally records: feedback --signal ci_failure --rule ci-test
  → governance DB accumulates check statistics
  → synodic status reports: "cargo test failed 12 times in 30 days"
```

**CLI:**

```
synodic run [OPTIONS]

Required:
  --prompt <TEXT>       Task description

Options:
  --max-rework <N>      Override pipeline.yml (default: from config)
  --auto-merge          Override pipeline.yml (default: from config)
  --branch <NAME>       Custom branch name
  --dry-run             Run INSPECT only (skip BUILD + PR)
  --local               Skip PR creation (run BUILD+INSPECT only)
  --dir <PATH>          Project directory
```

### Generated GHA workflow (simplified)

```yaml
name: Synodic Pipeline
on:
  workflow_dispatch:
    inputs:
      prompt:
        description: "Task description"
        required: true
        type: string

jobs:
  pipeline:
    runs-on: ubuntu-latest
    timeout-minutes: 60
    permissions: { contents: write, pull-requests: write }
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }

      # Language-specific setup (generated by synodic init)
      - uses: dtolnay/rust-toolchain@stable
        with: { components: "clippy, rustfmt" }

      - run: npm install -g @codervisor/synodic @anthropic-ai/claude-code

      - run: synodic run --prompt "${{ inputs.prompt }}"
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
```

~25 lines vs the current ~340.

### Crate changes

**harness-core** — new `pipeline` module:

```
harness-core/src/pipeline/
├── mod.rs       # pub mod config, checks, runner
├── config.rs    # PipelineConfig, Check, Stage — parse pipeline.yml
├── checks.rs    # run_checks() — execute checks, capture output
└── runner.rs    # run_pipeline() — Build→Inspect→Route state machine
```

Key types:

```rust
pub struct PipelineConfig {
    pub language: String,
    pub checks: Vec<Check>,
    pub pipeline: PipelineSettings,
}

pub struct Check {
    pub name: String,
    pub run: String,
    pub fix: Option<String>,
    pub stage: Stage,
}

pub enum Stage { Commit, Push, None }

pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub output: String,
    pub duration_ms: u64,
}

pub enum RunOutcome {
    Passed { attempts: u32, pr_url: Option<String> },
    Failed { attempts: u32, last_feedback: String },
    Error(String),
}
```

**harness-cli** — new `run.rs` command, simplified `init.rs`.

**Dependencies:** Add `serde_yaml` to harness-core.

## Plan

### Phase 1: Pipeline config + check runner (harness-core)

- [ ] Add `serde_yaml` dependency
- [ ] Implement `pipeline/config.rs` — parse pipeline.yml
- [ ] Implement `pipeline/checks.rs` — run checks as subprocesses, capture output
- [ ] Unit tests for config parsing and check execution

### Phase 2: Pipeline state machine (harness-core)

- [ ] Implement `pipeline/runner.rs` — Build→Inspect→Route loop
- [ ] Wire feedback events to storage during INSPECT
- [ ] Support dry-run mode (INSPECT only)
- [ ] Integration tests

### Phase 3: `synodic run` command (harness-cli)

- [ ] Create `cmd/run.rs`
- [ ] Wire into main.rs (visible command, alongside init/status/rules)
- [ ] Print structured progress output
- [ ] Test end-to-end with dry-run

### Phase 4: Simplify `synodic init`

- [ ] Generate pipeline.yml as source of truth
- [ ] Derive git hooks from pipeline.yml `stage` fields
- [ ] Generate simplified GHA workflow
- [ ] Keep L2 hooks generation (unchanged)

### Phase 5: `extends` support (corporate)

- [ ] Parse `extends:` field in pipeline.yml
- [ ] Fetch remote policy file (HTTPS/S3)
- [ ] Merge configs: org is floor, repo adds on top
- [ ] `synodic run` enforces merged config

### Phase 6: Event forwarding (corporate)

- [ ] `audit.forward_to` config in pipeline.yml
- [ ] Forward governance events to central PostgreSQL alongside local SQLite
- [ ] `synodic export` command for compliance reporting

## Test

### Config
- [ ] Parse valid pipeline.yml with all fields
- [ ] Parse minimal pipeline.yml (language + one check)
- [ ] Missing file → error with "run synodic init" suggestion
- [ ] Invalid YAML → clear error

### Check runner
- [ ] Passing check → CheckResult { passed: true }
- [ ] Failing check → CheckResult { passed: false, output: stderr }
- [ ] Multiple checks → all results collected

### Pipeline
- [ ] Dry-run: all pass → Passed
- [ ] Dry-run: failure → Failed with feedback
- [ ] Full run: pass on attempt 1 → PR created
- [ ] Full run: fail then pass on attempt 2 → PR created
- [ ] Exhausted budget → Failed
- [ ] Feedback events recorded to DB

### Init
- [ ] Generates pipeline.yml + hooks + workflow
- [ ] Pre-commit contains only stage:commit checks
- [ ] Pre-push contains only stage:push checks
- [ ] Workflow calls `synodic run`

### Corporate (Phase 5-6)
- [ ] `extends:` fetches and merges org policy
- [ ] Org `required_rules` cannot be removed by repo
- [ ] Org `auto_merge: false` cannot be overridden by repo
- [ ] Events forwarded to central DB when configured

## Notes

### Why `synodic run` is not competing with Claude Code

Claude Code provides the agent. Synodic provides the governed loop around it. They're complementary:

```
synodic run
  └── invokes claude --print (Claude Code is the BUILD agent)
  └── runs INSPECT checks (Synodic owns the quality gate)
  └── records governance events (Synodic's unique value)
```

If Claude Code adds a native "run and create PR" feature, Synodic's `run` command can wrap it or sit alongside it. The governance layer (rules, feedback, scoring, probing, crystallization) is orthogonal to how the agent is invoked.

### Why not a YAML pipeline engine

Specs 038-043 designed a sophisticated YAML-driven pipeline engine with stations, conveyors, fan/branch/loop primitives. This spec deliberately avoids that:

1. **YAGNI** — The factory Build→Inspect→PR loop covers 90%+ of real use cases
2. **Maintenance** — A DSL interpreter is a product in itself (see: GitHub Actions, Tekton, Argo)
3. **Focus** — Synodic's value is governance, not orchestration. A simple state machine in Rust is sufficient for the loop; complexity budget goes into the governance layer

If a generic pipeline engine is ever needed, it belongs in `codervisor/orchestra`, not Synodic.

### Migration from current generated workflows

Existing users keep working. `synodic init` generates the new simplified workflow. Old 300-line workflows continue to function since they don't depend on `synodic run`. Migration is optional and non-breaking.
