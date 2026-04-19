//! `POST /webhooks/github` — entry point for every GitHub webhook delivery.
//!
//! Pipeline:
//! 1. Parse `X-GitHub-Event`, `X-Hub-Signature-256`, raw body bytes.
//! 2. Look up the installation row by `installation.id` in the body.
//! 3. Decrypt `webhook_secret_cipher` and HMAC-verify the signature.
//! 4. Dispatch to a per-event-type handler.
//!
//! Bad signatures and unknown installations both return `401`. Malformed
//! payloads return `400`. Successful dispatches return `202` (accepted).

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::Value;

use crate::handlers::{issues, pull_request};
use crate::signature::{verify_signature, SignatureCheck};
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

    let outcome = match event.as_str() {
        "pull_request" => pull_request::handle(&state, &installation, &parsed).await,
        "issues" => issues::handle(&state, &installation, &parsed).await,
        // Best-effort: every other event type is acknowledged so GitHub stops
        // retrying, but no business logic runs. Adding new types is purely
        // additive.
        _ => Ok(serde_json::json!({"event": event, "ignored": true})),
    };

    match outcome {
        Ok(body) => (StatusCode::ACCEPTED, Json(body)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "handler failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
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
