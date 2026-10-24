//! Envelope encryption for platform secrets (provider API keys).
//!
//! AES-256-GCM with a 32-byte master key from `PROBE_FORGE_SETTINGS_KEY`
//! (base64). The stored form is `v1.<base64 nonce>.<base64 ciphertext+tag>`.
//! The plaintext is never logged, returned by the API, or written to audit.
//! The analysis worker implements the same format so it can decrypt for use.

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub const KEY_ENV: &str = "PROBE_FORGE_SETTINGS_KEY";
const VERSION: &str = "v1";
const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the settings key is not configured")]
    KeyMissing,
    #[error("the settings key must be base64-encoded 32 bytes")]
    KeyInvalid,
    #[error("the stored secret is not a valid v1 envelope")]
    CiphertextInvalid,
    #[error("the secret could not be encrypted or decrypted")]
    Crypto,
}

/// Load the master key from the environment. Returns `Ok(None)` when unset so
/// callers can fail closed only when a secret must be written or read.
pub fn load_key() -> Result<Option<[u8; KEY_BYTES]>, SecretError> {
    let raw = match std::env::var(KEY_ENV) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };
    let decoded = STANDARD
        .decode(raw.trim())
        .map_err(|_| SecretError::KeyInvalid)?;
    let bytes: [u8; KEY_BYTES] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| SecretError::KeyInvalid)?;
    Ok(Some(bytes))
}

/// Encrypt a secret into the `v1` envelope.
pub fn encrypt(key: &[u8; KEY_BYTES], plaintext: &str) -> Result<String, SecretError> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce_bytes).map_err(|_| SecretError::Crypto)?;
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| SecretError::Crypto)?;
    Ok(format!(
        "{VERSION}.{}.{}",
        STANDARD.encode(nonce_bytes),
        STANDARD.encode(ciphertext)
    ))
}

/// Decrypt a `v1` envelope back to plaintext.
pub fn decrypt(key: &[u8; KEY_BYTES], encoded: &str) -> Result<String, SecretError> {
    let mut parts = encoded.split('.');
    if parts.next() != Some(VERSION) {
        return Err(SecretError::CiphertextInvalid);
    }
    let nonce = STANDARD
        .decode(parts.next().ok_or(SecretError::CiphertextInvalid)?)
        .map_err(|_| SecretError::CiphertextInvalid)?;
    let ciphertext = STANDARD
        .decode(parts.next().ok_or(SecretError::CiphertextInvalid)?)
        .map_err(|_| SecretError::CiphertextInvalid)?;
    if parts.next().is_some() || nonce.len() != NONCE_BYTES {
        return Err(SecretError::CiphertextInvalid);
    }
    let nonce_bytes: [u8; NONCE_BYTES] = nonce
        .as_slice()
        .try_into()
        .map_err(|_| SecretError::CiphertextInvalid)?;
    let nonce = Nonce::from(nonce_bytes);
    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| SecretError::Crypto)?;
    String::from_utf8(plaintext).map_err(|_| SecretError::Crypto)
}

/// A short, non-reversible display hint for a configured secret.
pub fn hint(plaintext: &str) -> String {
    let tail: String = plaintext
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if tail.is_empty() {
        "••••".to_owned()
    } else {
        format!("••••{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; KEY_BYTES] {
        [7_u8; KEY_BYTES]
    }

    #[test]
    fn roundtrips_and_never_leaks_plaintext() {
        let secret = "sk-test-abcdefghijklmnop";
        let envelope = encrypt(&key(), secret).expect("encrypt");
        assert!(envelope.starts_with("v1."));
        assert!(!envelope.contains(secret));
        assert_eq!(decrypt(&key(), &envelope).expect("decrypt"), secret);
    }

    #[test]
    fn rejects_tampered_and_wrong_key_envelopes() {
        let envelope = encrypt(&key(), "another-secret-value").expect("encrypt");
        assert!(decrypt(&[9_u8; KEY_BYTES], &envelope).is_err());
        assert!(decrypt(&key(), "v1.bm90LWEtbm9uY2U=.AAAA").is_err());
        assert!(decrypt(&key(), "garbage").is_err());
    }

    #[test]
    fn hint_shows_only_the_tail() {
        assert_eq!(hint("sk-1234567890wxyz"), "••••wxyz");
        assert_eq!(hint(""), "••••");
    }

    #[test]
    fn decrypts_the_cross_language_test_vector() {
        // Shared with the TypeScript test `platform-crypto.test.ts`.
        let key: [u8; KEY_BYTES] = STANDARD
            .decode("BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=")
            .expect("key")
            .try_into()
            .expect("32 bytes");
        let envelope = "v1.BQUFBQUFBQUFBQUF.koPoSrSwGVySEDhd8Qj6wHowbWCHsTCmejbxTN2OTMcYZmNhQFSL";
        assert_eq!(
            decrypt(&key, envelope).expect("decrypt"),
            "probe-forge-test-vector"
        );
    }
}
