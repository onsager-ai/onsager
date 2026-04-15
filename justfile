# Onsager monorepo task runner.
# Rust + TS workspaces coexist; this file is just a command registry.

default:
    @just --list

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

lint-rust:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

lint-ui:
    pnpm --filter dashboard lint

# ── Dev (full stack) ─────────────────────────────────────────────────

# Start the full dev stack: Postgres + migrations + all services
dev: dev-infra
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill $(jobs -p) 2>/dev/null' EXIT

    echo "==> Starting stiglab on :3000..."
    ONSAGER_DATABASE_URL="postgres://onsager:onsager@localhost:5432/onsager" \
        cargo run -p stiglab -- server &

    echo "==> Starting synodic on :3001..."
    PORT=3001 cargo run -p synodic -- serve &

    echo "==> Starting dashboard on :5173..."
    pnpm --filter dashboard dev &

    echo ""
    echo "=== Onsager dev stack running ==="
    echo "  Dashboard:  http://localhost:5173"
    echo "  Stiglab:    http://localhost:3000"
    echo "  Synodic:    http://localhost:3001"
    echo "  Postgres:   postgres://onsager:onsager@localhost:5432/onsager"
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
    docker compose up migrate --exit-code-from migrate
    echo "==> Infrastructure ready."

# Stop infrastructure
dev-down:
    docker compose down

# ── Dev (individual services) ────────────────────────────────────────
dev-dashboard:
    pnpm --filter dashboard dev

dev-forge:
    cargo run -p forge -- serve --database-url "${DATABASE_URL}"

dev-ising:
    cargo run -p ising -- serve --database-url "${DATABASE_URL}"

dev-stiglab:
    cargo run -p stiglab -- server

dev-synodic port="3001":
    PORT={{port}} cargo run -p synodic -- serve

# ── DB ───────────────────────────────────────────────────────────────
db-migrate:
    psql "$DATABASE_URL" -f crates/onsager-spine/migrations/001_initial.sql
    psql "$DATABASE_URL" -f crates/onsager-spine/migrations/002_artifacts.sql

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

# ── Install from source ──────────────────────────────────────────────
install:
    cargo install --path crates/onsager
    cargo install --path crates/forge
    cargo install --path crates/ising
    cargo install --path crates/stiglab
    cargo install --path crates/synodic
