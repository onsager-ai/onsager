# Onsager monorepo task runner.
# Rust + TS workspaces coexist; this file is just a command registry.

# Load .env into every recipe's environment. Lets `just dev` (which runs
# cargo directly, not via docker-compose) pick up credential keys and other
# config from .env — e.g. ONSAGER_CREDENTIAL_KEY / STIGLAB_CREDENTIAL_KEY.
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
    cargo run -p xtask --quiet -- check-metaphor-leakage
    cargo run -p xtask --quiet -- check-detail-page-tabs
    cargo run -p xtask --quiet -- check-file-budget --mode=fail
    cargo run -p xtask --quiet -- check-orphan-crates --mode=warn
    cargo run -p xtask --quiet -- check-single-impl-traits --mode=warn
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
# Postgres-gated spine/stiglab integration tests are left to CI /
# `just test-spine`; `ci-local-full` adds them when Postgres is up.
ci-local: ci-local-rust ci-local-ui

# Rust half — mirrors rust.yml's `check` job. `lint-rust` already runs
# fmt + workspace clippy + every xtask lint CI runs (incl. check-events);
# this adds the explicit workspace build + test. The DB-gated spine
# /stiglab tests skip without DATABASE_URL, exactly as in the "Test
# remaining crates" CI step — see `ci-local-full` to include them.
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

# Full parity including the Postgres-gated spine/stiglab integration
# tests. Requires a running Postgres (`just dev-infra`) with DATABASE_URL
# set; otherwise use plain `ci-local` (DB-free). Mirrors CI's three-way
# test split exactly — spine and stiglab run serial (`--test-threads=1`)
# because they share one DB schema — rather than `cargo test --workspace`,
# which would race them. `lint-rust` (fmt + clippy + xtask lints) runs
# first as a dependency.
ci-local-full: lint-rust
    cargo build --workspace
    cargo test -p onsager-spine -- --test-threads=1
    cargo test -p stiglab -- --test-threads=1
    cargo test --workspace --exclude onsager-spine --exclude stiglab
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
    STIGLAB_PORT=$(pick_port 3000)
    SYNODIC_PORT=$(pick_port 3001)
    DASHBOARD_PORT=$(pick_port 5173)
    export PORTAL_PORT STIGLAB_PORT  # vite.config.ts proxy targets

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
    SYNODIC_URL="http://localhost:${SYNODIC_PORT}" \
    STIGLAB_INTERNAL_WS_URL="ws://localhost:${STIGLAB_PORT}/agent/ws-internal" \
        cargo run -p onsager-portal -- serve &

    echo "==> Starting stiglab on :${STIGLAB_PORT}..."
    ONSAGER_DATABASE_URL="$DATABASE_URL" \
    STIGLAB_PORT="$STIGLAB_PORT" \
        cargo run -p stiglab -- server &

    echo "==> Starting synodic on :${SYNODIC_PORT}..."
    DATABASE_URL="$DATABASE_URL" \
    PORT=$SYNODIC_PORT cargo run -p synodic --features postgres -- serve &

    echo "==> Starting onsager-scheduler (substrate scheduler)..."
    DATABASE_URL="$DATABASE_URL" \
        cargo run -p onsager-scheduler -- serve &

    echo "==> Starting dashboard on :${DASHBOARD_PORT}..."
    pnpm --filter dashboard dev -- --port "$DASHBOARD_PORT" --strictPort &

    echo ""
    echo "=== Onsager dev stack running ==="
    [ -n "$route_file" ] && echo "  Devproxy:   http://${DEV_HOST}:8000"
    echo "  Dashboard:  http://localhost:${DASHBOARD_PORT}"
    echo "  Stiglab:    http://localhost:${STIGLAB_PORT}"
    echo "  Synodic:    http://localhost:${SYNODIC_PORT}"
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
        cargo run -p onsager-scheduler -- serve

dev-stiglab:
    cargo run -p stiglab -- server

dev-synodic port="3001":
    PORT={{port}} cargo run -p synodic --features postgres -- serve

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
    cargo install --path crates/onsager-scheduler
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
