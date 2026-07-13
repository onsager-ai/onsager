# `create-spec-plan`

Duhem Verification Definition for Onsager's workspace-scoped guided
**Create Plan** flow: assemble a SpecPlan from form rows, lint it live
through the `compile_dry_run` MCP tool, and commit it behind a
human-in-the-loop confirmation. Drives the real Vite dashboard against
the real portal MCP server, the real plan compiler, and a real
Postgres — no mock at the web boundary (onsager-ai/duhem
`docs/duhem-spec.md` §8).

- Criterion prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | The Create Plan page renders the authoring form (heading, Plan ID field, submit control) for an authenticated user. |
| **AC-2**  | A valid plan (Plan ID + a spec whose kind the compiler accepts) enables submit; confirming persists the plan and returns to the Spec Plans list. |
| **AC-3**  | The new plan appears on the Spec Plans list exactly once, identified by its Plan ID. |

## Requirements

- The Onsager `just dev` stack (Postgres + portal :3002 + dashboard
  :5173); `scripts/up.sh` boots it and seeds a workflow so the compile
  gate has a valid spec kind.
- A Playwright **Chromium** (this is a `ui/*` suite).

## Running

```sh
duhem run .duhem/create-spec-plan/duhem.yml --inputs plan_id=duhem-fixture-$(uuidgen)
duhem run .duhem/create-spec-plan/duhem.yml --no-env-up --inputs plan_id=…   # if already up
```

Get `duhem` from [onsager-ai/duhem releases](https://github.com/onsager-ai/duhem/releases).

## CI status

**Dormant.** Onsager is paused, and this UI suite needs the full
`just dev` stack + a Chromium — heavier than a page-free lane. The
`.github/workflows/duhem.yml` self-gate is gated on
`vars.ONSAGER_DUHEM_READY == 'true'` and stays skipped until Onsager
resumes and a CI stack bring-up lands. The VD lives here so it's ready
then; Duhem also monitors it for drift once armed.
