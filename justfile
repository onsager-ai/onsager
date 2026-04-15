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

# ── Dev ──────────────────────────────────────────────────────────────
dev-dashboard:
    pnpm --filter dashboard dev

dev-stiglab:
    cargo run -p stiglab -- serve

dev-synodic:
    cargo run -p synodic -- serve

# ── DB ───────────────────────────────────────────────────────────────
db-migrate:
    psql "$DATABASE_URL" -f crates/onsager-spine/migrations/001_initial.sql
    psql "$DATABASE_URL" -f crates/onsager-spine/migrations/002_artifacts.sql

# ── Install from source ──────────────────────────────────────────────
install:
    cargo install --path crates/onsager
    cargo install --path crates/stiglab
    cargo install --path crates/synodic
