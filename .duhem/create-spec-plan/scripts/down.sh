#!/bin/sh
# Tear the Onsager dev stack back down after the create-spec-plan
# verification finishes. Best-effort: failures here are recorded as
# evidence but never alter the run verdict (mechanism from issue #50).
#
# Runs after the last criterion regardless of verdict, unless the
# operator passes `--keep-env` on `duhem run` to leave the SUT up for
# triage.

set -u

PID_FILE="${TMPDIR:-/tmp}/onsager-dev.pid"
if [ -f "$PID_FILE" ]; then
  PID=$(cat "$PID_FILE")
  if kill -0 "$PID" 2>/dev/null; then
    echo "down.sh: stopping onsager dev stack (pid $PID)"
    # `just dev` installs an EXIT trap that kills its child services;
    # signal the process group too so the cargo/pnpm children don't
    # outlive the recipe shell.
    kill -- "-$PID" 2>/dev/null || kill "$PID" 2>/dev/null || true
    sleep 2
    if kill -0 "$PID" 2>/dev/null; then
      kill -9 -- "-$PID" 2>/dev/null || kill -9 "$PID" 2>/dev/null || true
    fi
  fi
  rm -f "$PID_FILE"
fi

# Leave Postgres (docker compose db) running — it's cheap to reuse
# across runs and tearing it down on every verdict would slow the
# inner loop. The fixture plans accumulate under unique per-run ids,
# so they don't collide; clear them with a fresh DB
# (`just db-reset` on the Onsager side) when they pile up.

rm -f "${TMPDIR:-/tmp}/onsager-dev.cookies" 2>/dev/null || true
exit 0
