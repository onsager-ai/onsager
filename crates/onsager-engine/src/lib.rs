//! # onsager-engine
//!
//! The factory side of the two-process architecture (ADR 0027 / spec
//! #586): trigger handling → plan compilation → executor dispatch →
//! event + artifact emission. One crate merging the former
//! `onsager-nodes` (executor runtime) and `onsager-scheduler`
//! (deployed substrate-scheduler host) crates, with module boundaries
//! mirroring them; the former `onsager-trigger` CLI ships as the
//! `onsager-trigger` bin.
//!
//! The Postgres spine is the only coupling to portal (the edge
//! process). The engine depends on the shared below-seam crates
//! (`onsager-artifact`, `onsager-spine`, `onsager-registry`,
//! `onsager-substrate`, `onsager-agent-spawn`) and never on portal.

pub mod nodes;
pub mod scheduler;
