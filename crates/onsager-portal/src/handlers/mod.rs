//! HTTP handlers — webhook ingress (split by GitHub event type) plus
//! the user-facing `/api/auth/*` routes portal owns post-#222 Slice 5.

pub mod auth;
pub mod credentials;
pub mod github_app;
pub mod installations;
pub mod issues;
pub mod pats;
pub mod projects;
pub mod pull_request;
pub mod spec_link;
pub mod webhook;
pub mod workflow_kinds;
pub mod workflows;
pub mod workspaces;
