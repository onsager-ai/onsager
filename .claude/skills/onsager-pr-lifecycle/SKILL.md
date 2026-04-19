---
name: onsager-pr-lifecycle
description: Manage an Onsager PR after it's been pushed — spec-issue linking, CI triage, review-comment discipline, merge-conflict recovery on open PRs, webhook subscription, and label alignment. Triggers include "CI is failing", "check is red", "link this issue", "Closes vs Part of", "respond to review", "subscribe to PR", "triage PR", "the PR is ready", "PR has conflicts", "branch has conflicts with main", "merge conflict on the PR", or when a github-webhook-activity event arrives. Paired with `onsager-dev-process` (overall loop), `issue-spec` (spec creation), `onsager-pre-push` (which owns the pre-push conflict walkthrough), and the GitHub-triggered Claude Routines under `.claude/routines/`.
---

# onsager-pr-lifecycle

Everything that happens after `git push` on an Onsager PR. Covers spec-issue
linking (and its enforcement), CI triage, review-comment response discipline,
webhook subscriptions, and the manual fallback for label alignment when
routines aren't configured.

## Tool discipline

- **No `gh` CLI, no `hub`, no direct GitHub API.** Always use `mcp__github__*`.
- Scope is restricted to `onsager-ai/onsager`. Don't query other repos.
- Don't open PRs unless the user explicitly asks. Creating one is a
  one-way door in this project's workflow.

## Spec-issue linking (mandatory)

Every PR must either:

1. Link to a spec issue in its body via `Closes #N` / `Fixes #N` /
   `Resolves #N` (slice complete) or `Part of #N` / `Refs #N` (scaffolding),
   **OR**
2. Carry the `trivial` label (typo, doc-only, one-line obvious fix).

If neither, the PR is out of process. Ask the author (via PR comment) to
either add a spec link — creating one via `issue-spec` if none exists — or
apply the `trivial` label.

### Which keyword to use

GitHub closes issues on merge when PR body contains one of:
`close`, `closes`, `closed`, `fix`, `fixes`, `fixed`, `resolve`, `resolves`,
`resolved` — followed by `#N`.

Pick the keyword based on **what this PR actually delivers**:

| PR delivers                                                  | Use         |
| ------------------------------------------------------------ | ----------- |
| The acceptance test / vertical slice the spec asks for       | `Closes #N` |
| A bug fix for a specific defect                              | `Fixes #N`  |
| Scaffolding / one phase of a multi-phase spec                | `Part of #N` |
| Related work that shouldn't close the spec                   | `Refs #N`   |

`Part of` / `Refs` are **not** auto-close keywords — they just cross-link in
the UI. Use them for scaffolding so the spec stays open for the real slice.

Edit the PR body via `mcp__github__update_pull_request` (don't open a new PR
just to fix the link). Put the linking line at the top of the body.

### Multi-issue PRs — enumerate every closure

If a single PR delivers acceptance for more than one issue (rare but
legitimate — e.g. a refactor that completes two related specs), write
**one `Closes` keyword per issue** on the linking line:

```markdown
Closes #27, Closes #30, Closes #33
```

GitHub only honors the auto-close keyword on each `#N` individually;
`Closes #27, #30, #33` closes #27 and leaves #30/#33 open. This is how
PR #43 quietly left three issues open even though their acceptance
criteria were met — the PR title only mentioned `(#27)` and no `Closes`
line enumerated the others.

`onsager-pre-push` step 6.4 now scans the branch's commits for `#N`
mentions and warns when they're missing from the linking line. The
check is advisory at push time; this skill is where the discipline is
enforced post-push if the pre-push scan was skipped or overridden.

### The `## Delivers` subsection

For `Part of #N` PRs (and ideally all PRs), include a `## Delivers`
subsection in the body listing the exact Plan items this PR ticks, copied
verbatim from the spec's `## Plan`. The `pr-merged-progress` routine uses
this on merge to tick the parent spec's checkboxes. Without it, a human
has to tick them manually.

Example PR body:

```markdown
Part of #42

## Delivers
- [x] Add `STIGLAB_SESSION_TIMEOUT` env var to server config (default: 30m)
- [x] Implement per-session inactivity timer in `SessionManager`

## Summary
First slice of the session-timeout work. Timer plumbing only; event
emission lands in the next PR.
```

## Issue progress labels

The linked spec issue's status label should always reflect reality:

| Spec label | What it means | Who flips it |
|------------|---------------|--------------|
| `draft` | AI/human-drafted, not yet reviewed | Human (via `planned` move) |
| `planned` | Ready for implementation | Human (alignment gate) |
| `in-progress` | At least one open PR | `pr-spec-sync` workflow |
| closed | Delivered, tests passing | GitHub (via `Closes` keyword on merge) |

These transitions happen automatically:

- PR open → `planned` becomes `in-progress` (via `pr-spec-sync.yml`).
- PR closed unmerged with no other open PR → reverts to `planned` (via
  `pr-spec-sync.yml`).
- PR merged with `Closes #N` → issue auto-closes (GitHub).
- PR merged with `Part of #N` → Plan checkboxes tick on the parent spec
  (via the `pr-merged-progress` Claude routine, if configured).

**If the `pr-merged-progress` routine is NOT configured**, you are
responsible for ticking Plan checkboxes on parent specs after merge. The
label flips on open/close-unmerged always happen via the workflow.

Never bypass the `draft → planned` gate from within this skill — that's a
human decision. If the linked spec is still `draft`, comment on the PR
asking the author to drive the spec through review first.

## Umbrella tracker refresh

Some issues are **umbrella trackers** that reference several sub-issues as
a checklist — identified by a `[Tracking]` title prefix, a `tracking`
label, or a `## Progress` section whose items are `- [ ] #N` lines.
Examples: #40 (architectural review), anything opened with
`issue-spec`'s tracker flow.

When a PR closes a sub-issue, the tracker does **not** update itself.
After merge, for each auto-closed or explicitly-closed issue in the PR:

1. Search for umbrella trackers that reference it:
   `mcp__github__search_issues` with `repo:onsager-ai/onsager #N in:body
   is:issue is:open` — trackers will list `#N` in their Progress section.
2. For each match, read the tracker body. If there's a matching
   `- [ ] ... #N ...` line in a Progress / Plan section, flip it to `- [x]`.
3. Post one tracker comment summarizing the delta, not one per issue:
   "PR #<pr> landed #N1, #N2, #N3; ticked in Progress."
4. If after the tick every sub-issue in the Progress section is closed,
   note that the tracker itself is now a candidate for closure — don't
   close it unilaterally (the author or a human decides), just flag it.

The `pr-merged-progress` routine automates the common case. This section
is the manual fallback for when (a) routines aren't configured,
(b) routines ran but couldn't disambiguate, or (c) the tracker uses a
non-standard checklist shape the routine didn't recognize.

## CI triage

For classification taxonomy (`regression` / `flake` / `infra` / `needs-human`),
suspect-commit identification, and the rolling `main-red` issue convention,
use the [`ci-triage` skill](../ci-triage/SKILL.md). This section covers the
PR-side specifics — reproducing locally and the repo's common failure
patterns.

### Accessing logs

`WebFetch` **cannot read authenticated GitHub Actions logs** — both
`https://github.com/.../actions/runs/X/job/Y` and
`https://api.github.com/repos/.../actions/jobs/Y/logs` return 403 or an
error page. Don't waste time on them. Work instead from:

1. `mcp__github__pull_request_read` with `method: get_check_runs` — gives
   step name, status, timings.
2. **Local reproduction** after syncing main. Re-run the failing step with
   the exact flags from `.github/workflows/rust.yml`.

### Common failure patterns in this repo

| Symptom                                                       | Usual cause |
| ------------------------------------------------------------- | ----------- |
| `cargo build --workspace` fails, passes locally               | CI built the merge preview; main has drifted. `git fetch origin main && git merge origin/main` on the branch. |
| `error: no variant ... found for enum`                        | Same: main removed an enum variant. Grep match arms. |
| `cargo test -p onsager-spine` fails at runtime                | New migration not listed in `.github/workflows/rust.yml`'s migration step. |
| `assert!(events.0 >= N)` in DB tests returns 0                | SQL filter on `data->>'type'` — the tag is under `data->'event'->>'type'`, or use `event_type` column. |
| Flaky parallel test runs                                      | Global `DELETE FROM events WHERE stream_type = 'registry'` — scope by `data->'event'->>'workspace_id' = $1`. |

### Migration numbering collision (frequent!)

Main and PR both add `NNN_foo.sql` → rename yours to the next unused `NNN`.
Update **all three**: `justfile`, `docker-compose.yml`, `.github/workflows/rust.yml`.

## Merge conflicts on an open PR

When GitHub shows "This branch has conflicts that must be resolved" or the
red banner appears on `mcp__github__pull_request_read` (`mergeable: false`),
resolve **locally** — the GitHub web editor bypasses `cargo build` and
routinely lands broken merges.

1. **Don't** use `mcp__github__update_pull_request_branch` to auto-merge
   main in via GitHub. That surfaces the same conflicts without giving
   you the resolution workspace, then commits a broken merge if you
   accept the default.

2. Check out the branch locally and run the full conflict walkthrough
   in [`onsager-pre-push`](../onsager-pre-push/SKILL.md) (step 1,
   "Resolving conflicts") — inventory, pattern-match, resolve, verify,
   commit. That section owns the repo's recurring patterns (migrations,
   enum variants, event envelope, `Cargo.lock`, `pnpm-lock.yaml`, spine
   event schema); don't duplicate them here.

3. After the merge commit lands, continue with steps 2–5 of
   `onsager-pre-push` (build, test, clippy, fmt) before pushing.
   CI's `pull_request` job tests the new merge preview — if you didn't
   rebuild locally, CI finds out first.

4. Push the merge commit to the same branch with
   `git push` (no `--force`). The existing PR updates in place; the
   conflict banner clears when GitHub re-evaluates.

5. If the PR is tied to a `Closes #N` / `Part of #N` line and the merge
   touched the spec's surface area (enum variants, event schema), comment
   on the spec flagging what drifted, so the parent stays accurate.

If the branch is so far behind main that the conflict set is large
(>10 files or crosses subsystem boundaries), close the PR, rebase the
work into a fresh branch from `origin/main`, and open a new PR with the
same linking line. This is cheaper than a multi-hour merge and keeps the
audit trail clean — note the close reason on the old PR.

## Review comments

**Fix the code. Don't reply per comment.** Multiple reviewers (Copilot + human)
often flag the same defect; a single commit that fixes it resolves all of them
at once.

Reply *only* when:
- Declining a suggestion (explain why, briefly).
- The comment is a question, not a bug report.
- Asking for clarification before acting.

Use `mcp__github__add_reply_to_pull_request_comment` for threaded replies,
never top-level comments unless summarizing multiple responses at once.

**Copilot vs real defect**: Copilot sometimes flags idiomatic Rust as broken
(e.g. `&foo.to_string()` temporaries that actually live long enough). Verify
locally before "fixing" a non-bug — but prefer the clearer form (let binding)
even when the lint was wrong.

If a review comment raises a design concern that the spec didn't address,
pause and update the linked spec issue (add an open question under
`## Alignment`, comment on the spec, let a human decide). Don't silently
expand scope in the PR.

## Webhook subscription

Events from CI and reviewers arrive wrapped in `<github-webhook-activity>`
tags. The harness forwards them as user messages.

- Subscribe once per PR with `mcp__github__subscribe_pr_activity` after the
  PR is created (or the user asks you to watch it).
- Unsubscribe with `mcp__github__unsubscribe_pr_activity` when done — not
  strictly necessary but cleaner.
- Events are already filtered to CI failures + reviews. Treat each as
  actionable; skip only if it's a duplicate of one you just addressed.

Routine-triggered events (label flips, Plan checkbox ticks) do not flow
through webhook subscription — they're handled in the routine's own
session, visible at claude.ai/code under the routine's run list.

## Reporting back to the user

After handling a webhook event, end with one or two sentences: what the
failure was, what you changed, whether CI is re-running. Don't dump the full
commit message in chat — the user can see it on the PR.

## Relationship to other skills

| Related surface | Role |
|-----------------|------|
| [`onsager-dev-process`](../onsager-dev-process/SKILL.md) | Top-level SDD loop; points here for the post-push stage. |
| [`issue-spec`](../issue-spec/SKILL.md) | Creates the spec issue this PR links to. |
| [`onsager-pre-push`](../onsager-pre-push/SKILL.md) | Runs before `git push`; enforces the spec-link check locally. |
| [`.claude/routines/`](../../routines/README.md) | Automates the label transitions this skill describes manually. |
| [`ci-triage`](../ci-triage/SKILL.md) | Shared failure taxonomy + `main-red` issue convention; called from `main-ci-failure` routine and from this skill's CI triage flow. |
