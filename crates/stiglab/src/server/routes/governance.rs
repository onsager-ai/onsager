//! Reverse proxy for the synodic governance API.
//!
//! Forwards `/api/governance/{path}` to synodic running on an internal port.
//! This lets the unified dashboard access governance data through a single
//! origin without CORS or multi-port configuration.

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Base URL for the synodic governance API (internal, not exposed by Railway).
fn synodic_base_url() -> String {
    let port = std::env::var("SYNODIC_PORT").unwrap_or_else(|_| "3001".to_string());
    std::env::var("SYNODIC_URL").unwrap_or_else(|_| format!("http://localhost:{port}"))
}

/// Proxy handler: forward `/api/governance/{path}` to synodic's `/api/{path}`.
pub async fn proxy(req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();

    // Strip the /api/governance prefix to get the synodic-relative path.
    let path = uri.path().strip_prefix("/api/governance").unwrap_or("");
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let target = format!("{}/api{path}{query}", synodic_base_url());

    // Forward content-type header if present.
    let content_type = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to read proxy request body: {e}");
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
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let bytes = resp.bytes().await.unwrap_or_default();

            Response::builder()
                .status(status)
                .header("content-type", content_type)
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
