//! Cross-environment SSO — "auth-domain proxy with back-channel exchange."
//!
//! Prod (the OAuth app owner) registers a single GitHub callback. Preview
//! environments on Railway piggy-back off it: the preview redirects the
//! browser to prod's `/api/auth/github?return_to=…`, prod completes the
//! OAuth dance, and hands back a short-lived opaque code that the preview
//! redeems server-to-server (with a shared bearer secret) for the user
//! identity. The preview then mints its own local session.
//!
//! The state carried through GitHub is an HMAC-signed envelope containing
//! the CSRF nonce (also echoed in a cookie on the owner) and the optional
//! `return_to` URL. The owner refuses any `return_to` whose host is not in
//! `SSO_RETURN_HOST_ALLOWLIST`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

/// How this process relates to the GitHub OAuth app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsoMode {
    /// No auth configured — anonymous user.
    Disabled,
    /// Owns the GitHub OAuth app; handles callbacks directly.
    /// `delegate_enabled` is true when the owner-side secrets for serving
    /// preview environments are also set.
    Owner { delegate_enabled: bool },
    /// Delegates to a remote owner; never talks to GitHub itself.
    Relying,
}

/// Maximum clock skew tolerated when verifying a state envelope, in seconds.
const STATE_SKEW_SECS: i64 = 10;

/// Lifetime of an OAuth state envelope — long enough for a human to finish
/// typing their password on github.com, short enough that replayed states
/// don't linger.
pub const STATE_LIFETIME_SECS: i64 = 600;

/// Lifetime of an exchange code — long enough for the preview's server to
/// follow the redirect and call back to prod.
pub const EXCHANGE_CODE_LIFETIME_SECS: i64 = 30;

/// Signed-envelope payload. Field names are abbreviated so the encoded
/// form stays under URL-length limits even with long return_to values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateClaims {
    /// CSRF nonce — also stored in `stiglab_oauth_state` cookie on the owner.
    pub c: String,
    /// Optional return URL; when present, the callback mints an exchange
    /// code and 302s here instead of minting a local session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r: Option<String>,
    /// Unix expiry timestamp.
    pub e: i64,
}

/// Encode and sign the state payload. Output format: `<b64url(json)>.<hex(hmac)>`.
pub fn sign_state(secret: &str, claims: &StateClaims) -> String {
    let json = serde_json::to_vec(claims).expect("state claims serialize to JSON");
    let body = URL_SAFE_NO_PAD.encode(&json);
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, body.as_bytes());
    format!("{body}.{}", hex::encode(tag.as_ref()))
}

/// Verify an HMAC-signed state envelope. Returns the claims on success,
/// or `None` on any signature, format, or expiry failure.
pub fn verify_state(secret: &str, state: &str, now_unix: i64) -> Option<StateClaims> {
    let (body_b64, tag_hex) = state.split_once('.')?;
    let expected_tag = hex::decode(tag_hex).ok()?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hmac::verify(&key, body_b64.as_bytes(), &expected_tag).ok()?;

    let json = URL_SAFE_NO_PAD.decode(body_b64).ok()?;
    let claims: StateClaims = serde_json::from_slice(&json).ok()?;

    if claims.e + STATE_SKEW_SECS < now_unix {
        return None;
    }
    Some(claims)
}

/// Parse a comma-separated env-var list like `*.preview.onsager.ai,app.onsager.ai`
/// into an allowlist vector. Empty and whitespace-only entries are dropped.
pub fn parse_host_allowlist(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Check whether a full `return_to` URL is acceptable. Rules:
/// * URL must parse and be `http`- or `https`-scheme.
/// * Host must match an allowlist entry.
///   * `*.example.com` matches any strict subdomain of `example.com`.
///   * `example.com` matches only `example.com` exactly.
pub fn return_to_allowed(allowlist: &[String], return_to: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(return_to) else {
        return false;
    };
    match url.scheme() {
        "http" | "https" => {}
        _ => return false,
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host_matches_allowlist(allowlist, host)
}

/// Extract the host component of a URL — useful for comparing the host
/// claimed by the redeemer to the host baked into the exchange code.
pub fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
}

fn host_matches_allowlist(allowlist: &[String], host: &str) -> bool {
    for entry in allowlist {
        if let Some(suffix) = entry.strip_prefix("*.") {
            // Strict subdomain match: `host` must be `<something>.suffix`.
            if host.len() > suffix.len() + 1
                && host.ends_with(suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
            {
                return true;
            }
        } else if host.eq_ignore_ascii_case(entry) {
            return true;
        }
    }
    false
}

/// Generate an opaque single-use exchange code (URL-safe, 32 bytes of entropy).
pub fn generate_exchange_code() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes).expect("rng");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Constant-time secret comparison. Both inputs are compared as raw bytes.
/// Short-circuits only on length mismatch — the attacker controls whether
/// a comparison is made at all, not how long it takes once started.
pub fn secrets_equal(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let claims = StateClaims {
            c: "csrf123".into(),
            r: Some("https://pr-1.preview.example.com/cb".into()),
            e: 1_000_000_000,
        };
        let s = sign_state("secret", &claims);
        let verified = verify_state("secret", &s, claims.e - 1).unwrap();
        assert_eq!(verified, claims);
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let claims = StateClaims {
            c: "csrf".into(),
            r: None,
            e: 2_000_000_000,
        };
        let s = sign_state("secret-a", &claims);
        assert!(verify_state("secret-b", &s, 0).is_none());
    }

    #[test]
    fn verify_rejects_expired() {
        let claims = StateClaims {
            c: "csrf".into(),
            r: None,
            e: 1_000,
        };
        let s = sign_state("k", &claims);
        // `now` far beyond expiry + skew.
        assert!(verify_state("k", &s, 1_000 + STATE_SKEW_SECS + 1).is_none());
    }

    #[test]
    fn verify_tolerates_small_skew() {
        let claims = StateClaims {
            c: "csrf".into(),
            r: None,
            e: 1_000,
        };
        let s = sign_state("k", &claims);
        assert!(verify_state("k", &s, 1_000 + STATE_SKEW_SECS).is_some());
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let claims = StateClaims {
            c: "csrf".into(),
            r: Some("https://good.example/cb".into()),
            e: 9_999_999_999,
        };
        let s = sign_state("k", &claims);
        // Swap a character in the body portion (before the dot).
        let dot = s.find('.').unwrap();
        let mut bytes = s.into_bytes();
        bytes[0] = if bytes[0] == b'a' { b'b' } else { b'a' };
        let _ = dot; // (no-op to silence unused)
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(verify_state("k", &tampered, 0).is_none());
    }

    #[test]
    fn allowlist_subdomain_match() {
        let allow = vec!["*.preview.onsager.ai".into()];
        assert!(host_matches_allowlist(&allow, "pr-1.preview.onsager.ai"));
        assert!(host_matches_allowlist(&allow, "a.b.preview.onsager.ai"));
        // Apex does not match `*.preview.onsager.ai`.
        assert!(!host_matches_allowlist(&allow, "preview.onsager.ai"));
        // Suffix-only attack: `evilpreview.onsager.ai` must not match.
        assert!(!host_matches_allowlist(&allow, "evilpreview.onsager.ai"));
        // Totally unrelated.
        assert!(!host_matches_allowlist(&allow, "attacker.com"));
    }

    #[test]
    fn allowlist_exact_match() {
        let allow = vec!["app.onsager.ai".into()];
        assert!(host_matches_allowlist(&allow, "app.onsager.ai"));
        assert!(host_matches_allowlist(&allow, "APP.ONSAGER.AI"));
        assert!(!host_matches_allowlist(&allow, "foo.app.onsager.ai"));
        assert!(!host_matches_allowlist(&allow, "app.onsager.com"));
    }

    #[test]
    fn allowlist_empty_rejects_all() {
        let allow: Vec<String> = vec![];
        assert!(!host_matches_allowlist(&allow, "anything.com"));
    }

    #[test]
    fn return_to_requires_valid_url() {
        let allow = vec!["*.preview.onsager.ai".into()];
        assert!(return_to_allowed(
            &allow,
            "https://pr-1.preview.onsager.ai/api/auth/sso/finish"
        ));
        assert!(!return_to_allowed(&allow, "not a url"));
        assert!(!return_to_allowed(&allow, "javascript:alert(1)"));
        assert!(!return_to_allowed(
            &allow,
            "ftp://pr-1.preview.onsager.ai/x"
        ));
    }

    #[test]
    fn parse_allowlist_trims_and_filters() {
        let v = parse_host_allowlist(" *.preview.onsager.ai , app.onsager.ai ,  ");
        assert_eq!(
            v,
            vec![
                "*.preview.onsager.ai".to_string(),
                "app.onsager.ai".to_string()
            ]
        );
        assert!(parse_host_allowlist("").is_empty());
        assert!(parse_host_allowlist("  ,  ").is_empty());
    }

    #[test]
    fn generate_exchange_code_is_unique() {
        let a = generate_exchange_code();
        let b = generate_exchange_code();
        assert_ne!(a, b);
        assert!(URL_SAFE_NO_PAD.decode(&a).is_ok());
    }

    #[test]
    fn secrets_equal_matches() {
        assert!(secrets_equal("abc", "abc"));
        assert!(!secrets_equal("abc", "abd"));
        assert!(!secrets_equal("abc", "abcd"));
        assert!(!secrets_equal("", "abc"));
    }

    #[test]
    fn host_of_extracts_host() {
        assert_eq!(
            host_of("https://pr-1.preview.onsager.ai/cb?x=1"),
            Some("pr-1.preview.onsager.ai".into())
        );
        assert_eq!(host_of("not a url"), None);
    }
}
