//! Webhook ingest helpers — HMAC-SHA256 signature verification
//! (ported; the legacy `event` translator stub did not port — v2 types
//! payloads at the receiver).

pub mod signature;

pub use signature::{SignatureCheck, verify_signature};
