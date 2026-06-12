//! GitHub adapter identity constants.
//!
//! The 0.1 boot-time registration into the `artifact_adapters` catalog
//! retired with the registry diet (ADR 0028 / #603 dropped the table;
//! the registration fns were deleted at the adoption flip). What
//! remains is the adapter's stable identity, still stamped on
//! `external_ref.adapter` for GitHub-sourced artifact refs.

/// Stable adapter identifier. Matches the `external_ref.adapter` field
/// already in use on artifact rows for GitHub-sourced refs.
pub const ADAPTER_ID: &str = "github";
