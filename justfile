# Onsager monorepo task runner.
# Rust + TS workspaces coexist; this file is just a command registry.

default:
    @just --list

# ── Setup ────────────────────────────────────────────────────────────

# One-time dev setup: point git at the committed hooks directory
setup:
    git config core.hooksPath .githooks
    @echo "Git hooks installed from .githooks/"

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
    cargo run -p xtask --quiet -- gen-event-docs --check
    cargo run -p xtask --quiet -- lint-seams
    cargo run -p xtask --quiet -- check-api-contract

lint-ui:
    pnpm --filter dashboard lint

# ── Docs ─────────────────────────────────────────────────────────────

# Regenerate docs/events.md from FactoryEventKind. Run after editing
# crates/onsager-spine/src/factory_event.rs. CI runs `--check`.
gen-event-docs:
    cargo run -p xtask --quiet -- gen-event-docs

# ── Dev (full stack) ─────────────────────────────────────────────────

# Start the full dev stack: Postgres + migrations + all services
dev: dev-infra
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'pids=$(jobs -p); [ -n "$pids" ] && kill $pids 2>/dev/null || true' EXIT

    echo "==> Starting stiglab on :3000..."
    ONSAGER_DATABASE_URL="postgres://onsager:onsager@localhost:5432/onsager" \
        cargo run -p stiglab -- server &

    echo "==> Starting synodic on :3001..."
    PORT=3001 cargo run -p synodic -- serve &

    echo "==> Starting forge on :3003..."
    DATABASE_URL="postgres://onsager:onsager@localhost:5432/onsager" \
    FORGE_PORT=3003 \
        cargo run -p forge -- serve &

    echo "==> Starting dashboard on :5173..."
    pnpm --filter dashboard dev &

    echo ""
    echo "=== Onsager dev stack running ==="
    echo "  Dashboard:  http://localhost:5173"
    echo "  Stiglab:    http://localhost:3000"
    echo "  Synodic:    http://localhost:3001"
    echo "  Forge:      http://localhost:3003"
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
    docker compose run --rm migrate
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

# Start the production stack (Postgres + migrations + stiglab + synodic)
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
    echo "  Dashboard:  http://localhost:${STIGLAB_PORT:-3000}"
    echo "  Stiglab:    http://localhost:${STIGLAB_PORT:-3000}/api/health"
    echo "  Synodic:    http://localhost:${SYNODIC_PORT:-3001}/api/health"
    echo ""
    echo "Logs:  just deploy-logs"
    echo "Stop:  just deploy-down"

# ── Install from source ──────────────────────────────────────────────
install:
    cargo install --path crates/onsager
    cargo install --path crates/forge
    cargo install --path crates/ising
    cargo install --path crates/stiglab
    cargo install --path crates/synodic

# ── Per-worktree dev slots (spec #194) ───────────────────────────────
#
# Each worktree maps to a numbered slot. A slot owns a docker-compose
# project (`onsager-slot{N}`), a private postgres + spine, and a
# 10-port block on the VM (edge `9000+10*N`, postgres `9000+10*N+1`).
# Slot 0 is the main checkout and uses the legacy port layout via
# `just dev` — these recipes only touch slots 1..=99.
#
# Typical flow on the VM:
#   just worktree-new feat-a            # branch from main, allocate slot, bring up
#   just worktree-list                  # see slots, ports, project names
#   just slot-exec feat-a cargo test -p stiglab
#   just worktree-tunnel feat-a         # print SSH `-L` flags for the laptop
#   just worktree-rm feat-a             # tear down slot + volumes + worktree dir
#   just worktree-rm feat-a --with-branch  # also delete the branch
#
# All of these resolve the slot from `.dev-slots.json` via xtask, so
# the human never types port numbers.

# Allocate a fresh slot, create a worktree off main, bring the slot's
# compose stack up. The `<base>` arg defaults to `main`; pass another
# base branch to fork from a different point.
#
# All compose invocations run from inside the worktree dir so the
# `../..:/work` bind mount in docker-compose.slot.yml resolves to the
# worktree's source tree, not the main checkout's.
worktree-new name base="main":
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"; base="{{base}}"
    if [ -e "worktrees/$name" ]; then
      echo "worktrees/$name already exists" >&2; exit 1
    fi
    slot=$(cargo run -p xtask --quiet -- slot alloc "$name" --branch "$name")
    # If anything below this fails, hand the slot back so the manifest
    # doesn't accumulate orphan entries.
    trap 'rc=$?; if [ "$rc" != 0 ]; then cargo run -p xtask --quiet -- slot free "'"$name"'" >/dev/null 2>&1 || true; rm -rf "worktrees/'"$name"'" 2>/dev/null || true; fi' EXIT
    mkdir -p worktrees
    git worktree add -b "$name" "worktrees/$name" "$base"
    cargo run -p xtask --quiet -- slot env "$name" > "worktrees/$name/.env.slot"
    mkdir -p "worktrees/$name/.devcontainer"
    cp deploy/dev/devcontainer.json "worktrees/$name/.devcontainer/devcontainer.json"
    project="onsager-slot${slot}"
    (cd "worktrees/$name" && \
     docker compose --env-file .env.slot \
         -f deploy/dev/docker-compose.slot.yml \
         -p "$project" up -d --wait)
    trap - EXIT
    echo ""
    echo "=== slot $slot ($name) up ==="
    cargo run -p xtask --quiet -- slot tunnel "$name"

# Bring an existing slot's compose project back up after a VM reboot
# or manual stop. Does NOT re-allocate a slot; the worktree dir and
# branch must already exist.
worktree-up name:
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"
    project=$(cargo run -p xtask --quiet -- slot project "$name")
    (cd "worktrees/$name" && \
     docker compose --env-file .env.slot \
         -f deploy/dev/docker-compose.slot.yml \
         -p "$project" up -d --wait)
    cargo run -p xtask --quiet -- slot tunnel "$name"

# Show every slot — name, slot number, ports, project, worktree path.
# Includes container status (best-effort `docker compose ps`).
worktree-list:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p xtask --quiet -- slot list
    echo ""
    echo "=== container status ==="
    cargo run -p xtask --quiet -- slot list --json \
      | (command -v jq >/dev/null && jq -r '.slots[] | select(.slot != 0) | .name' || \
         python3 -c 'import json,sys;[print(s["name"]) for s in json.load(sys.stdin)["slots"] if s["slot"]!=0]') \
      | while read -r name; do
          [ -z "$name" ] && continue
          project=$(cargo run -p xtask --quiet -- slot project "$name" 2>/dev/null || true)
          [ -z "$project" ] && continue
          echo "--- $name ($project) ---"
          if [ -d "worktrees/$name" ]; then
            (cd "worktrees/$name" && \
             docker compose -f deploy/dev/docker-compose.slot.yml -p "$project" ps 2>/dev/null) \
              || echo "(not running)"
          else
            echo "(worktree dir missing)"
          fi
        done

# Tear down a slot's compose project + per-slot volumes, remove the
# worktree dir, free the slot. Pass --with-branch to also delete the
# git branch (default: keep — branches are cheap, accidental deletion
# is annoying).
worktree-rm name *flags:
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"; flags="{{flags}}"
    with_branch=0
    for f in $flags; do [ "$f" = "--with-branch" ] && with_branch=1; done
    project=$(cargo run -p xtask --quiet -- slot project "$name")
    if [ -d "worktrees/$name" ]; then
      (cd "worktrees/$name" && \
       docker compose --env-file .env.slot \
           -f deploy/dev/docker-compose.slot.yml \
           -p "$project" down -v --remove-orphans) || true
    else
      # Worktree dir already gone — fall back to no-bind compose teardown.
      docker compose -p "$project" down -v --remove-orphans 2>/dev/null || true
    fi
    git worktree remove --force "worktrees/$name" 2>/dev/null || rm -rf "worktrees/$name"
    if [ "$with_branch" = "1" ]; then
      git branch -D "$name" 2>/dev/null || true
      echo "deleted branch $name"
    fi
    cargo run -p xtask --quiet -- slot free "$name"
    echo "freed slot for $name"

# Print the SSH `-L` flags needed to reach a slot from a laptop.
# Pass the second arg to override the SSH host (defaults to "vm").
worktree-tunnel name host="vm":
    cargo run -p xtask --quiet -- slot tunnel "{{name}}" --host "{{host}}"

# Run a one-off command inside a slot's stiglab container — same shell,
# same target dir, same toolchain as the live cargo-watch loop.
#   just slot-exec feat-a cargo test -p stiglab
#   just slot-exec feat-a bash
slot-exec name *cmd:
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"
    project=$(cargo run -p xtask --quiet -- slot project "$name")
    cd "worktrees/$name"
    if [ -z "{{cmd}}" ]; then
      docker compose -f deploy/dev/docker-compose.slot.yml -p "$project" exec stiglab bash
    else
      docker compose -f deploy/dev/docker-compose.slot.yml -p "$project" exec stiglab {{cmd}}
    fi
