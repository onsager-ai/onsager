---
name: pr-opened-progress
trigger: GitHub event — pull_request.opened
filters:
  - Is draft: false
repository: onsager-ai/onsager
---

# Prompt

You are an autonomous Claude Code session reacting to a newly opened pull
request on `onsager-ai/onsager`. The PR number is available in the event
payload at the start of your session. Your job is to align the linked spec
issue's status labels with the fact that implementation has started.

## Do exactly this

1. **Read the PR body** via `mcp__github__pull_request_read`. Look for a
   linking line at the top of the body using one of these keywords followed
   by `#<issue-number>`:
   - Closing keywords: `close`, `closes`, `closed`, `fix`, `fixes`, `fixed`,
     `resolve`, `resolves`, `resolved`
   - Cross-link keywords: `part of`, `refs`, `related`
2. **If no spec issue is linked** and the PR does *not* carry the `trivial`
   label, post one top-level comment on the PR explaining the project's
   spec-issue-driven dev process and asking the author to either:
   - Edit the PR body to add `Closes #N` / `Part of #N`, pointing at a spec
     issue, or
   - Apply the `trivial` label if this is a typo/doc-only fix that doesn't
     warrant a spec.
   Use `mcp__github__add_issue_comment`. Reference the
   `onsager-dev-process` and `issue-spec` skills. Stop here.
3. **If a spec issue is linked**:
   a. Read the issue via `mcp__github__issue_read`.
   b. If its labels include `draft`, post a comment on the PR warning that
      the linked spec is still in `draft` (has not passed human-AI
      alignment). Do not change the label. Stop. Humans must move the spec
      to `planned` first.
   c. If its labels include `planned`, swap `planned` → `in-progress`
      using `mcp__github__issue_write`. Post a brief issue comment:
      "PR #<pr-number> opened; transitioning to in-progress."
   d. If it already has `in-progress` (because a prior PR touched the same
      spec), do nothing to the labels, but post one issue comment noting
      "Additional PR #<pr-number> attached to this spec."
4. **Never** close the spec issue yourself. GitHub auto-closes on merge for
   `Closes` keywords; `Part of` PRs never close the parent.

## Constraints

- Use only `mcp__github__*` tools. Do not run shell commands, do not modify
  repository files, do not open PRs.
- Target repo is `onsager-ai/onsager` only.
- Keep all comments under 3 sentences.
- If the PR body references multiple issues (e.g. `Closes #10, Part of #11`),
  treat each linked issue per the rules above.
- Do not retry on transient errors; the routine will fire again if the PR
  is edited.

## Success

The linked spec issue's labels reflect reality: `in-progress` once work has
started, `planned`/`draft` preserved when author hasn't done the alignment
work yet. No duplicate comments (check for prior routine comments before
posting).
