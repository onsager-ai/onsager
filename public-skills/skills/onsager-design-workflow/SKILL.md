---
name: onsager-design-workflow
description: Design a new Onsager workflow — pick a trigger, chain ordered stages, and create the workflow inactive via the portal MCP server. Triggers include "design a workflow", "create an automation", "build a pipeline", "set up a workflow that fires on issue webhooks", "schedule a workflow to run every morning", "deactivate this workflow". Allowed tools cover the full create/list/edit/schedule surface; activation runs through the REST PATCH endpoint, not MCP.
allowed_tools:
- propose_workflow
- edit_workflow
- list_workflows
- schedule_workflow
---

# onsager-design-workflow

A **workflow** in Onsager is a trigger plus an ordered chain of stages. The trigger says when to fire; the stages say what to do, in order, until something parks the artifact or releases it. This skill creates and shapes those blueprints. Running a workflow lives in `onsager-run-workflow`; triaging a stuck run lives in `onsager-triage-run`.

## When this skill triggers

Phrases that should route here:

- "design a workflow that runs on a GitHub issue label"
- "create an automation for merged pull requests"
- "build a pipeline that triages incoming issues"
- "schedule a workflow to run every morning at 09:00"
- "deactivate the morning-digest workflow"
- "list the workflows in this workspace"

If the user is asking to *fire* an existing workflow ("run this", "trigger a run"), hand off to `onsager-run-workflow`.

## Operating procedure

### Step 1 — confirm the workspace

Every tool here takes a `workspace_id`. If the user hasn't named one, call `list_workflows` against the workspace they're working in (the chat surface usually has this in scope; otherwise ask). Never invent a workspace id.

### Step 2 — pick a trigger

The portal MCP server accepts the trigger kinds defined in [`onsager-spine::TriggerKind`](https://github.com/onsager-ai/onsager/blob/main/crates/onsager-spine/src/trigger.rs). The wire form is `{ "kind": "<snake_case_tag>", … }` with the variant's fields alongside.

External (webhook) kinds:

- `github_issue_webhook` — fires on `issues.labeled` whose label matches. Fields: `repo` (`"owner/name"`), `label`.
- `github_pull_request_closed` — fires on `pull_request.closed`. Fields: `repo`, optional `predicate.merged` (`true` for merged-only, `false` for closed-without-merge, omit for any close).
- `github_workflow_run_completed` — fires on `workflow_run.completed`. Fields: `repo`, `workflow_name`, optional `event`, `head_branch`, `conclusion`.
- `telegram_webhook` — fires on a Telegram bot update. Fields: `bot_username`, optional `chat_id_allowlist`, optional `command_prefix` (e.g. `/onsager`).

Schedule kinds:

- `cron` — cron schedule. Fields: `expression` (5- or 6-field cron string), optional `timezone` (IANA name; defaults to UTC).
- `delay` — fire once after a delay. Fields: `seconds`, optional `anchor` (v1 only `workflow_activated_at`).
- `interval` — periodic. Fields: `period_seconds`.

Event kinds:

- `spine_event` — fire on a `FactoryEventKind` whose `type` matches. Fields: `event_kind`, optional `filter.equals` (JSON-shape predicate). **Forbidden**: `event_kind: "trigger.fired"` self-amplifies and the tool rejects it.
- `pg_notify` — fire on a Postgres `NOTIFY <channel>`. Fields: `channel`, optional `filter`.
- `outbox_row` — fire when a row matching `where_clause` is inserted into `table`. Fields: `table`, `where_clause`.

Manual kinds:

- `manual` — fire on demand. Fields: `name` (the button label).
- `replay` — re-emit a past `TriggerFired` event by id. Fields: `source_event_id`.

`install_id` is the GitHub App installation row id. Required for `github_*` triggers; pass `0` for schedule / manual / event-bus triggers.

### Step 3 — chain stages

Each stage is a `{ gate_kind, params? }` pair. `gate_kind` is a **kebab-case** string from [`onsager-portal::workflow::GateKind`](https://github.com/onsager-ai/onsager/blob/main/crates/onsager-portal/src/workflow.rs):

- `agent-session` — dispatches a stiglab agent session with a prompt. `params: { "prompt": "…" }`.
- `external-check` — an external pass/fail check (e.g. spec-link validation). `params` defines the check.
- `governance` — sends the artifact to the synodic gate for governance review.
- `manual-approval` — parks for a human click in the dashboard.

The order matters: stage `0` runs first, then `1`, then `2`. A stage that doesn't pass parks the artifact at that stage with a `workflow_parked_reason`; the workflow only releases when every stage passes (artifact state → `released`).

### Step 4 — call `propose_workflow`

```json
{
  "workspace_id": "ws_<uuid>",
  "name": "Triage every labeled issue",
  "trigger": { "kind": "github_issue_webhook", "repo": "acme/widgets", "label": "needs-triage" },
  "install_id": 12345,
  "stages": [
    { "gate_kind": "agent-session", "params": { "prompt": "Classify this issue and add labels." } },
    { "gate_kind": "governance" }
  ]
}
```

Important: **do not pass `active: true`**. The MCP entry point doesn't plumb the request headers the activation pipeline needs (it does the GitHub label-create + webhook-register side-effects), so the tool returns `InvalidParams` if you ask for inline activation. Workflows always land inactive; the user activates them via the dashboard or REST PATCH.

After the call succeeds, tell the user: "Created workflow `<name>` (id `<wf_…>`), currently inactive. Activate it in the dashboard at **Workflows → <name> → Activate**." Don't try to activate it through MCP.

### Step 5 — schedule changes

To change an existing workflow's trigger (e.g. swap cron expressions, or move from manual to cron), use `schedule_workflow`:

```json
{
  "workflow_id": "wf_…",
  "trigger": { "kind": "cron", "expression": "0 9 * * *", "timezone": "UTC" }
}
```

`schedule_workflow` replaces the trigger atomically. It validates the kind against the registry manifest and rejects the self-amplifying `spine_event { event_kind: "trigger.fired" }` case for the same reason `propose_workflow` does.

### Step 6 — deactivate

`edit_workflow` can deactivate (`active: false`). Re-activation is **not** supported via MCP today — same headers issue as `propose_workflow`. If the user asks to re-activate, point them at the dashboard or REST PATCH.

```json
{ "workflow_id": "wf_…", "active": false }
```

## Common shapes (copy-paste templates)

### "Run a stage on every merged PR"

```json
{
  "workspace_id": "<ws>",
  "name": "Merged-PR review pass",
  "trigger": {
    "kind": "github_pull_request_closed",
    "repo": "acme/widgets",
    "predicate": { "merged": true }
  },
  "install_id": <install_id>,
  "stages": [
    { "gate_kind": "agent-session", "params": { "prompt": "Review this PR for spec drift." } }
  ]
}
```

### "Run something every morning"

```json
{
  "workspace_id": "<ws>",
  "name": "Morning digest",
  "trigger": { "kind": "cron", "expression": "0 9 * * *", "timezone": "UTC" },
  "install_id": 0,
  "stages": [
    { "gate_kind": "agent-session", "params": { "prompt": "Summarise yesterday's merged PRs." } }
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
    { "gate_kind": "agent-session", "params": { "prompt": "Backfill missing labels on open issues." } }
  ]
}
```

### "Run on a spine event"

```json
{
  "workspace_id": "<ws>",
  "name": "On artifact registration",
  "trigger": {
    "kind": "spine_event",
    "event_kind": "artifact.registered",
    "filter": { "equals": { "$.payload.kind": "pull_request" } }
  },
  "install_id": 0,
  "stages": [
    { "gate_kind": "agent-session", "params": { "prompt": "Welcome the artifact author." } }
  ]
}
```

## Failure modes to watch for

- **`InvalidParams: MCP propose_workflow cannot activate inline …`** — you passed `active: true`. Drop it and tell the user to activate from the dashboard.
- **`InvalidParams: trigger kind `…` is not in the registry manifest`** — typo in the trigger kind, or the kind hasn't shipped yet. Fall back to `manual` while the user files a spec for the new kind.
- **`InvalidParams: spine_event workflow cannot listen for `trigger.fired` …`** — the requested workflow would self-amplify. Suggest a different event kind (e.g. `artifact.registered`).
- **Schema validation error on `gate_kind`** — `gate_kind` is kebab-case (`agent-session`, not `agent_session`). Same for `trigger.kind` — snake_case (`github_issue_webhook`, not `githubIssueWebhook`).
- **`Unauthorized`** — the PAT doesn't have access to the workspace the user named. Have them re-issue the PAT scoped to that workspace.

## Related skills

- `onsager-run-workflow` — fire a workflow once it's active.
- `onsager-explore-artifacts` — inspect what a run produced.
- `onsager-triage-run` — diagnose a run that failed or got stuck.
