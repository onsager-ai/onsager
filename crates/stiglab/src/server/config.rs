use std::env;

use crate::server::sso::{parse_host_allowlist, SsoMode};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub static_dir: Option<String>,
    pub cors_origin: Option<String>,
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    pub credential_key: Option<String>,
    pub public_url: Option<String>,
    /// Cross-environment SSO — owner side. `Some` on the prod deployment
    /// that owns the GitHub OAuth app and is willing to serve preview envs.
    pub sso_state_secret: Option<String>,
    /// Back-channel secret. Shared between owner and relying parties.
    /// * Owner: bearer required on `/api/auth/sso/redeem`.
    /// * Relying: bearer sent on outbound redeem calls.
    pub sso_exchange_secret: Option<String>,
    /// Allowlist of hosts the owner will redirect back to. Entries take the
    /// forms `*.subdomain.example.com` (strict-subdomain match) or
    /// `host.example.com` (exact match). Non-matching `return_to` is
    /// rejected at the start of the OAuth flow.
    pub sso_return_host_allowlist: Vec<String>,
    /// Cross-environment SSO — relying side. When set, `/api/auth/github`
    /// redirects here instead of talking to GitHub directly.
    pub sso_auth_domain: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let host = env::var("STIGLAB_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .or_else(|| env::var("STIGLAB_PORT").ok().and_then(|p| p.parse().ok()))
            .unwrap_or(3000);
        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://./data/stiglab.db".to_string());
        let static_dir = env::var("STIGLAB_STATIC_DIR").ok();
        let cors_origin = env::var("STIGLAB_CORS_ORIGIN").ok();
        let github_client_id = env::var("GITHUB_CLIENT_ID").ok().filter(|s| !s.is_empty());
        let github_client_secret = env::var("GITHUB_CLIENT_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let credential_key = env::var("STIGLAB_CREDENTIAL_KEY").ok();
        let public_url = env::var("STIGLAB_PUBLIC_URL").ok();
        let sso_state_secret = env::var("SSO_STATE_SECRET").ok().filter(|s| !s.is_empty());
        let sso_exchange_secret = env::var("SSO_EXCHANGE_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let sso_return_host_allowlist = env::var("SSO_RETURN_HOST_ALLOWLIST")
            .map(|raw| parse_host_allowlist(&raw))
            .unwrap_or_default();
        let sso_auth_domain = env::var("SSO_AUTH_DOMAIN")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string());

        let config = ServerConfig {
            host,
            port,
            database_url,
            static_dir,
            cors_origin,
            github_client_id,
            github_client_secret,
            credential_key,
            public_url,
            sso_state_secret,
            sso_exchange_secret,
            sso_return_host_allowlist,
            sso_auth_domain,
        };

        config.assert_sso_consistent();
        config
    }

    /// Returns true if GitHub OAuth is configured (both client ID and secret are set).
    pub fn auth_enabled(&self) -> bool {
        !matches!(self.sso_mode(), SsoMode::Disabled)
    }

    /// Classify the process's role in the SSO flow.
    pub fn sso_mode(&self) -> SsoMode {
        let has_github = self.github_client_id.is_some() && self.github_client_secret.is_some();
        let has_owner_secrets =
            self.sso_state_secret.is_some() && self.sso_exchange_secret.is_some();
        let has_relying =
            self.sso_auth_domain.is_some() && self.sso_exchange_secret.is_some() && !has_github;

        if has_github {
            let delegate_enabled = has_owner_secrets && !self.sso_return_host_allowlist.is_empty();
            SsoMode::Owner { delegate_enabled }
        } else if has_relying {
            SsoMode::Relying
        } else {
            SsoMode::Disabled
        }
    }

    /// Fail fast on ambiguous SSO configuration. Called at startup so
    /// misconfigured deploys never even begin serving traffic.
    fn assert_sso_consistent(&self) {
        let has_github = self.github_client_id.is_some() || self.github_client_secret.is_some();
        if has_github && self.sso_auth_domain.is_some() {
            panic!(
                "invalid SSO config: both GITHUB_CLIENT_ID/SECRET (owner-mode) and \
                 SSO_AUTH_DOMAIN (relying-mode) are set — these are mutually exclusive"
            );
        }

        if self.sso_state_secret.is_some() && !has_github {
            panic!(
                "invalid SSO config: SSO_STATE_SECRET is set but GITHUB_CLIENT_ID/SECRET \
                 are not — the state secret is only meaningful on the owner"
            );
        }

        if !self.sso_return_host_allowlist.is_empty() && !has_github {
            panic!(
                "invalid SSO config: SSO_RETURN_HOST_ALLOWLIST is set but \
                 GITHUB_CLIENT_ID/SECRET are not — the allowlist is only meaningful on \
                 the owner"
            );
        }

        if self.sso_auth_domain.is_some() && self.sso_exchange_secret.is_none() {
            panic!(
                "invalid SSO config: SSO_AUTH_DOMAIN is set but SSO_EXCHANGE_SECRET is \
                 not — the relying party cannot authenticate to the owner"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> ServerConfig {
        ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
            database_url: "sqlite://test.db".to_string(),
            static_dir: None,
            cors_origin: None,
            github_client_id: None,
            github_client_secret: None,
            credential_key: None,
            public_url: None,
            sso_state_secret: None,
            sso_exchange_secret: None,
            sso_return_host_allowlist: Vec::new(),
            sso_auth_domain: None,
        }
    }

    fn make_config(client_id: Option<&str>, client_secret: Option<&str>) -> ServerConfig {
        ServerConfig {
            github_client_id: client_id.map(|s| s.to_string()),
            github_client_secret: client_secret.map(|s| s.to_string()),
            ..base_config()
        }
    }

    #[test]
    fn auth_enabled_when_both_set() {
        let config = make_config(Some("id"), Some("secret"));
        assert!(config.auth_enabled());
    }

    #[test]
    fn auth_disabled_when_id_missing() {
        let config = make_config(None, Some("secret"));
        assert!(!config.auth_enabled());
    }

    #[test]
    fn auth_disabled_when_secret_missing() {
        let config = make_config(Some("id"), None);
        assert!(!config.auth_enabled());
    }

    #[test]
    fn auth_disabled_when_both_missing() {
        let config = make_config(None, None);
        assert!(!config.auth_enabled());
    }

    #[test]
    fn owner_mode_without_delegation() {
        let c = make_config(Some("id"), Some("secret"));
        assert_eq!(
            c.sso_mode(),
            SsoMode::Owner {
                delegate_enabled: false
            }
        );
    }

    #[test]
    fn owner_mode_with_delegation() {
        let c = ServerConfig {
            github_client_id: Some("id".into()),
            github_client_secret: Some("secret".into()),
            sso_state_secret: Some("state-secret".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            sso_return_host_allowlist: vec!["*.preview.onsager.ai".into()],
            ..base_config()
        };
        assert_eq!(
            c.sso_mode(),
            SsoMode::Owner {
                delegate_enabled: true
            }
        );
    }

    #[test]
    fn owner_mode_delegation_disabled_when_allowlist_empty() {
        let c = ServerConfig {
            github_client_id: Some("id".into()),
            github_client_secret: Some("secret".into()),
            sso_state_secret: Some("state-secret".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            sso_return_host_allowlist: vec![],
            ..base_config()
        };
        assert_eq!(
            c.sso_mode(),
            SsoMode::Owner {
                delegate_enabled: false
            }
        );
    }

    #[test]
    fn relying_mode_requires_auth_domain_and_exchange_secret() {
        let c = ServerConfig {
            sso_auth_domain: Some("https://app.onsager.ai".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            ..base_config()
        };
        assert_eq!(c.sso_mode(), SsoMode::Relying);
    }

    #[test]
    #[should_panic(expected = "mutually exclusive")]
    fn assert_panics_when_both_owner_and_relying() {
        let c = ServerConfig {
            github_client_id: Some("id".into()),
            github_client_secret: Some("secret".into()),
            sso_auth_domain: Some("https://other.example".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            ..base_config()
        };
        c.assert_sso_consistent();
    }

    #[test]
    #[should_panic(expected = "SSO_STATE_SECRET")]
    fn assert_panics_when_state_secret_without_github_creds() {
        let c = ServerConfig {
            sso_state_secret: Some("state-secret".into()),
            ..base_config()
        };
        c.assert_sso_consistent();
    }

    #[test]
    #[should_panic(expected = "SSO_EXCHANGE_SECRET")]
    fn assert_panics_when_auth_domain_without_exchange_secret() {
        let c = ServerConfig {
            sso_auth_domain: Some("https://app.onsager.ai".into()),
            ..base_config()
        };
        c.assert_sso_consistent();
    }
}
