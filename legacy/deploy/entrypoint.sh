#!/bin/sh
set -e

# Run onsager-spine migrations if ONSAGER_DATABASE_URL is set.
# The spine is a shared Postgres schema (events, events_ext, artifacts).
# All migrations are idempotent (IF NOT EXISTS / OR REPLACE).
if [ -n "$ONSAGER_DATABASE_URL" ] && [ -d /app/spine-migrations ]; then
    echo "Running onsager-spine migrations..."
    # sort -V orders by numeric segments so 001 < 002 < ... < 010
    # regardless of zero-padding; POSIX sh glob order is undefined.
    # Use for+command-substitution (not pipe+while) so set -e propagates
    # correctly and the loop runs in the current shell, not a subshell.
    for f in $(ls /app/spine-migrations/*.sql | sort -V); do
        echo "  applying $(basename "$f")..."
        psql -X -v ON_ERROR_STOP=1 "$ONSAGER_DATABASE_URL" -f "$f"
    done
    echo "Spine migrations complete."
fi

# Start onsager-portal on an internal port. Caddy (the edge dispatcher)
# routes /api/* and /mcp/* here (ADR 0006); portal is the front door —
# dashboard API, GitHub webhooks, MCP server, and the in-process agent
# session runner (ADR 0027 / #583). Skipped when the spine DB isn't
# configured (useful for local smoke tests without portal wiring).
if [ -n "$ONSAGER_DATABASE_URL" ]; then
    PORTAL_PORT="${PORTAL_PORT:-3002}"
    PORTAL_BIND="${PORTAL_BIND:-127.0.0.1:${PORTAL_PORT}}"
    PORTAL_CREDENTIAL_KEY="${ONSAGER_CREDENTIAL_KEY:-}"
    echo "Starting onsager-portal on ${PORTAL_BIND}..."
    gosu onsager sh -c "while true; do PORTAL_BIND=\"$PORTAL_BIND\" DATABASE_URL=\"$ONSAGER_DATABASE_URL\" ONSAGER_CREDENTIAL_KEY=\"$PORTAL_CREDENTIAL_KEY\" /app/onsager-portal serve 2>&1; echo 'onsager-portal exited, restarting in 1s...'; sleep 1; done" &
fi

# Start the substrate scheduler (RUN-03, #386) — listens on the spine
# for trigger.fired events, compiles each into an ExecutionPlan, and
# runs it through onsager_nodes::Scheduler. Skipped when the spine DB
# isn't configured; without DATABASE_URL there's nothing to listen on.
if [ -n "$ONSAGER_DATABASE_URL" ]; then
    SCHEDULER_ACTOR="${SCHEDULER_ACTOR:-substrate-scheduler}"
    SCHEDULER_REPLAY_HISTORY="${SCHEDULER_REPLAY_HISTORY:-false}"
    echo "Starting onsager-scheduler (actor=${SCHEDULER_ACTOR}, replay=${SCHEDULER_REPLAY_HISTORY})..."
    gosu onsager sh -c "while true; do DATABASE_URL=\"$ONSAGER_DATABASE_URL\" SCHEDULER_ACTOR=\"$SCHEDULER_ACTOR\" SCHEDULER_REPLAY_HISTORY=\"$SCHEDULER_REPLAY_HISTORY\" /app/onsager-scheduler serve 2>&1; echo 'onsager-scheduler exited, restarting in 1s...'; sleep 1; done" &
fi

# Caddy is the edge dispatcher — the only externally-reachable process.
# Routes /api/* and /mcp/* to portal on loopback, serves /assets/* +
# the SPA shell from /app/static. See ADR 0006.
#
# Default PORT for local `docker run` so the Caddyfile's `:{$PORT}` site
# address expands to a usable value when Railway/compose haven't set it.
: "${PORT:=8080}"
export PORT
echo "==> pre-exec: binaries present"
ls -la /app/onsager-portal /app/onsager-scheduler /usr/local/bin/caddy
echo "==> exec-ing caddy on :${PORT}..."
exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile
