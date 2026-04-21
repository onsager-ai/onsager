//! GitHub webhook receiver (issue #81).
//!
//! `POST /api/webhooks/github` validates the `X-Hub-Signature-256` HMAC,
//! dispatches by `X-GitHub-Event`, and emits spine events through the
//! configured [`SpineEmitter`]. Unlike the legacy `/webhooks/github` proxy
//! (which forwards to onsager-portal for PR lineage), this endpoint is the
//! workflow-runtime entry point — it lives on stiglab so workflow CRUD and
//! webhook ingestion share a single process and a single auth surface.
//!
//! Signature validation is non-negotiable (see spec Notes) — unsigned
//! deliveries, malformed signatures, and unknown installations all return
//! `401`. Malformed payloads return `400`. Successful routing returns `202`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use onsager_spine::factory_event::FactoryEventKind;
use serde_json::Value;

use crate::server::auth::decrypt_credential;
use crate::server::state::AppState;
use crate::server::webhook_router::{
    route_check_event, route_issues_labeled, route_pull_request_closed, RoutedEvent,
};
use crate::server::workflow_db;

const HDR_EVENT: &str = "x-github-event";
const HDR_SIG: &str = "x-hub-signature-256";

/// Outcome of a signature check. Kept local so this module doesn't depend on
/// onsager-portal (matches the stiglab ← portal decoupling rule).
#[derive(Debug, PartialEq, Eq)]
enum SignatureCheck {
    Valid,
    Mismatch,
    Malformed,
}

fn verify_signature(header: &str, body: &[u8], secret: &[u8]) -> SignatureCheck {
    use ring::hmac;
    let Some(sig_hex) = header.strip_prefix("sha256=") else {
        return SignatureCheck::Malformed;
    };
    let Ok(sig) = hex::decode(sig_hex) else {
        return SignatureCheck::Malformed;
    };
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    match hmac::verify(&key, body, &sig) {
        Ok(()) => SignatureCheck::Valid,
        Err(_) => SignatureCheck::Mismatch,
    }
}

pub async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let event = headers
        .get(HDR_EVENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let signature = headers
        .get(HDR_SIG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "body not JSON" })),
            )
                .into_response();
        }
    };

    let install_id = match parsed
        .get("installation")
        .and_then(|i| i.get("id"))
        .and_then(|i| i.as_i64())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing installation.id" })),
            )
                .into_response();
        }
    };

    let cipher = match workflow_db::get_install_webhook_secret_cipher(&state.db, install_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            tracing::warn!(install_id, "installation has no webhook secret");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "unknown installation" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("install secret lookup failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(key_hex) = state.config.credential_key.as_ref() else {
        tracing::error!("STIGLAB_CREDENTIAL_KEY not set — cannot verify webhook signature");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let secret = match decrypt_credential(key_hex, &cipher) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("webhook secret decrypt failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(sig) = signature.as_deref() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "missing signature" })),
        )
            .into_response();
    };
    match verify_signature(sig, &body, secret.as_bytes()) {
        SignatureCheck::Valid => {}
        other => {
            tracing::warn!(?other, "signature check failed");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "signature invalid" })),
            )
                .into_response();
        }
    }

    let events = route_event(&state, &event, &parsed).await;
    emit_events(&state, events).await;

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "event": event, "ack": true })),
    )
        .into_response()
}

/// Translate a verified webhook payload into the list of spine events to
/// emit. Bundled as its own function so it can be unit-tested against a
/// stubbed DB in future iterations.
async fn route_event(state: &AppState, event: &str, payload: &Value) -> Vec<RoutedEvent> {
    match event {
        "issues" => {
            let repo_owner = payload
                .pointer("/repository/owner/login")
                .and_then(Value::as_str)
                .unwrap_or("");
            let repo_name = payload
                .pointer("/repository/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let label = payload
                .pointer("/label/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            if payload.get("action").and_then(Value::as_str) != Some("labeled")
                || repo_owner.is_empty()
                || repo_name.is_empty()
                || label.is_empty()
            {
                return Vec::new();
            }
            let matched = match workflow_db::find_active_github_workflows_for_label(
                &state.db, repo_owner, repo_name, label,
            )
            .await
            {
                Ok(matched) => matched,
                Err(e) => {
                    tracing::error!(
                        repo_owner = repo_owner,
                        repo_name = repo_name,
                        label = label,
                        error = %e,
                        "failed to query active github workflows for labeled-issue webhook"
                    );
                    return Vec::new();
                }
            };
            route_issues_labeled(payload, &matched)
        }
        "check_suite" | "check_run" | "status" => {
            route_check_event(event, payload).into_iter().collect()
        }
        "pull_request" => route_pull_request_closed(payload).into_iter().collect(),
        _ => Vec::new(),
    }
}

async fn emit_events(state: &AppState, events: Vec<RoutedEvent>) {
    let Some(spine) = state.spine.as_ref() else {
        if !events.is_empty() {
            tracing::warn!("spine not configured; dropping {} events", events.len());
        }
        return;
    };
    for ev in events {
        let data = match serde_json::to_value(&ev.kind) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("failed to serialize spine event: {e}");
                continue;
            }
        };
        let namespace = spine_namespace(&ev.kind);
        if let Err(e) = spine
            .emit_raw(
                &ev.kind.stream_id(),
                namespace,
                "stiglab",
                ev.kind.event_type(),
                &data,
            )
            .await
        {
            tracing::warn!("failed to emit webhook-sourced spine event: {e}");
        }
    }
}

fn spine_namespace(kind: &FactoryEventKind) -> &'static str {
    match kind {
        FactoryEventKind::TriggerFired { .. } => "workflow",
        FactoryEventKind::GateCheckUpdated { .. }
        | FactoryEventKind::GateManualApprovalSignal { .. } => "gate",
        _ => "stiglab",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hmac(body: &[u8], secret: &[u8]) -> String {
        use ring::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let tag = hmac::sign(&key, body);
        format!("sha256={}", hex::encode(tag.as_ref()))
    }

    #[test]
    fn valid_signature_is_accepted() {
        let body = br#"{"hello":"world"}"#;
        let secret = b"shhh";
        let header = hmac(body, secret);
        assert_eq!(
            verify_signature(&header, body, secret),
            SignatureCheck::Valid
        );
    }

    #[test]
    fn tampered_body_is_rejected() {
        let body = br#"{"hello":"world"}"#;
        let secret = b"shhh";
        let header = hmac(body, secret);
        assert_eq!(
            verify_signature(&header, b"other", secret),
            SignatureCheck::Mismatch
        );
    }

    #[test]
    fn malformed_header_is_rejected() {
        assert_eq!(
            verify_signature("deadbeef", b"x", b"shhh"),
            SignatureCheck::Malformed
        );
    }
}
