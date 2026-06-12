# Onsager v2 — one process, one db, one dashboard (ADR 0029).

# Load .env so host-run recipes pick up ONSAGER_CREDENTIAL_KEY etc.
set dotenv-load := true

default:
    @just --list

# ── Setup ─────────────────────────────────────────────────────────────

# One-time repo setup: git hooks. Respects a global core.hooksPath if
# the user has one; otherwise points the repo at .githooks.
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "$(git config --global core.hooksPath || true)" ]; then
        git config core.hooksPath .githooks
        echo "hooks: repo-local .githooks"
    else
        echo "hooks: global hooksPath respected ($(git config --global core.hooksPath))"
    fi
    pnpm install --frozen-lockfile

# ── Dev ───────────────────────────────────────────────────────────────

# Start Postgres and wait for health.
dev-infra:
    #!/usr/bin/env bash
    set -euo pipefail
    docker compose up -d db --wait
    echo "db: $(docker compose port db 5432)"

# Full dev loop: db + the onsager process + Vite. Registers with the
# machine Traefik (worktree devproxy) when ~/dev-proxy/dynamic exists,
# so the stack is reachable at http://$DEV_HOST:8000.
dev: dev-infra
    #!/usr/bin/env bash
    set -euo pipefail

    DB_PORT=$(docker compose port db 5432 | head -1 | awk -F: '{print $NF}')
    export DATABASE_URL="postgres://onsager:onsager@localhost:${DB_PORT}/onsager"

    pick_port() {
        local p=$1
        while (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; do
            p=$(( (RANDOM % 20000) + 20000 ))
        done
        echo "$p"
    }
    ONSAGER_PORT=$(pick_port 3002)
    DASHBOARD_PORT=$(pick_port 5173)
    export ONSAGER_BIND="127.0.0.1:${ONSAGER_PORT}"
    export PORTAL_PORT="$ONSAGER_PORT"  # vite.config.ts proxy target

    # devproxy self-registration (#579): Host($DEV_HOST) → host-run Vite.
    DEVPROXY_DYNAMIC_DIR="${DEVPROXY_DYNAMIC_DIR:-$HOME/dev-proxy/dynamic}"
    route_file=""
    if [ -n "${DEV_HOST:-}" ] && [ -d "$DEVPROXY_DYNAMIC_DIR" ]; then
        route_file="$DEVPROXY_DYNAMIC_DIR/${COMPOSE_PROJECT_NAME:-onsager}.yml"
        cat > "$route_file" <<EOF
    http:
      routers:
        ${COMPOSE_PROJECT_NAME:-onsager}:
          rule: Host(\`${DEV_HOST}\`)
          service: ${COMPOSE_PROJECT_NAME:-onsager}
      services:
        ${COMPOSE_PROJECT_NAME:-onsager}:
          loadBalancer:
            servers:
              - url: http://host.docker.internal:${DASHBOARD_PORT}
    EOF
        export VITE_HMR_CLIENT_PORT=8000
        echo "devproxy: http://${DEV_HOST}:8000"
        trap 'rm -f "$route_file"' EXIT
    fi

    echo "==> onsager on :${ONSAGER_PORT}, dashboard on :${DASHBOARD_PORT}"
    cargo run -p onsager &
    pnpm --filter dashboard dev -- --port "$DASHBOARD_PORT" --strictPort &
    wait

# Stop Postgres.
dev-down:
    docker compose down

# ── Build / test / lint ───────────────────────────────────────────────

build:
    cargo build --workspace
    pnpm --filter dashboard build

test:
    cargo test --workspace
    pnpm --filter dashboard test

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm --filter dashboard lint
