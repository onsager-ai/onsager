//! Webhook ingest helpers — signature verification + payload typing.
//!
//! `signature` owns the HMAC-SHA256 verification GitHub requires. The
//! `event` module sketches the typed `WebhookEvent` enum and the
//! `to_spine_events` translator the spec calls for; the full
//! host-agnostic event vocabulary (`code.pr_merged`, …) lands with
//! the spine event-registry work in #150 (#220 Sub-issue C).

pub mod event;
pub mod signature;

pub use event::{to_spine_events, WebhookEvent};
pub use signature::{verify_signature, SignatureCheck};
