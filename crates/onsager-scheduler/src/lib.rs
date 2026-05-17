//! # onsager-scheduler
//!
//! Deployed host for the substrate scheduler (RUN-03, #386). Owns the
//! process-level wiring the library-only [`onsager_nodes::Scheduler`]
//! does not: a spine-backed [`SpineClient`], a long-running `Listener`
//! that watches the spine for `trigger.fired` events, and the
//! [`bridge`] adapter that turns each fire into a compiled
//! [`onsager_substrate::compiler::ExecutionPlan`] handed to
//! [`onsager_nodes::Scheduler::run`].
//!
//! ## Why this crate exists
//!
//! Per ADR 0009 the 0.2 substrate replaces the 0.1 Forge tick loop:
//! coordination flows Spec Plan → Workflow → Execution Plan →
//! substrate scheduler. RUN-01 (#359) landed the in-process scheduler
//! library; MIG-01 (#363) deleted the forge subsystem that *used* to
//! consume `trigger.fired`. The gap between "library exists" and
//! "deployed binary consuming the spine" is what this crate closes.
//!
//! ## Host shape
//!
//! A dedicated binary, not a task colocated inside another subsystem:
//!
//! - The `onsager` dispatcher commits to **zero business deps** (see
//!   `crates/onsager/Cargo.toml`); a substrate-scheduler task there
//!   would force every dispatcher rebuild to recompile the executor
//!   catalog. The dispatcher discovers `onsager-scheduler` on PATH
//!   like every other subsystem binary.
//! - `onsager-portal` is the **edge** subsystem (ADR 0006). The
//!   scheduler has no external HTTP surface; hosting it inside
//!   portal would conflate edge and runtime concerns.
//!
//! The binary obeys the seam rule: it depends on substrate / nodes /
//! spine / artifact only — never on a sibling subsystem crate.

pub mod bridge;
pub mod service;
pub mod spine_client;

pub use bridge::{TriggerBridge, TriggerBridgeError};
pub use service::{SchedulerService, ServiceConfig};
pub use spine_client::SpineEventStoreClient;
