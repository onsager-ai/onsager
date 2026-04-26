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
COPY crates/onsager-spine/Cargo.toml    crates/onsager-spine/Cargo.toml
COPY crates/onsager-artifact/Cargo.toml crates/onsager-artifact/Cargo.toml
COPY crates/onsager-protocol/Cargo.toml crates/onsager-protocol/Cargo.toml
COPY crates/onsager-warehouse/Cargo.toml crates/onsager-warehouse/Cargo.toml
COPY crates/onsager-delivery/Cargo.toml crates/onsager-delivery/Cargo.toml
COPY crates/onsager-registry/Cargo.toml crates/onsager-registry/Cargo.toml
COPY crates/onsager-portal/Cargo.toml   crates/onsager-portal/Cargo.toml
COPY crates/refract/Cargo.toml          crates/refract/Cargo.toml
COPY crates/onsager/Cargo.toml          crates/onsager/Cargo.toml
COPY crates/forge/Cargo.toml            crates/forge/Cargo.toml
COPY crates/ising/Cargo.toml            crates/ising/Cargo.toml
COPY crates/stiglab/Cargo.toml          crates/stiglab/Cargo.toml
COPY crates/synodic/Cargo.toml          crates/synodic/Cargo.toml
COPY xtask/Cargo.toml                   xtask/Cargo.toml
# Create dummy source files so Cargo can compile each crate stub.
# Crates with [lib] need src/lib.rs; crates with [[bin]] need src/main.rs.
RUN mkdir -p \
      crates/onsager-spine/src \
      crates/onsager-artifact/src \
      crates/onsager-protocol/src \
      crates/onsager-warehouse/src \
      crates/onsager-delivery/src \
      crates/onsager-registry/src \
      crates/onsager-portal/src \
      crates/refract/src \
      crates/onsager/src \
      crates/forge/src \
      crates/ising/src \
      crates/stiglab/src \
      crates/synodic/src \
      xtask/src \
    && touch \
      crates/onsager-spine/src/lib.rs \
      crates/onsager-artifact/src/lib.rs \
      crates/onsager-protocol/src/lib.rs \
      crates/onsager-warehouse/src/lib.rs \
      crates/onsager-delivery/src/lib.rs \
      crates/onsager-registry/src/lib.rs \
      crates/onsager-portal/src/lib.rs \
      crates/refract/src/lib.rs \
      crates/forge/src/lib.rs \
      crates/ising/src/lib.rs \
      crates/stiglab/src/lib.rs \
      crates/synodic/src/lib.rs \
    && echo "fn main() {}" | tee \
      crates/onsager/src/main.rs \
      crates/forge/src/main.rs \
      crates/ising/src/main.rs \
      crates/stiglab/src/main.rs \
      crates/synodic/src/main.rs \
      crates/onsager-portal/src/main.rs \
      crates/refract/src/main.rs \
      xtask/src/main.rs \
    && cargo build --release \
         -p stiglab -p synodic -p onsager-portal -p forge \
         --features synodic/postgres 2>/dev/null || true
# Copy actual source and rebuild.
# Touch all .rs files so their mtime is newer than the dummy-build artifacts —
# Docker's COPY preserves git timestamps which may predate the dummy build,
# causing Cargo to skip recompilation and deploy the stub binary instead.
COPY crates/ crates/
RUN find crates -name "*.rs" | xargs touch \
    && cargo build --release \
         -p stiglab -p synodic -p onsager-portal -p forge \
         --features synodic/postgres

# ---- Stage 3: Node.js for runtime (glibc-compatible) ----
FROM node:20-bookworm-slim AS node-runtime
RUN npm install -g @anthropic-ai/claude-code

# ---- Stage 4: Runtime ----
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

# Git credential helper — reads GITHUB_TOKEN from env at push time.
# Users add GITHUB_TOKEN as a credential in Dashboard > Settings; stiglab
# injects it as an env var into the claude subprocess, and this helper
# feeds it to git for HTTPS auth.
RUN printf '#!/bin/sh\necho "username=x-access-token\npassword=${GITHUB_TOKEN}"\n' \
      > /usr/local/bin/git-credential-env \
    && chmod +x /usr/local/bin/git-credential-env \
    && git config --system credential.helper /usr/local/bin/git-credential-env \
    && git config --system --add safe.directory '*'

RUN useradd -m -s /bin/bash onsager

WORKDIR /app
COPY --from=rust-builder /app/target/release/stiglab        /app/stiglab
COPY --from=rust-builder /app/target/release/synodic        /app/synodic
COPY --from=rust-builder /app/target/release/onsager-portal /app/onsager-portal
COPY --from=rust-builder /app/target/release/forge          /app/forge
COPY --from=ui-builder /app/apps/dashboard/dist /app/static
# Spine migrations — applied on startup when ONSAGER_DATABASE_URL is set
COPY crates/onsager-spine/migrations/ /app/spine-migrations/
# Synodic (governance) migrations
COPY crates/synodic/migrations/pg/ /app/synodic-migrations/
COPY crates/stiglab/deploy/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

RUN chown -R onsager:onsager /home/onsager

ENV STIGLAB_STATIC_DIR=/app/static
ENV STIGLAB_HOST=0.0.0.0
ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["server"]
