//! # onsager-warehouse
//!
//! Bundle model and [`Warehouse`] trait for sealed artifact storage.
//!
//! See `specs/warehouse-and-delivery-v0.1.md` §4.2 and §8. A bundle is an
//! immutable, content-addressed snapshot of what Forge produced at one
//! release. The `Warehouse` trait abstracts over storage backends; v0.1
//! ships the filesystem backend.

pub mod bundle;

pub use bundle::*;

// Re-export BundleId (which lives in onsager-artifact to avoid an
// artifact↔warehouse dependency cycle) so callers can reach it via
// the warehouse crate as well.
pub use onsager_artifact::BundleId;
