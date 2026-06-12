# Onsager v2 — single-container, single-process deploy (ADR 0029).
# The binary serves /api and the built dashboard; migrations apply at
# boot. No Caddy, no second process.

# ── dashboard build ──────────────────────────────────────────────────
FROM node:20-slim AS dashboard
RUN corepack enable
WORKDIR /work
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/dashboard/package.json apps/dashboard/package.json
RUN pnpm install --frozen-lockfile --filter dashboard
COPY apps/dashboard apps/dashboard
RUN pnpm --filter dashboard build

# ── rust build ────────────────────────────────────────────────────────
FROM rust:1-bookworm AS backend
WORKDIR /work
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release -p onsager

# ── runtime ───────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates git curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=backend /work/target/release/onsager /app/onsager
COPY --from=dashboard /work/apps/dashboard/dist /app/static
ENV ONSAGER_STATIC_DIR=/app/static
# Railway injects $PORT; default keeps local docker runs working.
CMD ["sh", "-c", "ONSAGER_BIND=0.0.0.0:${PORT:-3002} exec /app/onsager"]
