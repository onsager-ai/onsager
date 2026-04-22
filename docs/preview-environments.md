# Preview environments

Every open pull request gets its own ephemeral Railway deploy at
`https://onsager-pr-<number>.up.railway.app` with a freshly-forked Postgres.
The environment is created when the PR opens, redeployed on each push, and
torn down when the PR closes or merges.

This doc covers what's wired up, how to enable it on a new fork, and the
failure modes you're likely to hit.

## What ships per PR

Each preview is a full unified container (`stiglab` + `synodic` +
`onsager-portal`) plus a dedicated Postgres plugin. Migrations run at
container startup — see `crates/stiglab/deploy/entrypoint.sh` — so the
preview DB reflects the schema on the PR branch, not main.

The preview inherits project-level secrets (`STIGLAB_CREDENTIAL_KEY`, OAuth
client IDs, `GITHUB_TOKEN`) from the Railway project. Overrides that are
specific to preview:

| Variable | Preview default | Why |
|---|---|---|
| `STIGLAB_NODE_NAME` | `preview-<PR>` | Keeps agent-run heartbeats separated from prod |
| `STIGLAB_MAX_SESSIONS` | `2` | Preview envs get smaller quota — no multi-tenant load |
| `ONSAGER_PREVIEW` | `true` | Runtime flag subsystems can read to gate destructive ops |

See `[environments.preview]` in `railway.toml` for the source of truth.

## How it's wired

```
PR opened   ──▶  Railway (native PR envs)
                 ├─ forks Postgres plugin → empty DB
                 ├─ builds Docker image from PR SHA
                 └─ deploys → onsager-pr-<N>.up.railway.app
                                │
                                ▼
PR synced   ──▶  .github/workflows/preview-environment.yml
                 ├─ polls scripts/preview-url.sh until ready
                 ├─ curls /api/health (smoke)
                 └─ upserts a PR comment with URL + status

PR closed   ──▶  Railway auto-tears down env + plugin
                 └─ workflow updates the PR comment to note teardown
```

Railway owns the deploy lifecycle; the workflow only observes and
announces. That split is deliberate — we don't reinvent environment
provisioning in GitHub Actions.

## First-time setup (per Railway project)

1. **Enable PR environments**
   Railway dashboard → Project Settings → *Pull Request Environments* → **Enabled**
   - Ephemeral Postgres: **Enabled** (do not clone prod data)
   - Wait-for-check: `deploy-ready`
   - Environment TTL: leave default (tears down with PR)

2. **Create a project-scoped Railway token**
   Dashboard → Account Settings → Tokens → *New token* → scope to this
   project. Read-only is sufficient; the workflow never mutates Railway
   state.

3. **Set GitHub repo secrets**

   | Secret | Value |
   |---|---|
   | `RAILWAY_TOKEN` | the token from step 2 |
   | `RAILWAY_PROJECT_ID` | from `railway status --json` or the URL |
   | `RAILWAY_SERVICE_NAME` | optional; defaults to `onsager` |

4. **Verify**
   Open a throwaway PR. Within ~10 minutes the `preview-environment`
   workflow should post a comment with the preview URL. If it times out,
   check the workflow logs and the Railway project's `pr-<N>` environment
   directly.

## Cost and quota

A preview is a full workspace build (~4–7 min) plus a running container
and Postgres plugin. At `numReplicas = 1` and the preview-only
`STIGLAB_MAX_SESSIONS = 2`, expect ~½ the runtime footprint of prod while
the PR is open.

Railway's free/starter tiers have per-project resource caps. If previews
are failing to schedule, check the project's quota before debugging the
workflow.

## Failure modes

**Workflow times out, no URL posted.**
The build probably exceeded the 10-minute poll window. Check the Railway
project for the `pr-<N>` environment — the build is likely still running.
Re-run the workflow after the build completes to post the URL.

**Smoke test reports `failed`.**
The container deployed but `/api/health` did not return 200. Usual suspects:
migrations failed (fresh DB but migration SQL is broken), a required secret
is not forwarded to the preview env, or the entrypoint crashed. Inspect
Railway's deploy logs for the `pr-<N>` env.

**Workflow skips with a warning about missing secrets.**
`RAILWAY_TOKEN` or `RAILWAY_PROJECT_ID` are not configured. Previews still
deploy (Railway doesn't need the workflow), but no comment gets posted.
Fix by adding the secrets.

**Preview env exists but domain is empty.**
Railway sometimes needs a few seconds after deploy-success to provision the
domain. `scripts/preview-url.sh` exits 2 in this case so the workflow
retries.

**Preview DB is not reset between pushes.**
By design. The DB is fresh on *PR open*, not on every push. If you need a
clean DB mid-review, close and reopen the PR, or delete the `pr-<N>`
environment in Railway (it'll be recreated on the next push).

## Not in scope (yet)

- Preview envs for `push` to long-lived non-main branches — only PRs.
- Seeded data (demo workspace, canned sessions). Previews boot empty.
- Automatic e2e run against the preview URL. The smoke test is just
  `/api/health`; extend the workflow or run `just test-e2e-remote
  https://onsager-pr-<N>.up.railway.app` manually if you need full
  coverage.
- Pre-release (tagged) previews. That would be a separate environment
  (`rc` or similar), not a PR env.
