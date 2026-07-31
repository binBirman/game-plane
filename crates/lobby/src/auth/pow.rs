use rand::RngCore;
use sha2::{Digest, Sha256};

/// Default proof-of-work difficulty (leading zero bits required).
pub const DEFAULT_DIFFICULTY: u32 = 16;

/// Generate a fresh challenge (32 hex chars = 16 random bytes).
pub fn issue(difficulty: u32) -> (String, u32) {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    (hex(&bytes), difficulty)
}

/// Stateless verification: any `(challenge, nonce)` pair whose SHA256 has
/// at least `difficulty` leading zero bits is accepted.
pub fn verify(challenge: &str, nonce: &str, difficulty: u32) -> bool {
    let input = format!("{}:{}", challenge, nonce);
    let hash = Sha256::digest(input.as_bytes());
    leading_zero_bits(&hash) >= difficulty
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let mut bits = 0u32;
    for byte in hash {
        if *byte == 0 {
            bits += 8;
        } else {
            bits += byte.leading_zeros();
            return bits;
        }
    }
    bits
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_challenge_is_hex_and_unique() {
        let (a, _) = issue(16);
        let (b, _) = issue(16);
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn verify_rejects_wrong_nonce() {
        let (c, _) = issue(0);
        assert!(!verify(&c, "0", 16));
    }

    #[test]
    fn solve_and_verify_roundtrip() {
        let (c, _) = issue(8);
        let mut nonce: u64 = 0;
        loop {
            let input = format!("{}:{}", c, nonce);
            let h = Sha256::digest(input.as_bytes());
            if leading_zero_bits(&h) >= 8 {
                break;
            }
            nonce += 1;
        }
        assert!(verify(&c, &nonce.to_string(), 8));
    }
}