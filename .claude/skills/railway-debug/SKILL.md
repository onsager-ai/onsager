---
name: railway-debug
description: Debug and manage Railway deployments for the Onsager project. Use when asked to "check railway", "debug deployment", "railway logs", "why is deploy failing", "redeploy", "check service status", "railway variables", or any Railway deployment troubleshooting. Requires ONSAGER_RAILWAY_TOKEN env var.
allowed-tools: Bash(RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway *)
---

# Railway Deployment Debugging

Debug, inspect, and manage Railway deployments for the Onsager monorepo.

## Authentication

All commands use the project-scoped token via env var:

```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway <command>
```

This is a **project token** (not a user token). `whoami` will fail — that's expected. Use `railway status` to verify the token works.

If the token is not set, ask the user to export `ONSAGER_RAILWAY_TOKEN` or set it in their `.env` / shell profile.

## Project Layout

| Service       | Description                          | Port |
|---------------|--------------------------------------|------|
| **onsager**   | Stiglab (sessions, agents, dashboard, API) | 3000 |
| *(planned)*   | Synodic (governance API)             | 3001 |
| **PostgreSQL** | Shared event spine (Railway plugin)  | 5432 |

Config files:
- `railway.toml` — root config-as-code (stiglab primary service)
- `crates/stiglab/deploy/Dockerfile` — stiglab multi-stage build
- `crates/stiglab/deploy/entrypoint.sh` — auto-migrating entrypoint
- `deploy/synodic.Dockerfile` — synodic build

## Preflight Check

**Run this before any deploy or when triaging a failure.** It catches the known
classes of dev/prod divergence that have burned us before.

```bash
# 1. Lockfiles tracked in git (build context comes from git, not local fs)
git ls-files --error-unmatch Cargo.lock pnpm-lock.yaml

# 2. Dockerfiles don't COPY files that are gitignored
#    (parse COPY sources from Dockerfiles, verify each is tracked)
for f in crates/stiglab/deploy/Dockerfile deploy/synodic.Dockerfile; do
  grep -oP '^\s*COPY\s+\K\S+' "$f" | grep -v -- '--from=' | while read src; do
    [ -f "$src" ] && git ls-files --error-unmatch "$src" 2>/dev/null || true
  done
done

# 3. Railway vars don't contain localhost (dev values leaked to prod)
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway variable list --service onsager 2>&1 \
  | grep -i localhost && echo "WARN: localhost found in Railway vars" || echo "OK: no localhost refs"

# 4. Database URLs reference Railway plugin, not hardcoded
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway variable list --service onsager 2>&1 \
  | grep -E 'DATABASE_URL.*railway\.internal' > /dev/null \
  && echo "OK: DATABASE_URL points to Railway Postgres" \
  || echo "WARN: DATABASE_URL may not reference Railway Postgres plugin"
```

If any check fails, fix it before deploying. See "Common Failure Modes" below for
specific fixes.

## Debugging Workflow

### 1. Check Service Status

```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway service status --all
```

States: `DEPLOYING`, `SUCCESS`, `FAILED`, `CRASHED`, `REMOVED`

### 2. Inspect Logs

**Build logs** (Dockerfile build failures):
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway logs --service onsager --build --lines 100 --latest
```

**Deploy/runtime logs** (startup crashes, runtime errors):
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway logs --service onsager --lines 100 --latest
```

**Error-only logs**:
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway logs --service onsager --lines 50 --filter "@level:error"
```

**HTTP request logs** (health check failures, 5xx errors):
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway logs --service onsager --http --status ">=400" --lines 50
```

**Time-bounded logs**:
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway logs --service onsager --since 1h --until 10m
```

### 3. Check Environment Variables

```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway variable list --service onsager
```

Required variables for stiglab:
- `DATABASE_URL` — auto-injected by Railway Postgres plugin
- `ONSAGER_DATABASE_URL` — set to `${{Postgres.DATABASE_URL}}`
- `STIGLAB_CREDENTIAL_KEY` — 32-byte hex AES-256-GCM key
- `PORT` — auto-injected by Railway

### 4. Redeploy

After fixing a build issue (e.g., pushing a fix to the repo):
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway redeploy --service onsager
```

To restart without rebuild (runtime issues):
```bash
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway restart --service onsager
```

## Common Failure Modes

### Build Failures

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| `Cargo.lock not found` | `Cargo.lock` not in git (`.gitignore` excludes it) | Remove `/Cargo.lock` from `.gitignore`, commit lockfile |
| `pnpm-lock.yaml not found` | Lockfile not committed | Commit lockfile |
| Rust compilation error | Code doesn't compile | Fix locally with `cargo build -p stiglab` |
| `COPY ... not found` | File path in Dockerfile doesn't match repo layout | Check Dockerfile COPY paths match workspace structure |
| OOM during build | Rust compile uses too much memory | Check Railway plan limits; use `--release` build caching |

### Runtime Failures

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| Health check timeout | Service takes >300s to start, or `/api/health` not responding | Check entrypoint.sh, migration duration, port binding |
| `ONSAGER_DATABASE_URL` not set | Missing Railway variable | Set to `${{Postgres.DATABASE_URL}}` in Railway dashboard |
| Migration failure | SQL error in spine migrations | Check `crates/onsager-spine/migrations/` for issues |
| Port mismatch | `PORT` env not mapped to `STIGLAB_PORT` | Entrypoint should handle this; check entrypoint.sh |

### Deployment Stalls

If a deployment is stuck:
```bash
# Check latest deployment status
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway service status --all

# Stream live logs to see what's happening
RAILWAY_TOKEN="$ONSAGER_RAILWAY_TOKEN" railway logs --service onsager --latest
```

## Verifying a Fix

After pushing a fix and redeploying:

1. Watch build logs stream: `railway logs --service onsager --build --latest`
2. Once build succeeds, watch deploy logs: `railway logs --service onsager --latest`
3. Confirm status: `railway service status --all` → should show `SUCCESS`
4. Check health: `railway logs --service onsager --http --path /api/health --lines 5`
