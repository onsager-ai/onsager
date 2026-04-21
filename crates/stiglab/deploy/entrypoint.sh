#!/bin/sh
set -e

# Run onsager-spine migrations if ONSAGER_DATABASE_URL is set.
# The spine is a shared Postgres schema (events, events_ext, artifacts).
# All migrations are idempotent (IF NOT EXISTS / OR REPLACE).
if [ -n "$ONSAGER_DATABASE_URL" ] && [ -d /app/spine-migrations ]; then
    echo "Running onsager-spine migrations..."
    # sort -V orders by numeric segments so 001 < 002 < ... < 010
    # regardless of zero-padding; POSIX sh glob order is undefined.
    ls /app/spine-migrations/*.sql | sort -V | while read -r f; do
        echo "  applying $(basename "$f")..."
        psql -X -v ON_ERROR_STOP=1 "$ONSAGER_DATABASE_URL" -f "$f"
    done
    echo "Spine migrations complete."
fi

# Run synodic (governance) migrations.
if [ -n "$ONSAGER_DATABASE_URL" ] && [ -d /app/synodic-migrations ]; then
    echo "Running synodic migrations..."
    ls /app/synodic-migrations/*.sql | sort -V | while read -r f; do
        echo "  applying $(basename "$f")..."
        psql -X -v ON_ERROR_STOP=1 "$ONSAGER_DATABASE_URL" -f "$f"
    done
    echo "Synodic migrations complete."
fi

# Start synodic (governance API) on an internal port.
# Not exposed by Railway — stiglab reverse-proxies /api/governance/* to it.
SYNODIC_PORT="${SYNODIC_PORT:-3001}"
echo "Starting synodic on :${SYNODIC_PORT}..."
gosu onsager sh -c "while true; do DATABASE_URL=\"$ONSAGER_DATABASE_URL\" /app/synodic serve --port $SYNODIC_PORT 2>&1; echo 'synodic exited, restarting in 1s...'; sleep 1; done" &

# Start onsager-portal (GitHub webhook ingress) on an internal port.
# Not exposed by Railway — stiglab reverse-proxies /webhooks/github to it so
# the public surface stays a single service. Skipped entirely when the spine
# DB isn't configured (useful for local smoke tests without portal wiring).
if [ -n "$ONSAGER_DATABASE_URL" ]; then
    PORTAL_PORT="${PORTAL_PORT:-3002}"
    PORTAL_BIND="${PORTAL_BIND:-127.0.0.1:${PORTAL_PORT}}"
    echo "Starting onsager-portal on ${PORTAL_BIND}..."
    gosu onsager sh -c "\
        while true; do \
            PORTAL_BIND=\"$PORTAL_BIND\" \
            DATABASE_URL=\"$ONSAGER_DATABASE_URL\" \
            ONSAGER_CREDENTIAL_KEY=\"${STIGLAB_CREDENTIAL_KEY:-${ONSAGER_CREDENTIAL_KEY:-}}\" \
            SYNODIC_URL=\"${SYNODIC_URL:-http://127.0.0.1:$SYNODIC_PORT}\" \
            /app/onsager-portal serve 2>&1; \
            echo 'onsager-portal exited, restarting in 1s...'; \
            sleep 1; \
        done" &
fi

# Drop from root to unprivileged user and start stiglab.
# Claude Code CLI refuses --permission-mode bypassPermissions under root.
exec gosu onsager /app/stiglab "$@"
