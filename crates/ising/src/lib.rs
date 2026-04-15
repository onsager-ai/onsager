//! Ising — continuous improvement engine of the Onsager factory.
//!
//! Ising observes the entire factory event spine and surfaces insights that make
//! the factory smarter over time. It detects patterns (failures, waste, wins,
//! anomalies) and emits structured insights back to the spine.
//!
//! Ising is **advisory-only** — it cannot block production, deny gate requests,
//! or force scheduling decisions. Its path to influencing the factory is through
//! advisory forwarding to Forge and rule proposals to Synodic.
//!
//! See `specs/ising-v0.1.md` for the full specification.

pub mod analyzers;
pub mod cmd;
pub mod core;
