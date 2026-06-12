//! AES-256-GCM credential sealing + random token generation.
//!
//! Ported verbatim from `legacy/crates/onsager-portal/src/auth.rs`
//! (the format — `nonce (12B) || ciphertext || tag`, hex-encoded — has
//! been stable since the stiglab era). Fresh v2 databases carry no old
//! rows, but keeping the format means an operator's existing
//! `ONSAGER_CREDENTIAL_KEY` tooling keeps working.

use ring::aead;
use ring::rand::{SecureRandom, SystemRandom};

/// Encrypt a plaintext string using AES-256-GCM. Returns a hex-encoded
/// string of `nonce (12B) || ciphertext || tag`.
pub fn encrypt(key_hex: &str, plaintext: &str) -> anyhow::Result<String> {
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

    let mut result = nonce_bytes.to_vec();
    result.extend_from_slice(&in_out);
    Ok(hex::encode(result))
}

/// Decrypt a hex-encoded `nonce || ciphertext || tag` string.
pub fn decrypt(key_hex: &str, encrypted_hex: &str) -> anyhow::Result<String> {
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

/// Random 32-byte hex token — session cookie values (and, in M1, the
/// per-session bearer token handed to agent processes).
pub fn random_token() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .expect("failed to generate random bytes");
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = "11".repeat(32);
        let sealed = encrypt(&key, "sk-secret").unwrap();
        assert_ne!(sealed, "sk-secret");
        assert_eq!(decrypt(&key, &sealed).unwrap(), "sk-secret");
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = encrypt(&"11".repeat(32), "x").unwrap();
        assert!(decrypt(&"22".repeat(32), &sealed).is_err());
    }

    #[test]
    fn tokens_are_unique_and_hex64() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }
}
