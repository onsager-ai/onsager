#!/bin/sh
set -e

# If DATABASE_URL is a postgres:// URL, skip SQLite initialization
case "${DATABASE_URL:-}" in
    postgres://*|postgresql://*)
        echo "Using PostgreSQL: ${DATABASE_URL%%@*}@***"
        ;;
    *)
        # SQLite: ensure data directory and database exist
        # Strip sqlite:// prefix to get the file path, then strip query/fragment
        DB_URL="${DATABASE_URL:-sqlite:///data/synodic.db}"
        DB_PATH="${DB_URL#sqlite://}"
        DB_PATH_CLEAN="${DB_PATH%%\?*}"
        DB_PATH_CLEAN="${DB_PATH_CLEAN%%#*}"
        DB_DIR="$(dirname "$DB_PATH_CLEAN")"
        mkdir -p "$DB_DIR"
        if [ ! -f "$DB_PATH_CLEAN" ]; then
            echo "Initializing SQLite database at $DB_PATH_CLEAN"
            sqlite3 "$DB_PATH_CLEAN" "SELECT 1;" > /dev/null
        fi
        export DATABASE_URL="sqlite://$DB_PATH"
        ;;
esac

# Default to serve mode when no arguments provided
if [ $# -eq 0 ]; then
    exec synodic serve
else
    exec synodic "$@"
fi
