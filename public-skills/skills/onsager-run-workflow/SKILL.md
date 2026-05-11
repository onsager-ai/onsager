---
name: onsager-run-workflow
description: Fire an Onsager workflow's manual trigger to start a new run and watch its first few stages. Triggers include "run this workflow", "execute the pipeline", "trigger a run", "kick off the digest workflow", "fire the backfill once", "run the manual workflow named go". Allowed tools cover the manual-trigger fire and the initial inspection step; deep diagnosis lives in `onsager-triage-run`.
allowed_tools:
- run_workflow
- list_workflows
- inspect_run
---

# onsager-run-workflow

A workflow's `manual` trigger is a named door — fire it with `run_workflow` and one new run starts. This skill covers the "I want this workflow to run *now*" path. Scheduled / webhook-triggered runs fire on their own; you don't need this skill for them.

## When this skill triggers

Phrases that should route here:

- "run the morning-digest workflow"
- "fire the backfill once"
- "trigger a run of the PR-review pipeline"
- "kick off the workflow named `nightly`"
- "execute the manual workflow `go`"

If the user wants to *design* a workflow, hand off to `onsager-design-workflow`. If they want to *triage* a failing run, hand off to `onsager-triage-run`.

## Operating procedure

### Step 1 — find the workflow

If the user names the workflow by id (`wf_…`), use it directly. Otherwise call `list_workflows` against the workspace and match by name:

```json
{ "workspace_id": "<ws>" }
```

The response is `{ "workflows": [ { "id": "wf_…", "name": "…", "active": true, "trigger": { … } }, … ] }`. Pick the row whose `name` matches and confirm `active: true` (an inactive workflow can't fire — point the user at the dashboard's activate switch).

Also confirm the trigger is `manual`. If it's `cron` / `github_*` / `spine_event`, this skill doesn't apply — runs of that workflow happen on their own schedule or in response to external events. Tell the user that and don't call `run_workflow`.

### Step 2 — pick the trigger name

`manual` triggers have a `name` field (set when the workflow was designed). The default is `go`. If the user said "fire the `nightly` trigger", pass `trigger_name: "nightly"`. If they didn't name one, try `"go"` first.

### Step 3 — call `run_workflow`

```json
{
  "workflow_id": "wf_…",
  "trigger_name": "go",
  "payload": { "user_note": "manual fire from chat" }
}
```

`payload` is optional. Canonical fields (`workflow_id`, `workspace_id`, `name`, `actor`, `source`, `fired_at`, `trigger_kind`) always override any colliding keys, so you can pass arbitrary structured data without worrying about clobbering the trigger's own metadata.

The tool returns the new artifact id — that's your `run_id`. Surface it to the user with the dashboard's run-detail deep-link: `<portal-url>/runs/<artifact_id>`.

### Step 4 — first-look inspection

Right after firing, call `inspect_run` with the returned artifact id and a small `event_limit`:

```json
{ "artifact_id": "<artifact_id>", "event_limit": 20 }
```

The response carries:

- `artifact.state` — usually `registered` immediately after fire, then transitions through `provisioning` / `running` as stages start.
- `artifact.current_stage_index` — 0 right after fire; advances as stages pass.
- `artifact.workflow_parked_reason` — `null` for a healthy run. If non-null, the artifact is parked at the current stage and this is **not** a fire-success path: hand off to `onsager-triage-run`.
- `recent_events` — newest first. Look for `trigger.fired` (your fire), then `stage.entered` events as the workflow advances.

Report back to the user: "Started run `<artifact_id>`, currently on stage `<index>` (`<gate_kind>`). Open the run at `/runs/<artifact_id>`." Don't loop polling `inspect_run` — one read is enough; the user opens the dashboard for live state.

## Common shapes

### "Fire workflow X now"

```json
{ "workflow_id": "wf_abc", "trigger_name": "go" }
```

### "Fire workflow X with some context"

```json
{
  "workflow_id": "wf_abc",
  "trigger_name": "go",
  "payload": {
    "reason": "ad-hoc backfill, requested by @alice",
    "scope": "open issues created before 2026-01-01"
  }
}
```

## Failure modes to watch for

- **`InvalidParams: workflow has no manual trigger named <name>`** — the workflow's trigger is scheduled or webhook-driven, or `trigger_name` doesn't match. Re-read the workflow's `trigger` field from `list_workflows`; if it's not `manual`, this skill doesn't apply.
- **`InvalidParams: workflow is inactive`** — workflow exists but `active: false`. Tell the user to activate it in the dashboard first; this skill cannot activate it (per `onsager-design-workflow`'s headers limitation).
- **`Unauthorized`** — PAT doesn't have workspace access.
- **`artifact.workflow_parked_reason` non-null on first inspect** — handed off to `onsager-triage-run`.

## Related skills

- `onsager-design-workflow` — create or modify the workflow blueprint.
- `onsager-explore-artifacts` — list past runs or fetch a specific artifact.
- `onsager-triage-run` — diagnose a run that failed or got stuck.
