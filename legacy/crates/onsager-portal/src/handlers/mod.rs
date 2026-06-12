//! HTTP handlers — webhook ingress (split by GitHub event type) plus
//! the user-facing `/api/auth/*` routes portal owns post-#222 Slice 5.

pub mod activation;
pub mod auth;
pub mod build_info;
pub mod chat;
pub mod credentials;
pub mod github_app;
pub mod installations;
pub mod issues;
pub mod live_data;
pub mod pats;
pub mod projects;
pub mod pull_request;
pub mod push;
pub mod registry_events;
pub mod registry_triggers;
pub mod runs;
pub mod session_liveness;
pub mod sessions;
pub mod showcase;
pub mod spine;
pub mod tasks;
pub mod telegram_webhook;
pub mod triggers;
pub mod webhook;
pub mod webhook_health;
pub mod workflow_kinds;
pub mod workflow_views;
pub mod workflows;
pub mod workspaces;
