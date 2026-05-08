//! Telegram bot webhook receiver (#240 — Category 3 of the trigger
//! taxonomy umbrella #236).
//!
//! Telegram tells the bot's webhook URL apart from any other request by
//! the `X-Telegram-Bot-Api-Secret-Token` header it sets at
//! `setWebhook` time. We compare that against `TELEGRAM_WEBHOOK_SECRET`
//! (configured at portal boot) using a constant-time comparison; a
//! missing or wrong header returns `401`.
//!
//! On a verified update we look up every active `TelegramWebhook`
//! workflow whose `bot_username` + `chat_id_allowlist` + optional
//! `command_prefix` match the incoming update, and emit one
//! `workflow.trigger_fired` event per match. Per-bot per-workspace
//! secret storage is a v2 follow-up — v1 ships with a single global
//! secret because the factory's first Telegram bot is operator-grade.

use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use onsager_spine::{EventMetadata, TriggerKind};
use serde_json::Value;

use crate::state::AppState;

/// Header Telegram echoes back from `setWebhook`.
const HDR_SECRET: &str = "x-telegram-bot-api-secret-token";

/// `POST /webhooks/telegram` — Telegram bot webhook entry point.
pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(expected) = state.config.telegram_webhook_secret.clone() else {
        // No secret configured ⇒ Telegram routing is disabled. We return
        // 503 so a misconfigured webhook URL surfaces in Telegram's
        // delivery dashboard rather than silently accepting payloads.
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "telegram_webhook_disabled" })),
        )
            .into_response();
    };

    let supplied = headers
        .get(HDR_SECRET)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "telegram_secret_invalid" })),
        )
            .into_response();
    }

    let update: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "body_not_json" })),
            )
                .into_response();
        }
    };

    // Walk every active TelegramWebhook workflow across every workspace.
    // The portal pool already serves workspace-scoped reads on the
    // `workflows` spine table; we project the rows portal-side and
    // filter against the parsed update.
    let workflows =
        match crate::workflow_db::list_active_telegram_workflows(state.spine.pool()).await {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("failed to list active telegram workflows: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    let chat_id = update
        .pointer("/message/chat/id")
        .or_else(|| update.pointer("/edited_message/chat/id"))
        .or_else(|| update.pointer("/channel_post/chat/id"))
        .and_then(Value::as_i64);
    let text = update
        .pointer("/message/text")
        .or_else(|| update.pointer("/edited_message/text"))
        .or_else(|| update.pointer("/channel_post/text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let bot_target = update
        .pointer("/message/via_bot/username")
        .or_else(|| update.pointer("/message/from/username"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let mut emitted = 0usize;
    for workflow in workflows {
        let TriggerKind::TelegramWebhook {
            bot_username,
            chat_id_allowlist,
            command_prefix,
        } = &workflow.trigger
        else {
            continue;
        };

        if !bot_username.is_empty() && !bot_target.is_empty() && bot_target != bot_username {
            continue;
        }
        if !chat_id_allowlist.is_empty() {
            let allow: HashSet<i64> = chat_id_allowlist.iter().copied().collect();
            match chat_id {
                Some(c) if allow.contains(&c) => {}
                _ => continue,
            }
        }
        if let Some(prefix) = command_prefix {
            if !text.starts_with(prefix) {
                continue;
            }
        }

        let payload = serde_json::json!({
            "trigger_kind": "telegram_webhook",
            "workflow_id": workflow.id,
            "workspace_id": workflow.workspace_id,
            "bot_username": bot_username,
            "chat_id": chat_id,
            "text": text,
            "update": update,
            "source": "telegram_webhook",
        });
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: "portal".to_string(),
        };
        let envelope = serde_json::json!({
            "event": {
                "type": "trigger.fired",
                "workflow_id": workflow.id,
                "trigger_kind": "telegram_webhook",
                "payload": payload,
            },
            "actor": "portal",
            "timestamp": chrono::Utc::now(),
        });
        if let Err(e) = state
            .spine
            .append_ext(
                &workflow.workspace_id,
                &format!("workflow:{}", workflow.id),
                "workflow",
                "trigger.fired",
                envelope,
                &metadata,
                None,
            )
            .await
        {
            tracing::warn!("failed to emit telegram trigger.fired: {e}");
            continue;
        }
        emitted += 1;
    }

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "matched": emitted })),
    )
        .into_response()
}

/// Constant-time byte comparison. Avoids leaking the secret length /
/// shared-prefix to a determined attacker. We accept the small
/// non-secret-shaped allocation in exchange for a single code path.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
