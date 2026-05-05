//! Reverse proxy for the onsager-portal subsystem.
//!
//! Forwards a stable set of stiglab URLs to the portal so the Railway service
//! exposes a single external origin. Used today for:
//!
//! - GitHub webhooks (`/webhooks/github`, `/api/webhooks/github`,
//!   `/api/github-app/webhook`) — payload is HMAC-signed over raw bytes,
//!   so this proxy must forward bytes untouched and preserve the
//!   `X-Hub-Signature-256`, `X-GitHub-Event`, and `X-GitHub-Delivery`
//!   headers (#222 Slice 1).
//! - Auth routes (`/api/auth/*`) — preserves `Set-Cookie` and the redirect
//!   `Location` so the OAuth dance and SSO finish round-trip without the
//!   browser noticing portal is upstream (#222 Slice 5).

use std::sync::OnceLock;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::HeaderName;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// GitHub caps individual webhook payloads at 25 MiB; match that here so we
/// don't reject legitimate deliveries at the proxy. Auth payloads are tiny
/// in comparison, so this single ceiling covers both surfaces.
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
            // 302/303 redirects (used by the OAuth callback and SSO
            // finish flows) must reach the browser unchanged — disable
            // automatic following.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build portal proxy client")
    })
}

/// Base URL for the portal HTTP server (internal, not exposed by Railway).
fn portal_base_url() -> String {
    if let Ok(url) = std::env::var("PORTAL_URL") {
        return url;
    }
    let port = std::env::var("PORTAL_PORT").unwrap_or_else(|_| "3002".to_string());
    format!("http://127.0.0.1:{port}")
}

/// Proxy handler: forward the request to portal, preserving non-hop-by-hop
/// request headers and raw body bytes; on the response, preserve every
/// non-hop-by-hop header (including the multiple `Set-Cookie` headers the
/// auth flow emits and `Location` for redirects).
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
            let upstream_headers = resp.headers().clone();
            let bytes = resp.bytes().await.unwrap_or_default();

            let mut builder = Response::builder().status(status);
            // Forward every non-hop-by-hop response header. Multi-value
            // headers like `set-cookie` (the OAuth callback emits two —
            // session cookie + state-cookie cleanup) come through as
            // separate entries on `HeaderMap::iter`, so this loop
            // preserves them.
            for (name, value) in upstream_headers.iter() {
                if HOP_BY_HOP.contains(&name.as_str()) {
                    continue;
                }
                if let (Ok(n), Ok(v)) = (
                    HeaderName::from_bytes(name.as_str().as_bytes()),
                    HeaderValue::from_bytes(value.as_bytes()),
                ) {
                    builder = builder.header(n, v);
                }
            }
            builder
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            // Keep upstream detail in logs only — webhook + auth endpoints
            // are public, and error strings can reveal internal topology.
            tracing::error!("portal proxy error: {e}");
            (StatusCode::BAD_GATEWAY, "portal service unavailable").into_response()
        }
    }
}
