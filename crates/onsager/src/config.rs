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
    /// Shared secret a fleet machine presents to dial in (ADR 0030).
    /// `ONSAGER_MACHINE_TOKEN`; unset → `/api/fleet/connect` rejects every
    /// machine (fail closed), and the system runs single-process.
    pub machine_token: Option<String>,
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

/// Resolve the MCP callback URL (ADR 0030): explicit override wins, else
/// the Railway public domain (so remote workers' agents can reach it),
/// else the container-internal bind address.
fn resolve_mcp_url(explicit: Option<String>, railway_domain: Option<String>, bind: &str) -> String {
    if let Some(url) = explicit {
        return url;
    }
    if let Some(domain) = railway_domain {
        return format!("https://{domain}/mcp/messages");
    }
    format!("http://{bind}/mcp/messages")
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
            machine_token: std::env::var("ONSAGER_MACHINE_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }

    /// Dev-login availability: always in debug builds, opt-in in release.
    pub fn dev_login_enabled(&self) -> bool {
        cfg!(debug_assertions) || self.dev_login_flag
    }

    /// The MCP endpoint agent sessions call back into. Explicit
    /// `ONSAGER_MCP_URL` wins; else, on Railway, derive the public URL
    /// from `RAILWAY_PUBLIC_DOMAIN` so a *remote* worker's agent can phone
    /// home (the bind default is container-internal — unreachable off-box,
    /// ADR 0030); else fall back to the bind address.
    pub fn mcp_url(&self) -> Option<String> {
        Some(resolve_mcp_url(
            std::env::var("ONSAGER_MCP_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            std::env::var("RAILWAY_PUBLIC_DOMAIN")
                .ok()
                .filter(|s| !s.is_empty()),
            &self.bind,
        ))
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
            machine_token: None,
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
    fn mcp_url_explicit_wins() {
        assert_eq!(
            resolve_mcp_url(
                Some("https://x/mcp/messages".into()),
                Some("d".into()),
                "127.0.0.1:1"
            ),
            "https://x/mcp/messages"
        );
    }

    #[test]
    fn mcp_url_derives_from_railway_domain() {
        assert_eq!(
            resolve_mcp_url(None, Some("pr-5.up.railway.app".into()), "0.0.0.0:8080"),
            "https://pr-5.up.railway.app/mcp/messages"
        );
    }

    #[test]
    fn mcp_url_falls_back_to_bind() {
        assert_eq!(
            resolve_mcp_url(None, None, "127.0.0.1:3002"),
            "http://127.0.0.1:3002/mcp/messages"
        );
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
