//! Refract — the intent decomposer (issue #35).
//!
//! A Refract decomposer takes a high-level `Intent` ("migrate every auth
//! caller to the new SDK", "roll out the observability spec across the
//! monorepo") and expands it into an artifact tree the Forge pipeline can
//! drive to completion. The expansion itself is delegated to a registered
//! [`Decomposer`] implementation keyed on `intent_class`; this MVP ships
//! with one hard-coded decomposer (`FileMigrationDecomposer`) so the spine
//! wiring and event contract can be tested end-to-end without an LLM.
//!
//! Event contract (see `onsager-spine`):
//!   - `refract.intent_submitted`      — a new intent was filed
//!   - `refract.decomposed`            — decomposer produced artifact ids
//!   - `refract.failed`                — no decomposer matched or errored
//!
//! The crate itself has no runtime dependency on Forge, Stiglab, Synodic,
//! or Ising — it only knows the spine contract. Decomposers write their
//! results back to the spine so Forge's listener picks them up via
//! `artifact.registered` events (one per produced artifact id).

pub mod decomposer;
pub mod intent;
pub mod runtime;

pub use decomposer::{Decomposer, DecomposerError, DecomposerRegistry, DecompositionResult};
pub use intent::{Intent, IntentId};
pub use runtime::Refract;
