#!/usr/bin/env bash
# preview-url.sh — Resolve the public URL for a Railway PR preview environment.
#
# Usage:
#   PR_NUMBER=42 RAILWAY_TOKEN=... RAILWAY_PROJECT_ID=... \
#     scripts/preview-url.sh [service_name]
#
# Prints the preview URL (e.g. https://onsager-pr-42.up.railway.app) on stdout,
# or exits non-zero with a diagnostic on stderr if the environment is not yet
# deployed. Service name defaults to `onsager`.
#
# Contract:
#   - Exits 0 and prints URL on success.
#   - Exits 2 if the preview environment exists but has no active deployment
#     yet (caller should poll).
#   - Exits 1 on hard failure (missing token, project not found, etc.).
#
# Designed to be polled from CI — cheap, idempotent, no side effects.

set -euo pipefail

SERVICE="${1:-${RAILWAY_SERVICE_NAME:-onsager}}"
: "${PR_NUMBER:?PR_NUMBER is required}"
: "${RAILWAY_TOKEN:?RAILWAY_TOKEN is required}"
: "${RAILWAY_PROJECT_ID:?RAILWAY_PROJECT_ID is required}"

ENV_NAME="pr-${PR_NUMBER}"
API="https://backboard.railway.com/graphql/v2"

gql() {
    local query="$1"
    # Fail fast on stalled/flaky networks so the CI poller can retry on its
    # own cadence instead of hanging until the workflow timeout.
    curl -sS -X POST "$API" \
        --connect-timeout 10 \
        --max-time 30 \
        --retry 2 \
        --retry-delay 1 \
        --retry-all-errors \
        -H "Authorization: Bearer $RAILWAY_TOKEN" \
        -H "Content-Type: application/json" \
        -d "$(printf '{"query":%s}' "$(printf '%s' "$query" | jq -Rs .)")"
}

query=$(cat <<GQL
{
  environments(projectId: "$RAILWAY_PROJECT_ID") {
    edges {
      node {
        id
        name
        deployments(first: 1) {
          edges {
            node {
              status
              staticUrl
              url
            }
          }
        }
        serviceInstances {
          edges { node { serviceName domains { serviceDomains { domain } } } }
        }
      }
    }
  }
}
GQL
)

response=$(gql "$query")

env_node=$(echo "$response" | jq --arg name "$ENV_NAME" '
  .data.environments.edges[] | select(.node.name == $name) | .node
')

if [ -z "$env_node" ] || [ "$env_node" = "null" ]; then
    echo "preview environment '$ENV_NAME' not found in project $RAILWAY_PROJECT_ID" >&2
    exit 2
fi

status=$(echo "$env_node" | jq -r '.deployments.edges[0].node.status // "NONE"')
if [ "$status" != "SUCCESS" ]; then
    echo "preview environment '$ENV_NAME' deployment status: $status (not ready)" >&2
    exit 2
fi

domain=$(echo "$env_node" | jq -r --arg svc "$SERVICE" '
  .serviceInstances.edges[]
  | select(.node.serviceName == $svc)
  | .node.domains.serviceDomains[0].domain // empty
' | head -n1)

if [ -z "$domain" ]; then
    echo "preview environment '$ENV_NAME' has no domain for service '$SERVICE'" >&2
    exit 2
fi

echo "https://$domain"
