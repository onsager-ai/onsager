//! Delivery model and Consumer trait.
//!
//! See `specs/warehouse-and-delivery-v0.1.md` §4.3–§4.4, §5.2, §7. A
//! **delivery** is a single attempt to hand a sealed [`Bundle`] to one
//! [`ConsumerSink`] (GitHub, webhook, S3, …). Deliveries retry independently
//! of artifact state (invariant §9.5) and are at-least-once (§9.6), so
//! consumers must be idempotent keyed on `(bundle_id, consumer_id)`.
//!
//! This module defines the types and trait; a production delivery worker
//! that consumes `BundleSealed` events and drives the state machine lives in
//! its own subsystem (not in Forge — §5.2). v0.1 intentionally ships no real
//! consumer implementations; the GitHub and webhook consumers are §12 follow-up
//! slices.
//!
//! Spec crosswalk:
//! - [`DeliveryKind`]   ↔ §4.3 `kind: Initial | Rework`
//! - [`DeliveryStatus`] ↔ §4.3 `status: Pending | InFlight | Succeeded | Failed | Abandoned`
//! - [`Receipt`]        ↔ §7 `enum Receipt { GitHub, Webhook, S3, Filesystem }`
//! - [`DeliveryError`]  ↔ §7 retryable vs terminal

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use onsager_artifact::BundleId;
use onsager_warehouse::Bundle;
use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Identifier for a consumer sink row (§4.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConsumerId(String);

impl ConsumerId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(format!("csm_{}", ulid::Ulid::new()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConsumerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifier for a delivery attempt record (§4.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeliveryId(String);

impl DeliveryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(format!("dlv_{}", ulid::Ulid::new()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Consumer kinds and sinks
// ---------------------------------------------------------------------------

/// The kind of external system a sink ships to (§4.4).
///
/// `Custom` is an open escape hatch for operator-built integrations without
/// forcing a spine release. Each variant maps one-to-one to a [`Receipt`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerKind {
    #[serde(rename = "github")]
    GitHub,
    Webhook,
    S3,
    Filesystem,
    Custom(String),
}

impl fmt::Display for ConsumerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHub => f.write_str("github"),
            Self::Webhook => f.write_str("webhook"),
            Self::S3 => f.write_str("s3"),
            Self::Filesystem => f.write_str("filesystem"),
            Self::Custom(s) => f.write_str(s),
        }
    }
}

/// Retry policy applied to failing deliveries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub backoff_multiplier: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff_ms: 1_000,
            backoff_multiplier: 2,
        }
    }
}

/// An enabled external destination for a given artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerSink {
    pub consumer_id: ConsumerId,
    pub artifact_id: String,
    pub kind: ConsumerKind,
    /// Kind-specific config (GitHub repo + token ref, webhook URL, S3 bucket).
    pub config: serde_json::Value,
    pub retry_policy: RetryPolicy,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Delivery record
// ---------------------------------------------------------------------------

/// Initial delivery or a rework (superseding delivery for the same artifact).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryKind {
    Initial,
    Rework,
}

impl fmt::Display for DeliveryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initial => f.write_str("initial"),
            Self::Rework => f.write_str("rework"),
        }
    }
}

/// Delivery lifecycle state (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    Abandoned,
}

impl DeliveryStatus {
    /// Whether the delivery has reached a terminal state and will not change.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Abandoned)
    }
}

impl fmt::Display for DeliveryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::InFlight => "in_flight",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        };
        f.write_str(s)
    }
}

/// One delivery attempt record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub delivery_id: DeliveryId,
    pub bundle_id: BundleId,
    pub consumer_id: ConsumerId,
    pub kind: DeliveryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_receipt: Option<Receipt>,
    pub status: DeliveryStatus,
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Receipts
// ---------------------------------------------------------------------------

/// What the consumer returns on a successful delivery (§7).
///
/// Stored as JSONB on the delivery row and on the artifact's record of
/// most-recent receipts so that future reworks can pass `prior_receipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Receipt {
    #[serde(rename = "github")]
    GitHub {
        pr_url: String,
        commit_sha: String,
        branch: String,
    },
    Webhook {
        status: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        response_id: Option<String>,
    },
    S3 {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        version_id: Option<String>,
        etag: String,
    },
    Filesystem {
        path: PathBuf,
    },
    /// Consumer refused the rework — terminal; delivery becomes `Abandoned` (§6.4).
    RejectRework {
        reason: String,
    },
    /// Escape hatch for custom consumers (kind == `Custom(..)`).
    Custom {
        data: serde_json::Value,
    },
}

// ---------------------------------------------------------------------------
// Consumer contract
// ---------------------------------------------------------------------------

/// Retryable vs terminal so a retry policy can behave correctly (§7).
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// Transient (network, rate-limit) — worker should retry per policy.
    #[error("retryable delivery error: {0}")]
    Retryable(String),
    /// Terminal (bad config, permission denied, RejectRework) — worker should
    /// mark the delivery `Abandoned` without further retries.
    #[error("terminal delivery error: {0}")]
    Terminal(String),
}

#[derive(Debug, thiserror::Error)]
#[error("invalid consumer config: {0}")]
pub struct ConfigError(pub String);

/// Pluggable external delivery target (§7).
///
/// Implementations live in their own crates (per the subsystem isolation
/// invariant: `forge`, `stiglab`, `synodic`, `ising` cannot import each
/// other; but the delivery worker and its consumers are a separate subsystem
/// that depends only on `onsager-spine`).
#[async_trait]
pub trait Consumer: Send + Sync {
    async fn deliver(
        &self,
        bundle: &Bundle,
        kind: DeliveryKind,
        prior_receipt: Option<&Receipt>,
    ) -> Result<Receipt, DeliveryError>;

    fn kind(&self) -> ConsumerKind;

    fn validate_config(config: &serde_json::Value) -> Result<(), ConfigError>
    where
        Self: Sized;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_id_generate_format() {
        let id = ConsumerId::generate();
        assert!(id.as_str().starts_with("csm_"));
    }

    #[test]
    fn delivery_id_generate_format() {
        let id = DeliveryId::generate();
        assert!(id.as_str().starts_with("dlv_"));
    }

    #[test]
    fn delivery_status_terminal() {
        assert!(DeliveryStatus::Succeeded.is_terminal());
        assert!(DeliveryStatus::Abandoned.is_terminal());
        assert!(!DeliveryStatus::Pending.is_terminal());
        assert!(!DeliveryStatus::InFlight.is_terminal());
        assert!(!DeliveryStatus::Failed.is_terminal());
    }

    #[test]
    fn delivery_kind_serde() {
        assert_eq!(
            serde_json::to_string(&DeliveryKind::Initial).unwrap(),
            r#""initial""#
        );
        assert_eq!(
            serde_json::to_string(&DeliveryKind::Rework).unwrap(),
            r#""rework""#
        );
    }

    #[test]
    fn delivery_status_serde() {
        assert_eq!(
            serde_json::to_string(&DeliveryStatus::InFlight).unwrap(),
            r#""in_flight""#
        );
        assert_eq!(
            serde_json::to_string(&DeliveryStatus::Abandoned).unwrap(),
            r#""abandoned""#
        );
    }

    #[test]
    fn receipt_github_roundtrip() {
        let r = Receipt::GitHub {
            pr_url: "https://github.com/onsager-ai/onsager/pull/42".into(),
            commit_sha: "abc123".into(),
            branch: "feature/foo".into(),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["kind"], "github");
        let back: Receipt = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn receipt_reject_rework_roundtrip() {
        let r = Receipt::RejectRework {
            reason: "prior not yet accepted".into(),
        };
        let json = serde_json::to_value(&r).unwrap();
        let back: Receipt = serde_json::from_value(json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn retry_policy_defaults() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 5);
        assert_eq!(p.initial_backoff_ms, 1_000);
        assert_eq!(p.backoff_multiplier, 2);
    }

    #[test]
    fn consumer_kind_display() {
        assert_eq!(ConsumerKind::GitHub.to_string(), "github");
        assert_eq!(ConsumerKind::Webhook.to_string(), "webhook");
        assert_eq!(ConsumerKind::Custom("slack".into()).to_string(), "slack");
    }

    #[test]
    fn consumer_kind_serde_matches_display() {
        // Regression: default snake_case would emit "git_hub"; the explicit
        // rename keeps serde in sync with Display and Receipt tags.
        assert_eq!(
            serde_json::to_string(&ConsumerKind::GitHub).unwrap(),
            r#""github""#
        );
        let back: ConsumerKind = serde_json::from_str(r#""github""#).unwrap();
        assert_eq!(back, ConsumerKind::GitHub);
    }

    #[test]
    fn delivery_error_classification() {
        let retryable = DeliveryError::Retryable("rate limited".into());
        let terminal = DeliveryError::Terminal("permission denied".into());
        assert!(matches!(retryable, DeliveryError::Retryable(_)));
        assert!(matches!(terminal, DeliveryError::Terminal(_)));
    }
}
