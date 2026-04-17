---
name: onsager-pr-lifecycle
description: Manage an Onsager PR after it's been pushed — issue linking, CI triage, review-comment discipline, webhook subscription. Triggers include "CI is failing", "check is red", "link this issue", "Closes vs Part of", "respond to review", "subscribe to PR", "triage PR", "the PR is ready" or when a github-webhook-activity event arrives.
---

# onsager-pr-lifecycle

Everything that happens after `git push` on an Onsager PR. Covers CI triage,
issue↔PR linking conventions, review-comment response discipline, and webhook
subscriptions.

## Tool discipline

- **No `gh` CLI, no `hub`, no direct GitHub API.** Always use `mcp__github__*`.
- Scope is restricted to `onsager-ai/onsager`. Don't query other repos.
- Don't open PRs unless the user explicitly asks. Creating one is a
  one-way door in this project's workflow.

## Issue linking

GitHub closes issues on merge when PR body contains one of:
`close`, `closes`, `closed`, `fix`, `fixes`, `fixed`, `resolve`, `resolves`,
`resolved` — followed by `#N`.

Pick the keyword based on **what this PR actually delivers**:

| PR delivers                                                  | Use         |
| ------------------------------------------------------------ | ----------- |
| The acceptance test / vertical slice the issue asks for     | `Closes #N` |
| A bug fix for a specific defect                              | `Fixes #N`  |
| Scaffolding / one phase of a multi-phase issue               | `Part of #N` |
| Related work that shouldn't close the issue                  | `Refs #N`   |

`Part of` / `Refs` are **not** auto-close keywords — they just cross-link in
the UI. Use them for scaffolding so the issue stays open for the real slice.

Edit the PR body via `mcp__github__update_pull_request` (don't open a new PR
just to fix the link). Put the linking line at the top of the body.

## CI triage

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

## Webhook subscription

Events from CI and reviewers arrive wrapped in `<github-webhook-activity>`
tags. The harness forwards them as user messages.

- Subscribe once per PR with `mcp__github__subscribe_pr_activity` after the
  PR is created (or the user asks you to watch it).
- Unsubscribe with `mcp__github__unsubscribe_pr_activity` when done — not
  strictly necessary but cleaner.
- Events are already filtered to CI failures + reviews. Treat each as
  actionable; skip only if it's a duplicate of one you just addressed.

## Reporting back to the user

After handling a webhook event, end with one or two sentences: what the
failure was, what you changed, whether CI is re-running. Don't dump the full
commit message in chat — the user can see it on the PR.
