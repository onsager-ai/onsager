//! Web Push dispatch (ADR 0020 § Invocation channels, spec #472).
//!
//! Portal owns the HITL-notification dispatch surface. When an
//! agent-requested HITL becomes pending (today: not yet wired to the
//! spine — the elicitation flow lands in a follow-up; see ADR 0008),
//! the dispatch helper here is the seam: callers build a
//! [`PushPayload`] from the HITL + scope and call [`dispatch_to_user`].
//!
//! Open-only action shape per ADR 0020 § Notification action buttons —
//! the only action is `Open`, which the service worker translates to a
//! deep-link click that opens the chat at the HITL's scope with the
//! Queue tab + the matching card highlighted. Approval-kind action
//! buttons (Approve / Reject) are deferred per the spec's fallback
//! option until the action-button-binding ADR lands.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use std::sync::Arc;
use thiserror::Error;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, SubscriptionKeys, VapidSignatureBuilder,
    WebPushClient, WebPushError, WebPushMessageBuilder,
};

use crate::push_db;

/// Runtime VAPID configuration. Pulled from env vars at startup
/// (`PORTAL_VAPID_PRIVATE_KEY`, `PORTAL_VAPID_PUBLIC_KEY`,
/// `PORTAL_VAPID_SUBJECT`). When `None`, push is disabled — the
/// dashboard surfaces this via `GET /api/push/config` so the
/// subscription UI stays hidden, and `dispatch_to_user` is a no-op.
#[derive(Debug, Clone)]
pub struct VapidConfig {
    /// VAPID private key (PKCS#8 base64-url, single line — what
    /// `web-push generate-vapid-keys --json` writes as `privateKey`).
    pub private_key: String,
    /// VAPID public key (base64-url, uncompressed P-256 — what
    /// `web-push generate-vapid-keys --json` writes as `publicKey`).
    /// The dashboard fetches this via `GET /api/push/config` and uses
    /// it for `pushManager.subscribe({ applicationServerKey })`.
    pub public_key: String,
    /// Publisher identity URL or `mailto:` per RFC 8292 — the push
    /// service relays this back to the browser. e.g.
    /// `mailto:ops@onsager.ai`.
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitlKind {
    Approval,
    Selection,
    Clarify,
}

/// One push payload — serialised to JSON and shipped as the push
/// message body. The service worker parses this into a notification
/// title + body + click-target URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPayload {
    pub title: String,
    pub body: String,
    /// HITL id used to highlight the matching Queue card after the
    /// deep-link click resolves.
    pub hitl_id: String,
    /// Scope key in the same shape `scopeKey()` produces on the
    /// dashboard — e.g. `"workspace"`, `"workflow:wf-123"`,
    /// `"verdict:v-456"`. Carried through the service worker into the
    /// deep-link URL.
    pub scope_key: String,
    /// Path the click should land on (under the workspace root) — e.g.
    /// `/workspaces/acme/runs/r-42`. The service worker appends the
    /// deep-link query params on top of this.
    pub click_path: String,
    pub kind: HitlKind,
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("push is not configured (VAPID keys missing)")]
    NotConfigured,
    #[error("serialisation: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("web-push: {0}")]
    WebPush(#[from] WebPushError),
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
}

/// Dispatch a [`PushPayload`] to every Web Push subscription the user
/// has registered. Subscriptions that the push service marks
/// permanently gone (HTTP 410) are pruned. Soft failures (5xx,
/// transient network errors) are logged but don't break the fan-out.
///
/// Returns the number of subscriptions the payload was successfully
/// delivered to.
pub async fn dispatch_to_user(
    pool: &PgPool,
    vapid: Option<&VapidConfig>,
    user_id: &str,
    payload: &PushPayload,
) -> Result<usize, DispatchError> {
    let Some(vapid) = vapid else {
        return Err(DispatchError::NotConfigured);
    };

    let subs = push_db::list_for_user(pool, user_id).await?;
    if subs.is_empty() {
        return Ok(0);
    }

    let client = IsahcWebPushClient::new()?;
    let body = serde_json::to_vec(payload)?;
    let mut delivered = 0usize;

    for sub in subs {
        match send_one(&client, vapid, &sub, &body).await {
            Ok(()) => {
                delivered += 1;
                let _ = push_db::mark_used(pool, &sub.id).await;
            }
            Err(WebPushError::EndpointNotValid(_) | WebPushError::EndpointNotFound(_)) => {
                tracing::info!(user_id, endpoint = %sub.endpoint, "pruning gone push subscription");
                let _ = push_db::delete_by_endpoint(pool, user_id, &sub.endpoint).await;
            }
            Err(e) => {
                tracing::warn!(user_id, endpoint = %sub.endpoint, error = %e, "push dispatch transient error");
            }
        }
    }

    Ok(delivered)
}

async fn send_one(
    client: &IsahcWebPushClient,
    vapid: &VapidConfig,
    sub: &push_db::PushSubscriptionRow,
    body: &[u8],
) -> Result<(), WebPushError> {
    let info = SubscriptionInfo {
        endpoint: sub.endpoint.clone(),
        keys: SubscriptionKeys {
            p256dh: sub.p256dh_key.clone(),
            auth: sub.auth_secret.clone(),
        },
    };

    let mut sig_builder =
        VapidSignatureBuilder::from_base64_no_sub(&vapid.private_key)?.add_sub_info(&info);
    sig_builder.add_claim("sub", vapid.subject.clone());
    let signature = sig_builder.build()?;

    let mut message = WebPushMessageBuilder::new(&info);
    message.set_payload(ContentEncoding::Aes128Gcm, body);
    message.set_vapid_signature(signature);
    client.send(message.build()?).await
}

/// Decode a base64-url public key for cheap config validation. Returns
/// `Some(bytes_len)` when the key parses; `None` otherwise.
pub fn decoded_public_key_len(public_key: &str) -> Option<usize> {
    URL_SAFE_NO_PAD.decode(public_key).ok().map(|b| b.len())
}

/// Build a typed VAPID config from the env-var triple, or return a
/// reason string for the operator if any piece is missing or
/// malformed. Called at startup; `None` means "push disabled".
pub fn config_from_env() -> Result<Option<VapidConfig>, String> {
    let priv_k = std::env::var("PORTAL_VAPID_PRIVATE_KEY").ok();
    let pub_k = std::env::var("PORTAL_VAPID_PUBLIC_KEY").ok();
    let subject = std::env::var("PORTAL_VAPID_SUBJECT").ok();
    match (priv_k, pub_k, subject) {
        (None, None, None) => Ok(None),
        (Some(priv_k), Some(pub_k), Some(subject)) => {
            if priv_k.is_empty() || pub_k.is_empty() || subject.is_empty() {
                return Err("PORTAL_VAPID_* env vars must be non-empty".into());
            }
            if decoded_public_key_len(&pub_k) != Some(65) {
                return Err(
                    "PORTAL_VAPID_PUBLIC_KEY must decode to a 65-byte uncompressed P-256 point"
                        .into(),
                );
            }
            Ok(Some(VapidConfig {
                private_key: priv_k,
                public_key: pub_k,
                subject,
            }))
        }
        _ => Err(
            "PORTAL_VAPID_PRIVATE_KEY / PORTAL_VAPID_PUBLIC_KEY / PORTAL_VAPID_SUBJECT must \
             all be set together"
                .into(),
        ),
    }
}

/// Shared state for the push handlers — `Option<VapidConfig>` so the
/// "push disabled" branch is a quick `None` check at every endpoint.
#[derive(Clone, Default)]
pub struct PushState {
    pub vapid: Option<Arc<VapidConfig>>,
}

impl PushState {
    pub fn new(vapid: Option<VapidConfig>) -> Self {
        Self {
            vapid: vapid.map(Arc::new),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_public_key_len_round_trip() {
        let raw = vec![4u8; 65];
        let encoded = URL_SAFE_NO_PAD.encode(&raw);
        assert_eq!(decoded_public_key_len(&encoded), Some(65));
        assert_eq!(decoded_public_key_len("not base64!@#"), None);
    }

    #[test]
    fn push_payload_round_trip_json() {
        let p = PushPayload {
            title: "Approve gate".into(),
            body: "Run #42 is waiting on you".into(),
            hitl_id: "h-1".into(),
            scope_key: "run:r-42".into(),
            click_path: "/workspaces/acme/runs/r-42".into(),
            kind: HitlKind::Approval,
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: PushPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back.hitl_id, "h-1");
        assert_eq!(back.scope_key, "run:r-42");
        assert!(matches!(back.kind, HitlKind::Approval));
    }
}
