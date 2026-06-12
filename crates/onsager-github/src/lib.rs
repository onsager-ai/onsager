//! # onsager-github
//!
//! Typed GitHub client, webhook signature verification, and OAuth —
//! the single home for GitHub HTTP. Ported wholesale from the legacy
//! crate (ADR 0029 / #624); `adapter` / `polling` / the `event` stub
//! did not port (no consumer in v2).
//!
//! Treats GitHub as configurable infra: when no credential resolves the
//! library returns [`GithubError::NotConfigured`] and callers no-op the
//! feature. `just dev` boots without GitHub credentials.

pub mod api;
pub mod credential;
pub mod error;
pub mod webhook;

pub use credential::{AccountKind, Credential, GithubAuth};
pub use error::GithubError;
