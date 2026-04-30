//! # onsager-github
//!
//! Typed GitHub client, webhook receiver helpers, and OAuth flow — the
//! single home for GitHub HTTP in the Onsager workspace.
//!
//! Treats GitHub as configurable infra: when no credential resolves the
//! library returns [`GithubError::NotConfigured`] and callers no-op the
//! feature. `just dev` boots without GitHub credentials.
//!
//! ## Module groups
//!
//! - [`credential`] — `Credential` enum + `GithubAuth` trait. App and
//!   PAT modes share one shape.
//! - [`api`] — typed REST wrappers used today (App/install token, repo +
//!   label listing, default branch, OAuth code exchange, paginated
//!   issue/PR listing, check runs, label toggle, issue comments).
//! - [`webhook`] — signature verification + payload typing.
//!   `WebhookEvent::to_spine_events` is sketched here; the full
//!   host-agnostic translation lands with the spine event-registry work
//!   in #150 (#220 Sub-issue C).
//! - [`adapter`] — boot-time registration into the spine
//!   `artifact_adapters` catalog (closes the empty-catalog drift from
//!   migration 004).
//!
//! ## Seam
//!
//! No code outside this crate should construct a `reqwest::Client` (or
//! `octocrab::Octocrab`, when added) for `api.github.com` /
//! `github.com`. The `xtask lint-seams` rule enforces this.

pub mod adapter;
pub mod api;
pub mod credential;
pub mod error;
pub mod webhook;

pub use credential::{AccountKind, Credential, GithubAuth};
pub use error::GithubError;
