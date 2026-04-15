//! Core Forge logic — scheduling kernel, artifact store, pipeline, and state machine.

pub mod artifact_store;
pub mod kernel;
pub mod pipeline;
pub mod state;

pub use artifact_store::ArtifactStore;
pub use kernel::{BaselineKernel, SchedulingKernel, WorldState};
pub use pipeline::ForgePipeline;
pub use state::ForgeState;
