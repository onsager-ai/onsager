//! API/UI contract enforcement (spec #151 Lever F).
//!
//! Asserts that the dashboard ↔ backend HTTP surface stays wired in both
//! directions:
//!
//! 1. Every backend route registered in stiglab + synodic has at least one
//!    dashboard caller, **or** sits on the [`EXTERNAL_ONLY_ROUTES`]
//!    allowlist with a reason — webhooks, OAuth callbacks, agent WS,
//!    dev-login, the governance proxy catchall, and bridge-debt
//!    redirects.
//! 2. Every backend path the dashboard calls (from
//!    `apps/dashboard/src/lib/api.ts` and `apps/dashboard/src/lib/sse.ts`)
//!    matches a route registered on a backend subsystem.
//!
//! Backed by static parsing — `syn` for the Rust route chains, a small
//! hand-rolled scanner for the TS string literals. No server boot, no
//! runtime dependency on the dashboard build.
//!
//! Pairs with `lint_seams` (Lever B) and the future `check-events` (Lever
//! E #150). Together they cover the three #131 contract surfaces:
//! subsystem-to-subsystem (B), event types (E), API/UI (F).

use anyhow::Result;

pub fn run() -> Result<()> {
    // Implementation lands in subsequent chunks: route extraction, TS
    // scanner, normalization + allowlist, bidirectional comparison.
    println!("api-contract lint: skeleton (no checks wired yet — spec #151)");
    Ok(())
}
