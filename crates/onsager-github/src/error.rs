use thiserror::Error;

/// Errors returned by `onsager-github` API surfaces.
///
/// `NotConfigured` is the load-bearing variant — callers treat it as a
/// feature flag (no GitHub App, no PAT, no work to do) rather than a
/// hard error. Every other variant is an actual upstream / transport
/// failure.
#[derive(Debug, Error)]
pub enum GithubError {
    /// No credential could be resolved (no env vars, no PAT in DB).
    /// Callers should silently skip whatever they were going to do.
    #[error("github credentials are not configured")]
    NotConfigured,

    /// The configured credential is malformed (bad PEM, empty token).
    #[error("github credential is invalid: {0}")]
    InvalidCredential(String),

    /// GitHub returned a non-2xx response.
    #[error("github api error ({status}): {body}")]
    Api { status: u16, body: String },

    /// JSON / response decoding failed.
    #[error("github response decode error: {0}")]
    Decode(String),

    /// Underlying transport failure (DNS, TLS, timeout, …).
    #[error("github transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// JWT signing failure (App-mode only).
    #[error("github jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    /// Anything else worth surfacing without losing context.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl GithubError {
    /// Build an `Api` variant from a `reqwest::Response` after a
    /// non-success status — pulls the body for the breadcrumb.
    pub async fn from_response(resp: reqwest::Response) -> Self {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        GithubError::Api { status, body }
    }
}
