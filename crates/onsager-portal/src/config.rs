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
    /// Public origin under which portal-served routes are reached
    /// (e.g. `https://app.onsager.ai`). Used to construct the OAuth
    /// callback URL and the cookie `Secure` flag. When `None`, callbacks
    /// fall back to `http://localhost:<bind-port>`.
    pub public_url: Option<String>,
    /// GitHub OAuth client id (owner mode). Pair with `github_client_secret`.
    pub github_client_id: Option<String>,
    /// GitHub OAuth client secret (owner mode).
    pub github_client_secret: Option<String>,
    /// Cross-environment SSO — owner-side HMAC secret used to sign the
    /// `state` envelope carried through GitHub. Required when serving
    /// preview environments via delegation.
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
    /// Enable the `/api/auth/dev-login` route in release builds. Always
    /// enabled in debug builds. Set `DEV_LOGIN_ENABLED=true` on Railway
    /// preview environments that need a login path without GitHub OAuth.
    pub dev_login_enabled: bool,
    /// Shared secret echoed back by Telegram in the
    /// `X-Telegram-Bot-Api-Secret-Token` header on every webhook
    /// delivery. When `None` the `/webhooks/telegram` route returns
    /// `503` — the trigger kind is registered but the receiver is
    /// disabled. Set via `TELEGRAM_WEBHOOK_SECRET` (#240).
    pub telegram_webhook_secret: Option<String>,
    /// Per-workspace per-month USD spend cap for `propose_remediation`
    /// AI calls (#312). When a workspace's spend in the current
    /// calendar month reaches this value, further calls short-circuit
    /// to the stub envelope with `stub_reason = "budget_exceeded"`
    /// and a `details` object carrying `{spent_usd, cap_usd}`.
    /// Defaults to a conservative double-digit dollar value; override
    /// via `PORTAL_REMEDIATION_MONTHLY_CAP_USD`.
    pub remediation_monthly_cap_usd: f64,
    /// Deployment descriptor surfaced by `GET /api/build-info`. Set to
    /// `cloud` for the SaaS deploy; left unset for local OSS runs and
    /// self-hosted enterprise deploys that want the OSS chrome.
    /// Configured via `ONSAGER_DEPLOYMENT`.
    pub deployment: Option<String>,
    /// Pinned workflow_id for the public Dogfood showcase (spec #407).
    /// When `None`, `GET /api/showcase/dogfood` short-circuits to
    /// `{ enabled: false }`. The Cloud deploy sets this to the spine
    /// workflow row that models Onsager-managing-Onsager; OSS
    /// self-hosters can point it at their own dogfood workflow.
    /// Configured via `SHOWCASE_DOGFOOD_WORKFLOW_ID`.
    pub showcase_dogfood_workflow_id: Option<String>,
}

/// Default monthly per-workspace cap for `propose_remediation` AI
/// spend. Sized to absorb a steady stream of failed-run analyses at
/// Sonnet pricing without an operator surprise; revisit when real
/// usage data exists.
pub const DEFAULT_REMEDIATION_MONTHLY_CAP_USD: f64 = 10.0;

impl Config {
    /// Classify the process's role in the SSO flow. `None` means no GitHub
    /// OAuth is configured. The only remaining login path is then dev-login
    /// (debug builds, or release builds with `DEV_LOGIN_ENABLED=true`); if
    /// neither applies, there is no login path at all.
    pub fn sso_mode(&self) -> Option<crate::sso::SsoMode> {
        let has_github = self.github_client_id.is_some() && self.github_client_secret.is_some();
        let has_owner_secrets =
            self.sso_state_secret.is_some() && self.sso_exchange_secret.is_some();
        let has_relying =
            self.sso_auth_domain.is_some() && self.sso_exchange_secret.is_some() && !has_github;

        if has_github {
            let delegate_enabled = has_owner_secrets && !self.sso_return_host_allowlist.is_empty();
            Some(crate::sso::SsoMode::Owner { delegate_enabled })
        } else if has_relying {
            Some(crate::sso::SsoMode::Relying)
        } else {
            None
        }
    }

    /// Fail fast on ambiguous SSO configuration. Called at startup so
    /// misconfigured deploys never even begin serving traffic.
    pub fn assert_sso_consistent(&self) {
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

    fn base_config() -> Config {
        Config {
            bind: "0.0.0.0:3002".to_string(),
            database_url: "postgres://test".to_string(),
            credential_key: None,
            synodic_url: None,
            github_token: None,
            public_url: None,
            github_client_id: None,
            github_client_secret: None,
            sso_state_secret: None,
            sso_exchange_secret: None,
            sso_return_host_allowlist: Vec::new(),
            sso_auth_domain: None,
            dev_login_enabled: false,
            telegram_webhook_secret: None,
            remediation_monthly_cap_usd: DEFAULT_REMEDIATION_MONTHLY_CAP_USD,
            deployment: None,
            showcase_dogfood_workflow_id: None,
        }
    }

    fn make_config(client_id: Option<&str>, client_secret: Option<&str>) -> Config {
        Config {
            github_client_id: client_id.map(|s| s.to_string()),
            github_client_secret: client_secret.map(|s| s.to_string()),
            ..base_config()
        }
    }

    #[test]
    fn sso_mode_none_when_no_github_or_relying() {
        let config = make_config(None, None);
        assert!(config.sso_mode().is_none());
        let config = make_config(None, Some("secret"));
        assert!(config.sso_mode().is_none());
        let config = make_config(Some("id"), None);
        assert!(config.sso_mode().is_none());
    }

    #[test]
    fn owner_mode_without_delegation() {
        let c = make_config(Some("id"), Some("secret"));
        assert_eq!(
            c.sso_mode(),
            Some(crate::sso::SsoMode::Owner {
                delegate_enabled: false
            })
        );
    }

    #[test]
    fn owner_mode_with_delegation() {
        let c = Config {
            github_client_id: Some("id".into()),
            github_client_secret: Some("secret".into()),
            sso_state_secret: Some("state-secret".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            sso_return_host_allowlist: vec!["*.preview.onsager.ai".into()],
            ..base_config()
        };
        assert_eq!(
            c.sso_mode(),
            Some(crate::sso::SsoMode::Owner {
                delegate_enabled: true
            })
        );
    }

    #[test]
    fn owner_mode_delegation_disabled_when_allowlist_empty() {
        let c = Config {
            github_client_id: Some("id".into()),
            github_client_secret: Some("secret".into()),
            sso_state_secret: Some("state-secret".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            sso_return_host_allowlist: vec![],
            ..base_config()
        };
        assert_eq!(
            c.sso_mode(),
            Some(crate::sso::SsoMode::Owner {
                delegate_enabled: false
            })
        );
    }

    #[test]
    fn relying_mode_requires_auth_domain_and_exchange_secret() {
        let c = Config {
            sso_auth_domain: Some("https://app.onsager.ai".into()),
            sso_exchange_secret: Some("exchange-secret".into()),
            ..base_config()
        };
        assert_eq!(c.sso_mode(), Some(crate::sso::SsoMode::Relying));
    }

    #[test]
    #[should_panic(expected = "mutually exclusive")]
    fn assert_panics_when_both_owner_and_relying() {
        let c = Config {
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
        let c = Config {
            sso_state_secret: Some("state-secret".into()),
            ..base_config()
        };
        c.assert_sso_consistent();
    }

    #[test]
    #[should_panic(expected = "SSO_EXCHANGE_SECRET")]
    fn assert_panics_when_auth_domain_without_exchange_secret() {
        let c = Config {
            sso_auth_domain: Some("https://app.onsager.ai".into()),
            ..base_config()
        };
        c.assert_sso_consistent();
    }
}
