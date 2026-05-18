//! HTTP handlers — webhook ingress (split by GitHub event type) plus
//! the user-facing `/api/auth/*` routes portal owns post-#222 Slice 5.

pub mod agent_ws;
pub mod auth;
pub mod build_info;
pub mod chat;
pub mod credentials;
pub mod github_app;
pub mod governance;
pub mod installations;
pub mod issues;
pub mod live_data;
pub mod nodes;
pub mod pats;
pub mod projects;
pub mod pull_request;
pub mod registry_events;
pub mod registry_triggers;
pub mod runs;
pub mod sessions;
pub mod showcase;
pub mod spec_link;
pub mod spine;
pub mod tasks;
pub mod telegram_webhook;
pub mod triggers;
pub mod webhook;
pub mod workflow_kinds;
pub mod workflow_views;
pub mod workflows;
pub mod workspaces;
