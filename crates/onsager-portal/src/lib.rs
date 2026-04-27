//! # onsager-portal
//!
//! GitHub-webhook ingress for the Onsager factory (issue #58 / #60).
//!
//! Lives alone as a deployable so webhooks — bursty, latency-sensitive,
//! signature-sensitive — don't share an event loop with the stiglab
//! coordination plane. The portal reads tenant / installation / project
//! tables that stiglab manages, signs every event with the per-installation
//! webhook secret, and writes:
//!
//! - `events_ext` rows under namespace `git` (PR open/sync/close)
//! - `artifacts` skeleton rows for external items (`Kind::PullRequest`,
//!   `Kind::GithubIssue`) — identity + our derived state only, no
//!   provider-authored fields (per spec #170)
//! - `vertical_lineage` rows when a webhook PR's `head.ref` matches a recent
//!   stiglab session's working branch
//!
//! Provider-authored fields (PR/issue title, body, labels, author) are
//! served by stiglab's user-facing API (`/api/projects/:id/{issues,pulls}`
//! in `crates/stiglab/src/server/routes/projects.rs`), which hydrates live
//! from GitHub through a short-TTL cache. The dashboard joins skeleton rows
//! with the live response on `external_ref`. Portal does not expose
//! user-facing endpoints — webhook ingress only.
//!
//! Portal-owned tables (`factory_tasks`, `pr_gate_verdicts`, `pr_branch_links`)
//! are migrated at startup; everything else (tenant / installation / project /
//! events / artifacts / lineage) is owned by stiglab and the spine.

pub mod backfill;
pub mod config;
pub mod db;
pub mod gate;
pub mod github;
pub mod handlers;
pub mod migrate;
pub mod server;
pub mod signature;
pub mod state;
