//! Core Forge logic — scheduling kernel, artifact store, pipeline, and state machine.

pub mod artifact_store;
pub mod kernel;
pub mod persistence;
pub mod pipeline;
pub mod session_listener;
pub mod state;

pub use artifact_store::ArtifactStore;
pub use kernel::{BaselineKernel, SchedulingKernel, WorldState};
pub use pipeline::ForgePipeline;
pub use session_listener::{SessionCompleted, SessionCompletedHandler};
pub use state::ForgeState;
