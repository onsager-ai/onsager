# Changelog

All notable changes to this project are documented here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet publish numbered releases.

## [Unreleased]

### Added
- **Spine + forge**: warehouse & delivery v0.1 foundations with the spec
  in `crates/onsager-spine/specs/warehouse-and-delivery-v0.1.md`.
- **Spine**: factory pipeline foundations (issue #14, phases 0–3),
  including run DAG, flow controls, pull request kind, and git event
  scaffolding.
- **Dashboard**: factory pipeline becomes the primary entry point with
  artifact registration; mobile UX redesigned for desktop parity.
- **Dashboard**: shadcn/ui adoption — native form/interactive elements
  replaced with shadcn primitives, enforced by the `dashboard-ui` skill.
- **Dashboard tests**: L1 Playwright e2e scaffold and L2 AI-driven
  web-testing flow (issue #23).
- **Deployment**: Railway config-as-code, deploy-readiness CI, and
  unified container with all subsystems plus the combined dashboard.
- **Skills**: `onsager-pre-push`, `onsager-pr-lifecycle`, `dashboard-ui`,
  consolidated `railway` skill, and updated session-start hook docs.

### Changed
- **Spine**: `events_ext.namespace` now maps to `stream_type` in the
  spine API consumers; `sealed_at` truncated to microseconds for
  PostgreSQL round-trip stability.
- **Stiglab**: spine events query uses static SQL strings.
- **Dashboard**: rebranded from Stiglab to Onsager with integrated logo;
  favicon uses `currentColor` to match the logo convention.
- **CI**: per-crate path filters exclude docs; migrations 003 and 005
  are applied in the Rust workflow; Rust toolchain pinned to 1.95.0.
- **Migrations**: renumbered 003 → 004 after a main merge collision.

### Fixed
- **Forge**: dropped references to removed `onsager-spine` `Kind`
  variants.
- **Spine + forge**: addressed review comments on the warehouse &
  delivery foundations.
- **Sessions**: cleaned up log output — stopped JSON leak, deduplicated
  lines, tagged stderr; `streamLogs` handles `AbortError` cleanly.
- **Container**: drop root privileges via `gosu` so the Claude CLI
  accepts `bypassPermissions`.
- **Build**: commit `Cargo.lock` so Railway builds find it; copy all
  workspace manifests in the stiglab Dockerfile.

### Docs
- Root `README.md` and `apps/dashboard/README.md` rewritten to match the
  current monorepo layout (forge, ising) and `just dev` workflow.
- Outdated specs archived; spec indexes updated.
