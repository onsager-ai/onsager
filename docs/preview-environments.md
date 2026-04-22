# Preview environments

Every open pull request gets its own ephemeral Railway deploy with a
freshly-forked Postgres. The environment is created when the PR opens,
redeployed on each push, and torn down when the PR closes or merges.

This doc covers what's wired up, how to enable it on a fresh Railway
project, and the failure modes you're likely to hit.

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
| `STIGLAB_NODE_NAME` | `${{RAILWAY_ENVIRONMENT_NAME}}` → `pr-<N>` | Keeps agent-run heartbeats separated from prod |
| `STIGLAB_MAX_SESSIONS` | `2` | Preview envs get smaller quota — no multi-tenant load |
| `ONSAGER_PREVIEW` | `true` | Runtime flag subsystems can read to gate destructive ops |

See `[environments.preview]` in `railway.toml` for the source of truth.

## How it's wired

```
PR opened   ──▶  Railway (native PR envs)
                 ├─ forks Postgres plugin → empty DB
                 ├─ builds Docker image from PR SHA
                 └─ deploys → posts GitHub deployment_status (success)
                                │
                                ▼
deployment_status  ──▶  .github/workflows/preview-environment.yml
  event (from            ├─ extracts PR number from env name (`pr-<N>`)
  Railway's GH app)      ├─ pulls URL from event payload
                         ├─ curls /api/health (smoke)
                         └─ upserts a PR comment with URL + status

PR closed   ──▶  Railway auto-tears down env + plugin
                 └─ workflow updates the PR comment to note teardown
```

Railway owns the deploy lifecycle; the workflow only observes and
announces. The URL arrives in the event payload, so there's **no Railway
API token in CI** — previously we polled Railway's GraphQL API, but
project tokens are environment-locked and can't see ephemeral PR envs.

## First-time setup (per Railway project)

1. **Connect Railway to the GitHub repo** (standard Railway setup).
   This installs Railway's GitHub App, which is what fires the
   `deployment_status` events the workflow listens for. No extra config
   needed.

2. **Enable PR environments**
   Railway dashboard → Project Settings → *Pull Request Environments* → **Enabled**
   - Ephemeral Postgres: **Enabled** (do not clone prod data)
   - Wait-for-check: `deploy-ready`
   - Environment TTL: leave default (tears down with PR)

3. **Verify**
   Open a throwaway PR. When Railway's PR-env deploy finishes, the
   `preview-environment` workflow runs (triggered by `deployment_status`)
   and posts a sticky comment with the URL and smoke-test status. If no
   comment appears, check the workflow runs for this repo — they'll show
   whether `deployment_status` events are arriving.

No GitHub secrets are required for this workflow.

## Cost and quota

A preview is a full workspace build (~4–7 min) plus a running container
and Postgres plugin. At `numReplicas = 1` and the preview-only
`STIGLAB_MAX_SESSIONS = 2`, expect ~½ the runtime footprint of prod while
the PR is open.

Railway's free/starter tiers have per-project resource caps. If previews
are failing to schedule, check the project's quota before debugging the
workflow.

## Failure modes

**No preview comment on a PR.**
Either Railway didn't emit a `deployment_status` (build failed, PR envs
not enabled, Railway GitHub App not installed) or the workflow dropped
the event. Check:
1. The Railway project's `pr-<N>` environment — is there a deploy?
2. The repo's *Settings → Deployments* page — is there a deployment for
   this PR's SHA?
3. The `preview-environment` workflow runs — any failures?

**Smoke test reports `failed`.**
The container deployed but `/api/health` did not return 200. Usual suspects:
migrations failed (fresh DB but migration SQL is broken), a required secret
is not forwarded to the preview env, or the entrypoint crashed. Inspect
Railway's deploy logs for the `pr-<N>` env.

**PR number couldn't be extracted.**
The workflow expects Railway's env name to start with `pr-<N>`. If Railway
ever changes that convention, the `announce` job warns and exits without
commenting. Update the `sed` pattern in the workflow.

**Preview DB is not reset between pushes.**
By design. The DB is fresh on *PR open*, not on every push. If you need a
clean DB mid-review, close and reopen the PR, or delete the `pr-<N>`
environment in Railway (it'll be recreated on the next push).

**Fork PRs get no preview.**
Railway only deploys PR envs for branches in the same repo. Fork PRs
therefore emit no `deployment_status` and the workflow stays silent.
The `teardown-notice` job also short-circuits on fork PRs.

## Not in scope (yet)

- Preview envs for `push` to long-lived non-main branches — only PRs.
- Seeded data (demo workspace, canned sessions). Previews boot empty.
- Automatic e2e run against the preview URL. The smoke test is just
  `/api/health`; extend the workflow or run `just test-e2e-remote
  https://<preview-url>` manually if you need full coverage.
- Pre-release (tagged) previews. That would be a separate environment
  (`rc` or similar), not a PR env.
