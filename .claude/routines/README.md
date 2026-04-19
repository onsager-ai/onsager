# Onsager Claude Routines

Claude Routines are GitHub-event-triggered Claude Code sessions that run on
Anthropic's cloud infrastructure. We use them for the PR/spec workflow steps
that genuinely need LLM judgment — everything deterministic has been moved to
free GitHub Actions (see [Migrated to GitHub Actions](#migrated-to-github-actions)).

Routines are defined in your claude.ai account at
[claude.ai/code/routines](https://claude.ai/code/routines), not checked into
this repo. What *is* checked in, in this directory, is the **prompt** for each
recommended routine — version-controlled, reviewable, and easy to copy into
the routine setup form.

## Setup

For each `*.md` file below:

1. Go to [claude.ai/code/routines](https://claude.ai/code/routines) and click
   **New routine**.
2. Name it after the file (e.g. `pr-merged-progress`).
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
| [`pr-merged-progress.md`](pr-merged-progress.md) | `pull_request.closed` merged=true | Tick Plan checkboxes on parent spec for `Part of #N` merges; refresh umbrella trackers that reference the closed issues; flag issues mentioned only in commits (no `Closes` line) so implicit acceptance doesn't silently leave them open. |
| [`spec-planned-review.md`](spec-planned-review.md) | `issues.labeled` = `planned` | Sanity-check the spec before a human/agent picks it up (open questions, plan items, test items). |
| [`main-ci-failure.md`](main-ci-failure.md) | `workflow_run.completed` on `main` | Maintain one rolling `main-red` issue when L1 CI (`rust` / `frontend` / `e2e`) fails post-merge; close it when main goes green again. Delegates classification to the `ci-triage` skill. |

## Migrated to GitHub Actions

The label-flipping steps of the PR lifecycle are now handled by
[`.github/workflows/pr-spec-sync.yml`](../../.github/workflows/pr-spec-sync.yml).
The workflow replaces two former routines:

- **`pr-opened-progress`** — on `pull_request.opened` / `ready_for_review`,
  flips linked spec issues from `planned` → `in-progress`, warns on `draft`
  specs, and prompts for a spec link when none is present (unless the PR
  carries `trivial`).
- **`pr-closed-unmerged`** — on `pull_request.closed` with `merged=false`,
  reverts linked specs from `in-progress` → `planned` when no other open PR
  references the same issue.

Neither needed LLM judgment, and Actions runs don't count against the Claude
routine quota — freeing ~1.5 runs per PR for the paths that do (e.g.
`pr-merged-progress`, `main-ci-failure`).

## Updating a routine

Edit the prompt file here, commit it, then paste the new prompt into the
routine's edit form on the web. Keeping the canonical prompt in-repo makes
prompt drift auditable and reviewable like any other code change.

## Why not hooks or webhooks?

- **Hooks** (`settings.json`) run locally in a dev's CLI session. They can't
  react to PR events from reviewers on other machines.
- **GitHub Actions** are the right tool for deterministic reactions — we use
  them for label flips (`pr-spec-sync.yml`). Routines cover the judgment
  calls that would otherwise need prompt plus workflow YAML plus token
  secrets maintained separately.
- **onsager-pr-lifecycle skill** covers the interactive, human-present case
  (triaging CI, replying to review comments). Routines cover the unattended,
  mechanical case that still needs LLM reasoning (plan-checkbox mapping,
  umbrella refresh, spec quality check, CI failure triage).
- **ci-triage skill** is shared logic: `main-ci-failure` calls it
  unattended; humans call it via `onsager-pr-lifecycle` when looking at a
  red PR check. Keeping the taxonomy in one place avoids two definitions
  of "flake" drifting apart.

If you don't want to set up routines, read `onsager-pr-lifecycle` and handle
the remaining merge-time steps manually — chiefly ticking the matching Plan
checkbox and refreshing the umbrella issue after merge. Open/close-unmerged
label flips are already handled by `pr-spec-sync.yml`.

## Limits

Routines count against your daily run allowance (Pro: 5/day, Max: 15/day,
Team/Enterprise: 25/day). With the GH Action migration, budget per PR is:

- Opened → `pr-spec-sync.yml` (Actions, free).
- Closed unmerged → `pr-spec-sync.yml` (Actions, free).
- Merged → `pr-merged-progress` routine (~1 run).
- Spec planned → `spec-planned-review` routine (~1 run per transition).
- Main CI failure/green → `main-ci-failure` routine (1 run per L1 workflow
  completion on `main`).

The biggest remaining consumer is `main-ci-failure` — it fires once per
workflow completion on main, so a flaky main burns allowance fast. The
routine's first step filters the event to the three L1 workflows and only
the `push` event, keeping nightly + manual triggers out of the budget. If
the allowance still hurts, drop the green-close path (manually close the
`main-red` issue) before dropping the red-file path.
