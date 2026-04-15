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
# Cache dependencies: copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/onsager-spine/Cargo.toml crates/onsager-spine/Cargo.toml
COPY crates/stiglab/Cargo.toml crates/stiglab/Cargo.toml
# Create dummy source files for dependency caching
RUN mkdir -p crates/onsager-spine/src crates/stiglab/src \
    && echo "fn main() {}" > crates/stiglab/src/main.rs \
    && touch crates/onsager-spine/src/lib.rs \
    && touch crates/stiglab/src/lib.rs \
    && cargo build --release -p stiglab 2>/dev/null || true
# Copy actual source and rebuild.
# Touch all .rs files so their mtime is newer than the dummy-build artifacts —
# Docker's COPY preserves git timestamps which may predate the dummy build,
# causing Cargo to skip recompilation and deploy the stub binary instead.
COPY crates/ crates/
RUN find crates -name "*.rs" | xargs touch && cargo build --release -p stiglab

# ---- Stage 3: Node.js for runtime (glibc-compatible) ----
FROM node:20-bookworm-slim AS node-runtime
RUN npm install -g @anthropic-ai/claude-code

# ---- Stage 4: Runtime ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates libssl3 libsqlite3-0 libgcc-s1 libstdc++6 \
      curl git \
    && rm -rf /var/lib/apt/lists/*

# Copy Node.js + Claude Code CLI from the glibc-based node image
COPY --from=node-runtime /usr/local/bin/node /usr/local/bin/node
COPY --from=node-runtime /usr/local/bin/npm /usr/local/bin/npm
COPY --from=node-runtime /usr/local/lib/node_modules /usr/local/lib/node_modules
# Claude Code CLI installed globally via npm
COPY --from=node-runtime /usr/local/bin/claude /usr/local/bin/claude

WORKDIR /app
COPY --from=rust-builder /app/target/release/stiglab /app/stiglab
COPY --from=ui-builder /app/apps/dashboard/dist /app/static
ENV STIGLAB_STATIC_DIR=/app/static
ENV STIGLAB_HOST=0.0.0.0
ENTRYPOINT ["/app/stiglab"]
CMD ["server"]
