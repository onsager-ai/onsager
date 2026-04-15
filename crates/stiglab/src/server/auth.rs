use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ring::aead;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;

use crate::server::db;
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

// ── Auth Extractor ──

/// Authenticated user extracted from request cookies.
/// When auth is disabled (no GitHub config), returns a synthetic anonymous user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
}

impl AuthUser {
    fn anonymous() -> Self {
        AuthUser {
            user_id: "anonymous".to_string(),
            github_login: "anonymous".to_string(),
            github_name: None,
            github_avatar_url: None,
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // If auth is not enabled, return anonymous user
        if !state.config.auth_enabled() {
            return Ok(AuthUser::anonymous());
        }

        // Extract session cookie
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
            Ok(Some(auth_session)) => Ok(AuthUser {
                user_id: auth_session.user_id,
                github_login: auth_session.user.github_login,
                github_name: auth_session.user.github_name,
                github_avatar_url: auth_session.user.github_avatar_url,
            }),
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
    fn test_auth_user_anonymous() {
        let user = AuthUser::anonymous();
        assert_eq!(user.user_id, "anonymous");
        assert_eq!(user.github_login, "anonymous");
        assert!(user.github_name.is_none());
    }
}
