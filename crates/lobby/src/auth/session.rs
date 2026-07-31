use anyhow::Result;
use rand::RngCore;

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    to_hex(&bytes)
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn expires_at(ttl_days: i64) -> Result<String> {
    let now = chrono::Utc::now();
    let exp = now + chrono::Duration::days(ttl_days);
    Ok(exp.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}