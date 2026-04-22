//! GitHub App helpers — JWT minting, installation access tokens, and
//! thin wrappers around the App-scoped REST endpoints used by Phase 0
//! onboarding (closes the remaining items on issue #59).
//!
//! Configured via env:
//! - `GITHUB_APP_ID` — numeric App ID.
//! - `GITHUB_APP_SLUG` — App slug (used to build the install URL).
//! - `GITHUB_APP_PRIVATE_KEY` — PEM-encoded RSA private key (may be
//!   base64-encoded to survive env-var transport).
//!
//! When any of these are missing the App flow is disabled — callers fall
//! back to the manual-entry path that was in place since #63. That means
//! dev stacks without a registered App keep working unchanged.
//!
//! All network paths are best-effort: a failure fetching `default_branch`
//! or `list_repositories` never blocks onboarding. The backend logs and
//! falls back to the explicit user-supplied value (or `"main"`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use crate::core::GitHubAccountType;

/// Config resolved from env. `None` when the App flow is not configured.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_id: i64,
    pub slug: String,
    pub private_key_pem: String,
}

impl AppConfig {
    pub fn from_env() -> Option<Self> {
        let app_id = std::env::var("GITHUB_APP_ID").ok()?.parse::<i64>().ok()?;
        let slug = std::env::var("GITHUB_APP_SLUG").ok()?;
        let raw_key = std::env::var("GITHUB_APP_PRIVATE_KEY").ok()?;
        if slug.trim().is_empty() || raw_key.trim().is_empty() {
            return None;
        }
        let private_key_pem = normalize_pem(&raw_key);
        Some(Self {
            app_id,
            slug,
            private_key_pem,
        })
    }
}

/// Accept either a raw PEM with literal newlines or a base64-wrapped blob
/// (commonly how Railway / dotenv files carry multiline secrets). Also
/// tolerates escaped `\n` sequences from shell-quoted env values.
fn normalize_pem(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains("-----BEGIN") {
        return trimmed.replace("\\n", "\n");
    }
    // Try base64 decode → expect PEM text.
    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(trimmed.as_bytes()) {
        if let Ok(s) = String::from_utf8(bytes) {
            if s.contains("-----BEGIN") {
                return s;
            }
        }
    }
    // Fall through: return as-is and let JWT signing surface the error.
    trimmed.to_string()
}

#[derive(Debug, Serialize)]
struct AppJwtClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

/// Mint a short-lived App JWT (RS256, 9-minute TTL per GitHub guidance).
pub fn mint_app_jwt(cfg: &AppConfig) -> anyhow::Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system clock error: {e}"))?
        .as_secs();
    // Back-date iat by 60s to tolerate mild clock skew on GitHub's side.
    let claims = AppJwtClaims {
        iat: now.saturating_sub(60),
        exp: now + 9 * 60,
        iss: cfg.app_id.to_string(),
    };
    let key = EncodingKey::from_rsa_pem(cfg.private_key_pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("invalid GITHUB_APP_PRIVATE_KEY: {e}"))?;
    Ok(encode(&Header::new(Algorithm::RS256), &claims, &key)?)
}

#[derive(Debug, Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Exchange an App JWT for an installation-scoped access token.
pub async fn mint_installation_token(
    app_jwt: &str,
    install_id: i64,
) -> anyhow::Result<InstallationToken> {
    let url = format!("https://api.github.com/app/installations/{install_id}/access_tokens");
    let resp = gh_client()?
        .post(&url)
        .bearer_auth(app_jwt)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("installation token request failed ({status}): {body}");
    }

    let parsed: InstallationTokenResponse = resp.json().await?;
    Ok(InstallationToken {
        token: parsed.token,
        expires_at: parsed.expires_at,
    })
}

#[derive(Debug, Deserialize)]
struct AccountJson {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct InstallationJson {
    account: AccountJson,
}

#[derive(Debug, Clone)]
pub struct InstallationInfo {
    pub account_login: String,
    pub account_type: GitHubAccountType,
}

/// Look up public metadata for an installation (used by the callback to
/// avoid trusting any query-string fields for account identity).
pub async fn get_installation(app_jwt: &str, install_id: i64) -> anyhow::Result<InstallationInfo> {
    let url = format!("https://api.github.com/app/installations/{install_id}");
    let resp = gh_client()?
        .get(&url)
        .bearer_auth(app_jwt)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("installation lookup failed ({status}): {body}");
    }

    let parsed: InstallationJson = resp.json().await?;
    let kind = match parsed.account.kind.as_str() {
        "User" => GitHubAccountType::User,
        _ => GitHubAccountType::Organization,
    };
    Ok(InstallationInfo {
        account_login: parsed.account.login,
        account_type: kind,
    })
}

#[derive(Debug, Deserialize)]
struct RepoJson {
    name: String,
    owner: AccountJson,
    default_branch: Option<String>,
    private: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessibleRepo {
    pub owner: String,
    pub name: String,
    pub default_branch: Option<String>,
    pub private: bool,
}

#[derive(Debug, Deserialize)]
struct ListReposResponse {
    repositories: Vec<RepoJson>,
}

/// List repositories this installation can access. Paginated up to 200;
/// tenants with more repos should opt-in via direct project CRUD rather
/// than the dropdown (per #58 "opt-in per repo, never auto-mirror").
pub async fn list_installation_repos(
    token: &InstallationToken,
) -> anyhow::Result<Vec<AccessibleRepo>> {
    let mut out = Vec::new();
    for page in 1..=2 {
        let url =
            format!("https://api.github.com/installation/repositories?per_page=100&page={page}");
        let resp = gh_client()?
            .get(&url)
            .bearer_auth(&token.token)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("list_installation_repos failed ({status}): {body}");
        }
        let parsed: ListReposResponse = resp.json().await?;
        let count = parsed.repositories.len();
        out.extend(parsed.repositories.into_iter().map(|r| AccessibleRepo {
            owner: r.owner.login,
            name: r.name,
            default_branch: r.default_branch,
            private: r.private,
        }));
        if count < 100 {
            break;
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct LabelJson {
    name: String,
    color: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoLabel {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
}

/// List the labels defined on a repo. Paginated up to 200 (two 100-item
/// pages) — same budget as `list_installation_repos`. Repos with more
/// labels than that are rare; the combobox's free-text "create label"
/// path still works when a label isn't in the returned set.
pub async fn list_repo_labels(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
) -> anyhow::Result<Vec<RepoLabel>> {
    let mut out = Vec::new();
    for page in 1..=2 {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/labels?per_page=100&page={page}"
        );
        let resp = gh_client()?
            .get(&url)
            .bearer_auth(&token.token)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("list_repo_labels failed ({status}): {body}");
        }
        let parsed: Vec<LabelJson> = resp.json().await?;
        let count = parsed.len();
        out.extend(parsed.into_iter().map(|l| RepoLabel {
            name: l.name,
            color: l.color,
            description: l.description,
        }));
        if count < 100 {
            break;
        }
    }
    Ok(out)
}

/// Fetch a repo's default branch via the installation token. Used by
/// `add_project` to populate `default_branch` automatically.
pub async fn get_repo_default_branch(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
) -> anyhow::Result<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}");
    let resp = gh_client()?
        .get(&url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("get_repo failed ({status}): {body}");
    }
    let parsed: RepoJson = resp.json().await?;
    parsed
        .default_branch
        .ok_or_else(|| anyhow::anyhow!("repo {owner}/{repo} missing default_branch"))
}

/// Shared GitHub HTTP client — one per process so connection pools +
/// TLS state get reused across JWT, installation-token, and repo calls.
fn gh_client() -> anyhow::Result<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let c = reqwest::Client::builder()
        .user_agent("onsager-stiglab/0.1")
        .build()?;
    // `get_or_init` would be nicer but it can't surface the build error;
    // the race is benign — whichever thread wins sets the slot, and the
    // losing thread's locally-built client is dropped.
    Ok(CLIENT.get_or_init(|| c))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test RSA key generated with `openssl genrsa 2048` — unused
    // outside of tests; not associated with any real GitHub App.
    const TEST_RSA_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCu0gNSaTeR6s51
JThsj9+CPSnoKYwCJLFDJD2wpym9eRYvWPjZY0S3oKgQPenCOe8uEVr6iOt2m4VO
2/C3AYM/XWn0TimtdE301GWf9owTUigGMetN0RGOr8Wczs7YHxX8Nt1VTs8Bym2S
RR0AeQ7WLDN44ab2/Hiygwbab2IFI+GVOd4y5zsTUh3sBPzDso7rCfuipufwgGHA
K5RzF/B73XeLzygf8bSpV+j4cp/rssOJgRoxe1rx1ZVoZwfhQ5534TcPV68/6iDK
cF+pkJHp9z7tkBLD2X2E7hpMRrcWOr06yDjrmur047c3hzJbI5eIBjL1cr++ywri
8tjmLYlhAgMBAAECggEAAkvPy2umbTstkdVCuV4OxTFeBzKhwCrUxVNE4EjDSg3U
v0ukldgdeEkW4QL7qqtvsVHs+UO+bjv44VwvHEu7wluk9UaIKosXraezI6GhlYzB
ieKKpu6YM7jaPFLs7YKzw3Cx6bWVr2YOCC7yRHn6knCBXvDtjEKc2BkjiD+glK0+
Tgp44XQJMDhI8tj1Q53Fz6UgdYzMyYm0iiGn4wPw22uM9DuT1/Gjpi/dYY9BsGGA
zvgHAJLnuTfulihCg9GxppGWEZPWGrtTfrUN3gqNcCCcgWmYHi9Ye0CNgagTEZ+I
lk4JrWymM7u75F03hZs2+KGA2khEUT2FZHYJ65kTQQKBgQDj1auKQQtCfSQBydZu
dBWGTUO8IVwUrhqOr+b1KWMqf1n+10BTcUzdfTyHsAKKphTzCip+l14I+f2pjXJE
1Z1ZXg+5oRkASGMsZB40puiv3Z18TCpK7VakYvPuUaN35hSj020liEAcyCBOSMbS
u13Wj/fQblenvvYUHUPcCB4tQQKBgQDEbpbDHyxUjqQ0IMwET9gXCXjHyzHMegFa
ZzXBgOhFgw0mve90h9Z4VVu9Df5+Yc2taXtAvifmsHK0L7AJxr9htoRf/EPaPzHB
W2eWOV/HJ214YkMwZQ2ObWMM+juIz6qdKio0WJ22KqBMKFAxbiprsBBlshd3plx8
LDAKbHi0IQKBgFHsCZNb0f2lW6Yc+jKbIQY6kAl8gUyaUchOraAnspWcVzLQGTwn
uDjICFTN0AwkrdG6LQ95xAE8Sp6F0rm3ia2RqdvYdlHotWhH06ig/3gFGtSP2oE4
l/fh8M4XoszA+Vjy9AMT2+G9gAhGGN+7KYG2IKhclL4nZvpSj4z1ikxBAoGAM0Vy
UIfIeGGq9nhBCDcW/hxYzD17SBXoWIJsA4/0EIC+ZAhbgh0am9ob0eLfNHmux76q
jyGTJKGVrvZrioG33ndXYf5kb4jjIccL6KgdGcxuxGdRhkY6HZzrp62A8JrTu6YP
0g33TF8f7ADxvZU1uVoBTaoIehCQP1EBURczAkECgYAG6z7A4Ac1GSNLg1bp76Ys
el5f02ypFsAcGQHRbAIRpZX/u2/7VFIVgjlR5Dx6b4Y/pf6wwL88b4Y/Sgs6PMoc
ua1bcuTFsiOlTi/BawxRKJEbQniUm7uNpBSTysYxmVJdIooq2z1Md/vqBvtCkm53
KHLHs4NWfuFIhN/tCfpZ/g==
-----END PRIVATE KEY-----
";

    #[test]
    fn normalize_pem_passes_through_raw_pem() {
        // Trimming is expected — assert the core PEM structure survives.
        let raw = "-----BEGIN RSA PRIVATE KEY-----\nXX\n-----END RSA PRIVATE KEY-----\n";
        let out = normalize_pem(raw);
        assert!(out.starts_with("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(out.ends_with("-----END RSA PRIVATE KEY-----"));
        assert!(out.contains("XX"));
    }

    #[test]
    fn normalize_pem_decodes_escaped_newlines() {
        let raw = "-----BEGIN RSA PRIVATE KEY-----\\nXX\\n-----END RSA PRIVATE KEY-----";
        let out = normalize_pem(raw);
        assert!(out.contains("-----BEGIN"));
        assert!(out.contains('\n'));
    }

    #[test]
    fn normalize_pem_decodes_base64_wrapper() {
        use base64::Engine;
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nXX\n-----END RSA PRIVATE KEY-----\n";
        let b64 = base64::engine::general_purpose::STANDARD.encode(pem.as_bytes());
        assert_eq!(normalize_pem(&b64), pem);
    }

    #[test]
    fn app_config_missing_env_is_disabled() {
        let guard =
            EnvGuard::unset(&["GITHUB_APP_ID", "GITHUB_APP_SLUG", "GITHUB_APP_PRIVATE_KEY"]);
        assert!(AppConfig::from_env().is_none());
        drop(guard);
    }

    #[test]
    fn mint_app_jwt_produces_three_segments() {
        let cfg = AppConfig {
            app_id: 42,
            slug: "onsager".to_string(),
            private_key_pem: TEST_RSA_PEM.to_string(),
        };
        let jwt = mint_app_jwt(&cfg).expect("jwt should sign");
        assert_eq!(
            jwt.matches('.').count(),
            2,
            "JWT must have 3 dot-separated segments"
        );
    }

    #[test]
    fn mint_app_jwt_surfaces_invalid_pem() {
        let cfg = AppConfig {
            app_id: 1,
            slug: "onsager".to_string(),
            private_key_pem: "not a pem".to_string(),
        };
        assert!(mint_app_jwt(&cfg).is_err());
    }

    /// Serialize env var read/write in tests — `std::env::set_var` is
    /// process-global, and cargo runs tests in a single process by default.
    struct EnvGuard {
        keys: Vec<(String, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn unset(keys: &[&str]) -> Self {
            static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
            let lock = LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap();
            let snapshot: Vec<_> = keys
                .iter()
                .map(|k| (k.to_string(), std::env::var(k).ok()))
                .collect();
            for k in keys {
                std::env::remove_var(k);
            }
            EnvGuard {
                keys: snapshot,
                _lock: lock,
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.keys {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}
