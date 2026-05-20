---
name: onsager-pr-lifecycle
description: Manage an Onsager PR after it's been pushed — spec-issue linking, CI triage, review-comment discipline, merge-conflict recovery on open PRs, webhook subscription, and label alignment. Triggers include "CI is failing", "check is red", "link this issue", "Closes vs Part of", "respond to review", "subscribe to PR", "triage PR", "the PR is ready", "PR has conflicts", "branch has conflicts with main", "merge conflict on the PR", or when a github-webhook-activity event arrives. Paired with `onsager-dev-process` (overall loop), `issue-spec` (spec creation), and `onsager-pre-push` (which owns the pre-push conflict walkthrough).
---

# onsager-pr-lifecycle

Everything that happens after `git push` on an Onsager PR. Covers spec-issue
linking (and its enforcement), CI triage, review-comment response discipline,
webhook subscriptions, and the manual ticking of Plan items / umbrella
trackers on merge.

## Tool discipline

- **No `gh` CLI, no `hub`, no direct GitHub API.** Always use `mcp__github__*`.
- Scope is restricted to `onsager-ai/onsager`. Don't query other repos.
- Don't open PRs unless the user explicitly asks. Creating one is a
  one-way door in this project's workflow.

For exact tool names, params, and the easy-to-miss gotchas (body
replace-vs-merge, label replace-vs-merge, the spec-link regex), see
[`references/github-ops.md`](references/github-ops.md). The prose below
explains *why*; the reference is the *how*.

## Bundled scripts

For Claude, all GitHub ops go through `mcp__github__*` — see the reference
above. The `scripts/` folder is for **human and CI** use, where MCP isn't
available, and is deliberately limited to read-only audits.

| Script | Purpose | Auth |
|--------|---------|------|
| [`scripts/audit-open-prs.sh`](scripts/audit-open-prs.sh) | List every open PR and flag those missing a spec link / `trivial` label. Mirrors `pr-spec-sync.yml`'s regex. | `GITHUB_TOKEN` |

Don't add write-side scripts here. Anything that mutates GitHub state for
Claude belongs as an `mcp__github__*` call documented in
`references/github-ops.md`; anything that mutates state for humans
belongs in a workflow under `.github/workflows/`, not in this skill's
`scripts/`.

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
subsection in the body listing the exact Plan items this PR ticks. Copy
the item text verbatim from the spec's `## Plan`, but mark each as
`- [x]` (the PR delivers them, even though the corresponding boxes on
the spec are still `- [ ]` until you tick them after merge). Use this
list to tick the parent spec's checkboxes — see "Issue progress
labels" below.

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

## Issue progress

Spec issues use only their open/closed state — no status labels. Lifecycle
moves:

- PR merged with `Closes #N` → issue auto-closes (GitHub).
- PR merged with `Part of #N` → spec stays open; Plan checkboxes on the
  parent spec are ticked manually by whoever merges (or shortly after).
- PR closed unmerged → spec issue stays open as-is.

You are responsible for ticking Plan checkboxes on parent specs after
merge.

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

This refresh is manual. Run through it for every merged PR that closes
(or `Part of`) a sub-issue in a tracker.

## CI triage

For classification taxonomy (`regression` / `flake` / `infra` / `needs-human`),
suspect-commit identification, and the rolling `main-red` issue convention,
use the [`ci-triage` skill](https://github.com/onsager-ai/dev-skills/blob/main/skills/ci-triage/SKILL.md) (installed globally from `onsager-ai/dev-skills`). This section covers the
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
- Treat each event as actionable; skip only if it's a duplicate of one you
  just addressed.

### What the subscription does NOT cover

The subscription is **not** an "all CI events" feed. It's filtered to issue
comments and review activity, plus a small set of well-known check_run
state changes. In practice this leaks the following silent-failure
surfaces, observed on real PRs:

- **Commit statuses.** GitHub's `status` API is a separate channel from
  `check_run`. Per-PR Railway deploys, GitGuardian, and any GitHub App
  that posts via the legacy statuses API fire here, not as check_runs.
  No `<github-webhook-activity>` event arrives — the failure is only
  visible if you call `pull_request_read get_status` directly.
- **`check_run` completion transitions.** The subscription fires on some
  state changes but is not reliable for "the rust.yml `build` job just
  finished failure". An in-progress check that completes after you've
  ended your turn does not necessarily wake the session.

PR #279 hit the first failure mode — Railway's deploy failure landed as a
commit status, no webhook fired, and the agent ended its turn assuming
the subscription would surface CI completion. It didn't, and the user
had to flag it manually. The post-push sweep below closes the gap.

### Post-push CI sweep (mandatory)

Once the PR is open and you've subscribed, run this sweep **before
declaring "all green" or ending the turn**:

1. Read both surfaces explicitly:
   - `pull_request_read` with `method: get_status` — pulls every commit
     status for the head SHA. Treat any `state=failure` as actionable
     even if no webhook fired.
   - `pull_request_read` with `method: get_check_runs` — pulls every
     check_run. Treat any `conclusion=failure` as actionable.
2. Any check still `in_progress` or `queued` is **not** ground for
   "completed" — record a TodoWrite item like *"Re-poll PR #N CI
   ~5 min post-push"* and check it off only after a follow-up sweep
   shows the check finished. Don't sleep-poll inside the same turn;
   end the turn and let the user (or a subsequent webhook event) bring
   you back.
3. If the sweep returns nothing actionable but checks are still
   running, say so explicitly in the end-of-turn summary
   ("`build` and `Agent` still running; will follow up") so the user
   doesn't infer the PR is fully green.

The sweep is cheap (two MCP calls) and is the only mechanical defense
against the silent-failure surfaces above. Skipping it is how PR #279
shipped a broken Railway deploy that the agent never noticed.

## Reporting back to the user

After handling a webhook event, end with one or two sentences: what the
failure was, what you changed, whether CI is re-running. Don't dump the full
commit message in chat — the user can see it on the PR.

## Relationship to other skills

| Related surface | Role |
|-----------------|------|
| [`onsager-dev-process`](../onsager-dev-process/SKILL.md) | Top-level SDD loop; points here for the post-push stage. |
| [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) | Creates the spec issue this PR links to. Installed globally from `onsager-ai/dev-skills`. |
| [`onsager-pre-push`](../onsager-pre-push/SKILL.md) | Runs before `git push`; enforces the spec-link check locally. |
| [`pr-spec-sync.yml`](../../../.github/workflows/pr-spec-sync.yml) | Checks every non-trivial PR links a spec issue; comments if the link is missing. Plan-item ticks on merge remain manual. |
| [`ci-triage`](https://github.com/onsager-ai/dev-skills/blob/main/skills/ci-triage/SKILL.md) | Shared failure taxonomy + `main-red` issue convention; called from this skill's CI triage flow. Installed globally from `onsager-ai/dev-skills`. |
