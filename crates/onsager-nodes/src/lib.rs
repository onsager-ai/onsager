//! # onsager-nodes
//!
//! The runtime half of the executor catalog — the `async` side of the
//! `Executor` trait, the [`ExecutorRegistry`] that holds one instance
//! per executor kind, and a `dispatch(...)` helper that takes a
//! substrate `Node` and runs its executor.
//!
//! See:
//! - [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md)
//!   — why nodes carry `Box<dyn Executor>` instead of a closed
//!   `NodeKind` enum, and the flat-crate convention for executor
//!   implementations.
//! - The static / serializable half of the trait lives in
//!   [`onsager_substrate::Executor`] — `executor_kind()` /
//!   `declared_provenance()` for workflow validation and serde
//!   round-trips. This crate's [`Executor`] is the runtime sibling: it
//!   shares the same `executor_kind()` string (that's how dispatch
//!   resolves a node to its runtime), and adds `async fn execute(..)`.
//!
//! Subsequent issues (EXE-02 through EXE-06) land concrete executors
//! — Script, Agent, Verify, Human, SubWorkflow — as flat sibling
//! modules here. EXE-02 (#354) lands [`script::ScriptExecutor`].

pub mod agent;
pub mod context;
pub mod dispatch;
pub mod error;
pub mod executor;
pub mod registry;
pub mod scheduler;
pub mod script;
pub mod spine;
pub mod subworkflow;
pub mod verify;

pub use agent::{
    AgentExecutor, AgentRequest, AgentResponse, AgentRunError, AgentRunner, StubAgentRunner,
};
pub use context::{ExecutorContext, ExecutorOutputs};
pub use dispatch::dispatch;
pub use error::ExecutorError;
pub use executor::{Executor, NoOpExecutor};
pub use registry::ExecutorRegistry;
pub use scheduler::{
    EVENT_NODE_COMPLETED, EVENT_NODE_FAILED, EVENT_NODE_STARTED, InMemoryPlanStore, NodeState,
    PlanId, PlanStore, PlanStoreError, Scheduler, SchedulerError,
};
pub use script::{INLINE_URI_PREFIX, ScriptExecutor, decode_inline_body};
pub use spine::{SpineClient, SpineError};
pub use subworkflow::{
    SUBWORKFLOW_KIND, SchedulerSubWorkflowRunner, StubSubWorkflowRunner, SubWorkflowExecutor,
    SubWorkflowRunError, SubWorkflowRunner,
};
pub use verify::{Check, FailPolicy, VERIFY_KIND, VerifyExecutor};
