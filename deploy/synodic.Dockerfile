# Synodic — AI agent governance server.
# Build context: repository root (../ from deploy/).

# ---- Stage 1: Build Rust ----
FROM rust:1.94-bookworm AS rust-builder
WORKDIR /app
# Cache dependencies: copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/onsager-spine/Cargo.toml crates/onsager-spine/Cargo.toml
COPY crates/synodic/Cargo.toml crates/synodic/Cargo.toml
# Create dummy source files for dependency caching
RUN mkdir -p crates/onsager-spine/src crates/synodic/src \
    && echo "fn main() {}" > crates/synodic/src/main.rs \
    && touch crates/onsager-spine/src/lib.rs \
    && touch crates/synodic/src/lib.rs \
    && cargo build --release -p synodic --features postgres 2>/dev/null || true
# Copy actual source and rebuild.
# Touch all .rs files so their mtime is newer than the dummy-build artifacts.
COPY crates/ crates/
RUN find crates -name "*.rs" | xargs touch \
    && cargo build --release -p synodic --features postgres

# ---- Stage 2: Runtime ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates libssl3 libgcc-s1 libstdc++6 curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=rust-builder /app/target/release/synodic /app/synodic
ENTRYPOINT ["/app/synodic"]
CMD ["serve"]
