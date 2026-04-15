#!/usr/bin/env bash
# L2 Interception hook for Claude Code PreToolUse events.
#
# Reads tool call JSON from stdin, evaluates against Synodic's intercept
# rules, and returns the appropriate exit code + output for Claude Code.
#
# Exit 0 = allow, Exit 2 = block (with reason on stderr).
#
# Override flow (interactive only):
#   Block fires → user prompted → override with reason → feedback recorded → allow

set -euo pipefail

# Fail-open if jq is not available
if ! command -v jq &>/dev/null; then
  cat >/dev/null  # drain stdin
  exit 0
fi

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
SYNODIC_BIN="${SYNODIC_BIN:-${PROJECT_DIR}/rust/target/release/synodic}"

# Fall back to debug build if release doesn't exist
if [[ ! -x "$SYNODIC_BIN" ]]; then
  SYNODIC_BIN="${PROJECT_DIR}/rust/target/debug/synodic"
fi

# Fall back to PATH
if [[ ! -x "$SYNODIC_BIN" ]]; then
  SYNODIC_BIN="$(command -v synodic 2>/dev/null || true)"
fi

# If no binary, allow (don't block the agent on missing build)
if [[ -z "$SYNODIC_BIN" ]] || [[ ! -x "$SYNODIC_BIN" ]]; then
  exit 0
fi

# Read hook input from stdin
INPUT="$(cat)"

# Extract tool_name and tool_input from the hook's JSON payload (fail-open on parse error)
TOOL_NAME="$(echo "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)" || true
TOOL_INPUT="$(echo "$INPUT" | jq -c '.tool_input // {}' 2>/dev/null)" || TOOL_INPUT='{}'

# If we couldn't parse the input, allow
if [[ -z "$TOOL_NAME" ]]; then
  exit 0
fi

# Call synodic intercept
RESULT="$("$SYNODIC_BIN" intercept --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null)" || {
  # If the command fails, allow (fail-open)
  exit 0
}

DECISION="$(echo "$RESULT" | jq -r '.decision // "allow"' 2>/dev/null)" || true

if [[ "$DECISION" == "block" ]]; then
  REASON="$(echo "$RESULT" | jq -r '.reason // "Blocked by Synodic governance rule"' 2>/dev/null)" || REASON="Blocked by Synodic governance rule"
  RULE="$(echo "$RESULT" | jq -r '.rule // "unknown"' 2>/dev/null)" || RULE="unknown"

  # Interactive override (only when TTY is available)
  if [ -t 2 ]; then
    echo "" >&2
    echo "  Blocked by rule '$RULE': $REASON" >&2
    echo "" >&2

    read -p "  Override? (y/N): " -n 1 -r OVERRIDE </dev/tty 2>/dev/null || OVERRIDE="n"
    echo "" >&2

    if [[ "$OVERRIDE" =~ ^[Yy]$ ]]; then
      read -p "  Reason (optional): " OVERRIDE_REASON </dev/tty 2>/dev/null || OVERRIDE_REASON=""
      echo "" >&2

      # Record override feedback (fail-open — don't block on DB errors)
      "$SYNODIC_BIN" feedback --rule "$RULE" --signal override \
        --tool "$TOOL_NAME" --input "$TOOL_INPUT" \
        ${OVERRIDE_REASON:+--reason "$OVERRIDE_REASON"} 2>/dev/null || true

      echo "  Override recorded. Proceeding." >&2
      exit 0
    else
      # Record confirmed block
      "$SYNODIC_BIN" feedback --rule "$RULE" --signal confirmed \
        --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null || true

      echo "  Action blocked." >&2
      exit 2
    fi
  else
    # Non-interactive — always block, record confirmed
    "$SYNODIC_BIN" feedback --rule "$RULE" --signal confirmed \
      --tool "$TOOL_NAME" --input "$TOOL_INPUT" 2>/dev/null || true

    echo "Synodic L2 interception [$RULE]: $REASON" >&2
    exit 2
  fi
fi

exit 0
