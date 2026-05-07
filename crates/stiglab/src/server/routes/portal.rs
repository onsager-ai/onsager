//! Catch-all reverse proxy: forwards `/api/*` to onsager-portal.
//!
//! Post-#222 Slice 6 stiglab owns only `/api/health` and `/agent/ws`.
//! Everything else under `/api/` is handled by portal. In dev, Caddy
//! routes those requests directly to portal; in the Railway single-
//! container deployment stiglab is the only process reachable from
//! outside, so this handler forwards the request over the loopback to
//! portal at `config.portal_url` (default `http://127.0.0.1:3002`).

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::state::AppState;

pub async fn proxy(State(state): State<AppState>, req: Request) -> Response {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("");
    let target = format!("{}{}", state.config.portal_url, path_and_query);

    let method = match req.method().as_str().parse::<reqwest::Method>() {
        Ok(m) => m,
        Err(_) => {
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let mut builder = state.http_client.request(method, &target);

    for (name, value) in req.headers() {
        // Drop hop-by-hop headers that must not be forwarded.
        let n = name.as_str();
        if n == "host" || n == "connection" || n == "transfer-encoding" || n == "te" {
            continue;
        }
        builder = builder.header(name, value);
    }

    let body = match axum::body::to_bytes(req.into_body(), 16 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "portal proxy: failed to read request body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    builder = builder.body(body);

    match builder.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut response_builder = axum::http::Response::builder().status(status);
            for (name, value) in resp.headers() {
                // Drop transfer-encoding — reqwest reassembles chunked responses.
                if name.as_str() == "transfer-encoding" {
                    continue;
                }
                response_builder = response_builder.header(name, value.as_bytes());
            }
            let body_bytes = resp.bytes().await.unwrap_or_default();
            response_builder
                .body(Body::from(body_bytes))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => {
            tracing::warn!(error = %e, %target, "portal proxy: upstream request failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}
