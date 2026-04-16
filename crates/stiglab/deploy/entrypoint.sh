#!/bin/sh
set -e

# Run onsager-spine migrations if ONSAGER_DATABASE_URL is set.
# The spine is a shared Postgres schema (events, events_ext, artifacts).
# All migrations are idempotent (IF NOT EXISTS / OR REPLACE).
if [ -n "$ONSAGER_DATABASE_URL" ] && [ -d /app/spine-migrations ]; then
    echo "Running onsager-spine migrations..."
    for f in /app/spine-migrations/*.sql; do
        echo "  applying $(basename "$f")..."
        psql -X -v ON_ERROR_STOP=1 "$ONSAGER_DATABASE_URL" -f "$f"
    done
    echo "Spine migrations complete."
fi

# Run synodic (governance) migrations.
if [ -n "$ONSAGER_DATABASE_URL" ] && [ -d /app/synodic-migrations ]; then
    echo "Running synodic migrations..."
    for f in /app/synodic-migrations/*.sql; do
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

# Drop from root to unprivileged user and start stiglab.
# Claude Code CLI refuses --permission-mode bypassPermissions under root.
exec gosu onsager /app/stiglab "$@"
