use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::aead;
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;

use crate::server::db;
use crate::server::sso::secrets_equal;
use crate::server::state::AppState;

// ── Credential Encryption (AES-256-GCM) ──

/// Encrypt a plaintext string using AES-256-GCM.
/// Returns a hex-encoded string of nonce + ciphertext.
pub fn encrypt_credential(key_hex: &str, plaintext: &str) -> anyhow::Result<String> {
    let key_bytes = hex::decode(key_hex)?;
    let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| anyhow::anyhow!("invalid encryption key"))?;
    let sealing_key = aead::LessSafeKey::new(unbound_key);

    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| anyhow::anyhow!("failed to generate nonce"))?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.as_bytes().to_vec();
    sealing_key
        .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;

    // Prepend nonce to ciphertext
    let mut result = nonce_bytes.to_vec();
    result.extend_from_slice(&in_out);
    Ok(hex::encode(result))
}

/// Decrypt a hex-encoded nonce + ciphertext string using AES-256-GCM.
pub fn decrypt_credential(key_hex: &str, encrypted_hex: &str) -> anyhow::Result<String> {
    let key_bytes = hex::decode(key_hex)?;
    let data = hex::decode(encrypted_hex)?;
    if data.len() < 12 {
        anyhow::bail!("invalid encrypted data");
    }

    let (nonce_bytes, ciphertext) = data.split_at(12);
    let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| anyhow::anyhow!("invalid encryption key"))?;
    let opening_key = aead::LessSafeKey::new(unbound_key);
    let nonce = aead::Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| anyhow::anyhow!("invalid nonce"))?;

    let mut in_out = ciphertext.to_vec();
    let plaintext = opening_key
        .open_in_place(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| anyhow::anyhow!("decryption failed"))?;
    Ok(String::from_utf8(plaintext.to_vec())?)
}

/// Generate a random 32-byte hex-encoded key for AES-256-GCM.
pub fn generate_credential_key() -> String {
    let rng = SystemRandom::new();
    let mut key = [0u8; 32];
    rng.fill(&mut key).expect("failed to generate random key");
    hex::encode(key)
}

// ── GitHub OAuth ──

#[derive(Debug, Deserialize)]
pub struct GithubTokenResponse {
    pub access_token: String,
}

#[derive(Debug, Deserialize)]
pub struct GithubUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

pub fn github_authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    let mut url = reqwest::Url::parse("https://github.com/login/oauth/authorize")
        .expect("hardcoded GitHub OAuth authorize URL must be valid");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state)
        .append_pair("scope", "read:user");
    url.into()
}

pub async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> anyhow::Result<GithubTokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": code,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("GitHub token exchange failed: {text}");
    }

    Ok(resp.json().await?)
}

pub async fn get_github_user(access_token: &str) -> anyhow::Result<GithubUser> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "stiglab")
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("GitHub user API failed: {text}");
    }

    Ok(resp.json().await?)
}

/// Generate a random session token (hex-encoded, 32 bytes of randomness).
pub fn generate_session_token() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .expect("failed to generate random bytes");
    hex::encode(bytes)
}

/// Generate a random CSRF state parameter.
pub fn generate_state() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes)
        .expect("failed to generate random bytes");
    hex::encode(bytes)
}

// ── Personal Access Tokens (issue #143) ──

/// Token namespace prefix. Matches GitHub's `ghp_` style so secret scanners
/// can recognize Onsager PATs in source-control / log scrapes.
pub const PAT_TOKEN_NAMESPACE: &str = "ons_pat_";

/// Length of the prefix stored in the DB for display + indexed lookup.
/// `ons_pat_` (8 chars) + 4 chars of token entropy = 12 chars total.
pub const PAT_PREFIX_LEN: usize = 12;

/// Number of random bytes mixed into a PAT after the namespace prefix.
const PAT_RANDOM_BYTES: usize = 32;

/// A freshly generated PAT, ready to be persisted (`hash`) and shown to the
/// user exactly once (`token`).
pub struct GeneratedPat {
    /// The full token to hand to the user. Begins with `ons_pat_`.
    pub token: String,
    /// First [`PAT_PREFIX_LEN`] characters of `token`. Stored in the clear
    /// for display + lookup.
    pub prefix: String,
    /// Hex-encoded SHA-256 of `token`. The only thing persisted to the DB
    /// from the secret material.
    pub hash: String,
}

/// Generate a fresh personal access token. The full token is `ons_pat_` plus
/// 32 random url-safe bytes — about 256 bits of entropy after the namespace
/// prefix. Only the hash is stored; the caller is responsible for showing
/// `token` to the user exactly once.
pub fn generate_pat_token() -> GeneratedPat {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; PAT_RANDOM_BYTES];
    rng.fill(&mut bytes).expect("rng failed to generate bytes");
    let token = format!("{PAT_TOKEN_NAMESPACE}{}", URL_SAFE_NO_PAD.encode(bytes));
    let prefix = pat_prefix(&token);
    let hash = hash_pat_token(&token);
    GeneratedPat {
        token,
        prefix,
        hash,
    }
}

/// Hex-encoded SHA-256 of the raw token. Used both at insert time and on
/// every verification.
pub fn hash_pat_token(token: &str) -> String {
    let digest = digest::digest(&digest::SHA256, token.as_bytes());
    hex::encode(digest.as_ref())
}

/// Return the lookup prefix for a token. Always [`PAT_PREFIX_LEN`] chars
/// when the token is well-formed; for malformed (too-short) tokens returns
/// the entire string so callers can compare without panicking.
pub fn pat_prefix(token: &str) -> String {
    if token.len() >= PAT_PREFIX_LEN {
        token[..PAT_PREFIX_LEN].to_string()
    } else {
        token.to_string()
    }
}

/// Outcome of verifying a PAT against the DB. The `Invalid` arms are split
/// from `Ok` so the extractor can return the right `WWW-Authenticate` body
/// without leaking which step failed. `UserPat` is boxed to keep the enum
/// itself pointer-sized — most call sites match the small arms.
#[derive(Debug)]
pub enum PatVerifyOutcome {
    Ok(Box<db::UserPat>),
    /// Prefix matched no row, or the hash didn't match any candidate row.
    Unknown,
    /// Row matched but is revoked or past its `expires_at`.
    Revoked,
    Expired,
}

/// Verify a presented PAT. Looks up candidate rows by `token_prefix`, then
/// constant-time compares the SHA-256 hash against each candidate. The
/// prefix is non-secret; the hash compare is the actual identity check.
pub async fn verify_pat(
    pool: &sqlx::AnyPool,
    presented_token: &str,
) -> anyhow::Result<PatVerifyOutcome> {
    if !presented_token.starts_with(PAT_TOKEN_NAMESPACE) {
        return Ok(PatVerifyOutcome::Unknown);
    }
    let prefix = pat_prefix(presented_token);
    let presented_hash = hash_pat_token(presented_token);
    let candidates = db::find_pats_by_prefix(pool, &prefix).await?;
    if candidates.is_empty() {
        return Ok(PatVerifyOutcome::Unknown);
    }

    let mut matched: Option<db::UserPat> = None;
    for (pat, stored_hash) in candidates {
        if secrets_equal(&presented_hash, &stored_hash) {
            matched = Some(pat);
            // Don't break — keep iterating to keep the work uniform across
            // collision and non-collision paths. The hash space makes >1
            // match astronomically unlikely; the loop is still O(n) on the
            // (non-secret) prefix bucket.
        }
    }

    let Some(pat) = matched else {
        return Ok(PatVerifyOutcome::Unknown);
    };

    if pat.revoked_at.is_some() {
        return Ok(PatVerifyOutcome::Revoked);
    }
    if let Some(exp) = pat.expires_at {
        if exp < chrono::Utc::now() {
            return Ok(PatVerifyOutcome::Expired);
        }
    }
    Ok(PatVerifyOutcome::Ok(Box::new(pat)))
}

// ── Auth Extractor ──

/// Whether the request was authenticated via a browser session cookie or a
/// PAT bearer token. Surfaced on [`AuthUser`] so destructive endpoints can
/// gate behavior on the principal kind.
#[derive(Debug, Clone)]
pub enum RequestPrincipal {
    /// Cookie-based session (the `stiglab_session` flow).
    Session,
    /// PAT-authenticated request.  `pat_id` identifies the issuing token row;
    /// `workspace_id` pins the request to that workspace — every PAT is
    /// workspace-scoped post-#163, so this is mandatory at the type level.
    Pat {
        pat_id: String,
        workspace_id: String,
    },
}

impl RequestPrincipal {
    pub fn is_pat(&self) -> bool {
        matches!(self, RequestPrincipal::Pat { .. })
    }

    /// The workspace a PAT is pinned to.  `None` for cookie/session
    /// principals; PAT principals are always pinned.
    pub fn pinned_workspace_id(&self) -> Option<&str> {
        match self {
            RequestPrincipal::Pat { workspace_id, .. } => Some(workspace_id.as_str()),
            _ => None,
        }
    }
}

/// How the user behind a request was minted. The dashboard renders a
/// persistent dev-mode banner when this is `Dev` (issue #193).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// Real GitHub OAuth user.
    Github,
    /// Local-only dev user seeded by `seed_dev_user_and_workspace` and
    /// minted via `/api/auth/dev-login`. Only reachable in debug builds.
    Dev,
}

/// Negative `github_id` is the type-level signal for a dev-seeded user
/// (real GitHub user IDs are always positive). Centralized here so the
/// seeder, the auth extractor, and `/api/auth/me` agree on the rule.
pub fn session_kind_for_github_id(github_id: i64) -> SessionKind {
    if github_id < 0 {
        SessionKind::Dev
    } else {
        SessionKind::Github
    }
}

/// Authenticated user extracted from request cookies or a PAT Bearer token.
/// Auth is always-on as of issue #193 — every request must resolve to a
/// real `users` row. There is no synthetic principal.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub principal: RequestPrincipal,
    pub session_kind: SessionKind,
}

/// Pull the bearer token (if any) out of the `Authorization` header.
pub fn parse_bearer_token(parts: &Parts) -> Option<String> {
    let v = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = v.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn unauthorized_invalid_token() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            axum::http::header::WWW_AUTHENTICATE,
            "Bearer error=\"invalid_token\"",
        )
        .body(axum::body::Body::from("invalid token"))
        .unwrap()
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // 1) Try PAT Bearer token first. A valid PAT wins over any cookie
        //    on the same request — cookie + Bearer normally doesn't happen
        //    in practice, but a CLI smoke test of the dashboard shouldn't
        //    silently fall through to the browser session.
        if let Some(token) = parse_bearer_token(parts) {
            if token.starts_with(PAT_TOKEN_NAMESPACE) {
                match verify_pat(&state.db, &token).await {
                    Ok(PatVerifyOutcome::Ok(pat)) => {
                        let user = match db::get_user(&state.db, &pat.user_id).await {
                            Ok(Some(u)) => u,
                            Ok(None) => {
                                tracing::warn!(
                                    pat_id = %pat.id,
                                    "PAT references missing user — rejecting"
                                );
                                return Err(unauthorized_invalid_token());
                            }
                            Err(e) => {
                                tracing::error!("PAT auth: failed to load user: {e}");
                                return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                            }
                        };

                        // Best-effort touch — failure must not block the
                        // request. Capture client metadata before the move.
                        let pool = state.db.clone();
                        let pat_id = pat.id.clone();
                        let ip = parts
                            .headers
                            .get("x-forwarded-for")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.split(',').next())
                            .map(|s| s.trim().to_string());
                        let ua = parts
                            .headers
                            .get(axum::http::header::USER_AGENT)
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string());
                        tokio::spawn(async move {
                            if let Err(e) =
                                db::touch_user_pat(&pool, &pat_id, ip.as_deref(), ua.as_deref())
                                    .await
                            {
                                tracing::warn!(pat_id = %pat_id, "failed to touch PAT: {e}");
                            }
                        });

                        let session_kind = session_kind_for_github_id(user.github_id);
                        return Ok(AuthUser {
                            user_id: user.id,
                            github_login: user.github_login,
                            github_name: user.github_name,
                            github_avatar_url: user.github_avatar_url,
                            principal: RequestPrincipal::Pat {
                                pat_id: pat.id,
                                workspace_id: pat.workspace_id,
                            },
                            session_kind,
                        });
                    }
                    Ok(PatVerifyOutcome::Unknown)
                    | Ok(PatVerifyOutcome::Revoked)
                    | Ok(PatVerifyOutcome::Expired) => {
                        return Err(unauthorized_invalid_token());
                    }
                    Err(e) => {
                        tracing::error!("PAT verification failed: {e}");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                    }
                }
            }
            // Other Bearer tokens (e.g. SSO exchange secret on /sso/redeem)
            // are owned by their dedicated routes — fall through so the
            // cookie path still works for the dashboard.
        }

        // 2) Fall back to the session cookie.
        let session_id = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cookie_header| parse_cookie(cookie_header, "stiglab_session"))
            .map(|s| s.to_string());

        let Some(session_id) = session_id else {
            return Err((StatusCode::UNAUTHORIZED, "not authenticated").into_response());
        };

        // Look up session in DB
        match db::get_auth_session(&state.db, &session_id).await {
            Ok(Some(auth_session)) => {
                let session_kind = session_kind_for_github_id(auth_session.user.github_id);
                Ok(AuthUser {
                    user_id: auth_session.user_id,
                    github_login: auth_session.user.github_login,
                    github_name: auth_session.user.github_name,
                    github_avatar_url: auth_session.user.github_avatar_url,
                    principal: RequestPrincipal::Session,
                    session_kind,
                })
            }
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "session expired").into_response()),
            Err(e) => {
                tracing::error!("failed to look up auth session: {e}");
                Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        }
    }
}

pub fn parse_cookie<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(name) {
            let value = value.trim_start();
            if let Some(value) = value.strip_prefix('=') {
                return Some(value.trim());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_encryption_roundtrip() {
        let key = generate_credential_key();
        let plaintext = "sk-ant-test-token-12345";
        let encrypted = encrypt_credential(&key, plaintext).unwrap();
        let decrypted = decrypt_credential(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_credential_encryption_different_nonces() {
        let key = generate_credential_key();
        let plaintext = "same-plaintext";
        let enc1 = encrypt_credential(&key, plaintext).unwrap();
        let enc2 = encrypt_credential(&key, plaintext).unwrap();
        // Different nonces should produce different ciphertexts
        assert_ne!(enc1, enc2);
        // But both decrypt to the same value
        assert_eq!(decrypt_credential(&key, &enc1).unwrap(), plaintext);
        assert_eq!(decrypt_credential(&key, &enc2).unwrap(), plaintext);
    }

    #[test]
    fn test_parse_cookie() {
        assert_eq!(
            parse_cookie("stiglab_session=abc123; theme=dark", "stiglab_session"),
            Some("abc123")
        );
        assert_eq!(
            parse_cookie("theme=dark; stiglab_session=xyz", "stiglab_session"),
            Some("xyz")
        );
        assert_eq!(parse_cookie("theme=dark", "stiglab_session"), None);
    }

    #[test]
    fn test_parse_cookie_empty() {
        assert_eq!(parse_cookie("", "stiglab_session"), None);
    }

    #[test]
    fn test_parse_cookie_no_value() {
        assert_eq!(
            parse_cookie("stiglab_session; theme=dark", "stiglab_session"),
            None
        );
    }

    #[test]
    fn test_parse_cookie_whitespace() {
        assert_eq!(
            parse_cookie("  stiglab_session = abc123 ; theme=dark", "stiglab_session"),
            Some("abc123")
        );
    }

    #[test]
    fn test_generate_session_token_uniqueness() {
        let t1 = generate_session_token();
        let t2 = generate_session_token();
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), 64); // 32 bytes hex-encoded
    }

    #[test]
    fn test_generate_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
        assert_eq!(s1.len(), 32); // 16 bytes hex-encoded
    }

    #[test]
    fn test_generate_credential_key_length() {
        let key = generate_credential_key();
        assert_eq!(key.len(), 64); // 32 bytes hex-encoded
                                   // Should be valid hex
        assert!(hex::decode(&key).is_ok());
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let key1 = generate_credential_key();
        let key2 = generate_credential_key();
        let encrypted = encrypt_credential(&key1, "secret").unwrap();
        assert!(decrypt_credential(&key2, &encrypted).is_err());
    }

    #[test]
    fn test_decrypt_invalid_data() {
        let key = generate_credential_key();
        assert!(decrypt_credential(&key, "tooshort").is_err());
        assert!(decrypt_credential(&key, "not-hex!").is_err());
    }

    #[test]
    fn test_github_authorize_url() {
        let url = github_authorize_url("client123", "https://example.com/callback", "state456");
        assert!(url.contains("client_id=client123"));
        assert!(url.contains("state=state456"));
        assert!(url.contains("scope=read%3Auser"));
        assert!(url.starts_with("https://github.com/login/oauth/authorize"));
    }

    #[test]
    fn session_kind_negative_github_id_is_dev() {
        // Real GitHub IDs are always positive; the seeder uses negative IDs
        // for dev-only users so the type is recoverable from the user row
        // alone (no extra column on `auth_sessions`).
        assert_eq!(session_kind_for_github_id(-1), SessionKind::Dev);
        assert_eq!(session_kind_for_github_id(-9999), SessionKind::Dev);
        assert_eq!(session_kind_for_github_id(0), SessionKind::Github);
        assert_eq!(session_kind_for_github_id(123_456), SessionKind::Github);
    }

    #[test]
    fn session_kind_serializes_lowercase() {
        // The dashboard reads `session_kind: "github" | "dev"` from
        // `/api/auth/me`; lock the wire shape down here.
        assert_eq!(
            serde_json::to_string(&SessionKind::Github).unwrap(),
            "\"github\""
        );
        assert_eq!(serde_json::to_string(&SessionKind::Dev).unwrap(), "\"dev\"");
    }

    #[test]
    fn test_generate_pat_token_shape() {
        let pat = generate_pat_token();
        assert!(
            pat.token.starts_with(PAT_TOKEN_NAMESPACE),
            "token should start with the namespace prefix"
        );
        assert_eq!(pat.prefix.len(), PAT_PREFIX_LEN);
        assert_eq!(pat.prefix, &pat.token[..PAT_PREFIX_LEN]);
        // SHA-256 hex is 64 chars.
        assert_eq!(pat.hash.len(), 64);
        assert_eq!(pat.hash, hash_pat_token(&pat.token));
    }

    #[test]
    fn test_generate_pat_token_uniqueness() {
        let a = generate_pat_token();
        let b = generate_pat_token();
        assert_ne!(a.token, b.token);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn test_hash_pat_token_deterministic() {
        let token = "ons_pat_abc123";
        assert_eq!(hash_pat_token(token), hash_pat_token(token));
        assert_ne!(hash_pat_token(token), hash_pat_token("ons_pat_abc124"));
    }

    #[test]
    fn test_pat_prefix_short_token_doesnt_panic() {
        let s = pat_prefix("abc");
        assert_eq!(s, "abc");
    }

    #[test]
    fn test_request_principal_helpers() {
        assert!(!RequestPrincipal::Session.is_pat());
        assert_eq!(RequestPrincipal::Session.pinned_workspace_id(), None);
        let p = RequestPrincipal::Pat {
            pat_id: "p1".into(),
            workspace_id: "w1".into(),
        };
        assert!(p.is_pat());
        assert_eq!(p.pinned_workspace_id(), Some("w1"));
    }
}
