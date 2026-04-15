#!/usr/bin/env bash
# Smoke test for the Onsager dev stack.
# Verifies that all services are running and responding.
#
# Usage:
#   just smoke-test                         # defaults
#   STIGLAB_URL=http://host:3000 bash scripts/smoke-test.sh

set -euo pipefail

STIGLAB_URL="${STIGLAB_URL:-http://localhost:3000}"
SYNODIC_URL="${SYNODIC_URL:-http://localhost:3001}"
DASHBOARD_URL="${DASHBOARD_URL:-http://localhost:5173}"
SPINE_URL="${SPINE_URL:-postgres://onsager:onsager@localhost:5432/onsager}"

PASS=0
FAIL=0

check() {
    local name="$1" url="$2" expect="$3"
    if curl -sf --max-time 5 "$url" | grep -q "$expect"; then
        echo "  PASS  $name"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $name  ($url)"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== Onsager Smoke Test ==="
echo ""

echo "-- Stiglab --"
check "health"   "$STIGLAB_URL/api/health"   '"status"'
check "nodes"    "$STIGLAB_URL/api/nodes"     'nodes'
check "sessions" "$STIGLAB_URL/api/sessions"  'sessions'

echo ""
echo "-- Synodic --"
check "health"   "$SYNODIC_URL/api/health"    '"status"'

echo ""
echo "-- Dashboard --"
check "html"     "$DASHBOARD_URL"             '<'

echo ""
echo "-- Spine (Postgres) --"
if command -v psql > /dev/null 2>&1; then
    if psql "$SPINE_URL" -c "SELECT 1 FROM events LIMIT 0;" > /dev/null 2>&1; then
        echo "  PASS  events table accessible"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  events table  (db unreachable via local psql)"
        FAIL=$((FAIL + 1))
    fi
elif command -v docker > /dev/null 2>&1 && docker compose version > /dev/null 2>&1; then
    if docker compose exec -T db psql -U onsager -d onsager -c "SELECT 1 FROM events LIMIT 0;" > /dev/null 2>&1; then
        echo "  PASS  events table accessible"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  events table  (db unreachable via docker compose)"
        FAIL=$((FAIL + 1))
    fi
else
    echo "  SKIP  events table  (no psql or docker compose available)"
fi

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] || exit 1
