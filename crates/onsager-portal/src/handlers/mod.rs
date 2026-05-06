//! HTTP handlers — webhook ingress (split by GitHub event type) plus
//! the user-facing `/api/auth/*` routes portal owns post-#222 Slice 5.

pub mod auth;
pub mod credentials;
pub mod issues;
pub mod pats;
pub mod pull_request;
pub mod spec_link;
pub mod webhook;
