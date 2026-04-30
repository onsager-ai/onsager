# GitHub Operations Reference

Canonical `mcp__github__*` tool-call sequences for the recurring PR/issue
flows in the Onsager dev process. The prose lives in
[`../SKILL.md`](../SKILL.md); this file is the cheat sheet — exact tool
name, minimum params, and the one thing that's easy to get wrong.

Scope is `onsager-ai/onsager`. Every call below assumes
`owner: "onsager-ai"`, `repo: "onsager"` unless noted.

## Conventions

- `repo` / `owner` are required on most calls — don't omit them.
- PR / issue numbers are integers, not `"#42"`.
- Body strings use real newlines, not `\n` escapes.
- All keyword detection follows `pr-spec-sync.yml`:
  - Closing: `close[sd]?` / `fix(e[sd])?` / `resolve[sd]?` + `#N`
  - Reference: `part of` / `refs` / `related` + `#N`

## Opening a PR with a spec link

Run the spec-vs-trivial gate **before** calling `create_pull_request` and
bake the answer into the body / labels at creation time. This is the
upstream answer to `pr-spec-sync`'s `<!-- pr-spec-sync:no-spec-link -->`
reminder.

```
mcp__github__create_pull_request
  owner: "onsager-ai"
  repo: "onsager"
  title: "<short imperative — under 70 chars>"
  head: "<branch>"
  base: "main"
  body: |
    Closes #N            # or "Part of #N" for scaffolding slices

    ## Delivers          # required for Part-of PRs; recommended for Closes
    - [x] <Plan item copied verbatim from spec>

    ## Summary
    <1-3 sentences of why>
  labels: []             # add ["trivial"] only if no spec issue is warranted
  draft: false           # always open as ready-for-review
```

Get the spec-link line wrong and `pr-spec-sync` posts a bot comment within
seconds. See SKILL.md → "Spec-issue linking" for the keyword table and
multi-issue rules.

## Updating an existing PR's body

Use this to fix a missing spec link, add a `## Delivers` block, or tighten
the summary. Don't open a new PR for a body fix.

```
mcp__github__update_pull_request
  owner: "onsager-ai"
  repo: "onsager"
  pullNumber: <int>
  body: "<full new body>"   # replaces, doesn't merge
```

The body parameter **replaces** the whole body. Read the current PR first
(`pull_request_read` with `method: get`) and patch the string locally
before sending it back.

## Reading PR state and CI status

```
mcp__github__pull_request_read
  owner: "onsager-ai"
  repo: "onsager"
  pullNumber: <int>
  method: "get"             # full PR object: body, labels, mergeable, head sha
```

```
mcp__github__pull_request_read
  ...
  method: "get_check_runs"  # per-step CI status; replaces parsing logs by hand
```

`WebFetch` cannot read authenticated GitHub Actions logs (403 / login page).
For CI failures, work from `get_check_runs` + local reproduction — see
SKILL.md → "Accessing logs".

## Replying to a review comment

```
mcp__github__add_reply_to_pull_request_comment
  owner: "onsager-ai"
  repo: "onsager"
  pullNumber: <int>
  commentId: <int>          # the original review comment id
  body: "<reply>"
```

Default behavior: **don't reply, fix the code.** Reply only when declining
a suggestion, the comment is a question, or you need clarification before
acting. See SKILL.md → "Review comments".

## Subscribing to PR activity

Call once per PR after creating it (or when a user asks you to watch one).
CI failures and review comments arrive wrapped in `<github-webhook-activity>`
tags as user messages.

```
mcp__github__subscribe_pr_activity
  owner: "onsager-ai"
  repo: "onsager"
  pullNumber: <int>
```

```
mcp__github__unsubscribe_pr_activity
  owner: "onsager-ai"
  repo: "onsager"
  pullNumber: <int>
```

Workflow-triggered label flips (via `pr-spec-sync.yml`) do **not** flow
through this subscription. Plan checkbox ticks on `Part of #N` parents
are manual.

## Ticking an umbrella tracker checkbox

When a PR closes a sub-issue, the tracker doesn't update itself. For each
auto-closed or explicitly-closed issue:

1. Find the trackers that reference it.

   ```
   mcp__github__search_issues
     query: "repo:onsager-ai/onsager #N in:body is:issue is:open"
   ```

2. Read the tracker's current body.

   ```
   mcp__github__issue_read
     owner: "onsager-ai"
     repo: "onsager"
     issueNumber: <tracker int>
     method: "get"
   ```

3. Flip `- [ ] ... #N ...` → `- [x] ... #N ...` in the body string locally,
   then write it back. **Replace** the whole body; don't try to patch.

   ```
   mcp__github__issue_write
     owner: "onsager-ai"
     repo: "onsager"
     issueNumber: <tracker int>
     method: "update"
     body: "<full new body>"
   ```

4. Post **one** comment summarizing the delta, not one per sub-issue.

   ```
   mcp__github__add_issue_comment
     owner: "onsager-ai"
     repo: "onsager"
     issueNumber: <tracker int>
     body: "PR #<pr> landed #N1, #N2, #N3; ticked in Progress."
   ```

5. If every sub-issue in Progress is now closed, **flag** the tracker as a
   closure candidate — don't close it unilaterally. Author/human decides.

## Spec-issue label transitions (manual fallback)

The `pr-spec-sync.yml` workflow handles open/close-unmerged automatically.
Use this only when the workflow is disabled or you're flipping `draft → planned`
intentionally (the latter is **always a human call** — don't do it from a
session).

```
mcp__github__issue_write
  owner: "onsager-ai"
  repo: "onsager"
  issueNumber: <int>
  method: "update"
  labels: ["spec", "in-progress", ...]   # full label set; replaces
```

The `labels` field is a **replace**, not a merge — include every label the
issue should still have, or you'll strip area/priority labels by accident.
Prefer `add_labels` / `remove_labels` if available on this MCP server;
otherwise read first, mutate the array, write back.

## Listing open PRs missing a spec link (audit)

Bulk read-only audits are cheaper as a single API loop than dozens of MCP
calls. See [`../scripts/audit-open-prs.sh`](../scripts/audit-open-prs.sh) —
runs from a human shell or CI with `GITHUB_TOKEN`. Claude should not call
this script for normal flows; use the per-PR MCP calls above.

## Things deliberately not in this list

- **Force-pushing**, branch deletion, repo settings — not Claude's job.
- **Merging a PR** — `mcp__github__merge_pull_request` exists, but per
  CLAUDE.md "Executing actions with care", merge is a one-way door that
  needs explicit user confirmation in-session.
- **Enabling auto-merge** — same caveat. Both are listed here so you know
  they exist, not so you reach for them.
