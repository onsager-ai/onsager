# Dev-overlay image for the three Rust services (stiglab, synodic, forge).
# Used only by docker-compose.slot.yml. Production builds live in
# crates/stiglab/deploy/Dockerfile and crates/synodic/docker/Dockerfile.
#
# What this image gives a slot:
#   - The exact rust toolchain pinned in rust-toolchain.toml.
#   - cargo-watch for the edit/rebuild/restart loop.
#   - A workspace mount-point at /work plus a per-slot /work/target volume
#     and a shared /sccache volume — both wired by the compose file.
#   - sccache as the rustc wrapper so object-file output cached by one
#     slot accelerates builds in others.
#   - SYS_PTRACE + seccomp=unconfined enabled in the compose overlay
#     (not here) so gdb/lldb attach works without rebuilding the image.
#
# Source lives on a bind mount, so no COPY is needed at build time —
# the image is just toolchain + watcher + cache wrappers.

FROM rust:1.95-bookworm

ARG CARGO_WATCH_VERSION=8.5.3
ARG SCCACHE_VERSION=0.10.0

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      git \
      pkg-config \
      libssl-dev \
      libsqlite3-dev \
      postgresql-client \
      gdb \
      strace \
      lldb \
    && rm -rf /var/lib/apt/lists/*

# sccache binary release — much faster than `cargo install sccache` and
# avoids dragging the build deps into this image.
RUN ARCH=$(uname -m) \
    && curl -fsSL "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/sccache-v${SCCACHE_VERSION}-${ARCH}-unknown-linux-musl.tar.gz" \
       | tar -xzC /tmp \
    && mv "/tmp/sccache-v${SCCACHE_VERSION}-${ARCH}-unknown-linux-musl/sccache" /usr/local/bin/sccache \
    && chmod +x /usr/local/bin/sccache \
    && rm -rf "/tmp/sccache-v${SCCACHE_VERSION}-${ARCH}-unknown-linux-musl"

# cargo-watch — the dev entrypoint (`cargo watch -x 'run -p <bin>'`).
RUN cargo install --locked cargo-watch --version "${CARGO_WATCH_VERSION}"

# All compose services point CARGO_TARGET_DIR at the per-slot volume so
# the host tree is never written to from inside the container.
ENV CARGO_TARGET_DIR=/work/target \
    CARGO_HOME=/usr/local/cargo \
    SCCACHE_DIR=/sccache \
    RUSTC_WRAPPER=/usr/local/bin/sccache

WORKDIR /work
