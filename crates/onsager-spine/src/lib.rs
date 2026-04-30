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
//!
//! ## Split layout
//!
//! Value objects, storage backends, and protocols that used to live here have
//! moved into focused sibling crates:
//!
//! - `onsager-artifact` — `Artifact`, `ArtifactId`, `ArtifactVersionId`, lineage, quality.
//! - `onsager-warehouse` — `Bundle`, `Warehouse`, `FilesystemWarehouse`.
//! - `onsager-delivery` — `Consumer`, `Delivery`, `Receipt`.
//! - `onsager-registry` — type catalog, adapters, gate evaluators, seed loader.
//!
//! Spine keeps what every subsystem needs to speak to the event bus: the
//! `EventStore`, the `Listener`, `Namespace`, the `FactoryEvent` envelope,
//! and the typed request/response payloads carried inside those events
//! (`protocol`, formerly the `onsager-protocol` crate; merged in per
//! ADR 0004 / spec #131 Lever C). Artifact value objects are re-exported
//! here for backward compatibility.

pub mod extension_event;
pub mod factory_event;
pub mod listener;
pub mod namespace;
pub mod protocol;
pub mod store;

// Backward-compat re-exports of the artifact value objects. Spine depends on
// `onsager-artifact` because `FactoryEvent` references `ArtifactId`, `Kind`,
// `ArtifactState`, `ArtifactVersionId`, and `QualitySignal`. These re-exports let
// existing callers keep using `onsager_spine::{ArtifactId, ArtifactVersionId, ...}`
// without pulling in the warehouse/delivery/registry/protocol crates.
pub use onsager_artifact as artifact;
pub use onsager_artifact::{
    Artifact, ArtifactId, ArtifactState, ArtifactVersion, ArtifactVersionId, Consumer,
    ConsumerType, ContentRef, GitContext, HorizontalLineage, Kind, QualitySignal, QualitySource,
    QualityValue, VerticalLineage,
};

pub use extension_event::ExtensionEventRecord;
pub use factory_event::{
    EscalationResolution, EventRef, FactoryEvent, FactoryEventKind, ForgeProcessState, GatePoint,
    InsightKind, InsightScope, LineageType, ShapingOutcome, VerdictSummary,
};
pub use listener::{EventHandler, Listener};
pub use namespace::{Namespace, NamespaceError};
pub use store::{
    append_factory_event_tx, EventMetadata, EventNotification, EventRecord, EventStore,
};
