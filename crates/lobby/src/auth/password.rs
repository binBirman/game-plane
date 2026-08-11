use anyhow::{anyhow, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash error: {e}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(hash).map_err(|e| anyhow!("argon2 parse error: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Password policy: length >= 8 (Unicode chars; bytes-only limit could be
/// added if abuse appears in metrics). Complexity rules (digit/letter/special)
/// intentionally dropped — users find them hostile and length is the
/// dominant factor against brute force once we hash with argon2.
pub fn validate_strength(password: &str) -> Result<(), &'static str> {
    let len = password.chars().count();
    if len < 8 {
        return Err("password must be at least 8 characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_eight_or_more() {
        assert!(validate_strength("abcdefgh").is_ok());         // 8 lowercase
        assert!(validate_strength("12345678").is_ok());         // 8 digits
        assert!(validate_strength("Test1234").is_ok());         // 8 mixed
        assert!(validate_strength("Secret_123_long_password").is_ok()); // long
    }

    #[test]
    fn rejects_below_eight() {
        assert!(validate_strength("").is_err());
        assert!(validate_strength("a").is_err());
        assert!(validate_strength("1234567").is_err());         // 7 chars
    }
}