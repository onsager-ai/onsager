# Criteria — Onsager dashboard: create a spec plan

Human commitments about what "done" means for the Onsager
dashboard's guided **Create Plan** flow (Onsager spec #542). These
criteria are stable across implementation churn; the checks in
`duhem.yml` are the derivative, mechanically-judged translation and
may change as the dashboard's markup changes
(`docs/duhem-spec.md` §7.2 / §7.3).

The flow is workspace-scoped: an authenticated user opens the
Create Plan surface for their active workspace, assembles a plan
from spec rows, and submits it. The plan is persisted and the user
returns to the workspace's Spec Plans list with the new plan
present.

## AC-1 — The Create Plan surface presents the authoring form

An authenticated user who opens the Create Plan page for their
workspace sees the plan-authoring form: a heading identifying the
page, a Plan ID field ready for entry, a spec row, and a submit
control.

## AC-2 — Submitting a valid plan commits it and returns to the list

When the user fills the form with a valid plan — a Plan ID and at
least one spec whose kind the plan compiler accepts — the submit
control becomes available, and confirming the submission persists
the plan and navigates the user back to the workspace's Spec Plans
list.

## AC-3 — The committed plan appears in the Spec Plans list

After submission, the new plan is listed on the workspace's Spec
Plans page, identified by the Plan ID the user supplied, exactly
once.
