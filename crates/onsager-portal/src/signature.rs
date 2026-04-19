//! GitHub webhook signature verification.
//!
//! GitHub signs every webhook delivery with HMAC-SHA256 using the per-app
//! installation's webhook secret. The signature arrives in the
//! `X-Hub-Signature-256` header as `"sha256=<hex digest>"`. Constant-time
//! comparison is mandatory to avoid leaking the secret via timing.

use ring::hmac;

/// Outcome of a signature check.
#[derive(Debug, PartialEq, Eq)]
pub enum SignatureCheck {
    Valid,
    Mismatch,
    Malformed,
}

/// Verify a `sha256=...` signature header against the raw body using `secret`.
///
/// The header MUST be the full `sha256=<hex>` string GitHub sends. `secret`
/// is the per-installation webhook secret (the plaintext, after the portal
/// has decrypted `webhook_secret_cipher`).
pub fn verify_signature(header: &str, body: &[u8], secret: &[u8]) -> SignatureCheck {
    let Some(sig_hex) = header.strip_prefix("sha256=") else {
        return SignatureCheck::Malformed;
    };
    let Ok(sig) = hex::decode(sig_hex) else {
        return SignatureCheck::Malformed;
    };
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    match hmac::verify(&key, body, &sig) {
        Ok(()) => SignatureCheck::Valid,
        Err(_) => SignatureCheck::Mismatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::hmac;

    fn sign(body: &[u8], secret: &[u8]) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let tag = hmac::sign(&key, body);
        format!("sha256={}", hex::encode(tag.as_ref()))
    }

    #[test]
    fn valid_signature_passes() {
        let body = b"hello world";
        let secret = b"shhh";
        let header = sign(body, secret);
        assert_eq!(
            verify_signature(&header, body, secret),
            SignatureCheck::Valid
        );
    }

    #[test]
    fn tampered_body_fails() {
        let body = b"hello world";
        let secret = b"shhh";
        let header = sign(body, secret);
        assert_eq!(
            verify_signature(&header, b"different body", secret),
            SignatureCheck::Mismatch
        );
    }

    #[test]
    fn wrong_secret_fails() {
        let body = b"hello world";
        let header = sign(body, b"good secret");
        assert_eq!(
            verify_signature(&header, body, b"bad secret"),
            SignatureCheck::Mismatch
        );
    }

    #[test]
    fn missing_prefix_is_malformed() {
        let body = b"hello world";
        let header = "deadbeef".to_string();
        assert_eq!(
            verify_signature(&header, body, b"shhh"),
            SignatureCheck::Malformed
        );
    }

    #[test]
    fn bad_hex_is_malformed() {
        let header = "sha256=zzzz";
        assert_eq!(
            verify_signature(header, b"x", b"shhh"),
            SignatureCheck::Malformed
        );
    }
}
