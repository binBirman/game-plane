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

/// Password policy:
///   - length >= 9
///   - at least one ASCII digit
///   - at least one ASCII letter
///   - at least one non-alphanumeric character (special)
pub fn validate_strength(password: &str) -> Result<(), &'static str> {
    let len = password.chars().count();
    if len < 9 {
        return Err("password must be at least 9 characters");
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err("password must contain a digit");
    }
    if !password.chars().any(|c| c.is_ascii_alphabetic()) {
        return Err("password must contain a letter");
    }
    if !password.chars().any(|c| !c.is_alphanumeric()) {
        return Err("password must contain a special character");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_strong_password() {
        assert!(validate_strength("Secret_123").is_ok());
        assert!(validate_strength("abc123XYZ!").is_ok());
        assert!(validate_strength("passw0rd!").is_ok());
    }

    #[test]
    fn rejects_short() {
        assert!(validate_strength("Ab1!").is_err());
        assert!(validate_strength("Aa1!2345").is_err()); // 8 chars
    }

    #[test]
    fn rejects_missing_classes() {
        assert!(validate_strength("nodigits!!").is_err());
        assert!(validate_strength("NOLOWER123!").is_ok()); // upper is still letter
        assert!(validate_strength("ONLYLetters!").is_err()); // no digit
        assert!(validate_strength("NoSpecial123").is_err());
    }
}