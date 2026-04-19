# Onsager Claude Routines

Claude Routines are GitHub-event-triggered Claude Code sessions that run on
Anthropic's cloud infrastructure. We use them to keep issue-progress labels in
sync with PR state so humans don't have to flip them by hand.

Routines are defined in your claude.ai account at
[claude.ai/code/routines](https://claude.ai/code/routines), not checked into
this repo. What *is* checked in, in this directory, is the **prompt** for each
recommended routine — version-controlled, reviewable, and easy to copy into
the routine setup form.

## Setup

For each `*.md` file below:

1. Go to [claude.ai/code/routines](https://claude.ai/code/routines) and click
   **New routine**.
2. Name it after the file (e.g. `pr-opened-progress`).
3. Paste the prompt from the file (everything below the `---` frontmatter).
4. Select repository: `onsager-ai/onsager`.
5. Select an environment that has the Claude GitHub App installed.
6. Under **Select a trigger**, add the GitHub trigger described in the
   frontmatter (event + filters).
7. Leave the default connectors. The prompt only uses the GitHub MCP.
8. Save.

Install the Claude GitHub App on `onsager-ai/onsager` if prompted.

## Recommended routines

| File | Trigger | Purpose |
|------|---------|---------|
| [`pr-opened-progress.md`](pr-opened-progress.md) | `pull_request.opened` | Flip linked spec issue `planned`/`draft` → `in-progress`; verify PR body links a spec. |
| [`pr-merged-progress.md`](pr-merged-progress.md) | `pull_request.closed` merged=true | Tick Plan checkboxes on parent spec for `Part of #N` merges; refresh umbrella trackers that reference the closed issues; flag issues mentioned only in commits (no `Closes` line) so implicit acceptance doesn't silently leave them open. |
| [`pr-closed-unmerged.md`](pr-closed-unmerged.md) | `pull_request.closed` merged=false | Revert linked spec back to `planned` if no other PR is in flight. |
| [`spec-planned-review.md`](spec-planned-review.md) | `issues.labeled` = `planned` | Sanity-check the spec before a human/agent picks it up (open questions, plan items, test items). |
| [`main-ci-failure.md`](main-ci-failure.md) | `workflow_run.completed` on `main` | Maintain one rolling `main-red` issue when L1 CI (`rust` / `frontend` / `e2e`) fails post-merge; close it when main goes green again. Delegates classification to the `ci-triage` skill. |

## Updating a routine

Edit the prompt file here, commit it, then paste the new prompt into the
routine's edit form on the web. Keeping the canonical prompt in-repo makes
prompt drift auditable and reviewable like any other code change.

## Why not hooks or webhooks?

- **Hooks** (`settings.json`) run locally in a dev's CLI session. They can't
  react to PR events from reviewers on other machines.
- **GitHub Actions** could do this with the Anthropic API, but then we
  maintain workflow YAML plus prompt plus token secrets. Routines package
  all three and run under the account that owns the subscription.
- **onsager-pr-lifecycle skill** covers the interactive, human-present case
  (triaging CI, replying to review comments). The routines cover the
  unattended, mechanical case (label alignment, main-red bookkeeping).
- **ci-triage skill** is shared logic: `main-ci-failure` calls it
  unattended; humans call it via `onsager-pr-lifecycle` when looking at a
  red PR check. Keeping the taxonomy in one place avoids two definitions
  of "flake" drifting apart.

The two are complementary. If you don't want to set up routines, read
`onsager-pr-lifecycle` and flip labels manually — the skill documents the
same transitions.

## Limits

Routines count against your daily run allowance (Pro: 5/day, Max: 15/day,
Team/Enterprise: 25/day). For a project with steady PR flow, budget
~2 runs per PR (open + merge) and consider combining triggers on one
routine if you're pressed for allowance.

`main-ci-failure` fires once per workflow completion on main, so a flaky
main burns allowance fast. The routine's first step filters the event to
the three L1 workflows and only the `push` event — that keeps nightly +
manual triggers out of the budget. If the allowance still hurts, drop the
green-close path (manually close the `main-red` issue) before dropping the
red-file path.
