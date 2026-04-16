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

pub mod artifact;
pub mod extension_event;
pub mod factory_event;
pub mod listener;
pub mod namespace;
pub mod protocol;
pub mod registry;
pub mod seed;
pub mod store;

pub use artifact::{
    Artifact, ArtifactId, ArtifactState, ArtifactVersion, Consumer, ConsumerType, ContentRef,
    HorizontalLineage, Kind, QualitySignal, QualitySource, QualityValue, VerticalLineage,
};
pub use extension_event::ExtensionEventRecord;
pub use factory_event::{FactoryEvent, FactoryEventKind};
pub use listener::{EventHandler, Listener};
pub use namespace::{Namespace, NamespaceError};
pub use protocol::{GateRequest, Insight, ShapingDecision, ShapingRequest, ShapingResult};
pub use registry::{
    AdapterMaterial, AdapterResult, AgentProfile, ArtifactAdapter, CompositeGate, ExternalRef,
    GateContext, GateEvaluator, GateVerdict, RegisteredType, RegistryId, RegistryStatus,
    TypeDefinition, DEFAULT_WORKSPACE, SEED_ACTOR,
};
pub use seed::{apply_seed, SeedCatalog, SeedOutcome};
pub use store::{
    append_factory_event_tx, EventMetadata, EventNotification, EventRecord, EventStore,
};
