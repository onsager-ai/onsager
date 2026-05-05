//! `POST /webhooks/github` — entry point for every GitHub webhook delivery.
//!
//! Pipeline:
//! 1. Parse `X-GitHub-Event`, `X-Hub-Signature-256`, raw body bytes.
//! 2. Look up the installation row by `installation.id` in the body.
//! 3. Decrypt `webhook_secret_cipher` and HMAC-verify the signature.
//! 4. Dispatch to per-event-type lineage handlers (PR / issue artifact
//!    upserts) and to the workflow trigger router (issues.labeled →
//!    `TriggerFired`, check_*.completed → `GateCheckUpdated`,
//!    pull_request.closed+merged → `GateManualApprovalSignal`).
//! 5. Emit any routed events through the spine.
//!
//! Bad signatures and unknown installations both return `401`. Malformed
//! payloads return `400`. Successful dispatches return `202` (accepted).

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::Value;

use onsager_github::webhook::{verify_signature, SignatureCheck};
use onsager_spine::webhook_routing::{
    route_check_event, route_issues_labeled, route_pull_request_closed, spine_namespace,
    RoutedEvent,
};

use crate::handlers::{issues, pull_request};
use crate::state::AppState;

/// Header GitHub sends with the event type (e.g. `pull_request`,
/// `issues`, `installation`).
const HDR_EVENT: &str = "x-github-event";
/// Header carrying the HMAC signature.
const HDR_SIG: &str = "x-hub-signature-256";

pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let event = headers
        .get(HDR_EVENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let signature = headers
        .get(HDR_SIG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "body not JSON"})),
            )
                .into_response();
        }
    };

    // Resolve installation. Webhooks without an `installation.id` block can't
    // be routed to a tenant — return 400 so GitHub stops retrying immediately.
    let install_id = match parsed
        .get("installation")
        .and_then(|i| i.get("id"))
        .and_then(|i| i.as_i64())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "missing installation.id"})),
            )
                .into_response();
        }
    };

    let installation =
        match crate::db::find_installation_by_install_id(&state.pool, install_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                tracing::warn!(install_id, "unknown installation");
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "unknown installation"})),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::error!(error = %e, "installation lookup failed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    // Fail closed: an installation row without a configured secret would
    // otherwise let an attacker send unsigned webhooks and have them accepted.
    // Configuration must be completed (install row gets a `webhook_secret_cipher`)
    // before the installation can route traffic.
    let Some(cipher) = installation.webhook_secret_cipher.as_ref() else {
        tracing::warn!(
            "installation {} has no webhook secret configured; rejecting webhook",
            installation.id
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "webhook secret not configured"})),
        )
            .into_response();
    };
    let Some(key_hex) = state.config.credential_key.as_ref() else {
        tracing::error!(
            "installation {} has webhook_secret_cipher but ONSAGER_CREDENTIAL_KEY not set",
            installation.id
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let secret = match decrypt(key_hex, cipher) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "webhook secret decrypt failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(sig) = signature.as_deref() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "missing signature"})),
        )
            .into_response();
    };
    match verify_signature(sig, &body, secret.as_bytes()) {
        SignatureCheck::Valid => {}
        other => {
            tracing::warn!(?other, "signature check failed");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "signature invalid"})),
            )
                .into_response();
        }
    }

    // Lineage handlers (PR / issue artifact upserts). These are portal-owned
    // and were here before workflow trigger routing moved in.
    let outcome = match event.as_str() {
        "pull_request" => pull_request::handle(&state, &installation, &parsed).await,
        "issues" => issues::handle(&state, &installation, &parsed).await,
        // Best-effort: every other event type is acknowledged so GitHub stops
        // retrying, but no business logic runs. Adding new types is purely
        // additive.
        _ => Ok(serde_json::json!({"event": event, "ignored": true})),
    };
    if let Err(e) = &outcome {
        tracing::error!(error = %e, "lineage handler failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }

    // Workflow trigger routing. Independent of lineage — a PR-merged
    // delivery emits both lineage state changes AND a manual-approval
    // signal. Failures here are logged but do not fail the webhook
    // response: GitHub should not retry a delivery whose signature we
    // already accepted.
    let routed = route_workflow_events(&state, &event, install_id, &parsed).await;
    // `installation.tenant_id` carries the workspace_id at runtime —
    // the column was renamed `tenant_id` → `workspace_id` per #163 but
    // portal's row struct still uses the legacy field name (cleanup
    // tracked separately).
    let workspace_id = installation.tenant_id.as_str();
    emit_routed_events(&state, routed, workspace_id).await;

    let body = outcome.unwrap_or_else(|_| serde_json::json!({"event": event, "ignored": true}));
    (StatusCode::ACCEPTED, Json(body)).into_response()
}

/// Translate a verified webhook payload into the list of spine events to
/// emit on the workflow / gate streams. Lineage upserts are handled by the
/// per-event-type lineage handlers; this function only produces the
/// `TriggerFired` / `GateCheckUpdated` / `GateManualApprovalSignal` shapes.
async fn route_workflow_events(
    state: &AppState,
    event: &str,
    install_id: i64,
    payload: &Value,
) -> Vec<RoutedEvent> {
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
            let matched = match crate::db::find_active_github_workflows_for_label(
                &state.pool,
                install_id,
                repo_owner,
                repo_name,
                label,
            )
            .await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(
                        repo_owner,
                        repo_name,
                        label,
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

async fn emit_routed_events(state: &AppState, events: Vec<RoutedEvent>, workspace_id: &str) {
    for ev in events {
        let mut data = match serde_json::to_value(&ev.kind) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("failed to serialize spine event: {e}");
                continue;
            }
        };
        // Stamp workspace_id (#164) so downstream consumers — including
        // the workspace-scoped `/api/spine/events` listing — can filter
        // by workspace without re-resolving the install. The
        // `TriggerFired` payload already includes its workflow's
        // workspace; we don't overwrite it (a missing entry is the
        // common case for `gate.*` events from check / PR webhooks).
        if let Some(obj) = data.as_object_mut() {
            obj.entry("workspace_id".to_string())
                .or_insert(serde_json::Value::String(workspace_id.to_string()));
        }
        let namespace = spine_namespace(&ev.kind);
        let metadata = onsager_spine::EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: "portal".to_string(),
        };
        if let Err(e) = state
            .spine
            .append_ext(
                workspace_id,
                &ev.kind.stream_id(),
                namespace,
                ev.kind.event_type(),
                data,
                &metadata,
                None,
            )
            .await
        {
            tracing::warn!("failed to emit webhook-sourced spine event: {e}");
        }
    }
}

/// Decrypt a `nonce||ciphertext` hex-encoded blob using AES-256-GCM, exactly
/// matching `stiglab::server::auth::decrypt_credential`. Duplicated here so
/// the portal doesn't take a stiglab crate dependency.
fn decrypt(key_hex: &str, encrypted_hex: &str) -> anyhow::Result<String> {
    use ring::aead;
    let key_bytes = hex::decode(key_hex)?;
    let data = hex::decode(encrypted_hex)?;
    if data.len() < 12 {
        anyhow::bail!("invalid encrypted data");
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| anyhow::anyhow!("invalid encryption key"))?;
    let opening = aead::LessSafeKey::new(unbound);
    let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| anyhow::anyhow!("invalid nonce"))?;
    let mut in_out = ciphertext.to_vec();
    let plaintext = opening
        .open_in_place(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| anyhow::anyhow!("decryption failed"))?;
    Ok(String::from_utf8(plaintext.to_vec())?)
}
