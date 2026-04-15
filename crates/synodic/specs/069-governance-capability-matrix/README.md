---
status: draft
created: 2026-03-29
priority: critical
tags:
- governance
- hooks
- interception
- audit
- architecture
created_at: 2026-03-29T00:00:00Z
updated_at: 2026-03-29T00:00:00Z
depends_on:
- "068"
---

# Governance Capability Matrix — Interception vs Audit, L1 vs L2

> **Status**: draft · **Priority**: critical · **Created**: 2026-03-29

## Overview

Spec 068 established that git hooks and CI are L1, Synodic is L2. This spec refines the model by splitting governance into two orthogonal axes: **interception** (real-time blocking before side effects) vs **audit** (post-hoc analysis), crossed with **L1** (deterministic) vs **L2** (semantic/AI). This produces a 2×2 capability matrix that defines exactly what Synodic can do for each agent tool.

### Why now

Claude Code Hooks (`PreToolUse`, `PostToolUse`) enable L2 interception — blocking agent actions *before* they execute, based on semantic rules. No other AI coding tool offers this today. This creates a split: Claude Code gets the full matrix; Cursor, Codex, Windsurf get a degraded but still valuable subset. Synodic needs to model this explicitly so it can deliver maximum value per tool and evolve as other tools open their hook APIs.

## Design

### The capability matrix

```
              Interception          Audit
              (pre-action block)    (post-hoc analysis)
            ┌─────────────────────┬─────────────────────┐
   L1       │ Git Hooks           │ CI Pipeline         │
   (static/ │ pre-commit/push     │ GitHub Actions      │
   determ.) │ lint, fmt, deps     │ quality signals,    │
            │ forbidden paths     │ error attribution,  │
            │                     │ trend detection     │
            ├─────────────────────┼─────────────────────┤
   L2       │ Agent Hooks         │ Session Analysis    │
   (AI/     │ Claude Code         │ Transcript review,  │
   semantic)│ PreToolUse rules    │ CI failure attrib., │
            │ intent validation   │ drift detection,    │
            │                     │ pattern mining      │
            └─────────────────────┴─────────────────────┘
```

### Per-tool capability profiles

| Tool | L1 Intercept | L2 Intercept | L1 Audit | L2 Audit |
|------|:---:|:---:|:---:|:---:|
| Claude Code | ✓ Git Hooks | ✓ PreToolUse | ✓ CI | ✓ JSONL logs |
| Cursor | ✓ Git Hooks | △ Emerging | ✓ CI | ✓ Log parsing |
| GitHub Copilot | ✓ Git Hooks | ✗ | ✓ CI | ✓ events.jsonl |
| Codex (CLI) | ✓ Git Hooks | ✗ | ✓ CI | ✓ Log parsing |
| Windsurf | ✓ Git Hooks | ✗ | ✓ CI | ✓ Log parsing |

**Key insight**: L1 (git hooks + CI) and L2 audit work for *every* tool. L2 interception is currently Claude Code-only but the adapter layer is designed so other tools can be added as they expose hook APIs.

### Quadrant definitions

#### L1 Interception — Git Hooks

Deterministic, zero-cost checks that block before side effects. Tool-agnostic because every agent commits through git.

**Mechanism**: `.githooks/pre-commit`, `.githooks/pre-push`
**Scope**: Format, lint, test, dependency rules, forbidden file paths
**Enforceability**: Advisory (bypassable with `--no-verify`), backed by CI as hard gate
**Synodic's role**: Provide hooks, collect failure events, detect failure patterns

#### L2 Interception — Agent Hooks

Semantic checks that evaluate agent *intent* before execution. Requires tool-specific hook APIs.

**Mechanism**: Claude Code `PreToolUse` hooks calling Synodic rule engine
**Scope**:
- "Don't modify production config without running tests first"
- "Mutations to auth module must include test file changes"
- "No file writes outside the project root"
- "Block destructive git operations on protected branches"

**Enforceability**: Hard block (hook returns `{"decision": "block"}`)
**Synodic's role**: Rule engine evaluates tool call context, returns allow/block decision

```yaml
# .claude/settings.json hook example
hooks:
  - matcher: "PreToolUse"
    command: "synodic intercept --tool $CLAUDE_TOOL_NAME --input $CLAUDE_TOOL_INPUT"
```

```json
// synodic intercept returns:
{ "decision": "allow" }
// or
{ "decision": "block", "reason": "Auth module changes require test coverage" }
```

#### L1 Audit — CI Pipeline

Aggregated quality signals from CI runs. Tool-agnostic, remote-enforced.

**Mechanism**: GitHub Actions, branch protection rules
**Scope**: Test results, coverage delta, lint errors, build status, dependency audit
**Synodic's role**: Collect CI signals as governance events, attribute failures to agent vs human commits, detect cross-PR architectural degradation trends

#### L2 Audit — Session Analysis

AI-powered post-hoc review of agent session transcripts and outputs.

**Mechanism**: `synodic collect` parses session logs; `synodic harness run` invokes AI judge
**Scope**:
- Goal drift (agent wandered from user intent)
- Inefficiency patterns (repeated failed attempts, circular debugging)
- Hallucination detection (references to nonexistent APIs/files)
- Compliance review (secrets in output, unsafe operations)

**Synodic's role**: This is Synodic's core unique value. No other tool does cross-session semantic analysis of agent behavior.

### Degradation model

For tools without L2 interception, Synodic operates in **audit-heavy mode**:

```
Full mode (Claude Code):        L1 intercept + L2 intercept + L1 audit + L2 audit
Degraded mode (Cursor, etc.):   L1 intercept +               L1 audit + L2 audit
```

The degraded mode is still valuable because:

1. **L2 audit findings crystallize into L1 rules.** When Synodic's AI judge detects a recurring pattern (e.g., "agent keeps modifying `deploy.yml` without updating version"), it becomes a candidate L1 rule. After backtest validation, it graduates to a git hook check — which works for *all* tools.

2. **CI remains a hard gate.** L1 audit via CI catches everything that reaches the remote, regardless of which agent produced it.

3. **The gap narrows over time.** As Cursor, Codex, etc. add hook APIs, Synodic's adapter layer lights up L2 interception for those tools. The rule definitions stay the same; only the delivery mechanism changes.

```
Crystallization flow:
  L2 Audit (pattern detected 3+ times)
    → Rule candidate
    → Backtest (>90% precision)
    → Promote to L1
    → Git hook (all tools benefit)
```

### Adapter architecture

L2 interception requires a per-tool adapter. Each adapter translates the tool's hook API into Synodic's rule evaluation interface.

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│ Claude Code  │     │   Cursor     │     │  Future Tool │
│  PreToolUse  │     │  (emerging)  │     │   hook API   │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │
       ▼                    ▼                    ▼
┌─────────────────────────────────────────────────────────┐
│              Synodic Adapter Layer                       │
│  translate(tool_hook_event) → InterceptRequest          │
└──────────────────────┬──────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────┐
│              Synodic Rule Engine                         │
│  evaluate(InterceptRequest) → Allow | Block(reason)     │
└─────────────────────────────────────────────────────────┘
```

**InterceptRequest** (tool-agnostic):
- `tool_name`: what the agent is trying to do (write_file, run_command, etc.)
- `tool_input`: the arguments (file path, command string, etc.)
- `session_context`: recent actions, current goal, modified files list

## Plan

### Phase 1: L2 interception for Claude Code

- [ ] Add `synodic intercept` CLI command
- [ ] Implement `InterceptRequest` and `InterceptResponse` types in harness-core
- [ ] Implement Claude Code adapter (translates PreToolUse env vars to InterceptRequest)
- [ ] Define interception rule format (YAML, distinct from detection rules)
- [ ] Ship 5 default interception rules (destructive git, prod config, auth without tests, secrets in args, writes outside project)
- [ ] Document `.claude/settings.json` hook setup

### Phase 2: Crystallization pipeline

- [ ] Track L2 audit findings with occurrence counts in GovernanceLog
- [ ] Implement pattern detection: group similar findings, count recurrences
- [ ] Auto-generate candidate L1 rules when pattern hits threshold (≥3)
- [ ] Backtest candidates against historical governance logs
- [ ] `synodic rules promote` command to graduate candidate to git hook

### Phase 3: CI audit integration

- [ ] GitHub Actions workflow that runs `synodic collect` on CI logs
- [ ] Attribute CI failures to originating agent/session
- [ ] Cross-PR trend detection (test coverage, lint error rate, build time)
- [ ] Surface CI audit findings in dashboard

### Phase 4: Adapter layer for additional tools

- [ ] Define adapter trait/interface
- [ ] Cursor adapter (when hook API stabilizes)
- [ ] Documentation for third-party adapter development

## Test

- [ ] `synodic intercept` blocks a destructive git command and returns block decision
- [ ] `synodic intercept` allows a benign file write and returns allow decision
- [ ] Claude Code PreToolUse hook integration works end-to-end (hook → synodic → block)
- [ ] Degraded mode: tools without L2 intercept still get L1 hooks + L1/L2 audit
- [ ] Crystallization: a pattern detected 3+ times generates a rule candidate
- [ ] Promoted rule works as a git hook check for all tools
- [ ] CI audit correctly attributes a test failure to an agent commit
- [ ] Adapter layer: Claude Code adapter produces valid InterceptRequest from env vars

## Notes

- Spec 068 removed Synodic's old L1 infrastructure. This spec reintroduces L1 *interception* via git hooks (not Synodic code) and adds L2 *interception* as a new Synodic capability via agent hooks.
- The `synodic intercept` command must be fast (<100ms) — it runs on every tool call in Claude Code. Rule evaluation should be pure pattern matching, no AI calls.
- Crystallization closes the loop: L2 audit → L1 rules → git hooks → all tools benefit. This means even tools without L2 interception improve over time as the project accumulates governance knowledge.
- As agent hook APIs emerge across tools, the adapter pattern ensures Synodic can expand L2 interception without changing the rule engine.
