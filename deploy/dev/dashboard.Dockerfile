# Dev-overlay image for the dashboard (Vite dev server with HMR).
# Used only by docker-compose.slot.yml. Production dashboard is built
# into stiglab's runtime image; this exists for the per-slot dev loop.
#
# Source lives on a bind mount; node_modules is a per-container layer
# (re-installed on first up) so OS-level mismatches between the host
# and the container don't break native modules like esbuild.

FROM node:20-bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/* \
    && corepack enable \
    && corepack prepare pnpm@10.29.3 --activate

WORKDIR /work

# Vite binds to 0.0.0.0 inside the container; Caddy reverse-proxies
# `/` (HTTP) and `/?token=` style HMR upgrades to this service. The
# dashboard's vite.config.ts reads VITE_HMR_CLIENT_PORT to point the
# browser-side WebSocket at the slot's edge port.
EXPOSE 5173
