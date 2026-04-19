//! # onsager-delivery
//!
//! Delivery model and [`Consumer`] trait — the contract by which sealed
//! bundles are handed to external sinks (GitHub, webhooks, S3, filesystem).

pub mod delivery;

pub use delivery::*;
