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

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::Value;

use onsager_github::webhook::{SignatureCheck, verify_signature};
use onsager_spine::webhook_routing::{
    RoutedEvent, WorkflowTrigger, route_check_event, route_issues_labeled,
    route_pull_request_closed, route_pull_request_closed_workflows,
    route_workflow_run_completed_workflows,
};

use crate::db::{issue_external_ref, pr_external_ref};
use crate::handlers::{issues, pull_request};
use crate::reconciliation::emit::emit_routed_events;
use crate::reconciliation::translator::GITHUB_ADAPTER_ID;
use crate::state::AppState;

/// Header GitHub sends with the event type (e.g. `pull_request`,
/// `issues`, `installation`).
const HDR_EVENT: &str = "x-github-event";
/// Header carrying the HMAC signature.
const HDR_SIG: &str = "x-hub-signature-256";

/// Wrapper for `/api/github-app/webhook` — the "plausible-looking but
/// wrong" path PR #119 healed (spec #120). Logs the install ID at `info`
/// before delegating so operators can identify tenants whose App is
/// still configured to post here and reach out to migrate them. The
/// other accepted URLs (`/webhooks/github`, `/api/webhooks/github`) are
/// both paths portal itself uses for registration (`WEBHOOK_PATH` in
/// `workflow_activation.rs`) — those deliveries are normal traffic and
/// must NOT be logged at this volume.
pub async fn handle_alias_github_app(
    state: State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    log_alias_delivery("/api/github-app/webhook", &body);
    handle(state, headers, body).await
}

/// Emit a structured `info` log identifying a delivery that arrived on a
/// misconfigured URL. `install_id` is extracted from the body when the
/// payload is well-formed JSON with an `installation.id` field; otherwise
/// `None`. Operators use this log to identify tenants who should
/// reconfigure their App's webhook URL to `/webhooks/github`.
fn log_alias_delivery(alias_path: &str, body: &Bytes) {
    let install_id = extract_install_id_from_payload(body);
    tracing::info!(
        target: "portal::webhook::alias",
        alias_path,
        install_id,
        "webhook delivery received on misconfigured alias path; tenant should reconfigure App webhook URL to /webhooks/github"
    );
}

/// Parse the webhook body just enough to surface `installation.id`. Best
/// effort: any parse failure or missing field yields `None`. Kept
/// independent of the main handler's parse so the alias log can fire
/// before signature verification (which is the point — we want to know
/// who's using the wrong URL even if the delivery later fails auth).
fn extract_install_id_from_payload(body: &Bytes) -> Option<i64> {
    let parsed: Value = serde_json::from_slice(body).ok()?;
    parsed.get("installation")?.get("id")?.as_i64()
}

pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
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
    // Stamp (adapter_id, external_ref) per-event so the spine's
    // partial unique index (migration 032) deduplicates this delivery
    // against any reconciliation-poller emit for the same resource
    // update. Best-effort: when project resolution fails we still
    // emit, just without the dedup key, matching pre-#430 behavior.
    let decorated = decorate_routed_with_dedup(&state, &parsed, &installation.id, routed).await;
    let _ = emit_routed_events(&state.spine, decorated, workspace_id, "portal").await;

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
        "pull_request" => {
            // Gate-side `manual_approval_signal` is unconditional on a
            // `closed+merged` delivery (legacy issue-#118 behaviour).
            // The new `github_pull_request_closed` workflow trigger
            // (#240) fans out separately and respects per-workflow
            // filters (e.g. only-merged).
            let mut events: Vec<RoutedEvent> =
                route_pull_request_closed(payload).into_iter().collect();
            let repo_owner = payload
                .pointer("/repository/owner/login")
                .and_then(Value::as_str)
                .unwrap_or("");
            let repo_name = payload
                .pointer("/repository/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !repo_owner.is_empty() && !repo_name.is_empty() {
                let candidates =
                    match crate::workflow_db::find_active_pull_request_closed_workflows(
                        &state.pool,
                        repo_owner,
                        repo_name,
                    )
                    .await
                    {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(
                                repo_owner,
                                repo_name,
                                error = %e,
                                "failed to query active pull-request-closed workflows"
                            );
                            Vec::new()
                        }
                    };
                let triggers: Vec<WorkflowTrigger> = candidates
                    .into_iter()
                    .map(|w| WorkflowTrigger {
                        id: w.id,
                        workspace_id: w.workspace_id,
                        trigger: w.trigger,
                    })
                    .collect();
                events.extend(route_pull_request_closed_workflows(payload, &triggers));
            }
            events
        }
        "workflow_run" => {
            let repo_owner = payload
                .pointer("/repository/owner/login")
                .and_then(Value::as_str)
                .unwrap_or("");
            let repo_name = payload
                .pointer("/repository/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let workflow_name = payload
                .pointer("/workflow_run/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            if repo_owner.is_empty() || repo_name.is_empty() || workflow_name.is_empty() {
                return Vec::new();
            }
            let candidates = match crate::workflow_db::find_active_workflow_run_completed_workflows(
                &state.pool,
                repo_owner,
                repo_name,
                workflow_name,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        repo_owner,
                        repo_name,
                        workflow_name,
                        error = %e,
                        "failed to query active workflow-run-completed workflows"
                    );
                    return Vec::new();
                }
            };
            let triggers: Vec<WorkflowTrigger> = candidates
                .into_iter()
                .map(|w| WorkflowTrigger {
                    id: w.id,
                    workspace_id: w.workspace_id,
                    trigger: w.trigger,
                })
                .collect();
            route_workflow_run_completed_workflows(payload, &triggers)
        }
        _ => Vec::new(),
    }
}

/// Decorate the routed events with the `(adapter_id, external_ref)`
/// dedup key the reconciliation poller also stamps. The spine partial
/// unique index on `events_ext (adapter_id, external_ref)` then
/// collapses webhook/reconciler races to one row (spec #430).
///
/// The `external_ref` is keyed on Onsager-side `project_id`, so we
/// resolve `(install, owner, repo) → project` once for the delivery
/// and reuse it. The caller passes the already-resolved
/// installation row id from `handle()` so we don't re-query the
/// `github_app_installations` table on every webhook delivery. When
/// the project can't be resolved (e.g. opt-in not completed), the
/// event is emitted *without* a dedup key — at worst a
/// reconciliation tick will write a duplicate row, which is a
/// degradation of dedup, not of correctness.
async fn decorate_routed_with_dedup(
    state: &AppState,
    payload: &Value,
    installation_id: &str,
    events: Vec<RoutedEvent>,
) -> Vec<RoutedEvent> {
    let repo_owner = payload
        .pointer("/repository/owner/login")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let repo_name = payload
        .pointer("/repository/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if repo_owner.is_empty() || repo_name.is_empty() {
        return events;
    }
    let project =
        match crate::db::find_project_for_repo(&state.pool, installation_id, repo_owner, repo_name)
            .await
        {
            Ok(Some(p)) => p,
            _ => return events,
        };

    events
        .into_iter()
        .map(|ev| match dedup_key_for(&ev, &project.id) {
            Some(external_ref) => ev.with_dedup(GITHUB_ADAPTER_ID, external_ref),
            None => ev,
        })
        .collect()
}

/// Compute the `external_ref` for one routed event under a given
/// project. Returns `None` for event kinds whose dedup key we don't
/// yet model (those continue to emit without a dedup key — a
/// reconciler race on those is currently impossible because the
/// reconciler doesn't emit them either).
fn dedup_key_for(ev: &RoutedEvent, project_id: &str) -> Option<String> {
    use onsager_spine::FactoryEventKind as K;
    match &ev.kind {
        K::TriggerFired {
            workflow_id,
            payload,
            ..
        } => {
            // Per-(resource, workflow) dedup. We don't know the resource
            // identity from the `TriggerFired` payload alone (it
            // carries `issue_number` for issue-triggers, `pr_number`
            // for PR-triggers). Sniff which is present.
            let issue_number = payload.get("issue_number").and_then(Value::as_u64);
            let pr_number = payload.get("pr_number").and_then(Value::as_u64);
            let resource = match (issue_number, pr_number) {
                (Some(n), _) => issue_external_ref(project_id, n),
                (None, Some(n)) => pr_external_ref(project_id, n),
                _ => return None,
            };
            Some(format!("{resource}:trigger:{workflow_id}"))
        }
        K::GateManualApprovalSignal { pr_number, .. } => Some(format!(
            "{}:manual_approval",
            pr_external_ref(project_id, *pr_number)
        )),
        // `gate.check_updated` is webhook-only in v1 (reconciler
        // doesn't poll `check_*`) so no race exists to dedup against.
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_install_id_returns_id_for_canonical_payload() {
        let body = Bytes::from(r#"{"installation":{"id":42},"action":"labeled"}"#);
        assert_eq!(extract_install_id_from_payload(&body), Some(42));
    }

    #[test]
    fn extract_install_id_returns_none_for_missing_installation() {
        let body = Bytes::from(r#"{"action":"labeled"}"#);
        assert_eq!(extract_install_id_from_payload(&body), None);
    }

    #[test]
    fn extract_install_id_returns_none_for_malformed_json() {
        let body = Bytes::from("not json at all");
        assert_eq!(extract_install_id_from_payload(&body), None);
    }

    #[test]
    fn extract_install_id_returns_none_when_id_is_not_integer() {
        let body = Bytes::from(r#"{"installation":{"id":"forty-two"}}"#);
        assert_eq!(extract_install_id_from_payload(&body), None);
    }
}
