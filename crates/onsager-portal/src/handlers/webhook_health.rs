//! Webhook delivery health surface (spec #120 item 3).
//!
//! Surfaces "is this tenant's App webhook actually working?" to the
//! dashboard so a misconfigured / silently-dropping App is a glance
//! instead of a log-dive. Returns one row per installation belonging to
//! the workspace, summarising the last K (= 30) deliveries the App
//! emitted across *all* installations and filtered to this workspace.
//!
//! Backed by the App-scoped `GET /app/hook/deliveries` endpoint (single
//! URL across every installation), cached for `PORTAL_PROXY_CACHE_TTL_SECS`
//! to keep the dashboard usable without exhausting the App's 5000/hr
//! REST budget on render storms.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use onsager_github::api::app as gh_app;
use schemars::JsonSchema;
use serde::Serialize;

use crate::auth::AuthUser;
use crate::handlers::installations::require_workspace_access;
use crate::installation_db;
use crate::state::AppState;

/// Page size we ask GitHub for. GitHub's max is 100; 30 mirrors the API
/// default and is the K the spec settled on per the human decision in
/// #120's Alignment section.
const DELIVERIES_PAGE: u32 = 30;

/// Cache key for the raw `GET /app/hook/deliveries` response. App-scoped,
/// so the cache lives across every workspace served by this replica.
const CACHE_KEY: &str = "app_webhook_deliveries:per_page=30";

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InstallationDeliveryHealth {
    /// Numeric GitHub installation ID. Workflow rows store this same value
    /// in `install_id` so the dashboard can join by it without an extra
    /// fetch.
    pub install_id: i64,
    /// How many of the K=30 most recent deliveries belong to this
    /// installation. Zero is meaningful — it implies GitHub has not
    /// delivered anything to this installation in the recent window.
    pub checked: usize,
    /// Number of non-2xx deliveries within `checked`.
    pub non_2xx: usize,
    pub last_delivered_at: Option<DateTime<Utc>>,
    pub last_non_2xx_at: Option<DateTime<Utc>>,
    pub last_non_2xx_status_code: Option<i32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkspaceDeliveryHealthResponse {
    /// One row per installation registered to this workspace. Always
    /// includes every workspace installation, even ones with `checked = 0`,
    /// so the dashboard can distinguish "no recent deliveries" from
    /// "endpoint not implemented".
    pub installations: Vec<InstallationDeliveryHealth>,
    /// Total deliveries inspected (≤ `DELIVERIES_PAGE`). Lets the
    /// dashboard caption the warning ("0 of 30 recent deliveries
    /// succeeded").
    pub window: usize,
}

/// GET /api/workspaces/:workspace_id/github-installations/webhook-deliveries-health
///
/// Per spec #120, surfaces non-2xx delivery counts so a misconfigured App
/// webhook URL self-diagnoses on the workflow card.
pub async fn workspace_webhook_deliveries_health(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }

    let installs =
        match installation_db::list_installations_for_workspace(&state.pool, &workspace_id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("failed to list installations for workspace: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        };

    if installs.is_empty() {
        return Json(WorkspaceDeliveryHealthResponse {
            installations: Vec::new(),
            window: 0,
        })
        .into_response();
    }

    let deliveries = match fetch_deliveries_cached(&state).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            // App not configured — the dashboard treats this the same as
            // "no recent deliveries" so the warning doesn't fire for OSS
            // installs that haven't wired up the App yet.
            return Json(WorkspaceDeliveryHealthResponse {
                installations: installs
                    .iter()
                    .map(|i| InstallationDeliveryHealth {
                        install_id: i.install_id,
                        checked: 0,
                        non_2xx: 0,
                        last_delivered_at: None,
                        last_non_2xx_at: None,
                        last_non_2xx_status_code: None,
                    })
                    .collect(),
                window: 0,
            })
            .into_response();
        }
        Err(e) => {
            tracing::warn!("list_app_webhook_deliveries failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub API request failed" })),
            )
                .into_response();
        }
    };

    let installations = installs
        .iter()
        .map(|inst| summarise_for_install(inst.install_id, &deliveries))
        .collect();

    Json(WorkspaceDeliveryHealthResponse {
        installations,
        window: deliveries.len(),
    })
    .into_response()
}

/// Fetch the App-scoped deliveries list, served from the proxy cache when
/// hot. `Ok(None)` means the App isn't configured on this server (no
/// `GITHUB_APP_*` env). `Err` is bubbled up so the handler can surface a
/// 502 instead of pretending everything is fine.
async fn fetch_deliveries_cached(
    state: &AppState,
) -> anyhow::Result<Option<Vec<gh_app::AppWebhookDelivery>>> {
    if let Some(cached) = state.proxy_cache.get(CACHE_KEY)
        && let Ok(parsed) = serde_json::from_value::<Vec<gh_app::AppWebhookDelivery>>(cached)
    {
        return Ok(Some(parsed));
    }

    let Some(cfg) = gh_app::AppConfig::from_env() else {
        return Ok(None);
    };
    let jwt = gh_app::mint_app_jwt(&cfg)?;
    let deliveries = gh_app::list_app_webhook_deliveries(&jwt, DELIVERIES_PAGE).await?;
    if let Ok(value) = serde_json::to_value(&deliveries) {
        state.proxy_cache.put(CACHE_KEY.to_string(), value);
    }
    Ok(Some(deliveries))
}

fn summarise_for_install(
    install_id: i64,
    deliveries: &[gh_app::AppWebhookDelivery],
) -> InstallationDeliveryHealth {
    let mut checked = 0usize;
    let mut non_2xx = 0usize;
    let mut last_delivered_at: Option<DateTime<Utc>> = None;
    let mut last_non_2xx_at: Option<DateTime<Utc>> = None;
    let mut last_non_2xx_status_code: Option<i32> = None;
    for d in deliveries {
        if d.installation_id != Some(install_id) {
            continue;
        }
        checked += 1;
        if last_delivered_at.is_none_or(|prev| d.delivered_at > prev) {
            last_delivered_at = Some(d.delivered_at);
        }
        let ok = (200..300).contains(&d.status_code);
        if !ok {
            non_2xx += 1;
            if last_non_2xx_at.is_none_or(|prev| d.delivered_at > prev) {
                last_non_2xx_at = Some(d.delivered_at);
                last_non_2xx_status_code = Some(d.status_code);
            }
        }
    }
    InstallationDeliveryHealth {
        install_id,
        checked,
        non_2xx,
        last_delivered_at,
        last_non_2xx_at,
        last_non_2xx_status_code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn delivery(install_id: i64, status_code: i32, secs: i64) -> gh_app::AppWebhookDelivery {
        gh_app::AppWebhookDelivery {
            id: secs,
            guid: format!("guid-{secs}"),
            delivered_at: Utc.timestamp_opt(secs, 0).unwrap(),
            redelivery: false,
            status: if (200..300).contains(&status_code) {
                "OK".into()
            } else {
                "Internal Server Error".into()
            },
            status_code,
            event: "issues".into(),
            action: Some("labeled".into()),
            installation_id: Some(install_id),
        }
    }

    #[test]
    fn summarise_filters_by_install_and_counts_non_2xx() {
        let deliveries = vec![
            delivery(42, 200, 100),
            delivery(99, 500, 110), // wrong install — ignored
            delivery(42, 500, 120),
            delivery(42, 404, 130),
            delivery(42, 202, 140),
        ];
        let h = summarise_for_install(42, &deliveries);
        assert_eq!(h.checked, 4);
        assert_eq!(h.non_2xx, 2);
        assert_eq!(h.last_non_2xx_status_code, Some(404));
        assert_eq!(
            h.last_delivered_at,
            Some(Utc.timestamp_opt(140, 0).unwrap())
        );
        assert_eq!(h.last_non_2xx_at, Some(Utc.timestamp_opt(130, 0).unwrap()));
    }

    #[test]
    fn summarise_returns_zero_when_no_matching_deliveries() {
        let deliveries = vec![delivery(99, 200, 100), delivery(99, 500, 110)];
        let h = summarise_for_install(42, &deliveries);
        assert_eq!(h.checked, 0);
        assert_eq!(h.non_2xx, 0);
        assert!(h.last_delivered_at.is_none());
        assert!(h.last_non_2xx_at.is_none());
        assert!(h.last_non_2xx_status_code.is_none());
    }

    #[test]
    fn summarise_treats_full_2xx_range_as_ok() {
        let deliveries = vec![
            delivery(42, 200, 100),
            delivery(42, 201, 110),
            delivery(42, 202, 120),
            delivery(42, 204, 130),
            delivery(42, 299, 140),
        ];
        let h = summarise_for_install(42, &deliveries);
        assert_eq!(h.checked, 5);
        assert_eq!(h.non_2xx, 0);
    }
}
