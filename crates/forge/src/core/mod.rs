//! Core Forge logic — scheduling kernel, artifact store, pipeline, and state machine.

pub mod artifact_store;
pub mod gate_verdict_listener;
pub mod insight_cache;
pub mod insight_listener;
pub mod kernel;
pub mod pending;
pub mod persistence;
pub mod pipeline;
pub mod session_listener;
pub mod shaping_result_listener;
pub mod signal_cache;
pub mod stage_runner;
pub mod state;
pub mod trigger_subscriber;
pub mod workflow;
pub mod workflow_gates;
pub mod workflow_persistence;
pub mod workflow_signal_listener;

pub use artifact_store::ArtifactStore;
pub use insight_cache::{InsightCache, DEFAULT_INSIGHT_CACHE_CAPACITY};
pub use kernel::{BaselineKernel, SchedulingKernel, WorldState};
pub use pipeline::ForgePipeline;
pub use session_listener::{SessionCompleted, SessionCompletedHandler};
pub use signal_cache::{Signal, SignalCache, SignalOutcome};
pub use stage_runner::{advance_workflow_artifacts, enter_workflow, GateEvaluator, StageEvent};
pub use state::ForgeState;
pub use workflow::{GateOutcome, GateSpec, StageSpec, TriggerKind, Workflow};
