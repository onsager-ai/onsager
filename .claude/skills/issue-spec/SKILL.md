---
name: issue-spec
description: Create lean-spec style GitHub issues as specs for human-AI aligned implementation on Onsager. Use when asked to "create a spec", "write a spec issue", "spec this feature", "spec this", or when planning work that needs a specification before implementation. Follows the lean-spec SDD methodology — small focused specs (<2000 tokens), intent over implementation, context economy. Creates GitHub issues with Overview, Design, Plan, Test, Alignment, and Notes sections. Paired with `onsager-dev-process` (the SDD loop), `onsager-pre-push` (pre-push checks), and `onsager-pr-lifecycle` (post-push).
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(git diff:*), Bash(git log:*), Bash(git show:*), mcp__github__issue_write, mcp__github__issue_read, mcp__github__list_issues, mcp__github__search_issues, mcp__github__sub_issue_write, mcp__github__get_label
---

# issue-spec

Create GitHub issues as lean-spec style specifications for human-AI aligned implementation on Onsager. GitHub issues are the sole spec medium — no spec files.

## Why GitHub Issues, Not Files

lean-spec uses Markdown files with YAML frontmatter for metadata. We replace that entirely with GitHub issues because:

- **Status** → Issue state (open/closed) + status labels (`draft`, `planned`, `in-progress`)
- **Priority** → Labels (`priority:critical`, `priority:high`, `priority:medium`, `priority:low`)
- **Tags** → Labels (`area:<subsystem>`, `feat`, `fix`, `refactor`, `perf`)
- **Dependencies** → Issue references (`depends on #42`) and sub-issues
- **Parent/Child** → Sub-issues via `mcp__github__sub_issue_write`
- **Transitions** → Issue timeline (automatic, auditable)
- **Collaboration** → Comments, reactions, assignments, mentions

GitHub gives us versioned metadata, collaboration, and relationship tracking for free. No CLI needed, no frontmatter to manage, no sync problems.

## Philosophy

Three principles from lean-spec:

1. **Context Economy** — Keep issue body under ~2000 tokens. Larger features split into parent + child issues. Small specs produce better AI output and better human review.
2. **Intent Over Implementation** — Document the *why* and *what*, not the *how*. Implementation details belong in PRs, not spec issues. The spec captures human intent that isn't in the code.
3. **Living Documents** — Specs evolve via issue comments and edits. Status labels track lifecycle. The issue thread becomes the decision record.

Plus one Onsager-specific principle:

4. **Reach ships with the primitive.** A spec that introduces a new user-facing resource (workspace, project, credential, session type — anything a user must create to use) must scope in the discovery surface: primary navigation entry, first-run flow for zero-instance users, empty-state CTAs on pages that expect instances, and auth gating on every touchpoint. Without those, the primitive is functionally present but invisible — users land in dead-empty states with no way to recover. This principle also applies to backend primitives that are only usable via a UI affordance (e.g. a new governance action needs the button that triggers it).

   **The "minimal v1" trap.** It's tempting to defer the surface ("we'll add the sidebar entry later"). That choice is almost always wrong. The cheap option — ship CRUD behind a Settings card, call it done — looks complete and is functionally invisible. The follow-up spec to add the surface ends up bigger than building it up front, and between the two PRs the feature ships as dead code. **Most of the time the cheap option is the bad decision.** Explicitly-scoped-out UX (a workspace switcher, role editor, invite flow) is fine to defer; *discoverability is not deferrable*.

   See [references/reach-checklist.md](references/reach-checklist.md) for the per-spec Plan items this implies.

## When to use this skill

Use when:
- A change touches multiple files or subsystems.
- Multiple stakeholders need alignment before implementation.
- The AI needs explicit boundaries for a non-trivial feature.
- Work will span multiple PRs (parent + child specs).

Skip when:
- A typo or doc-only fix. Use the `trivial` label on the PR instead.
- A one-line bug fix with an obvious reproduction. Just open a PR with `Fixes #existing`.
- The feature already has a spec issue — extend that spec, don't create another.

## Setup

| Parameter | Default | Example override |
|-----------|---------|-----------------|
| **Topic** | _(required)_ | `"session timeout"`, `"fix heartbeat race"` |
| **Scope** | Inferred from codebase | `"only stiglab"` |
| **Priority** | `medium` | `critical`, `high`, `low` |
| **Labels** | Auto from type + area | `"spec, feat, area:stiglab"` |
| **Parent** | None | `#42` (umbrella issue) |

If the user says "spec session timeout", start immediately. Do not ask clarifying questions unless the topic is genuinely ambiguous.

## Workflow

```
1. Discover     Search existing issues and codebase
2. Design       Draft the spec issue body
3. Align        Partition human decisions vs AI work
4. Validate     Self-check before creating
5. Publish      Create GitHub issue (+ sub-issues if splitting)
```

### 1. Discover

Before writing anything, understand what exists:

- Search existing GitHub issues for related or duplicate specs
- Grep codebase for types, functions, modules related to the topic
- Read key files that will be affected
- Check git log for recent changes in the area

If a related spec issue already exists, reference it — don't duplicate.

### 2. Design

Read [references/spec-format.md](references/spec-format.md) for the section-by-section format guide.

Draft the issue body using the lean-spec structure:

```markdown
## Overview
Problem statement and motivation. Why does this matter?

## Design
Technical approach: data flow, API changes, architecture decisions.
Keep it high-level — intent, not implementation.

## Plan
- [ ] Checklist of concrete deliverables
- [ ] Each item independently verifiable
- [ ] Order reflects implementation sequence

## Test
- [ ] How to verify each plan item
- [ ] Include: unit tests, integration tests, manual checks

## Notes
Tradeoffs, context, references. Optional — omit if empty.
```

**Context economy check**: If the issue body exceeds ~2000 tokens, split it:
- Create a parent issue with Overview + high-level Plan
- Create child issues (sub-issues), one per independent concern
- Each child has its own Design, Plan, Test sections
- Link children to parent via `mcp__github__sub_issue_write`

### 3. Align

Add an **Alignment** section to the issue body (this extends lean-spec for human-AI collaboration):

```markdown
## Alignment

### Human decides
- [ ] Architectural tradeoffs, scope, UX, go/no-go

### AI implements
- [ ] Concrete code tasks tied to Plan items

### Open questions
> Items that block AI implementation until a human decides
```

**Rules:**
- Every Plan item maps to either "Human decides" or "AI implements"
- If an item requires both, split it — the decision part is human, the execution is AI
- Open questions use `>` blockquotes so they're visually distinct
- Once a human answers a question (via issue comment), update the Alignment section

### 4. Validate

Before creating the issue, self-check:

- [ ] Body is under ~2000 tokens (context economy)
- [ ] Overview explains *why*, not just *what*
- [ ] Design captures intent, not implementation details
- [ ] Plan items are concrete and independently verifiable
- [ ] Test items map to Plan items
- [ ] Human/AI boundaries are explicit — no "figure it out" items
- [ ] No duplicate of an existing issue
- [ ] Dependencies are referenced by issue number

### 5. Publish

Create the issue using `mcp__github__issue_write`:

**Title format**: `spec(<area>): <short description>`

Examples: `spec(stiglab): add session timeout`, `spec(dashboard): fix node status badge`, `spec(forge): enforce synodic fail policy`

**Labels**: Apply via the issue creation:
- `spec` — always, marks this as a spec issue
- Type: `feat`, `fix`, `refactor`, `perf`
- Area (see taxonomy below)
- Priority: `priority:critical`, `priority:high`, `priority:medium`, `priority:low`
- Status: `draft` (initial state)

**Sub-issues**: If this is a child of a parent spec, link it using `mcp__github__sub_issue_write`.

**After creating**, report to the user:
- Issue number and URL
- Token count estimate (flag if over 2000)
- Any open questions that need human decisions
- Sub-issue links if the spec was split

## Area label taxonomy (Onsager monorepo)

Pick one or more, aligned with `crates/` and `apps/`:

- `area:spine` — `onsager-spine` (event bus client library)
- `area:forge` — production line / artifact lifecycle
- `area:ising` — continuous improvement engine
- `area:stiglab` — agent session orchestration
- `area:synodic` — agent governance
- `area:dashboard` — React UI under `apps/dashboard`
- `area:onsager` — the `onsager` dispatcher CLI
- `area:infra` — CI, migrations, docker-compose, justfile, workflows
- `area:docs` — README, CLAUDE.md, specs

Respect the architectural invariant: a spec should not cross subsystem boundaries except via the spine event bus. If a spec requires changes in two subsystems, split it — one child spec per subsystem, parent tracks the end-to-end slice.

## Status Lifecycle via Labels

GitHub issue state (open/closed) combined with status labels:

```
open + draft  →  open + planned  →  open + in-progress  →  closed (complete)
```

- **draft**: Spec created, open questions may remain. AI wrote it, human hasn't reviewed.
- **planned**: Human reviewed, decisions made, ready for implementation. Remove `draft`, add `planned`.
- **in-progress**: Someone/something is actively working (PR opened). Remove `planned`, add `in-progress`. The `pr-spec-sync.yml` GitHub Actions workflow automates this transition on PR open.
- **closed**: All plan items done, tests passing. PR merge with `Closes #N` closes it automatically.

**Key rule**: `draft → planned` is the human-AI alignment gate. A spec moves to `planned` only after a human reviews it and resolves open questions. The AI does not flip this label unprompted.

## Spec Relationships via Sub-Issues

Use GitHub sub-issues for parent/child decomposition:

| Relationship | GitHub mechanism | When to use |
|-------------|-----------------|-------------|
| **Parent/Child** | Sub-issues (`mcp__github__sub_issue_write`) | Large feature decomposed into pieces |
| **Depends On** | Issue body reference (`depends on #N`) | Spec blocked until another finishes |
| **Related** | Issue body reference (`related: #N`) | Loosely connected specs |

**Decision rule**: Remove the dependency — does the spec still make sense? If no → sub-issue (child). If yes but blocked → depends on.

**Example decomposition:**
```
spec(stiglab): session lifecycle improvements       ← parent issue
├── spec(stiglab): session timeout mechanism        ← sub-issue
├── spec(stiglab): session retry on failure         ← sub-issue
└── spec(dashboard): timeout warning indicator      ← sub-issue
```

## Guidance

- **Small is better.** A 500-token spec that captures intent clearly beats a 3000-token spec that tries to cover everything. Split into sub-issues early.
- **Discover first.** Always search existing issues before creating. Duplicate specs create confusion.
- **Status labels reflect reality.** Don't label `planned` if decisions are still open. Don't label `in-progress` until a PR is open.
- **One concern per issue.** If a spec covers two independent changes, split into sub-issues with a shared parent.
- **Reference code, not concepts.** Point to actual types, functions, files — not abstract ideas. Use `crates/stiglab/src/core/session.rs` not "the session module."
- **Open questions are alignment points.** These are where AI must stop and ask a human. Make them explicit, specific, and include the impact of each decision.
- **Comments are the decision record.** When a human resolves an open question, they comment on the issue. The thread becomes the audit trail.
- **Use specs for alignment, not for everything.** Regular bugs and small tasks don't need specs. Use specs when: multiple stakeholders need alignment, intent needs persistence, or the AI needs clear boundaries.

## Handoff to implementation

Once a spec moves to `planned`:

1. Create a branch referencing the issue: `claude/spec-<N>-<slug>` or similar.
2. Follow the SDD loop in `onsager-dev-process`.
3. Pre-push via `onsager-pre-push` (includes a spec-link check).
4. PR body must include `Closes #N` (slice complete) or `Part of #N` (scaffolding).
5. The `pr-spec-sync.yml` workflow flips the issue to `in-progress` on PR open; the `pr-merged-progress` routine handles merge.

## References

| Reference | When to Read |
|-----------|--------------|
| [references/spec-format.md](references/spec-format.md) | Always — section-by-section guide with examples |

## Templates

| Template | Purpose |
|----------|---------|
| [templates/issue-spec-template.md](templates/issue-spec-template.md) | Issue body template — copy and fill |
