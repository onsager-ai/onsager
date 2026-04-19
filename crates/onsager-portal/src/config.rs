/// Runtime configuration shared across the webhook server and its handlers.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub database_url: String,
    /// AES-256-GCM key (hex), shared with stiglab. When `None`, webhook-secret
    /// decryption is disabled and only installations with a `NULL`
    /// `webhook_secret_cipher` (signature verification skipped) are accepted.
    /// Production deployments must always configure this.
    pub credential_key: Option<String>,
    /// Synodic gate URL (`http://host:port`). When `None`, the portal
    /// short-circuits gate calls to a synthetic `Allow` verdict — useful for
    /// local development without synodic running, but means real gates never
    /// evaluate.
    pub synodic_url: Option<String>,
    /// Optional fallback GitHub token for posting check runs / comments.
    /// Per-installation tokens (Phase 2 follow-up) are preferred.
    pub github_token: Option<String>,
}
