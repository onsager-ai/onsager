//! # Onsager
//!
//! Client library for the Onsager event spine — shared PostgreSQL event stream
//! coordination for the [onsager-ai](https://github.com/onsager-ai) polyrepo.
//!
//! ## Core concepts
//!
//! - **[`EventStore`]** — read/write access to the `events` and `events_ext`
//!   tables, plus real-time `pg_notify` subscription.
//! - **[`Listener`]** — high-level consumer that filters notifications by
//!   [`Namespace`] and dispatches them to an [`EventHandler`].
//! - **[`Namespace`]** — validated newtype that partitions the `events_ext`
//!   table between components (`stiglab`, `synodic`, `ising`, `telegramable`).
//!
//! ## Schema
//!
//! This library does **not** manage database schema. The contract lives in
//! `migrations/001_initial.sql`; downstream services apply it themselves.

pub mod core_event;
pub mod extension_event;
pub mod listener;
pub mod namespace;
pub mod store;

pub use core_event::CoreEvent;
pub use extension_event::ExtensionEventRecord;
pub use listener::{EventHandler, Listener};
pub use namespace::{Namespace, NamespaceError};
pub use store::{EventMetadata, EventNotification, EventRecord, EventStore};
