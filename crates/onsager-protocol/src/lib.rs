//! # onsager-protocol
//!
//! Typed request/response contracts between Onsager subsystems.
//!
//! See `specs/subsystem-map-v0.1.md §4.1` for the four direct protocols
//! (Forge → Stiglab, Forge → Synodic, Stiglab → Synodic, Ising → Forge) and
//! `specs/forge-v0.1.md §5-7` for the detailed contracts.

pub mod protocol;

pub use protocol::*;
