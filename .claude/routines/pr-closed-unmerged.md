---
name: pr-closed-unmerged
trigger: GitHub event — pull_request.closed
filters:
  - Is merged: false
repository: onsager-ai/onsager
---

# Prompt

You are an autonomous Claude Code session reacting to a pull request that
was closed *without* being merged on `onsager-ai/onsager`. Your job is to
back out the `in-progress` label on the linked spec issue if no other PRs
are still open against it.

## Do exactly this

1. **Read the PR body** via `mcp__github__pull_request_read` to find
   linked spec issues (any of `Closes`, `Fixes`, `Resolves`, `Part of`,
   `Refs`, `Related`).
2. **For each linked issue that is still open and labeled `in-progress`:**
   a. Search open PRs against the same repository that reference the same
      issue number in their body (`mcp__github__list_pull_requests` with
      state=open, then filter in memory).
   b. If at least one *other* open PR still references the issue, do
      nothing — another in-flight PR keeps the spec in progress.
   c. If no other open PR references the issue, swap `in-progress` →
      `planned` using `mcp__github__issue_write`. Post an issue comment:
      "PR #<pr-number> closed without merging. No other PRs in flight —
      spec returned to `planned`."
3. **Do not reopen closed issues.** If an issue was already closed (by a
   different PR's `Closes`), leave it alone.

## Constraints

- Use only `mcp__github__*` tools.
- Do not delete or modify the Plan checkboxes. Closing a PR unmerged
  should not revert work that actually landed elsewhere.
- Keep comments short.

## Success

A spec that genuinely has no active implementation returns to `planned` so
the next implementer knows it's available. A spec with another PR in
flight keeps its `in-progress` label.
