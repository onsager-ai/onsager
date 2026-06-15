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
    /// Concurrency cap on in-flight agent sessions (ADR 0030 MVP stance):
    /// the one cheap knob that bounds the single-process ceiling before
    /// the fleet exists. `ONSAGER_MAX_CONCURRENT_RUNS`, default 4, floored
    /// at 1 (a zero cap would wedge every fire forever).
    pub max_concurrent_runs: usize,
}

/// Default session concurrency when the env var is unset or unparseable.
const DEFAULT_MAX_CONCURRENT_RUNS: usize = 4;

/// Parse `ONSAGER_MAX_CONCURRENT_RUNS`: unset/garbage → default; an
/// explicit `0` is floored to 1 so the semaphore can never deadlock.
fn parse_max_concurrent_runs(raw: Option<String>) -> usize {
    raw.and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENT_RUNS)
        .max(1)
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
            max_concurrent_runs: parse_max_concurrent_runs(
                std::env::var("ONSAGER_MAX_CONCURRENT_RUNS").ok(),
            ),
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

#[cfg(test)]
impl Config {
    /// A minimal config for tests that need an `AppState` without reading
    /// the process environment.
    pub(crate) fn for_test() -> Self {
        Self {
            database_url: "postgres://unused".into(),
            bind: "127.0.0.1:0".into(),
            credential_key: None,
            public_url: None,
            dev_login_flag: false,
            github_webhook_secret: None,
            github_client_id: None,
            github_client_secret: None,
            max_concurrent_runs: DEFAULT_MAX_CONCURRENT_RUNS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_concurrent_runs_defaults_when_unset() {
        assert_eq!(parse_max_concurrent_runs(None), DEFAULT_MAX_CONCURRENT_RUNS);
    }

    #[test]
    fn max_concurrent_runs_parses_a_valid_value() {
        assert_eq!(parse_max_concurrent_runs(Some("9".into())), 9);
    }

    #[test]
    fn max_concurrent_runs_floors_zero_to_one() {
        // A zero-permit semaphore would wedge every fire forever.
        assert_eq!(parse_max_concurrent_runs(Some("0".into())), 1);
    }

    #[test]
    fn max_concurrent_runs_falls_back_on_garbage() {
        assert_eq!(
            parse_max_concurrent_runs(Some("lots".into())),
            DEFAULT_MAX_CONCURRENT_RUNS
        );
    }
}
