//! # stiglab
//!
//! Distributed AI agent session orchestration. Part of the Onsager factory stack.
//!
//! This crate exposes `core` (shared types), `server` (control plane), and
//! `agent` (node agent runtime) as public modules so the `main.rs` CLI and
//! integration tests can reach into any of them. They are not intended as a
//! stable public API — other Onsager crates do NOT depend on this one.

pub mod agent;
pub mod core;
pub mod server;
