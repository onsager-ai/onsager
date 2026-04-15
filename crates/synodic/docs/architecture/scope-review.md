# Synodic Scope Review — March 2026

## Problem Statement

Synodic has 43 specs (~8,140 lines of design) and ~4 commits of implementation.
The spec-to-code ratio is unsustainable. We need to trim scope, archive premature
specs, and focus on proving the core thesis before designing the cathedral.

## Key Insight: Claude Code Already Has Orchestration Built In

As of March 2026, Claude Code ships with multi-agent orchestration features that
overlap heavily with what Synodic was designing:

| Synodic Concept | Claude Code Built-in | Status |
|----------------|---------------------|--------|
| BUILD station | Core agent loop | Production |
| INSPECT station | `/simplify` (3 parallel review agents) | Production |
| Rework loops | Native test-fix iteration | Production |
| Parallel builds | `/batch` (5-30 worktree agents) | Production |
| Fleet coordination | Agent Teams (lead + N teammates) | Experimental |
| Process supervisor | Built into Agent Teams | Experimental |
| Message bus | Agent Teams mailbox + shared task list | Experimental |
| Cost routing | `--model` flag per spawned instance | Production |
| Metrics capture | `--output-format json` (tokens, turns) | Production |
| Subprocess mgmt | Subagents (Explore, Plan, general-purpose) | Production |
| Git isolation | `claude -w branch` (worktree sessions) | Production |

### What Claude Code Still Lacks (Synodic's Real Niche)

The gaps that remain are narrow but real:

1. **Assembly line semantics** — `/batch` is parallel, not sequential-staged.
   No concept of "this unit must pass BUILD → INSPECT → HARDEN in order."
2. **Adversarial review by design** — `/simplify` reviews within the same
   context. The builder's blind spots leak through. True adversarial review
   requires a separate subagent with its own context and no builder bias.
3. **Spec-to-PR pipeline** — No "give me a spec, produce a PR" one-shot
   workflow with structured verdict protocol.
4. **Cross-session metrics** — Each session tracks its own tokens, but no
   unified metrics across a factory batch.
5. **Structured rework protocol** — No APPROVE/REWORK verdict parsing with
   specific rework items routed back to the builder.

### Why Subagents Are the Right Primitive

General-purpose subagents have **all tools** (Read, Write, Edit, Bash, Grep,
Glob) and run in their **own context window**. This makes them ideal for both
BUILD and INSPECT:

| | Subagent | `claude -p` subprocess |
|---|---|---|
| Context isolation | Own window (clean, no builder bias) | Fully separate process |
| File changes | Direct or worktree-isolated | Separate worktree |
| Tool access | All tools (general-purpose) | All tools |
| Cost | Lower (results summarized back) | Higher (full independent session) |
| Coordination | Returns results to parent naturally | Must parse stdout JSON |
| Resumable | Yes (agent ID) | Must manage session IDs manually |
| Git operations | Full (branch, commit, push) | Full |
| Limitation | Cannot spawn sub-subagents | Cannot nest either |

The no-nesting constraint is not a problem: BUILD (implement + test + commit)
and INSPECT (review diff against spec) are each single focused tasks that
don't need further delegation.

## Decision: Skill-First Approach

**Synodic becomes a skill package, not a standalone binary.**

The Rust codebase and Node.js wrapper are unnecessary — Claude Code's built-in
subagents provide all the orchestration primitives needed. General-purpose
subagents have full tool access (Read, Write, Edit, Bash) and run in isolated
context windows. Synodic's value is the **workflow definition**, not the runtime.

A skill orchestrates subagents as pipeline stations:

1. Read the target spec (skill context)
2. Spawn **BUILD subagent** (`isolation: worktree`) — implements code, runs
   tests, commits to feature branch
3. Spawn **INSPECT subagent** (fresh context) — reviews diff against spec
   with no builder bias, returns structured verdict
4. Parse verdict: APPROVE → PR, REWORK → re-invoke BUILD with feedback
5. Record metrics to JSON manifest (file I/O)
6. On APPROVE: create PR via `gh`

No `claude -p` subprocess spawning needed. No JSON stdout parsing. The main
conversation is just an orchestration loop over subagent calls.

For parallel execution, compose with `/batch` or Agent Teams rather than
reimplementing subprocess management.

**What changes:**

| Before | After |
|--------|-------|
| Rust binary orchestrating Claude Code | Skill orchestrating built-in subagents |
| npm platform packages for distribution | `npx skills add` for distribution |
| Custom message bus, state persistence | Subagent results + file manifests |
| 43 specs across 5 architectural layers | Skills + minimal supporting specs |

## Revised Core Thesis

> A Claude Code skill that implements BUILD → INSPECT with adversarial review
> (separate agent instance) produces measurably better results than a single
> agent run, with acceptable overhead.

## What to Build

### Phase 1 — Factory Skill (the MVP)

**One skill: `factory`**

```
/factory run specs/038-factory-mvp/README.md
```

The skill orchestrates two subagents:

```
Main conversation (orchestrator)
  │
  ├─→ BUILD subagent (general-purpose, isolation: worktree)
  │     • Reads spec, implements code
  │     • Runs tests, fixes failures
  │     • Commits to factory/{work-id} branch
  │     • Returns: files changed, test results, tokens used
  │
  ├─→ INSPECT subagent (general-purpose, fresh context)
  │     • Reads diff + spec only (no builder context = adversarial)
  │     • Reviews correctness, security, completeness
  │     • Returns: VERDICT: APPROVE or VERDICT: REWORK + items
  │
  └─→ Orchestration loop
        • If REWORK: re-invoke BUILD with rework items (max 3 loops)
        • If APPROVE: record metrics, create PR via gh
```

The skill:
1. Reads the target spec
2. Spawns BUILD subagent in worktree (`isolation: worktree`)
3. BUILD implements, tests, commits to `factory/{work-id}` branch
4. Spawns INSPECT subagent with fresh context (diff + spec only)
5. Parses verdict: APPROVE or REWORK with specific items
6. If REWORK: re-invokes BUILD with feedback (max 3 loops)
7. Records metrics: cycle time, tokens, rework count, first-pass yield
8. On APPROVE: creates PR via `gh`

**Skill structure:**
```
skills/factory/
├── SKILL.md              # Skill definition (AgentSkills.io format)
├── references/
│   └── manifest.schema.json   # Work manifest schema
├── fixtures/
│   └── sample-spec/           # Test spec for validation
└── evals/
    ├── evals.json             # Behavioral evals
    └── prompts/               # Eval prompts
```

**Exit criteria:**
- Skill runs end-to-end on a real spec
- Independent review catches deliberate bugs
- Rework loop fires and fixes issues
- Metrics are recorded to `.factory/{work-id}/manifest.json`
- At least 3 real specs processed successfully

### Phase 2 — Parallel Execution (if Phase 1 validates)

**Compose with existing Claude Code parallelism:**
- Use `/batch` for independent specs (5-30 worktree agents)
- Or Agent Teams for coordinated multi-spec work
- Aggregate metrics from `--output-format json` across sessions
- Report throughput (specs/hour)

No custom parallel execution needed — Claude Code already has this.

### Phase 3 — Cost Routing (if Phase 2 validates)

- Use `model: haiku` on BUILD subagent for simple tasks, `model: opus` for INSPECT
- Measure cost-per-spec vs quality tradeoff
- Use `maxTurns` to cap per-task cost

---

## Spec Disposition

### KEEP (Rewrite for skill-first)

| # | Spec | Action |
|---|------|--------|
| 037 | Coding Factory Vision | Rewrite as skill-first vision. Trim to phases 1-2. |
| 038 | Factory MVP | Rewrite as skill spec, not Rust binary spec. |

### DEFER (After skill MVP validates)

| # | Spec | Why Defer |
|---|------|-----------|
| 001 | Workspace Persistence | Useful for cross-session memory, not needed for MVP |
| 036 | Competitive Analysis | Reference material, no action needed |
| 039 | Assembly Line Abstraction | Extract patterns from working skill, don't pre-design |
| 040 | Quality System | After basic pipeline works |
| 041 | Metrics Dashboard | After metrics are being collected |

### ARCHIVE (No longer relevant to skill-first approach)

**All 36 remaining specs.** The entire fleet execution layer (002-010), coordination
theory (011-035), and advanced factory features (042-043) were designed for a
platform architecture that we're not building.

| Range | Category | Count |
|-------|----------|-------|
| 002-006 | Fleet Execution | 5 |
| 007-010 | Auth & RBAC | 4 |
| 011-035 | Coordination Theory & Primitives | 25 |
| 042-043 | Advanced Factory | 2 |
| **Total** | | **36** |

The existing `coordination-model` skill (in spec 017) can remain as a standalone
skill if its concepts prove useful — but it's not a prerequisite for the factory
skill.

### DELETED (Dead infrastructure — removed March 2026)

The following were removed as part of the skill-first pivot:

| Path | Why |
|------|-----|
| `crates/` | Rust binary no longer needed |
| `packages/cli/` | npm platform wrapper no longer needed |
| `scripts/` | Publishing scripts for binary distribution |
| `.github/workflows/publish.yml` | Binary publishing pipeline |
| `publish.config.ts` | Forge publishing config |
| `Cargo.toml`, `Cargo.lock` | Rust workspace |
| `turbo.json`, `pnpm-workspace.yaml` | Build orchestration for removed packages |
| `specs/harness-integration/` | One-time implementation prompts (not LeanSpec) |
| `specs/harness-spec/` | Empty stub |

**Kept and simplified:** `.github/workflows/ci.yml`, `.lean-spec/`, `AGENTS.md`, `package.json`.

---

## Resulting Project Shape

```
synodic/
├── skills/
│   └── factory/
│       ├── SKILL.md
│       ├── references/
│       ├── fixtures/
│       └── evals/
├── specs/
│   ├── 037-coding-factory-vision/    # Rewritten
│   ├── 038-factory-mvp/              # Rewritten
│   └── (archived specs remain)
├── .github/workflows/ci.yml          # Skill validation
├── .lean-spec/
├── AGENTS.md
├── README.md                          # Rewritten
└── package.json                       # Simplified
```

**From: Rust+Node.js hybrid platform with 43 specs**
**To: One skill with 2 active specs and behavioral evals**

---

## Recommended Next Steps

1. **Write the `factory` SKILL.md** — Define the skill in AgentSkills.io format
2. **Create manifest schema** — JSON schema for work item tracking
3. **Build evals** — Behavioral evals that verify the skill works end-to-end
4. **Test on real specs** — Run the skill on 3-5 existing specs in this repo
5. **Measure** — Compare factory-produced code vs single-agent code on same tasks
6. **Decide on Rust cleanup** — Remove or archive the binary infrastructure

---

## Claude Code Primitives Used (March 2026 Reference)

### Subagent capabilities (production-stable)

| Feature | Detail |
|---------|--------|
| Tool access | General-purpose: ALL tools (Read, Write, Edit, Bash, Grep, Glob) |
| Context | Own isolated context window (no parent bleed) |
| Worktree isolation | `isolation: "worktree"` for git-isolated file changes |
| Resumable | Agent ID returned, can resume with full context |
| Model override | `model: sonnet\|opus\|haiku` per subagent |
| Turn limits | `maxTurns: N` to cap iterations |
| Custom agents | `.claude/agents/` with tool allowlists and system prompts |

### Built-in skills to compose with

| Skill | What It Does |
|-------|-------------|
| `/batch` | Parallel worktree agents (5-30 units) |
| `/simplify` | 3-agent parallel code review |
| `/loop` | Recurring execution on interval |

### CLI flags (for Phase 2+ parallel execution)

| Flag | Purpose |
|------|---------|
| `-p` / `--print` | Non-interactive mode (scriptable) |
| `--output-format json` | Structured output with metadata |
| `--model` | Per-instance model selection |
| `--max-turns N` | Limit agent iterations |
| `--max-budget-usd N` | Cost cap per session |

## Risks

**Skill-first risks:**
- Subagent cannot spawn sub-subagents (no nesting) — BUILD and INSPECT
  must each be self-contained single tasks
- Skill control flow relies on the main conversation parsing subagent
  results and making routing decisions
- Worktree cleanup if BUILD subagent fails mid-implementation

**Mitigations:**
- BUILD and INSPECT are naturally self-contained (no nesting needed)
- Main conversation orchestration loop is simple: spawn → parse → route
- Worktree isolation handles cleanup automatically on no changes
- Fall back to `claude -p` subprocess if subagent limitations emerge
- Keep archived specs as reference if we need to escalate complexity later
