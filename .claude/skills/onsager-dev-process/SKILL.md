---
name: onsager-dev-process
description: The end-to-end spec-issue-driven dev loop for Onsager — spec → branch → implement → PR → merge → closure. Use when asked "how do I start work", "what's the process", "SDD loop", "spec-driven development", "how do we ship a change", "from scratch what do I do", or when you're about to begin a non-trivial change and haven't yet decided how to split spec/PR. Delegates to `issue-spec` (spec writing) and the global `pre-push` (pre-push checks) and `pr-lifecycle` (post-push) skills. This skill carries Onsager's overlay for `pre-push` / `pr-lifecycle` — the check gate, merge-collision patterns, CI-failure table, and the bundled GitHub-ops reference + audit script.
---

# onsager-dev-process

The spec-issue-driven development (SDD) loop on Onsager. Every non-trivial
change starts as a GitHub spec issue, proceeds through a PR that references
it, and closes when the PR merges. Issue open/closed is the only lifecycle
state — status labels (`draft`, `planned`, `in-progress`) were retired.
Plan-checkbox ticks, umbrella tracker refresh, and `main-red` issue
maintenance are documented in the global `pr-lifecycle` and `ci-triage`
skills.

## The loop

```
     ┌─────────────────────────────────────────────────────────────────┐
     │                                                                 │
     │   idea/request                                                  │
     │        ↓                                                        │
     │   spec(<area>): ...    ← issue-spec skill                       │
     │        │                                                        │
     │        ↓                                                        │
     │   branch + implement                                            │
     │        │                                                        │
     │        ↓                                                        │
     │   pre-push skill        ← merge preview, strict warnings        │
     │        │                                                        │
     │        ↓                                                        │
     │   git push → open PR (body: "Closes #N" or "Part of #N")        │
     │        │                                                        │
     │        ↓                                                        │
     │   pr-lifecycle skill    ← CI triage, review, iterate            │
     │        │                                                        │
     │        ↓                                                        │
     │   merge                                                         │
     │        │                                                        │
     │        │ Closes #N → GitHub auto-closes spec                    │
     │        │ Part of #N → tick Plan items manually (pr-lifecycle)   │
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
  one `priority:*`.
- Open questions under `### Open questions` in the Alignment section.

Hard rule: no spec → no PR, unless the PR is labeled `trivial` (typos,
doc-only fixes, one-line obvious bug repair).

Body size: <~2000 tokens. Larger features split into parent + sub-issues
via `mcp__github__sub_issue_write`. The SDD loop runs independently on each
sub-issue; the parent tracks overall progress.

### 2. Resolve open questions

Before opening a PR, resolve any open questions on the spec issue thread.
A spec with unanswered `### Open questions` is not ready to implement —
its design isn't pinned yet.

### 3. Branch and implement

Branch naming convention:

- Human-owned branches: any name.
- Claude-owned branches: `claude/spec-<N>-<slug>` or `claude/<descriptor>`.
  The harness enforces the `claude/` prefix on cloud sessions.

Implement the spec's Plan items in order. Keep commits small and focused.
Commit messages should be imperative and under 72 chars.

Respect the architectural invariant: subsystems (`forge`, `stiglab`,
`ising`) do not import each other. The seam rule (canonical
form, also persisted in root `CLAUDE.md` and each subsystem's
`CLAUDE.md`) makes this concrete:

> HTTP APIs exist only at external boundaries:
> - **User-facing endpoints** called by the dashboard.
> - **Webhooks** called by external services (GitHub, etc.).
>
> Subsystems (`forge`, `stiglab`, `ising`) coordinate
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

Onsager's pre-push gate (the overlay below, run via the global
`pre-push` skill) includes a seam-rule self-check — `just lint-rust`
runs `xtask lint-seams` + `check-api-contract` — that scans the diff
for these violations before push.

### 4. Pre-push

Trigger the global `pre-push` skill (or say "ready to push"). It owns
the generic flow — sync the merge preview, the conflict walkthrough
(inventory, pattern-match, verify, commit; Onsager's recurring
collisions are in the overlay below), the spec-link check, the push.

Onsager's check gate (the overlay below) is:

1. `RUSTFLAGS="-D warnings" cargo build --workspace`.
2. `RUSTFLAGS="-D warnings" cargo test --workspace --lib`.
3. `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings`.
4. `cargo fmt --all --check`.
5. `just lint-rust` (seam + API-contract lints + `_check-tools-and-skills`).

Fix any blocker; don't paper over with `#[allow(dead_code)]` or `--no-verify`.

If a PR is already open and GitHub later flags "This branch has
conflicts", don't use the web editor — the global `pr-lifecycle`
covers the checkout-and-rerun flow that re-uses the same pre-push
walkthrough.

### 5. Open the PR

PR body must begin with a linking line:

| PR delivers                                       | Use            |
| ------------------------------------------------- | -------------- |
| The full spec / acceptance test / vertical slice  | `Closes #N`    |
| A bug fix for a specific defect                   | `Fixes #N`     |
| Scaffolding / one phase of a multi-phase spec     | `Part of #N`   |
| Related work that shouldn't close the spec        | `Refs #N`      |

Under `## Delivers`, list the Plan items this PR ticks (exact text from the
spec's Plan). After merge, tick those checkboxes manually on the parent
spec — see the global `pr-lifecycle` skill.

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

Trigger the global `pr-lifecycle` skill (or say "triage PR" / "CI is
failing" / respond to a webhook). It covers:

- CI triage: delegates the taxonomy to `ci-triage`; the Onsager
  failure table (merge preview? migration collision? enum variant
  removed on main?) is in the overlay below.
- Review-comment discipline: fix the code, don't reply per comment;
  Copilot vs real defects.
- Webhook subscription + the post-push CI sweep (commit-status vs
  check_run gap).

No label flips during review — the spec stays open until its PR(s) close
it.

### 7. Merge

- `Closes #N` PRs auto-close the spec on merge.
- `Part of #N` / `Refs #N` PRs leave the spec open; tick the delivered
  Plan items manually on the parent spec, and if all sub-issues of a
  parent are closed, ping the parent. See the global `pr-lifecycle`.
- A human closes the parent once the spec is end-to-end verified.

### 8. Closed-unmerged path

If you close a PR without merging (e.g. abandoned approach), the spec
issue stays open as-is — the next implementer can pick it up from there.

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

A spec issue's open/closed state plus its Plan checkboxes are the source
of truth. Use `Closes #N` only on a PR that delivers the final unticked
Plan items, so GitHub's auto-close fires once the spec is actually
complete; use `Part of #N` for partial slices that leave items behind,
then tick the delivered checkboxes manually on merge. If a multi-PR spec
finishes via `Part of` PRs only, a human closes the parent once the last
Plan item ticks. Plan-item ticks on merge are manual; the
global `pr-lifecycle` skill documents the procedure.

## Anti-patterns (don't)

- **PR without a spec and no `trivial` label.** The `pr-spec-sync`
  workflow will comment; the PR should not merge until the author either
  adds a spec link or the `trivial` label.
- **Closing a spec manually when you meant `Closes #N`.** Let GitHub do it
  via the PR merge so the timeline has the auditable link.
- **Editing Plan checkboxes to mark items done before the PR merges.**
  Tick them on merge, not before. Editing them early breaks the audit
  trail.
- **Cross-subsystem PRs.** If a PR touches two subsystems (other than
  spine), it should have been split at the spec stage. Stop, split the
  spec, split the PR.
- **Skipping `pre-push`.** CI failures cost more time than the
  checklist does.
- **Shipping a primitive without its discovery surface.** Specs that
  introduce a new user-facing resource (workspace, project, credential —
  anything a user must create to use) must scope in navigation entry,
  first-run flow, empty-state CTAs, and auth gating in the *same* spec.
  Deferring the surface to a follow-up ships dead code in the interim
  and usually costs more overall. See
  [reach-checklist](references/reach-checklist.md) for the per-spec
  Plan items this implies. The cheap option (ship CRUD behind a hidden
  Settings card) is almost always the wrong one.

## Delegation map

| Stage | Skill / workflow |
|-------|------------------|
| Write the spec | [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) (installed globally from `onsager-ai/dev-skills`) |
| Pre-push checks | [`pre-push`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pre-push/SKILL.md) (global) + the overlay below |
| On PR open → spec-link check | [`pr-spec-sync.yml`](../../../.github/workflows/pr-spec-sync.yml) |
| CI triage, review, iterate | [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (global) + the overlay below |
| On PR merge → tick Plan items / refresh tracker | [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (global, manual) |

## Pre-push & PR overlay (for the global `pre-push` / `pr-lifecycle` skills)

The global `pre-push` / `pr-lifecycle` skills carry the generic methodology. This is Onsager's repo-specific overlay — the check gate (in step 4 above), the collision patterns the conflict walkthrough should watch for, the CI-failure table, and the bundled GitHub-ops reference + audit script.

### Merge-collision patterns in this repo

Match the symptom in `git status` / build output to the pattern:

- **Migrations (`migrations/NNN_*.sql`)**: both branches added the same `NNN`. Keep main's file at `NNN`, renumber yours to the next unused `NNN+k`, then update **all three** reference sites or the CI migrate step skips it: `justfile` (`db-migrate` recipe), `docker-compose.yml` (`migrate` service entrypoint), `.github/workflows/rust.yml` (`Apply database migrations` step). Sanity check: `git grep -n '<old-NNN>_'` returns zero hits after renumber.
- **Enum variants** (e.g. `Kind::PullRequest`): main removed variants your branch still uses; build reports `variant not found`. `git grep` each removed variant and update every `match` arm; don't add a catch-all `_ =>` — the exhaustive check is load-bearing.
- **Event envelope variants** (`FactoryEventKind`, `FactoryEvent`): textual merge succeeds but may duplicate or re-order variants; after merge, dedupe and re-run `cargo build`; serde-renamed variants sharing a tag cause runtime dispatch errors that compile fine.
- **`Cargo.lock` / `pnpm-lock.yaml`**: regenerate by re-running `cargo build --workspace` / `pnpm install` after taking main's side; never hand-edit.
- **Spine event schema (`crates/onsager-spine/src/events/`)**: schema drift silently changes wire format; after resolving, run `cargo test -p onsager-spine --lib` to catch serde round-trip failures.

### CI-failure table

| Symptom | Usual cause |
| ------- | ----------- |
| `cargo build --workspace` fails, passes locally | CI built the merge preview; main drifted. `git fetch origin main && git merge origin/main`. |
| `error: no variant ... found for enum` | Main removed an enum variant. Grep match arms. |
| `cargo test -p onsager-spine` fails at runtime | New migration not listed in `.github/workflows/rust.yml`'s migration step. |
| `assert!(events.0 >= N)` in DB tests returns 0 | SQL filter on `data->>'type'` — the tag is under `data->'event'->>'type'`, or use the `event_type` column. |
| Flaky parallel test runs | Global `DELETE FROM events WHERE stream_type = 'registry'` — scope by `data->'event'->>'workspace_id' = $1`. |
| Seam / API-contract lint fires | `xtask lint-seams` / `check-api-contract` (ADR 0004 / spec #131/#151) — fix the seam, don't add an unscoped `seam-allow`. |

### Bundled GitHub-ops reference + audit script

- [`references/github-ops.md`](references/github-ops.md) — exact `mcp__github__*` tool names, params, and the easy-to-miss gotchas (body replace-vs-merge, label replace-vs-merge, the spec-link regex) the global `pr-lifecycle` flow relies on for this repo.
- [`scripts/audit-open-prs.sh`](scripts/audit-open-prs.sh) — read-only audit (human/CI use, where MCP isn't available): lists every open PR and flags those missing a spec link / `trivial` label, mirroring `pr-spec-sync.yml`'s regex.
