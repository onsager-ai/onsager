---
name: onsager-triage-run
description: Diagnose why an Onsager run failed, got stuck, or parked unexpectedly — pull the artifact summary, fetch agent-session logs, surface log pointers from the v1 stub, and (when there's no path forward) cancel the run. Triggers include "the run failed", "diagnose this", "why did it fail", "this workflow is stuck", "the artifact is parked", "look at the logs for run X", "cancel run X". This skill does the *reasoning* client-side because `propose_remediation` is a v1 stub that only returns pointers — server-side AI reasoning is a follow-up to ADR 0007.
allowed_tools:
- inspect_run
- get_stage_logs
- propose_remediation
- cancel_run
---

# onsager-triage-run

A run failed, got stuck at a stage, or parked with a reason the user can't decode at a glance. This skill walks the inspection chain — artifact summary → log pointers → stage logs → client-side reasoning → optional cancellation.

**Heads-up:** `propose_remediation` is a v1 **stub**. It does not call an LLM server-side; it returns the failed run's state plus structured log pointers and a fixed `suggested_next_tools` list. The reasoning happens here, in this skill, in the client. The Full version is a follow-up to [ADR 0007](https://github.com/onsager-ai/onsager/blob/main/docs/adr/0007-tools-and-skills-as-the-public-contract.md). Until then, treat the stub's response as a starting point and chain `get_stage_logs` yourself.

## When this skill triggers

Phrases that should route here:

- "the run failed"
- "why did this workflow fail?"
- "diagnose this run"
- "the artifact is parked at stage 2"
- "look at the logs for session `sess_…`"
- "cancel the run for artifact `art_…`"
- "this workflow is stuck and I don't know why"

## Operating procedure

### Step 1 — summary

Start with `inspect_run` on the artifact:

```json
{ "artifact_id": "<artifact_id>", "event_limit": 50 }
```

Read these fields:

- `artifact.state` — `archived` means the run was cancelled or hit a terminal failure. `released` means it actually finished. `provisioning` / `running` means in-flight; `parked` means stuck at a gate.
- `artifact.workflow_parked_reason` — non-null means parked. This string is the **first thing to surface** to the user; it's often the whole answer ("`spec_link_check: no spec issue linked`").
- `artifact.current_stage_index` — which stage parked / failed.
- `recent_events` — newest first. Scan for the last `stage.entered`, `stage.exited`, `synodic.gate_verdict`, `stiglab.session_*` events. Each `stiglab.session_*` event carries a `session_id` you'll need below.

If `workflow_parked_reason` plus the recent events make the failure obvious (e.g. "spec_link_check parked because no `Closes #N` in the PR body"), tell the user and stop. Don't fetch logs you don't need.

### Step 2 — gather log pointers

If the failure was inside an `agent_session` stage, call `propose_remediation`:

```json
{ "artifact_id": "<artifact_id>" }
```

The v1 response shape:

```json
{
  "v1_stub": true,
  "stub_reason": "v1 returns log pointers; server-side AI reasoning is a follow-up.",
  "failure_summary": { … same as inspect_run … },
  "log_pointers": [
    { "session_id": "sess_…", "event_type": "stiglab.session_failed", "created_at": "…" },
    …
  ],
  "suggested_next_tools": ["get_stage_logs"]
}
```

`log_pointers` is the input you need for step 3. Each entry's `session_id` corresponds to one agent-session stage execution; the newest (first in the list) is usually the one that failed.

### Step 3 — read the logs

For the failing session, call `get_stage_logs`:

```json
{ "session_id": "sess_…", "since_seq": 0 }
```

The response carries:

- `state` — `failed`, `completed`, `cancelled`, etc.
- `chunks` — ordered log chunks, each with `seq`, `stream` (`stdout` / `stderr` / `tool`), and `chunk` (the text). Read from the **end** — the failure tail is usually in the last few chunks.

If the session is long-running and you only want the tail, pass `since_seq` larger than 0. The lint accepts any non-negative integer.

### Step 4 — reason about the failure (client-side)

Combine:

- The `workflow_parked_reason` (if any) — the gate's own diagnosis.
- The session `state` — terminal failure vs. interrupted.
- The last ~20 chunks of the failing session — what the agent was doing when it died.
- The `recent_events` from step 1 — what the workflow runtime saw.

Surface to the user **one** of:

- **Mechanical failure** ("the agent timed out", "rate-limited by GitHub") — tell them what the failure was and that re-running often fixes it.
- **Prompt / data failure** ("the agent didn't see a spec issue to link") — tell them which input was missing.
- **Workflow design failure** ("`spec_link_check` is gating a PR that legitimately has no spec because it's a docs-only change") — suggest the workflow needs a different gate, and hand off to `onsager-design-workflow` for a fix.

Don't fabricate a fix you can't verify. If the logs don't tell you, say so explicitly.

### Step 5 — optional cancellation

If the user wants to stop the run (it's not going to recover on its own, or they want a clean slate before re-firing), use `cancel_run`:

```json
{ "artifact_id": "<artifact_id>", "reason": "manual cancel after triage — root cause: <one-line summary>" }
```

`cancel_run` is **destructive and irreversible at the artifact level**: it sets `state = 'archived'` and emits `artifact.archived` on the `forge:<artifact_id>` stream. Confirm with the user before firing it. After cancel, the same trigger event won't re-run the workflow automatically — the user has to re-fire via `onsager-run-workflow` (if manual) or re-trigger upstream (if webhook-driven).

If the artifact is already `archived`, `cancel_run` returns `InvalidParams: artifact already archived`. Just tell the user — no further action.

## Common failure shapes

### `workflow_parked_reason = "spec_link_check: no spec issue linked"`

The PR body doesn't have `Closes #N` or `Part of #N`. Tell the user; if they want, suggest a one-line edit to the PR body and that the workflow will re-evaluate on the next push.

### `workflow_parked_reason = "synodic_review: pending"`

A human needs to approve via the dashboard's governance surface. Not a bug — just waiting.

### `stiglab.session_failed` with `chunks` showing `RATE_LIMITED`

Anthropic API rate limit or GitHub rate limit. Suggest re-firing in a few minutes.

### `stiglab.session_failed` with `chunks` showing the agent looping on the same tool

Prompt design bug — the agent's prompt has no exit condition. Hand off to `onsager-design-workflow` to revise the stage's `params.prompt`.

## Related skills

- `onsager-design-workflow` — when triage points at a workflow-design fix.
- `onsager-run-workflow` — when you cancel and want to re-fire.
- `onsager-explore-artifacts` — when you want to see what *did* land before the failure.
