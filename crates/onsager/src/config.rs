//! Environment-only configuration. No CLI flags, no config file — the
//! deploy surface is a container with env vars and the dev surface is
//! `just dev` exporting the same names.

#[derive(Debug, Clone)]
pub struct Config {
    /// Postgres connection string. Required.
    pub database_url: String,
    /// Listen address. `ONSAGER_BIND`, default `127.0.0.1:3002`.
    pub bind: String,
    /// 32-byte hex AES-256-GCM key sealing credential values at rest.
    /// `ONSAGER_CREDENTIAL_KEY`; optional — credential routes 503
    /// without it.
    pub credential_key: Option<String>,
    /// Public origin (`https://...`) when served behind TLS; drives the
    /// `Secure` cookie attribute. `ONSAGER_PUBLIC_URL`, optional.
    pub public_url: Option<String>,
    /// Release-build opt-in for dev-login (`DEV_LOGIN_ENABLED=true`).
    /// Debug builds always allow it.
    pub dev_login_flag: bool,
    /// HMAC secret for the GitHub App's webhook (`GITHUB_WEBHOOK_SECRET`).
    /// Unset → the webhook route rejects everything (fail closed).
    pub github_webhook_secret: Option<String>,
    /// GitHub OAuth app (`GITHUB_CLIENT_ID` / `GITHUB_CLIENT_SECRET`).
    /// Unset → no GitHub login button.
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("ONSAGER_DATABASE_URL"))
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is required"))?;
        Ok(Self {
            database_url,
            bind: std::env::var("ONSAGER_BIND").unwrap_or_else(|_| "127.0.0.1:3002".into()),
            credential_key: std::env::var("ONSAGER_CREDENTIAL_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            public_url: std::env::var("ONSAGER_PUBLIC_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            dev_login_flag: std::env::var("DEV_LOGIN_ENABLED").is_ok_and(|v| v == "true"),
            github_webhook_secret: std::env::var("GITHUB_WEBHOOK_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            github_client_id: std::env::var("GITHUB_CLIENT_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            github_client_secret: std::env::var("GITHUB_CLIENT_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }

    /// Dev-login availability: always in debug builds, opt-in in release.
    pub fn dev_login_enabled(&self) -> bool {
        cfg!(debug_assertions) || self.dev_login_flag
    }

    /// The MCP endpoint agent sessions call back into — this process.
    /// `ONSAGER_MCP_URL` overrides (containers where the agent's view
    /// of the host differs); defaults to the bind address.
    pub fn mcp_url(&self) -> Option<String> {
        if let Ok(url) = std::env::var("ONSAGER_MCP_URL")
            && !url.is_empty()
        {
            return Some(url);
        }
        Some(format!("http://{}/mcp/messages", self.bind))
    }
}
