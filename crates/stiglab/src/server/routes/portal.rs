//! Reverse proxy for the onsager-portal webhook ingress.
//!
//! Forwards `/webhooks/github` to the portal running on an internal port so
//! the Railway service exposes a single external origin. GitHub webhook
//! signatures are computed over the raw request body, so this proxy must
//! forward bytes untouched and preserve the `X-Hub-Signature-256`,
//! `X-GitHub-Event`, and `X-GitHub-Delivery` headers.

use std::sync::OnceLock;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// GitHub caps individual webhook payloads at 25 MiB; match that here so we
/// don't reject legitimate deliveries at the proxy.
const MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

/// Hop-by-hop headers per RFC 7230 §6.1 that must not be forwarded by a
/// proxy. `content-length` and `host` are also stripped — reqwest derives
/// both from the body and target URL.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "host",
];

/// Process-wide `reqwest::Client` — pooled connections + bounded timeouts
/// so a stalled portal can't tie up stiglab request capacity indefinitely.
fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build portal proxy client")
    })
}

/// Base URL for the portal webhook server (internal, not exposed by Railway).
fn portal_base_url() -> String {
    if let Ok(url) = std::env::var("PORTAL_URL") {
        return url;
    }
    let port = std::env::var("PORTAL_PORT").unwrap_or_else(|_| "3002".to_string());
    format!("http://127.0.0.1:{port}")
}

/// Proxy handler: forward `/webhooks/github` to the portal, preserving
/// non-hop-by-hop headers and raw body bytes.
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

    let mut upstream = shared_client().request(method, &target);
    // HeaderName is normalized to lowercase, so a direct `.contains` works.
    // Signature verification depends on the GitHub headers falling through;
    // event dispatch depends on `x-github-event`.
    for (name, value) in headers.iter() {
        if HOP_BY_HOP.contains(&name.as_str()) {
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
            // Keep upstream detail in logs only — the webhook endpoint is
            // public, and error strings can reveal internal topology.
            tracing::error!("portal proxy error: {e}");
            (StatusCode::BAD_GATEWAY, "portal service unavailable").into_response()
        }
    }
}
