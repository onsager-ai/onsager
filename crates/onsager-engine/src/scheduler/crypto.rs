//! AES-256-GCM credential decryption for the plan-scoped session token
//! (spec #536).
//!
//! Portal mints the plan token, encrypts it with the shared credential
//! key, and embeds the ciphertext in the `plan.run_requested` event. The
//! scheduler decrypts it here once per plan and threads the plaintext
//! down to every agent node for `ONSAGER_SESSION_TOKEN` injection.
//!
//! This helper is a deliberate **replica** of
//! `onsager-portal::auth::decrypt_credential` /
//! `stiglab::server::auth::decrypt_credential` — the seam rule forbids
//! the scheduler from depending on portal or stiglab crates, and the
//! function is small (hex-decode → split 12-byte nonce → AES-256-GCM
//! open). The wire format (hex of nonce ++ ciphertext) is the contract
//! the three copies share; a change on one side must change all three.

use ring::aead;

/// Decrypt a hex-encoded `nonce(12) ++ ciphertext` string using
/// AES-256-GCM under `key_hex` (the shared 32-byte hex credential key).
/// Mirrors the portal/stiglab `decrypt_credential` byte-for-byte.
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

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::{SecureRandom, SystemRandom};

    /// Local encrypt mirroring portal's `encrypt_credential`, so the
    /// round-trip is provable without depending on portal. The
    /// production ciphertext always comes from portal — this only
    /// exercises that this crate's `decrypt_credential` reads that exact
    /// wire format.
    fn encrypt(key_hex: &str, plaintext: &str) -> String {
        let key_bytes = hex::decode(key_hex).unwrap();
        let unbound_key = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes).unwrap();
        let sealing_key = aead::LessSafeKey::new(unbound_key);
        let rng = SystemRandom::new();
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes).unwrap();
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.as_bytes().to_vec();
        sealing_key
            .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
            .unwrap();
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        hex::encode(result)
    }

    fn key() -> String {
        let rng = SystemRandom::new();
        let mut k = [0u8; 32];
        rng.fill(&mut k).unwrap();
        hex::encode(k)
    }

    #[test]
    fn round_trips_portal_wire_format() {
        let k = key();
        let enc = encrypt(&k, "ons_pat_plan_token");
        assert_eq!(decrypt_credential(&k, &enc).unwrap(), "ons_pat_plan_token");
    }

    #[test]
    fn wrong_key_fails() {
        let enc = encrypt(&key(), "secret");
        assert!(decrypt_credential(&key(), &enc).is_err());
    }

    #[test]
    fn malformed_input_fails() {
        let k = key();
        assert!(decrypt_credential(&k, "tooshort").is_err());
        assert!(decrypt_credential(&k, "not-hex!").is_err());
    }
}
