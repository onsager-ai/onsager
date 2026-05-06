//! Governance API proxy.
//!
//! Forwards `/api/governance/{path}` to synodic's `/api/{path}`, making
//! portal the single external HTTP boundary for the dashboard. Portal is
//! the edge subsystem — no `seam-allow` annotation needed here; the lint
//! only checks factory subsystems (forge / stiglab / synodic / ising).
//!
//! The proxy is intentionally transparent: status codes, content-type,
//! and body bytes pass through unchanged so synodic's error shapes reach
//! the dashboard unmodified.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Forward `/api/governance/{path}` to synodic's `/api/{path}`.
///
/// Returns `503 Service Unavailable` when `SYNODIC_URL` is not configured
/// (dev setups without synodic running) so the dashboard can surface a
/// graceful "governance unavailable" state instead of a generic proxy error.
pub async fn proxy(State(state): State<AppState>, req: Request) -> Response {
    let Some(ref synodic_base) = state.config.synodic_url else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "governance service not configured",
        )
            .into_response();
    };
    let base = synodic_base.trim_end_matches('/');

    let method = req.method().clone();
    let uri = req.uri().clone();

    let path = uri.path().strip_prefix("/api/governance").unwrap_or("");
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let target = format!("{base}/api{path}{query}");

    let content_type = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("governance proxy: failed to read request body: {e}");
            return (StatusCode::BAD_REQUEST, "bad request body").into_response();
        }
    };

    let client = reqwest::Client::new();
    let mut upstream = client.request(method, &target);
    if let Some(ct) = content_type {
        upstream = upstream.header("content-type", ct);
    }
    if !body_bytes.is_empty() {
        upstream = upstream.body(body_bytes);
    }

    match upstream.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ct = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let bytes = resp.bytes().await.unwrap_or_default();
            Response::builder()
                .status(status)
                .header("content-type", ct)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            tracing::error!("governance proxy error: {e}");
            (
                StatusCode::BAD_GATEWAY,
                format!("governance service unavailable: {e}"),
            )
                .into_response()
        }
    }
}
