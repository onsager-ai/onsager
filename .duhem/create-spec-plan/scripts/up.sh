#!/bin/sh
# Bring the Onsager dev stack up for the create-spec-plan verification
# and seed the one precondition the Create Plan compile gate needs: a
# workflow registered in the active workspace, so a valid spec kind
# exists for the combobox.
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment" — under
# Duhem control via the v1 environment-provisioning mechanism
# (onsager-ai/duhem#50), wired for the spec-plan flow by #79.
#
# Inherits a sanitized env (PATH, HOME, TMPDIR, LANG, LC_*, DUHEM_*).
# The suite lives inside the Onsager repo it verifies (Pattern D), so
# the target checkout is this repo — derived from the script's own
# location. DUHEM_ONSAGER_REPO_DIR still overrides it if you want to
# point at a different checkout; see ../README.md.
#
# Best-effort: prints what it's doing on stdout and surfaces a hard
# boot failure via a non-zero exit (Duhem maps `up:` non-zero to
# Inconclusive(EnvironmentError)). The workflow seed is non-fatal — if
# it can't complete headlessly it warns and exits 0 so the operator
# can seed manually (see ../README.md "Seeding the workflow").

set -eu

# The script sits at `.duhem/create-spec-plan/scripts/up.sh`, so the
# repo root is three levels up. DUHEM_ONSAGER_REPO_DIR overrides.
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../../.." && pwd)"
ONSAGER_REPO="${DUHEM_ONSAGER_REPO_DIR:-$REPO_ROOT}"
if [ ! -d "$ONSAGER_REPO" ]; then
  echo "up.sh: cannot find Onsager checkout at $ONSAGER_REPO" >&2
  echo "up.sh: set DUHEM_ONSAGER_REPO_DIR to an onsager-ai/onsager clone" >&2
  exit 2
fi

PORTAL_URL="${DUHEM_ONSAGER_PORTAL_URL:-http://localhost:3002}"
SPEC_KIND="${DUHEM_SPEC_KIND:-Issue}"
LOG="${TMPDIR:-/tmp}/onsager-dev.log"
COOKIES="${TMPDIR:-/tmp}/onsager-dev.cookies"

# Boot the full dev stack (Postgres + migrations + portal :3002 +
# stiglab :3000 + synodic :3001 + scheduler + dashboard :5173). `just
# dev` runs in the foreground with its own `wait`, so background it and
# record the PID for down.sh. Dev-login is on by default in Onsager's
# debug builds, and portal boot auto-seeds the dev user + `dev`
# workspace — so no user/workspace seeding is needed here.
echo "up.sh: starting onsager dev stack (just dev)"
( cd "$ONSAGER_REPO" && exec just dev >"$LOG" 2>&1 ) &
echo $! > "${TMPDIR:-/tmp}/onsager-dev.pid"

# Wait for portal before seeding — the readiness probe in duhem.yml
# gates the *checks* on the dashboard, but the seed below needs the
# portal MCP/REST surface up first.
echo "up.sh: waiting for portal at $PORTAL_URL/api/health"
i=0
until curl -fsS "$PORTAL_URL/api/health" >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 180 ]; then
    echo "up.sh: portal did not become healthy within ~180s; see $LOG" >&2
    exit 1
  fi
  sleep 1
done
echo "up.sh: portal healthy"

# --- Seed a workflow so the compile gate has a valid spec kind -------
# Non-fatal: a failure here means the operator must register a workflow
# of kind "$SPEC_KIND" manually before the run goes green (../README.md).
seed_workflow() {
  # Dev-login to obtain a session cookie (portal auto-creates the dev
  # user + `dev` workspace on first call).
  curl -fsS -c "$COOKIES" -X POST "$PORTAL_URL/api/auth/dev-login" \
    >/dev/null 2>&1 || return 1

  # Resolve the dev workspace id (slug `dev`). Split the array into one
  # line per workspace object (`tr '{' '\n'`), pick the line whose slug
  # is `dev`, and read its `id`. The response shape (confirmed live) is
  # `{"workspaces":[{...,"id":"<uuid>","name":...,"slug":"dev"}]}`.
  ws_json=$(curl -fsS -b "$COOKIES" "$PORTAL_URL/api/workspaces" 2>/dev/null) || return 1
  ws_id=$(printf '%s' "$ws_json" | tr '{' '\n' \
    | grep '"slug"[[:space:]]*:[[:space:]]*"dev"' | head -1 \
    | sed -E 's/.*"id"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
  # Fall back to the first id in the payload if the slug-scoped parse
  # missed (e.g. a single-workspace env or a shape change).
  [ -n "$ws_id" ] || ws_id=$(printf '%s' "$ws_json" \
    | grep -oE '"id"[[:space:]]*:[[:space:]]*"[^"]+"' | head -1 \
    | sed -E 's/.*"([^"]+)"$/\1/')
  [ -n "$ws_id" ] || return 1

  # Register a minimal single-node (no-op executor) workflow for the
  # spec kind via the portal MCP `submit_workflow` tool. The executor
  # `kind` discriminator is `noop` (onsager-substrate typetag); the
  # node `id` must be a UUID (NodeId), not a free string. This payload
  # was confirmed end-to-end against a live portal: `submit_workflow`
  # registers it and `compile_dry_run` then returns `ok: true` for a
  # one-spec plan of this kind — i.e. the dashboard's submit gate opens.
  node_id=$(uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid 2>/dev/null)
  body=$(printf '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"submit_workflow","arguments":{"workspace_id":"%s","spec_kind":"%s","workflow":{"nodes":[{"id":"%s","executor":{"kind":"noop"},"inputs":[],"outputs":[]}],"edges":[],"entry_specs":[],"output_specs":[]}}}}' \
    "$ws_id" "$SPEC_KIND" "$node_id")
  curl -fsS -b "$COOKIES" -X POST "$PORTAL_URL/mcp/messages" \
    -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' \
    -d "$body" >/dev/null 2>&1 || return 1
  echo "up.sh: registered workflow kind '$SPEC_KIND' in workspace $ws_id"
}

if seed_workflow; then
  echo "up.sh: workflow seed complete"
else
  echo "up.sh: WARNING — could not seed a workflow headlessly." >&2
  echo "up.sh: register a workflow of kind '$SPEC_KIND' in the 'dev'" >&2
  echo "up.sh: workspace manually before running (see ../README.md)." >&2
fi

# Exit zero — Duhem polls dashboard readiness asynchronously via the
# `environment.ready.http` probe. The stack survives this script
# because `just dev` was backgrounded.
exit 0
