//! Forge — production line subsystem of the Onsager factory.
//!
//! Forge drives artifacts through their lifecycle: deciding what to shape next,
//! dispatching shaping work to Stiglab, consulting Synodic at every gate point,
//! advancing artifact state, and routing released artifacts to consumers.
//!
//! See `specs/forge-v0.1.md` for the full specification.

pub mod cmd;
pub mod core;
