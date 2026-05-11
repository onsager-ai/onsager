---
name: onsager-explore-artifacts
description: Browse the artifacts an Onsager workflow has produced — list recent runs for a workflow, then drill into a specific artifact's metadata and current state. Triggers include "show me the artifacts", "what did this run produce", "list recent runs of the workflow", "what artifacts has this workflow created", "show me artifact `art_…`", "what's the state of run `art_…`". Read-only — no mutations. For diagnosis when something failed, use `onsager-triage-run`.
allowed_tools:
- get_artifact
- list_runs
---

# onsager-explore-artifacts

Workflows produce **artifacts** — issues, PRs, agent sessions, deployments, anything with a lifecycle. This skill is the read-only browse surface: list the runs of a workflow, drill into one artifact. No mutations. If something failed and the user wants a fix, that's `onsager-triage-run`.

## When this skill triggers

Phrases that should route here:

- "show me the artifacts this workflow has produced"
- "list recent runs of the morning-digest workflow"
- "what's the state of artifact `art_…`?"
- "show me the metadata for run `<id>`"
- "what artifacts has my PR-review workflow created this week?"

If the user wants to fire a run, that's `onsager-run-workflow`. If they want to fix one that failed, that's `onsager-triage-run`.

## Operating procedure

### Step 1 — listing runs

`list_runs` returns the recent runs for a single workflow. One artifact == one run (per the workflow runtime), so the response is the artifact list with a derived `status`:

```json
{ "workflow_id": "wf_…", "limit": 50 }
```

`limit` is optional, defaults to 50, clamped to `[1, 500]`.

Response shape:

```json
{
  "runs": [
    {
      "id": "art_…",
      "workflow_id": "wf_…",
      "artifact_id": "art_…",
      "status": "passed" | "failed" | "blocked" | "pending",
      "current_stage_index": 2,
      "parked_reason": null | "spec_link_check: no spec issue linked",
      "started_at": "2026-…",
      "updated_at": "2026-…"
    },
    …
  ]
}
```

`status` derivations:

- `passed` — artifact `state == 'released'` (every stage passed).
- `failed` — artifact `state == 'archived'` (cancelled or terminally failed).
- `blocked` — `workflow_parked_reason` is non-null (stuck at a gate).
- `pending` — anything else (in-flight).

Surface to the user as a table or a tight list. The dashboard's run-detail URL is `<portal-url>/runs/<artifact_id>` — link each row when you can.

### Step 2 — drilling into one artifact

`get_artifact` returns the full row for a single artifact:

```json
{ "artifact_id": "art_…" }
```

Response shape includes:

- `id`, `workspace_id`, `kind` (`issue` / `pull_request` / `agent_session` / `deployment` / …), `name`, `state`.
- `owner`, `current_version`, `consumers` (downstream artifacts), `external_ref` (e.g. `github.com/org/repo/issues/42`).
- `workflow_id`, `current_stage_index`, `workflow_parked_reason`.
- `created_at`, `updated_at`, `last_observed_at`.

`external_ref` is the most useful field for surfacing to the user — if the artifact is a GitHub issue or PR, that's the link they actually want to click.

`consumers` is a JSON array of downstream artifact ids. If the user asks "what did this produce?", drill into each `consumer` with another `get_artifact` call. Don't fan out beyond two levels without asking — the consumer graph can be deep.

## Common shapes

### "What's running for workflow X right now?"

1. `list_runs { "workflow_id": "wf_…", "limit": 20 }`.
2. Filter client-side for `status == 'pending'` or `status == 'blocked'`.
3. Report.

### "Show me the last 5 runs of workflow X"

```json
{ "workflow_id": "wf_…", "limit": 5 }
```

### "What's the state of artifact `art_…`?"

```json
{ "artifact_id": "art_…" }
```

### "What did run `art_…` produce?"

1. `get_artifact { "artifact_id": "art_…" }`.
2. Read the `consumers` array.
3. For each entry, `get_artifact { "artifact_id": "<consumer_id>" }` and report the `kind` + `external_ref`.

## Failure modes to watch for

- **`NotFound: artifact `…` not found`** — typo in the id, or the artifact was hard-deleted (rare). Confirm the id with the user.
- **`NotFound: workflow `…` not found`** — typo in the workflow id. Use `list_workflows` (from `onsager-design-workflow`) to find the right one.
- **`Unauthorized`** — PAT doesn't have workspace access.

## Related skills

- `onsager-design-workflow` — list and edit the workflows themselves.
- `onsager-run-workflow` — fire a new run.
- `onsager-triage-run` — diagnose when `status` is `failed` or `blocked`.
