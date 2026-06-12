# ---- Stage 1: Build UI ----
FROM node:20-alpine AS ui-builder
RUN corepack enable && corepack prepare pnpm@10.29.3 --activate
WORKDIR /app
COPY pnpm-workspace.yaml package.json pnpm-lock.yaml ./
COPY apps/dashboard/package.json apps/dashboard/
RUN pnpm install --frozen-lockfile --filter dashboard
COPY apps/dashboard/ apps/dashboard/
RUN pnpm --filter dashboard build

# ---- Stage 2: Build Rust ----
FROM rust:1.94-bookworm AS rust-builder
WORKDIR /app
# Cache dependencies: copy ALL workspace member manifests first.
# Cargo requires every member listed in [workspace.members] to have a
# Cargo.toml before it can resolve the workspace — missing manifests cause
# "no such file or directory" errors even when building a single crate (-p).
COPY Cargo.toml Cargo.lock ./
COPY crates/onsager-spine/Cargo.toml      crates/onsager-spine/Cargo.toml
COPY crates/onsager-artifact/Cargo.toml   crates/onsager-artifact/Cargo.toml
COPY crates/onsager-registry/Cargo.toml   crates/onsager-registry/Cargo.toml
COPY crates/onsager-substrate/Cargo.toml  crates/onsager-substrate/Cargo.toml
COPY crates/onsager-github/Cargo.toml     crates/onsager-github/Cargo.toml
COPY crates/onsager-portal/Cargo.toml     crates/onsager-portal/Cargo.toml
COPY crates/onsager-engine/Cargo.toml     crates/onsager-engine/Cargo.toml
COPY xtask/Cargo.toml                     xtask/Cargo.toml
# Create dummy source files so Cargo can compile each crate stub.
# Crates with [lib] need src/lib.rs; crates with [[bin]] need src/main.rs.
RUN mkdir -p \
      crates/onsager-spine/src \
      crates/onsager-artifact/src \
      crates/onsager-registry/src \
      crates/onsager-substrate/src \
      crates/onsager-github/src \
      crates/onsager-portal/src \
      crates/onsager-engine/src/bin \
      xtask/src \
    && touch \
      crates/onsager-spine/src/lib.rs \
      crates/onsager-artifact/src/lib.rs \
      crates/onsager-registry/src/lib.rs \
      crates/onsager-substrate/src/lib.rs \
      crates/onsager-github/src/lib.rs \
      crates/onsager-portal/src/lib.rs \
      crates/onsager-engine/src/lib.rs \
    && echo "fn main() {}" | tee \
      crates/onsager-portal/src/main.rs \
      crates/onsager-engine/src/bin/onsager-scheduler.rs \
      crates/onsager-engine/src/bin/onsager-trigger.rs \
      xtask/src/main.rs \
    && cargo build --release \
         -p onsager-portal -p onsager-engine 2>/dev/null || true
# Copy actual source and rebuild.
# Touch all .rs files so their mtime is newer than the dummy-build artifacts —
# Docker's COPY preserves git timestamps which may predate the dummy build,
# causing Cargo to skip recompilation and deploy the stub binary instead.
COPY crates/ crates/
RUN find crates -name "*.rs" | xargs touch \
    && cargo build --release \
         -p onsager-portal -p onsager-engine

# ---- Stage 3: Caddy binary ----
# Pulled as a named build stage so older BuildKit / Docker frontends that
# don't auto-pull external images on `COPY --from=image:tag` still work.
FROM caddy:2.8-alpine AS caddy-bin

# ---- Stage 4: Node.js for runtime (glibc-compatible) ----
FROM node:20-bookworm-slim AS node-runtime
RUN npm install -g @anthropic-ai/claude-code

# ---- Stage 5: Runtime ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates libssl3 libsqlite3-0 libgcc-s1 libstdc++6 \
      curl git postgresql-client gosu \
    && rm -rf /var/lib/apt/lists/*

# Copy Node.js + Claude Code CLI from the glibc-based node image
COPY --from=node-runtime /usr/local/bin/node /usr/local/bin/node
COPY --from=node-runtime /usr/local/bin/npm /usr/local/bin/npm
COPY --from=node-runtime /usr/local/lib/node_modules /usr/local/lib/node_modules
# Claude Code CLI installed globally via npm
COPY --from=node-runtime /usr/local/bin/claude /usr/local/bin/claude

# Caddy — the edge dispatcher. Caddy terminates the public port and
# routes /api/* + /mcp/* to portal and /* to the static dashboard
# files. Caddy's official image ships a static (CGO_ENABLED=0) Go
# binary, safe to drop into a debian-slim runtime.
# See ADR 0006 (edge dispatcher); /agent/ws retired with ADR 0027.
COPY --from=caddy-bin /usr/bin/caddy /usr/local/bin/caddy

# Git credential helper — reads GITHUB_TOKEN from env at push time.
# Users add GITHUB_TOKEN as a credential in Dashboard > Settings; portal's
# in-process session runner injects it as an env var into the claude
# subprocess, and this helper feeds it to git for HTTPS auth.
RUN printf '#!/bin/sh\necho "username=x-access-token\npassword=${GITHUB_TOKEN}"\n' \
      > /usr/local/bin/git-credential-env \
    && chmod +x /usr/local/bin/git-credential-env \
    && git config --system credential.helper /usr/local/bin/git-credential-env \
    && git config --system --add safe.directory '*'

RUN useradd -m -s /bin/bash onsager

WORKDIR /app
COPY --from=rust-builder /app/target/release/onsager-portal    /app/onsager-portal
COPY --from=rust-builder /app/target/release/onsager-scheduler /app/onsager-scheduler
COPY --from=ui-builder /app/apps/dashboard/dist /app/static
# Spine migrations — applied on startup when ONSAGER_DATABASE_URL is set
COPY crates/onsager-spine/migrations/ /app/spine-migrations/
COPY deploy/entrypoint.sh /app/entrypoint.sh
# Caddy serves /api/* + /mcp/* via reverse proxy and /* (the
# dashboard) from /app/static. See ADR 0006.
COPY deploy/Caddyfile /etc/caddy/Caddyfile
RUN chmod +x /app/entrypoint.sh

RUN chown -R onsager:onsager /home/onsager

ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["server"]
