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

# Drop from root to unprivileged user.
# Claude Code CLI refuses --permission-mode bypassPermissions under root.
exec gosu onsager /app/stiglab "$@"
