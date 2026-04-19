//! Reverse proxy for the onsager-portal webhook ingress.
//!
//! Forwards `/webhooks/github` to the portal running on an internal port so
//! the Railway service exposes a single external origin. GitHub webhook
//! signatures are computed over the raw request body, so this proxy must
//! forward bytes untouched and preserve the `X-Hub-Signature-256`,
//! `X-GitHub-Event`, and `X-GitHub-Delivery` headers.

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// GitHub caps individual webhook payloads at 25 MiB; match that here so we
/// don't reject legitimate deliveries at the proxy.
const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

/// Base URL for the portal webhook server (internal, not exposed by Railway).
fn portal_base_url() -> String {
    if let Ok(url) = std::env::var("PORTAL_URL") {
        return url;
    }
    let port = std::env::var("PORTAL_PORT").unwrap_or_else(|_| "3002".to_string());
    format!("http://127.0.0.1:{port}")
}

/// Proxy handler: forward `/webhooks/github` to the portal, preserving
/// headers and raw body bytes.
pub async fn proxy(req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    let path = uri.path();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let target = format!("{}{path}{query}", portal_base_url());

    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to read portal proxy request body: {e}");
            return (StatusCode::BAD_REQUEST, "bad request body").into_response();
        }
    };

    let client = reqwest::Client::new();
    let mut upstream = client.request(method, &target);
    // Forward every incoming header — signature verification in the portal
    // depends on `X-Hub-Signature-256` + the exact body bytes, and event
    // dispatch depends on `X-GitHub-Event`. Stripping `host` avoids leaking
    // the stiglab origin into the upstream request.
    for (name, value) in headers.iter() {
        if name == axum::http::header::HOST {
            continue;
        }
        upstream = upstream.header(name, value);
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
            tracing::error!("portal proxy error: {e}");
            (
                StatusCode::BAD_GATEWAY,
                format!("portal service unavailable: {e}"),
            )
                .into_response()
        }
    }
}
