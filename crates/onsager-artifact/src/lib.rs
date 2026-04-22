//! # onsager-artifact
//!
//! Artifact value objects shared across the Onsager factory stack.
//!
//! This crate holds the pure data model — no database, no traits binding to
//! storage backends, no event spine dependency. It sits at the bottom of the
//! dependency graph so every other Onsager crate can talk about artifacts
//! without pulling in unrelated concerns.

pub mod artifact;
pub mod deliverable;

pub use artifact::*;
pub use deliverable::*;
