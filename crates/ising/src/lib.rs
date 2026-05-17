//! # ising (deprecated)
//!
//! Ising was the 0.1 continuous-improvement subsystem — a polled
//! analyzer engine that rebuilt a `FactoryModel` each tick and ran
//! a hand-rolled emitter against it. In 0.2 its responsibilities
//! move to the substrate's **Observer** citizen
//! ([ADR 0013](../../../docs/adr/0013-observer-as-second-substrate-citizen.md)):
//!
//! - The four detection patterns (`gate_override`, `gate_deny_rate`,
//!   `shape_retry_spike`, `pr_churn`) live in `onsager-observers`
//!   under the same names, ported to the
//!   [`Observer`](https://docs.rs/onsager-observers) trait. Spec
//!   #362 (OBS-02) did the move.
//! - Output flows through `observer_outputs` rather than the
//!   bespoke `events_ext` insight emitter.
//! - There is no longer a tick loop; observers consume the spine
//!   directly via [`ObserverRuntime`].
//!
//! This crate remains in the workspace as an empty shell. A future
//! migration spec retires it entirely (event manifest rows, xtask
//! `Subsystem::Ising` entries, dashboard references).
#![deny(missing_docs)]
