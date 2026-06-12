//! # onsager-portal
//!
//! The **edge subsystem** — the only process hosting public HTTP routes
//! (ADR 0006): the dashboard API, GitHub webhooks, OAuth, credential
//! CRUD, the MCP server, and (post-ADR 0027 / spec #583) the in-process
//! agent session runner that replaced stiglab.
//!
//! Webhook ingress writes:
//!
//! - `events_ext` rows under namespace `git` (PR open/sync/close)
//! - `artifacts` skeleton rows for external items (`Kind::PullRequest`,
//!   `Kind::GithubIssue`) — identity + our derived state only, no
//!   provider-authored fields (per spec #170)
//! - `vertical_lineage` rows when a webhook PR's `head.ref` matches a
//!   recent agent session's working branch
//!
//! Provider-authored fields (PR/issue title, body, labels, author) are
//! hydrated live from GitHub through a short-TTL cache
//! (`/api/projects/:id/{issues,pulls}`); the dashboard joins skeleton
//! rows with the live response on `external_ref`.
//!
//! Portal-owned tables are migrated at startup (`migrate.rs`); the spine
//! tables (events / events_ext / artifacts / lineage / workspaces) come
//! from `crates/onsager-spine/migrations/`.

pub mod anthropic;
pub mod auth;
pub mod auth_db;
pub mod backfill;
pub mod chat_db;
pub mod config;
pub mod core;
pub mod credential_db;
pub mod db;
pub mod dev_auth;
pub mod feedback;
pub mod handlers;
pub mod installation;
pub mod installation_db;
pub mod listeners;
pub mod mcp;
pub mod migrate;
pub mod pat_db;
pub mod plan_run;
pub mod preset;
pub mod proxy_cache;
pub mod push;
pub mod push_db;
pub mod reconciliation;
pub mod remediation_db;
pub mod repo_access;
pub mod runtime;
pub mod server;
pub mod session_db;
pub mod session_runner;
pub mod session_token;
pub mod spec_plan_db;
pub mod sso;
pub mod state;
pub mod substrate_library_db;
pub mod workflow;
pub mod workflow_activation;
pub mod workflow_create;
pub mod workflow_dag;
pub mod workflow_db;
pub mod workflow_install;
pub mod workflow_version_db;
pub mod workspace_db;
