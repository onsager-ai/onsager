#!/bin/sh
# Audit open PRs on onsager-ai/onsager for spec-link discipline.
# Read-only. For human / CI use; Claude should use mcp__github__* per-PR.
#
# For each open PR, reports:
#   OK       — body has Closes/Fixes/Resolves/Part-of/Refs #N
#   OK       — labelled `trivial`
#   MISSING  — neither (the bot will / has commented on this PR)
#
# Requires: GITHUB_TOKEN with `repo` scope, curl, jq.
# Usage:    GITHUB_TOKEN=... sh audit-open-prs.sh
# Exit:     0 if every open PR is OK, 1 if any is MISSING.
set -e

OWNER="onsager-ai"
REPO="onsager"
API="https://api.github.com"

if [ -z "$GITHUB_TOKEN" ]; then
    echo "ERROR: GITHUB_TOKEN not set" >&2
    exit 2
fi

for tool in curl jq; do
    if ! command -v "$tool" > /dev/null 2>&1; then
        echo "ERROR: $tool not installed" >&2
        exit 2
    fi
done

# Same regex as .github/workflows/pr-spec-sync.yml — keep in sync.
SPEC_LINK_RE='\b(close[sd]?|fix(e[sd])?|resolve[sd]?|part of|refs|related)[[:space:]]+#[0-9]+\b'

ok=0
missing=0
page=1

echo "=== Auditing open PRs on $OWNER/$REPO ==="
echo ""

while :; do
    response=$(curl -sS \
        -H "Authorization: Bearer $GITHUB_TOKEN" \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "$API/repos/$OWNER/$REPO/pulls?state=open&per_page=100&page=$page")

    count=$(echo "$response" | jq 'length')
    [ "$count" -eq 0 ] && break

    echo "$response" | jq -c '.[] | {number, title, body: (.body // ""), labels: [.labels[].name]}' \
        | while IFS= read -r pr; do
            num=$(echo "$pr" | jq -r '.number')
            title=$(echo "$pr" | jq -r '.title')
            body=$(echo "$pr" | jq -r '.body')
            trivial=$(echo "$pr" | jq -r '.labels | index("trivial") // empty')

            if echo "$body" | grep -Eqi "$SPEC_LINK_RE"; then
                printf "  OK       #%-5s  %s\n" "$num" "$title"
                echo "ok" >> /tmp/audit-prs.$$
            elif [ -n "$trivial" ]; then
                printf "  OK       #%-5s  %s  [trivial]\n" "$num" "$title"
                echo "ok" >> /tmp/audit-prs.$$
            else
                printf "  MISSING  #%-5s  %s\n" "$num" "$title"
                echo "missing" >> /tmp/audit-prs.$$
            fi
        done

    [ "$count" -lt 100 ] && break
    page=$((page + 1))
done

if [ -f /tmp/audit-prs.$$ ]; then
    ok=$(grep -c '^ok$' /tmp/audit-prs.$$ || true)
    missing=$(grep -c '^missing$' /tmp/audit-prs.$$ || true)
    rm -f /tmp/audit-prs.$$
fi

echo ""
echo "=== Results: $ok ok, $missing missing ==="
[ "$missing" -eq 0 ] || exit 1
