//! # synodic
//!
//! AI agent governance via hooks and event spine integration. Part of the
//! Onsager factory stack.
//!
//! Modules:
//! - `core`: interception engine, storage, scoring, clustering (was harness-core)
//! - `cmd`: CLI subcommand implementations (orchestrate, intercept, serve, rules, ...)

pub mod cmd;
pub mod core;
pub mod util;
