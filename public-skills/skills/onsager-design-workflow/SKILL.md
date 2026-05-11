---
name: onsager-design-workflow
description: Design a new Onsager workflow — pick a trigger, chain ordered stages, and create the workflow inactive via the portal MCP server. Triggers include "design a workflow", "create an automation", "build a pipeline", "set up a workflow that runs on every issue comment", "schedule a workflow to run every morning", "deactivate this workflow". Allowed tools cover the full create/list/edit/schedule surface; activation runs through the REST PATCH endpoint, not MCP.
allowed_tools:
- propose_workflow
- edit_workflow
- list_workflows
- schedule_workflow
---

# onsager-design-workflow

A **workflow** in Onsager is a trigger plus an ordered chain of stages. The trigger says when to fire; the stages say what to do, in order, until something gates the artifact or releases it. This skill creates and shapes those blueprints. Running a workflow lives in `onsager-run-workflow`; triaging a stuck run lives in `onsager-triage-run`.

## When this skill triggers

Phrases that should route here:

- "design a workflow that runs on every issue comment"
- "create an automation for new pull requests"
- "build a pipeline that triages incoming issues"
- "schedule a workflow to run every morning at 09:00"
- "deactivate the spec-link workflow"
- "list the workflows in this workspace"

If the user is asking to *fire* an existing workflow ("run this", "trigger a run"), hand off to `onsager-run-workflow`.

## Operating procedure

### Step 1 — confirm the workspace

Every tool here takes a `workspace_id`. If the user hasn't named one, call `list_workflows` against the workspace they're working in (the chat surface usually has this in scope; otherwise ask). Never invent a workspace id.

### Step 2 — pick a trigger

The portal MCP server accepts these trigger kinds (canonical list in [`onsager-spine::TriggerKind`](https://github.com/onsager-ai/onsager/blob/main/crates/onsager-spine/src/trigger.rs)):

- `manual` — fires only on `run_workflow`.
- `cron` — fires on a cron schedule.
- `interval` — fires every `N` seconds/minutes/hours.
- `delay` — fires once, after a delay.
- `github_issue_opened`, `github_issue_commented`, `github_pull_request_opened`, `github_pull_request_review`, `github_pr_comment`, `github_check_run` — fire on the named GitHub webhook.
- `spine_event` — fires on a spine event kind (e.g. `artifact.registered`). **Forbidden case**: `spine_event { event_kind: "trigger.fired" }` self-amplifies and the tool rejects it.

GitHub triggers need an `install_id` (the GitHub App installation row id). For schedule and manual triggers, pass `install_id: 0`.

### Step 3 — chain stages

Each stage is a `{ gate_kind, params? }` pair. Gate kinds are registered in [`onsager-portal::workflow::GateKind`](https://github.com/onsager-ai/onsager/blob/main/crates/onsager-portal/src/workflow.rs). The most common today:

- `agent_session` — dispatches a stiglab agent session with a prompt. `params: { prompt: "..." }`.
- `spec_link_check` — passes only if the linked spec issue is in a valid state.
- `synodic_review` — sends the artifact to the synodic gate for human/governance review.

The order matters: stage `0` runs first, then `1`, then `2`. A stage failing parks the artifact at that stage with a `parked_reason`; the workflow only releases when every stage passes.

### Step 4 — call `propose_workflow`

```json
{
  "workspace_id": "ws_<uuid>",
  "name": "Triage every new issue",
  "trigger": { "kind": "github_issue_opened" },
  "install_id": 12345,
  "stages": [
    { "gate_kind": "agent_session", "params": { "prompt": "Classify this issue and add labels." } },
    { "gate_kind": "synodic_review" }
  ]
}
```

Important: **do not pass `active: true`**. The MCP entry point doesn't plumb the request headers the activation pipeline needs (it does the GitHub label-create + webhook-register side-effects), so the tool returns `InvalidParams` if you ask for inline activation. Workflows always land inactive; the user activates them via the dashboard or REST PATCH.

After the call succeeds, tell the user: "Created workflow `<name>` (id `<wf_…>`), currently inactive. Activate it in the dashboard at **Workflows → <name> → Activate**." Don't try to activate it through MCP.

### Step 5 — schedule changes

To change an existing workflow's trigger (e.g. swap cron expressions, or move from manual to cron), use `schedule_workflow`:

```json
{ "workflow_id": "wf_…", "trigger": { "kind": "cron", "expr": "0 9 * * *" } }
```

`schedule_workflow` replaces the trigger atomically. It validates the kind against the registry manifest and rejects the self-amplifying `spine_event { event_kind: "trigger.fired" }` case for the same reason `propose_workflow` does.

### Step 6 — deactivate

`edit_workflow` can deactivate (`active: false`). Re-activation is **not** supported via MCP today — same headers issue as `propose_workflow`. If the user asks to re-activate, point them at the dashboard or REST PATCH.

```json
{ "workflow_id": "wf_…", "active": false }
```

## Common shapes (copy-paste templates)

### "Run a stage on every new PR"

```json
{
  "workspace_id": "<ws>",
  "name": "PR review pass",
  "trigger": { "kind": "github_pull_request_opened" },
  "install_id": <install_id>,
  "stages": [
    { "gate_kind": "agent_session", "params": { "prompt": "Review this PR for spec drift." } }
  ]
}
```

### "Run something every morning"

```json
{
  "workspace_id": "<ws>",
  "name": "Morning digest",
  "trigger": { "kind": "cron", "expr": "0 9 * * *" },
  "install_id": 0,
  "stages": [
    { "gate_kind": "agent_session", "params": { "prompt": "Summarise yesterday's merged PRs." } }
  ]
}
```

### "A one-shot workflow I'll fire by hand"

```json
{
  "workspace_id": "<ws>",
  "name": "Ad-hoc backfill",
  "trigger": { "kind": "manual", "name": "go" },
  "install_id": 0,
  "stages": [
    { "gate_kind": "agent_session", "params": { "prompt": "Backfill missing labels on open issues." } }
  ]
}
```

## Failure modes to watch for

- **`InvalidParams: MCP propose_workflow cannot activate inline …`** — you passed `active: true`. Drop it and tell the user to activate from the dashboard.
- **`InvalidParams: trigger kind `…` is not in the registry manifest`** — typo in the trigger kind, or the kind hasn't shipped yet. Fall back to `manual` while the user files a spec for the new kind.
- **`InvalidParams: spine_event workflow cannot listen for `trigger.fired` …`** — the requested workflow would self-amplify. Suggest a different event kind (e.g. `artifact.registered`).
- **`Unauthorized`** — the PAT doesn't have access to the workspace the user named. Have them re-issue the PAT scoped to that workspace.

## Related skills

- `onsager-run-workflow` — fire a workflow once it's active.
- `onsager-explore-artifacts` — inspect what a run produced.
- `onsager-triage-run` — diagnose a run that failed or got stuck.
