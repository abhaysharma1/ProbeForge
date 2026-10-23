use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, errors::Error as PasswordHashError},
};

pub const MIN_PASSWORD_CHARS: usize = 12;
pub const MAX_PASSWORD_BYTES: usize = 256;
const SALT_BYTES: usize = 16;

/// Validate credentials before any expensive hashing or database work.
pub fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err("Password must be at least 12 characters.");
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("Password must be at most 256 bytes.");
    }
    Ok(())
}

/// Hash with Argon2id using the maintained RustCrypto implementation.
/// The returned PHC string includes the algorithm, parameters, and salt.
pub fn hash_password(password: &str) -> Result<String, PasswordHashError> {
    let mut salt_bytes = [0_u8; SALT_BYTES];
    fill_random(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

/// Verify an Argon2id PHC string. Malformed hashes fail closed.
pub fn verify_password(password: &str, encoded_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded_hash) else {
        tracing::warn!("stored password hash is malformed");
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Spend comparable work when the account does not exist so login timing does
/// not disclose whether an email is registered.
pub fn dummy_verify(password: &str) -> bool {
    if let Ok(hash) = hash_password(password) {
        let _ = verify_password(password, &hash);
    }
    false
}

/// Restrict emails to ASCII so PostgreSQL `lower(email)` and application
/// normalization cannot disagree on Unicode case folding.
pub fn normalize_email(value: &str) -> Option<String> {
    let email = value.trim().to_ascii_lowercase();
    if email.len() < 3 || email.len() > 320 || !email.is_ascii() {
        return None;
    }
    if email.chars().any(char::is_whitespace) || email.chars().any(char::is_control) {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return None;
    }
    Some(email)
}

pub(crate) fn fill_random(bytes: &mut [u8]) {
    getrandom::fill(bytes).expect("the operating system random source must be available");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_with_argon2id_and_verifies() {
        let hash = hash_password("correct horse battery staple").expect("hash");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn malformed_hash_fails_closed() {
        assert!(!verify_password("anything", "not-a-phc-string"));
    }

    #[test]
    fn enforces_password_bounds() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("long enough password").is_ok());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
    }

    #[test]
    fn normalizes_only_bounded_ascii_emails() {
        assert_eq!(
            normalize_email("  Owner@Example.Test ").as_deref(),
            Some("owner@example.test")
        );
        assert!(normalize_email("not-an-email").is_none());
        assert!(normalize_email("üser@example.test").is_none());
        assert!(normalize_email("a@b@c.test").is_none());
        assert!(normalize_email("a@localhost").is_none());
    }
}
