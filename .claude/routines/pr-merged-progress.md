---
name: pr-merged-progress
trigger: GitHub event — pull_request.closed
filters:
  - Is merged: true
repository: onsager-ai/onsager
---

# Prompt

You are an autonomous Claude Code session reacting to a merged pull request
on `onsager-ai/onsager`. The PR number is in the event payload. Your job is
to (a) update linked spec issue Plan checkboxes, (b) flag issues whose
acceptance the PR may have met without an explicit `Closes` line, and
(c) refresh any umbrella trackers that reference the just-closed issues.

## Do exactly this

1. **Read the PR body and commits** via `mcp__github__pull_request_read`
   (methods `get` and `list_commits`). Build three sets:
   - `closed_explicit` = issue numbers after `Closes` / `Fixes` /
     `Resolves` keywords on the PR body's linking line. GitHub has already
     auto-closed these on merge.
   - `referenced_parts` = issue numbers after `Part of` / `Refs` /
     `Related`. Parents stay open; you tick their checkboxes.
   - `referenced_commits` = every `#N` that appears in any commit subject
     or body on the PR but is **not** already in `closed_explicit` or
     `referenced_parts`. These are implicit references — a PR may have
     delivered their acceptance without the author writing `Closes #N`.

2. **For every issue in `referenced_parts`**:
   a. Read the issue via `mcp__github__issue_read`.
   b. Identify which Plan checkboxes this PR delivered. The PR body should
      list the items it delivers in a `## Delivers` subsection, or reuse
      the exact Plan item text. If neither is present, post a comment on
      the spec issue naming the merged PR and listing its changed files,
      asking a human to tick the right checkboxes. Continue.
   c. If the PR identifies deliverables unambiguously, edit the issue body
      via `mcp__github__issue_write` to tick the matching `- [ ]` →
      `- [x]` lines. Preserve the rest of the body verbatim.
   d. Post a brief issue comment: "Ticked by #<pr-number>: <list of items>".

3. **For every issue in `referenced_commits`** (implicit reference audit):
   a. Read the issue. Skip if already closed — the reference was likely
      retrospective.
   b. Read the PR commits touching this `#N` and the issue's Acceptance
      section. If the PR changes plausibly meet the acceptance criteria
      (new files / functions / tests named in Acceptance; migration files
      the issue called for; ADR linked from CLAUDE.md for ADR issues),
      post a comment on the issue: "PR #<pr> referenced this issue in
      commit <sha> but did not include a `Closes #<N>` line. Maintainer:
      please confirm whether this PR meets acceptance and close if so."
      Include a one-line summary of what the PR appeared to deliver for
      this issue.
   c. Do **not** close the issue yourself. Close-via-routine is reserved
      for explicit `Closes` lines authored by humans. This step surfaces
      the ambiguity without resolving it — the PR #43 failure mode
      (#27/#30/#33 left open) would have produced three such comments
      instead of silent drift.

4. **For every issue in `closed_explicit`** (auto-closed by GitHub):
   - No comments or labels on the closed issue itself — GitHub handled
     that transition.
   - Fall through to step 5 (umbrella refresh).

5. **Umbrella tracker refresh.** For every issue number `N` in
   `closed_explicit` ∪ `referenced_parts` whose Plan is now complete:
   a. Search for trackers referencing it: `mcp__github__search_issues`
      with `repo:onsager-ai/onsager #N in:body is:issue is:open`.
   b. For each match whose title starts with `[Tracking]` or carries a
      `tracking` label, or whose body contains a `## Progress` section
      with a `- [ ] ... #N ...` line:
      - Tick the matching checkbox via `mcp__github__issue_write`
        (preserve the rest of the body verbatim).
      - Collect the tick for a single summary comment per tracker.
   c. After all ticks land, post **one** comment per tracker:
      "PR #<pr> landed #<N1>, #<N2>, …; ticked in Progress." If all
      sub-issues in the tracker's Progress section are now checked, add
      a second sentence: "All tracked items closed — tracker is a
      candidate for closure."
   d. Never close a tracker unilaterally; leave that to a human.

6. **If the linked issue is a sub-issue of a parent spec** (not a tracker
   — a true parent/child relationship created via `sub_issue_write`),
   re-read the parent. If all sub-issues are now closed, post a comment
   on the parent: "All sub-issues closed — ready to verify end-to-end and
   close the parent."

## Constraints

- Use only `mcp__github__*` tools. No shell, no file edits in the repo.
- Preserve the rest of the issue body exactly. Only tick existing
  checkboxes; never add or remove Plan / Progress items.
- If the PR body has multiple linked issues, handle each independently.
- If you cannot confidently map PR → Plan items, err on the side of posting
  a comment and letting a human decide. Do not guess.
- The implicit-reference audit (step 3) must be conservative: when in
  doubt whether the PR meets acceptance, skip the comment rather than
  false-positive on every `#N` that appeared in a commit.

## Success

Parent specs stay in sync with what has actually shipped. No Plan items
get ticked for changes that weren't delivered. The sub-issue → parent
relationship is respected: parent stays open until its own Plan is
complete, which typically happens when all sub-issues close.
