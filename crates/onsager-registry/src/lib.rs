//! # onsager-registry
//!
//! Factory pipeline registry — type catalog, adapters, gate evaluators,
//! agent profiles, and the idempotent seed loader.
//!
//! See GitHub issue #14 for the design: types are data, registered by id,
//! mutated via spine events; the registry tables are the projection of that
//! event stream.

pub mod catalog;
pub mod evaluators;
pub mod events;
pub mod registry;
pub mod registry_store;
pub mod seed;
pub mod triggers;

pub use catalog::*;
pub use evaluators::*;
pub use events::{EventDefinition, EventManifest, Subsystem, EVENTS};
pub use registry::*;
pub use registry_store::*;
pub use seed::*;
pub use triggers::{TriggerCategory, TriggerDefinition, TriggerManifest, TriggerUiKind, TRIGGERS};
