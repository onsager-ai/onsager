#!/usr/bin/env bash
# Blocks direct edits to skills synced from an upstream repo.
# To change a shared skill: edit it in its upstream repo, merge the PR,
# then re-run: npx skills add <upstream>

input=$(cat)
path=$(printf '%s' "$input" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('file_path', ''))
except Exception:
    print('')
" 2>/dev/null) || path=""

[[ -z "$path" ]] && exit 0

# Match absolute or relative paths containing .claude/skills/<skill>/
if [[ "$path" =~ (^|/)(\.claude/skills/([^/]+))/ ]]; then
  skill="${BASH_REMATCH[3]}"
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
  marker="${repo_root}/.claude/skills/${skill}/.upstream-source"
  if [[ -f "$marker" ]]; then
    upstream=$(cat "$marker")
    echo "BLOCKED: .claude/skills/${skill}/ is synced from ${upstream}." >&2
    echo "  Edit it there → merge → npx skills add ${upstream}" >&2
    exit 2
  fi
fi

exit 0
