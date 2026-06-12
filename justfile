# Onsager monorepo task runner.
# Rust + TS workspaces coexist; this file is just a command registry.

# Load .env into every recipe's environment. Lets `just dev` (which runs
# cargo directly, not via docker-compose) pick up credential keys and other
# config from .env — e.g. ONSAGER_CREDENTIAL_KEY.
set dotenv-load := true

default:
    @just --list

# ── Setup ────────────────────────────────────────────────────────────

# One-time dev setup: point git at the committed hooks directory.
# Skipped when a machine-global core.hooksPath is set — the global hook
# (worktree-discipline) delegates to .githooks/pre-commit, and a repo-local
# hooksPath would override the global one and silently drop the guard.
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    if git config --global core.hooksPath >/dev/null 2>&1; then
        git config --unset core.hooksPath 2>/dev/null || true
        echo "Global core.hooksPath active — it delegates to .githooks/; repo-local hooksPath left unset."
    else
        git config core.hooksPath .githooks
        echo "Git hooks installed from .githooks/"
    fi

# ── Build ────────────────────────────────────────────────────────────
build: build-rust build-ui

build-rust:
    cargo build --workspace

build-ui:
    pnpm install
    pnpm --filter dashboard build

# ── Test ─────────────────────────────────────────────────────────────
test: test-rust test-ui

test-rust:
    cargo test --workspace

test-ui:
    pnpm --filter dashboard test

# ── Lint ─────────────────────────────────────────────────────────────
lint: lint-rust lint-ui

lint-rust: _check-tools-and-skills
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo run -p xtask --quiet -- gen-event-docs --check
    cargo run -p xtask --quiet -- lint-seams
    cargo run -p xtask --quiet -- check-api-contract
    cargo run -p xtask --quiet -- check-events
    cargo run -p xtask --quiet -- check-generated-types
    cargo run -p xtask --quiet -- check-file-budget --mode=fail
    cargo run -p xtask --quiet -- check-deferred-todos --mode=warn
    cargo run -p xtask --quiet -- check-adr-adoption --mode=warn

# Resolve the skills bundle then run check-tools-and-skills.
# Override: ONSAGER_SKILLS_DIR=../onsager-skills just lint  (two-checkout local dev)
# Default: clones/updates onsager-ai/onsager-skills into target/onsager-skills/
_check-tools-and-skills:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${ONSAGER_SKILLS_DIR:-}" ]; then
        skills_dir="$ONSAGER_SKILLS_DIR"
    else
        skills_dir="target/onsager-skills"
        mkdir -p "$(dirname "$skills_dir")"
        if [ -d "$skills_dir/.git" ]; then
            git -C "$skills_dir" fetch --quiet && git -C "$skills_dir" reset --quiet --hard origin/main
        else
            git clone --quiet https://github.com/onsager-ai/onsager-skills "$skills_dir"
        fi
    fi
    ONSAGER_SKILLS_DIR="$skills_dir" cargo run -p xtask --quiet -- check-tools-and-skills

lint-ui:
    pnpm --filter dashboard lint

# ── CI parity (spec #513) ────────────────────────────────────────────
#
# `just ci-local` runs the exact gate set CI runs — the union of
# rust.yml's `check` job and frontend.yml's `build` job — fail-fast,
# against the committed tree. It exists because "I verified locally"
# kept meaning narrower per-crate commands (`cargo test -p X --lib`,
# `tsc --noEmit`) that passed while the real CI jobs (workspace clippy
# /test, the frontend lint+BUILD+test triad) failed (PR #510, ~12 red
# rounds). Run it ONCE before pushing — not on every edit (it's slow:
# workspace clippy + workspace test + dashboard build).
#
# check-ci-parity: the step set below must stay a superset of every
# `run:` step in .github/workflows/rust.yml (`check` job) and
# frontend.yml (`build` job). When a CI step is added in either file,
# mirror it here (or in the recipe it composes — `lint-rust`). The
# Postgres-gated spine integration tests are left to CI /
# `just test-spine`; `ci-local-full` adds them when Postgres is up.
ci-local: ci-local-rust ci-local-ui

# Rust half — mirrors rust.yml's `check` job. `lint-rust` already runs
# fmt + workspace clippy + every xtask lint CI runs (incl. check-events);
# this adds the explicit workspace build + test. The DB-gated spine
# tests skip without DATABASE_URL, exactly as in the "Test remaining
# crates" CI step — see `ci-local-full` to include them.
ci-local-rust: lint-rust
    cargo build --workspace
    cargo test --workspace

# Frontend half — mirrors frontend.yml's `build` job exactly. The
# `build` step (`pnpm --filter dashboard build`, the `tsc -b` + vite
# step) is the one that was invisible to a narrower `tsc --noEmit` and
# only runs in CI on PRs touching apps/dashboard/** — so a backend-only
# base PR's green CI says nothing about a dashboard-touching child.
ci-local-ui:
    pnpm install
    pnpm --filter dashboard lint
    pnpm --filter dashboard build
    pnpm --filter dashboard test

# Full parity including the Postgres-gated spine integration tests.
# Requires a running Postgres (`just dev-infra`) with DATABASE_URL
# set; otherwise use plain `ci-local` (DB-free). Mirrors CI's test
# split exactly — spine runs serial (`--test-threads=1`) because its
# integration tests share one DB schema — rather than `cargo test
# --workspace`, which would race them. `lint-rust` (fmt + clippy +
# xtask lints) runs first as a dependency.
ci-local-full: lint-rust
    cargo build --workspace
    cargo test -p onsager-spine -- --test-threads=1
    cargo test --workspace --exclude onsager-spine
    just ci-local-ui

# ── Docs ─────────────────────────────────────────────────────────────

# Regenerate docs/events.md from FactoryEventKind. Run after editing
# crates/onsager-spine/src/factory_event.rs. CI runs `--check`.
gen-event-docs:
    cargo run -p xtask --quiet -- gen-event-docs

# Source-of-truth token count for one file via Anthropic's count_tokens
# API, printed alongside the offline tiktoken `o200k_base` count that
# `xtask check-file-budget` uses in CI. Used to validate / recalibrate
# the budget. Spec #261 Move 4c.
#
# Requires ANTHROPIC_API_KEY in the environment. Override the model
# with ANTHROPIC_TOKEN_MODEL (defaults to claude-sonnet-4-5).
measure-tokens FILE:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
        echo "error: ANTHROPIC_API_KEY is not set" >&2
        exit 2
    fi
    if [ ! -f "{{FILE}}" ]; then
        echo "error: {{FILE}} is not a regular file" >&2
        exit 2
    fi
    model="${ANTHROPIC_TOKEN_MODEL:-claude-sonnet-4-5}"
    payload=$(jq -Rs --arg model "$model" '{
        model: $model,
        messages: [{role: "user", content: .}]
    }' < "{{FILE}}")
    response=$(curl -fsS https://api.anthropic.com/v1/messages/count_tokens \
        -H "x-api-key: $ANTHROPIC_API_KEY" \
        -H "anthropic-version: 2023-06-01" \
        -H "content-type: application/json" \
        --data "$payload")
    api_tokens=$(echo "$response" | jq -r '.input_tokens // empty')
    if [ -z "$api_tokens" ]; then
        echo "error: count_tokens returned no input_tokens" >&2
        echo "$response" >&2
        exit 1
    fi
    tt_tokens=$(cargo run -p xtask --quiet -- count-tokens "{{FILE}}")
    drift=$(awk -v a="$api_tokens" -v t="$tt_tokens" \
        'BEGIN { if (a == 0) print "n/a"; else printf "%+.1f%%", (t - a) / a * 100 }')
    printf 'file:        %s\n' "{{FILE}}"
    printf 'api tokens:  %s  (model: %s)\n' "$api_tokens" "$model"
    printf 'tiktoken:    %s  (o200k_base, prod-content only)\n' "$tt_tokens"
    printf 'drift:       %s  (spec #261 expects within ~15%%)\n' "$drift"

# ── Dev (full stack) ─────────────────────────────────────────────────

# Start the full dev stack: Postgres + migrations + all services
dev: dev-infra
    #!/usr/bin/env bash
    set -euo pipefail

    # The committed compose publishes db on an OS-assigned loopback port
    # (a local override may add a fixed one) — read back what we got.
    DB_PORT=$(docker compose port db 5432 | head -1 | awk -F: '{print $NF}')
    DATABASE_URL="postgres://onsager:onsager@localhost:${DB_PORT}/onsager"

    # Canonical-first port picker (#579): keep muscle-memory ports when
    # free; fall back to a random free port so parallel worktree stacks
    # never collide.
    pick_port() {
        local p=$1
        while (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; do
            p=$(( (RANDOM % 20000) + 20000 ))
        done
        echo "$p"
    }
    PORTAL_PORT=$(pick_port 3002)
    DASHBOARD_PORT=$(pick_port 5173)
    export PORTAL_PORT  # vite.config.ts proxy target

    # Self-register with the machine Traefik (worktree devproxy) when its
    # file-provider dir exists: Host($DEV_HOST) → host-run Vite. Vite
    # fronts /api, /mcp and /agent same-origin, so one route suffices;
    # HMR must aim at the proxy port when reached through it. (#579)
    DEVPROXY_DYNAMIC_DIR="${DEVPROXY_DYNAMIC_DIR:-$HOME/dev-proxy/dynamic}"
    route_file=""
    if [ -n "${DEV_HOST:-}" ] && [ -d "$DEVPROXY_DYNAMIC_DIR" ]; then
        route_file="$DEVPROXY_DYNAMIC_DIR/${COMPOSE_PROJECT_NAME:-onsager}.yml"
        cat > "$route_file" <<ROUTE
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
    ROUTE
        export VITE_HMR_CLIENT_PORT="${VITE_HMR_CLIENT_PORT:-8000}"
    else
        echo "==> devproxy registration skipped (need DEV_HOST + $DEVPROXY_DYNAMIC_DIR)"
    fi

    trap 'pids=$(jobs -p); [ -n "$pids" ] && kill $pids 2>/dev/null || true; [ -n "$route_file" ] && rm -f "$route_file"' EXIT

    echo "==> Starting portal on :${PORTAL_PORT}..."
    DATABASE_URL="$DATABASE_URL" \
    PORTAL_BIND="0.0.0.0:${PORTAL_PORT}" \
        cargo run -p onsager-portal -- serve &

    echo "==> Starting onsager-scheduler (substrate scheduler)..."
    # Agent sessions need the portal MCP endpoint to get tools
    # (emit_artifact etc.) — without it every run parks at the
    # liveness gate. Default to this stack's portal; allow override.
    DATABASE_URL="$DATABASE_URL" \
    ONSAGER_MCP_URL="${ONSAGER_MCP_URL:-http://localhost:${PORTAL_PORT}/mcp/messages}" \
        cargo run -p onsager-engine --bin onsager-scheduler -- serve &

    echo "==> Starting dashboard on :${DASHBOARD_PORT}..."
    pnpm --filter dashboard dev -- --port "$DASHBOARD_PORT" --strictPort &

    echo ""
    echo "=== Onsager dev stack running ==="
    [ -n "$route_file" ] && echo "  Devproxy:   http://${DEV_HOST}:8000"
    echo "  Dashboard:  http://localhost:${DASHBOARD_PORT}"
    echo "  Portal:     http://localhost:${PORTAL_PORT}"
    echo "  Scheduler:  spine listener (no HTTP port)"
    echo "  Postgres:   postgres://onsager:onsager@localhost:${DB_PORT}/onsager"
    echo ""
    echo "Press Ctrl+C to stop all services."
    wait

# Start infrastructure only (Postgres + migrations)
dev-infra:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Starting Postgres..."
    docker compose up db -d --wait
    echo "==> Running spine migrations..."
    docker compose run --rm migrate
    echo "==> Infrastructure ready."

# Stop infrastructure
dev-down:
    docker compose down

# ── Dev (individual services) ────────────────────────────────────────
dev-dashboard:
    pnpm --filter dashboard dev

dev-portal port="3002":
    DATABASE_URL="${DATABASE_URL:-postgres://onsager:onsager@localhost:5432/onsager}" \
    PORTAL_BIND="0.0.0.0:{{port}}" \
        cargo run -p onsager-portal -- serve

dev-scheduler:
    DATABASE_URL="${DATABASE_URL:-postgres://onsager:onsager@localhost:5432/onsager}" \
    ONSAGER_MCP_URL="${ONSAGER_MCP_URL:-http://localhost:3002/mcp/messages}" \
        cargo run -p onsager-engine --bin onsager-scheduler -- serve

# ── DB ───────────────────────────────────────────────────────────────
db-migrate:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in crates/onsager-spine/migrations/*.sql; do
        echo "==> applying $f"
        psql -v ON_ERROR_STOP=1 "$DATABASE_URL" -f "$f"
    done

# ── Test (with spine integration) ────────────────────────────────────

# Run onsager-spine integration tests (requires running Postgres via dev-infra)
test-spine:
    DATABASE_URL="postgres://onsager:onsager@localhost:5432/onsager" \
        cargo test -p onsager-spine -- --test-threads=1

# Run all tests including spine integration tests
test-all: test-spine test-rust test-ui

# Smoke test the running dev stack
smoke-test:
    bash scripts/smoke-test.sh

# ── E2E (product tests — real agent sessions) ───────────────────────

# Run live E2E tests against a running Onsager stack (requires just dev + credentials)
test-e2e:
    pnpm --filter onsager-e2e test

# Run a single E2E test file (e.g. just test-e2e-file session-lifecycle)
test-e2e-file name:
    pnpm --filter onsager-e2e exec vitest run "product/{{name}}.test.ts"

# Run E2E tests against a remote Onsager instance
test-e2e-remote url:
    ONSAGER_URL="{{url}}" pnpm --filter onsager-e2e test

# ── Deploy (production) ──────────────────────────────────────────────

# Build production Docker images
deploy-build:
    docker compose -f deploy/docker-compose.yml build

# Start the production stack (Postgres + migrations + services)
deploy-up:
    docker compose -f deploy/docker-compose.yml up -d

# Stop the production stack
deploy-down:
    docker compose -f deploy/docker-compose.yml down

# Tail production logs
deploy-logs:
    docker compose -f deploy/docker-compose.yml logs -f

# Full deploy: build images then start everything
deploy: deploy-build deploy-up
    #!/usr/bin/env bash
    echo ""
    echo "=== Onsager production stack running ==="
    echo "  Dashboard:  http://localhost:${EDGE_PORT:-8080}"
    echo "  API:        http://localhost:${EDGE_PORT:-8080}/api/health"
    echo ""
    echo "Logs:  just deploy-logs"
    echo "Stop:  just deploy-down"

# ── Install from source ──────────────────────────────────────────────
install:
    cargo install --path crates/onsager-engine
    cargo install --path crates/onsager-portal

