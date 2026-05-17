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

# Run synodic (governance) migrations.
if [ -n "$ONSAGER_DATABASE_URL" ] && [ -d /app/synodic-migrations ]; then
    echo "Running synodic migrations..."
    for f in $(ls /app/synodic-migrations/*.sql | sort -V); do
        echo "  applying $(basename "$f")..."
        psql -X -v ON_ERROR_STOP=1 "$ONSAGER_DATABASE_URL" -f "$f"
    done
    echo "Synodic migrations complete."
fi

# Start synodic (governance API) on an internal port.
# Not exposed by Railway — portal reverse-proxies /api/governance/* to it.
SYNODIC_PORT="${SYNODIC_PORT:-3001}"
echo "Starting synodic on :${SYNODIC_PORT}..."
gosu onsager sh -c "while true; do DATABASE_URL=\"$ONSAGER_DATABASE_URL\" /app/synodic serve --port $SYNODIC_PORT 2>&1; echo 'synodic exited, restarting in 1s...'; sleep 1; done" &

# Start onsager-portal on an internal port. Caddy (the edge dispatcher)
# routes /api/* and /agent/ws here (ADR 0006 + ADR 0008). Skipped when
# the spine DB isn't configured (useful for local smoke tests without
# portal wiring).
if [ -n "$ONSAGER_DATABASE_URL" ]; then
    PORTAL_PORT="${PORTAL_PORT:-3002}"
    PORTAL_BIND="${PORTAL_BIND:-127.0.0.1:${PORTAL_PORT}}"
    # Resolve nested expansion before passing to gosu to avoid dash multi-line
    # continuation bugs with complex parameter expansions inside "-quoted strings.
    PORTAL_CREDENTIAL_KEY="${STIGLAB_CREDENTIAL_KEY:-${ONSAGER_CREDENTIAL_KEY:-}}"
    PORTAL_SYNODIC_URL="${SYNODIC_URL:-http://127.0.0.1:${SYNODIC_PORT}}"
    echo "Starting onsager-portal on ${PORTAL_BIND}..."
    gosu onsager sh -c "while true; do PORTAL_BIND=\"$PORTAL_BIND\" DATABASE_URL=\"$ONSAGER_DATABASE_URL\" ONSAGER_CREDENTIAL_KEY=\"$PORTAL_CREDENTIAL_KEY\" SYNODIC_URL=\"$PORTAL_SYNODIC_URL\" /app/onsager-portal serve 2>&1; echo 'onsager-portal exited, restarting in 1s...'; sleep 1; done" &
fi

# Issue #156: legacy callers still expect STIGLAB_INTERNAL_DISPATCH_TOKEN
# in the environment as a per-boot ephemeral secret. The 0.1 forge ↔
# stiglab dispatch path is gone after spec #363, but synodic + stiglab
# still source the variable on startup; auto-generate to keep
# co-located processes trusting each other.
if [ -z "$STIGLAB_INTERNAL_DISPATCH_TOKEN" ]; then
    # 32 bytes of urandom rendered as 64 hex chars = 128 bits of entropy,
    # plenty for an in-container shared secret that's never written to
    # disk and rotates on every redeploy. `od` is in coreutils and ships
    # on every Debian/Alpine slim base — `xxd` is NOT (it's a separate
    # package in Debian and missing on most slim images).
    STIGLAB_INTERNAL_DISPATCH_TOKEN="$(od -An -vN32 -tx1 /dev/urandom | tr -d ' \n')"
    echo "Generated ephemeral STIGLAB_INTERNAL_DISPATCH_TOKEN"
fi
export STIGLAB_INTERNAL_DISPATCH_TOKEN

# Stiglab binds to 127.0.0.1:3000 (loopback only). The agent
# control-plane WebSocket is reachable from outside only through Caddy
# → portal → stiglab (ADR 0008). Stiglab is no longer the externally-
# reachable process (ADR 0006).
STIGLAB_PORT="${STIGLAB_PORT:-3000}"
STIGLAB_HOST="${STIGLAB_HOST:-127.0.0.1}"
echo "Starting stiglab on ${STIGLAB_HOST}:${STIGLAB_PORT}..."
gosu onsager sh -c "while true; do STIGLAB_HOST=\"$STIGLAB_HOST\" STIGLAB_PORT=\"$STIGLAB_PORT\" PORT=\"$STIGLAB_PORT\" STIGLAB_INTERNAL_DISPATCH_TOKEN=\"$STIGLAB_INTERNAL_DISPATCH_TOKEN\" /app/stiglab \"\$@\" 2>&1; echo 'stiglab exited, restarting in 1s...'; sleep 1; done" stiglab-supervisor "$@" &

# Caddy is the edge dispatcher — the only externally-reachable process.
# Routes /api/* and /agent/ws to portal on loopback, serves /assets/* +
# the SPA shell from /app/static. See ADR 0006 + ADR 0008.
#
# Default PORT for local `docker run` so the Caddyfile's `:{$PORT}` site
# address expands to a usable value when Railway/compose haven't set it.
: "${PORT:=8080}"
export PORT
echo "==> pre-exec: binaries present"
ls -la /app/stiglab /app/synodic /app/onsager-portal /usr/local/bin/caddy
echo "==> exec-ing caddy on :${PORT}..."
exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile
