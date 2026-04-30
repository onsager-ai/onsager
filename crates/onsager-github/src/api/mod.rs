//! Typed REST wrappers over the GitHub API surface used by Onsager.
//!
//! Every outbound request goes through [`http::client`] so we have one
//! shared `reqwest::Client` (connection pool, TLS state) for the whole
//! workspace. New API helpers should live in a sub-module here rather
//! than constructing a client directly.

pub mod app;
pub mod http;
pub mod issues;
pub mod oauth;
pub mod pulls;
