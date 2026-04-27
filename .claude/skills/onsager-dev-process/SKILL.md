---
name: onsager-dev-process
description: The end-to-end spec-issue-driven dev loop for Onsager — spec → branch → implement → PR → merge → closure. Use when asked "how do I start work", "what's the process", "SDD loop", "spec-driven development", "how do we ship a change", "from scratch what do I do", or when you're about to begin a non-trivial change and haven't yet decided how to split spec/PR. Delegates to `issue-spec` (spec writing), `onsager-pre-push` (pre-push checks), `onsager-pr-lifecycle` (post-push), and the GitHub-triggered Claude Routines under `.claude/routines/`.
---

# onsager-dev-process

The spec-issue-driven development (SDD) loop on Onsager. Every non-trivial
change starts as a GitHub spec issue, proceeds through a PR that references
it, and closes when the PR merges. When Claude Routines are configured,
status labels on the issue track progress automatically; otherwise follow
the manual fallback documented in `onsager-pr-lifecycle`. Humans only touch
the `draft → planned` alignment gate.

## The loop

```
     ┌─────────────────────────────────────────────────────────────────┐
     │                                                                 │
     │   idea/request                                                  │
     │        ↓                                                        │
     │   spec(<area>): ...    ← issue-spec skill                       │
     │        │                                                        │
     │        │ label: draft                                           │
     │        ↓                                                        │
     │   human review   ← alignment gate (human sets label=planned)    │
     │        │                                                        │
     │        │ label: planned                                         │
     │        ↓                                                        │
     │   spec-planned-review routine  ← sanity check                   │
     │        │                                                        │
     │        ↓                                                        │
     │   branch + implement                                            │
     │        │                                                        │
     │        ↓                                                        │
     │   onsager-pre-push skill  ← merge preview, strict warnings      │
     │        │                                                        │
     │        ↓                                                        │
     │   git push → open PR (body: "Closes #N" or "Part of #N")        │
     │        │                                                        │
     │        │ pr-spec-sync workflow → label: in-progress             │
     │        ↓                                                        │
     │   onsager-pr-lifecycle skill  ← CI triage, review, iterate      │
     │        │                                                        │
     │        ↓                                                        │
     │   merge                                                         │
     │        │                                                        │
     │        │ pr-merged-progress routine → tick Plan items / close   │
     │        ↓                                                        │
     │   spec closed (Closes) OR Plan items ticked (Part of)           │
     │                                                                 │
     └─────────────────────────────────────────────────────────────────┘
```

## Stages

### 1. Write the spec

Trigger `issue-spec` (or say "spec this"). It creates a GitHub issue with:

- `## Overview`, `## Design`, `## Plan`, `## Test`, `## Alignment`, `## Notes`
- Labels: `spec`, one `area:*`, one type (`feat`/`fix`/`refactor`/`perf`),
  one `priority:*`, status `draft`.
- Open questions under `### Open questions` in the Alignment section.

Hard rule: no spec → no PR, unless the PR is labeled `trivial` (typos,
doc-only fixes, one-line obvious bug repair).

Body size: <~2000 tokens. Larger features split into parent + sub-issues
via `mcp__github__sub_issue_write`. The SDD loop runs independently on each
sub-issue; the parent tracks overall progress.

### 2. The alignment gate (`draft → planned`)

Only a human moves the `draft` label to `planned`. This signals:

- Open questions resolved (answered via comments; Alignment section
  updated).
- Design approach approved.
- Scope and priority accepted.

If the `spec-planned-review` routine is configured, it posts a sanity-check
comment when the label flips. If the routine flags issues, fix them before
starting implementation.

Never bypass this gate automatically. An AI may draft the spec and propose
the flip; it may not execute the flip.

### 3. Branch and implement

Branch naming convention:

- Human-owned branches: any name.
- Claude-owned branches: `claude/spec-<N>-<slug>` or `claude/<descriptor>`.
  The harness enforces the `claude/` prefix on cloud routines.

Implement the spec's Plan items in order. Keep commits small and focused.
Commit messages should be imperative and under 72 chars.

Respect the architectural invariant: subsystems (`forge`, `stiglab`,
`synodic`, `ising`) do not import each other. The seam rule (canonical
form, also persisted in root `CLAUDE.md` and each subsystem's
`CLAUDE.md`) makes this concrete:

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `synodic`, `ising`) coordinate
> **exclusively** via the spine: events on the bus + reads against
> shared spine tables. No subsystem makes HTTP calls to another
> subsystem. No subsystem imports another subsystem's crate.

If implementation surfaces a need to break this rule — a sibling-port
HTTP call, a `*_mirror.rs` translator, a `serde(alias)` shim, a "for
compat" type alias — stop. The right move is one of: emit an event +
listener pair (cross-subsystem coordination), collapse the schema into
the spine with a discriminator (shared state), or land the rename in
one PR (no aliases). If the spec doesn't yet describe that path,
update the spec first; do not add a bridge "for now". Spec #131 Lever
B will hard-fail these in CI, so a bridge that ships today is a
revert tomorrow.

The `onsager-pre-push` skill includes a seam-rule self-check that
scans the diff for these violations before push.

### 4. Pre-push

Trigger `onsager-pre-push` (or say "ready to push"). It runs:

1. Sync `origin/main` into the branch (CI tests a merge preview, not the
   branch alone). If that surfaces merge conflicts, `onsager-pre-push`
   owns the resolution walkthrough — inventory, pattern-match against
   the repo's recurring collisions (migrations, enum variants, event
   envelope, lockfiles), verify, then commit. Resolve locally, never on
   the PR web editor.
2. `RUSTFLAGS="-D warnings" cargo build --workspace`.
3. `RUSTFLAGS="-D warnings" cargo test --workspace --lib`.
4. `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings`.
5. `cargo fmt --all --check`.
6. Verify a spec issue is linked (this skill is what enforces the
   no-PR-without-spec rule locally).

Fix any blocker; don't paper over with `#[allow(dead_code)]` or `--no-verify`.

If a PR is already open and GitHub later flags "This branch has
conflicts", don't use the web editor — `onsager-pr-lifecycle` covers
the checkout-and-rerun flow that re-uses the same pre-push walkthrough.

### 5. Open the PR

PR body must begin with a linking line:

| PR delivers                                       | Use            |
| ------------------------------------------------- | -------------- |
| The full spec / acceptance test / vertical slice  | `Closes #N`    |
| A bug fix for a specific defect                   | `Fixes #N`     |
| Scaffolding / one phase of a multi-phase spec     | `Part of #N`   |
| Related work that shouldn't close the spec        | `Refs #N`      |

Under `## Delivers`, list the Plan items this PR ticks (exact text from the
spec's Plan). The `pr-merged-progress` routine uses this to tick checkboxes.

If the PR is genuinely trivial (typo, doc-only, one-line obvious fix),
apply the `trivial` label and skip the spec-linking requirement. Use
sparingly — if reviewers flag it as needing context, escalate to a spec.

**Decide before opening, not after.** The `pr-spec-sync.yml` workflow
posts a "no spec link / no `trivial` label" comment on every PR that
opens without one of the two. To keep that bot silent, answer the gate
at PR creation: pass the `Closes #N` / `Part of #N` line in the PR
body, or pass `labels: ["trivial"]` to `mcp__github__create_pull_request`.
Don't push the PR and let the bot ask.

### 6. During review

Trigger `onsager-pr-lifecycle` (or say "triage PR" / "CI is failing" /
respond to a webhook). It covers:

- CI triage: what `cargo build` failed for (merge preview? migration
  collision? enum variant removed on main?).
- Review-comment discipline: fix the code, don't reply per comment.
- Copilot vs real defects.
- Webhook subscription to stream CI + review events.

The `pr-spec-sync` workflow has already flipped the spec to `in-progress`.
No human action needed on labels during review.

### 7. Merge

- `Closes #N` PRs auto-close the spec on merge.
- `Part of #N` / `Refs #N` PRs leave the spec open; the
  `pr-merged-progress` routine ticks the delivered Plan items and, if all
  sub-issues of a parent are closed, pings the parent.
- The `in-progress` label disappears when the issue closes; for parents,
  a human closes the parent once the spec is end-to-end verified.

### 8. Closed-unmerged path

If you close a PR without merging (e.g. abandoned approach), the
`pr-spec-sync` workflow checks whether any other PR still references the
spec. If none, it flips the spec back to `planned` so the next
implementer can pick it up.

## The `trivial` escape hatch

Not every change needs a spec. The `trivial` label on a PR explicitly opts
out. Use for:

- Typos in comments, docs, commit messages.
- One-line obvious bug fixes where the repro is in the diff itself.
- Formatting-only changes (`cargo fmt`, `prettier`).
- Dependency version bumps (unless they break APIs).

Do NOT use for:

- Anything touching multiple files.
- Anything in `crates/` that changes behavior.
- Anything that could plausibly merit a follow-up.

When in doubt, write the spec. A 200-token spec is cheap; a merged change
with no spec is invisible to the next maintainer.

## Issue progress is the source of truth

The labels on a spec issue must reflect reality at all times:

| Label | Meaning |
|-------|---------|
| `draft` | AI-drafted or human-drafted, human review pending. |
| `planned` | Ready for implementation. Preconditions met. |
| `in-progress` | At least one PR is open against this spec. |
| (closed) | All Plan items delivered, spec closed. |

The `pr-spec-sync` workflow handles the open and close-unmerged
transitions; the `pr-merged-progress` routine handles Plan-item ticks on
merge. If the merged-progress routine is disabled, the
`onsager-pr-lifecycle` skill documents the manual transitions.

## Anti-patterns (don't)

- **PR without a spec and no `trivial` label.** The `pr-spec-sync`
  workflow will comment; the PR should not merge until the author either
  adds a spec link or the `trivial` label.
- **Moving `draft → planned` as the AI.** Human-only transition.
- **Closing a spec manually when you meant `Closes #N`.** Let GitHub do it
  via the PR merge so the timeline has the auditable link.
- **Editing Plan checkboxes to mark items done before the PR merges.** The
  routine ticks them on merge. Editing them early breaks the audit trail.
- **Cross-subsystem PRs.** If a PR touches two subsystems (other than
  spine), it should have been split at the spec stage. Stop, split the
  spec, split the PR.
- **Skipping `onsager-pre-push`.** CI failures cost more time than the
  checklist does.
- **Shipping a primitive without its discovery surface.** Specs that
  introduce a new user-facing resource (workspace, project, credential —
  anything a user must create to use) must scope in navigation entry,
  first-run flow, empty-state CTAs, and auth gating in the *same* spec.
  Deferring the surface to a follow-up ships dead code in the interim
  and usually costs more overall. See `issue-spec`'s
  [reach-checklist](../issue-spec/references/reach-checklist.md).
  The cheap option (ship CRUD behind a hidden Settings card) is almost
  always the wrong one.

## Delegation map

| Stage | Skill / routine |
|-------|-----------------|
| Write the spec | [`issue-spec`](../issue-spec/SKILL.md) |
| Sanity-check `planned` specs | [`spec-planned-review`](../../routines/spec-planned-review.md) |
| Pre-push checks | [`onsager-pre-push`](../onsager-pre-push/SKILL.md) |
| On PR open → flip to `in-progress` | [`pr-spec-sync.yml`](../../../.github/workflows/pr-spec-sync.yml) |
| CI triage, review, iterate | [`onsager-pr-lifecycle`](../onsager-pr-lifecycle/SKILL.md) |
| On PR merge → tick Plan items | [`pr-merged-progress`](../../routines/pr-merged-progress.md) |
| On PR close (unmerged) → revert label | [`pr-spec-sync.yml`](../../../.github/workflows/pr-spec-sync.yml) |

Routines live at [claude.ai/code/routines](https://claude.ai/code/routines);
their version-controlled prompts are in `.claude/routines/`. See that
directory's `README.md` for setup.
