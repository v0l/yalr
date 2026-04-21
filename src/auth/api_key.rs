use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Generate a random API key in the format `sk_xxxxx` (24 random chars, base64 encoded)
pub fn generate_api_key() -> String {
    let mut bytes = [0u8; 18]; // 18 bytes -> exactly 24 base64 chars (no padding)
    OsRng.fill_bytes(&mut bytes);
    let key = URL_SAFE_NO_PAD.encode(bytes);
    format!("sk_{}", &key[..24])
}

/// Hash an API key for secure storage
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract the last 4 characters of an API key for display
pub fn get_last_four(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() > 4 {
        chars[chars.len() - 4..].iter().collect()
    } else {
        key.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_api_key_format() {
        let key = generate_api_key();
        assert!(key.starts_with("sk_"));
        assert_eq!(key.len(), 27); // "sk_" + 24 chars
    }

    #[test]
    fn test_hash_api_key() {
        let key = "sk_test123";
        let hash = hash_api_key(key);
        assert_eq!(hash.len(), 64); // SHA256 hex length
    }

    #[test]
    fn test_get_last_four() {
        assert_eq!(get_last_four("sk_abc123"), "c123");
        assert_eq!(get_last_four("sk_1234567890"), "7890");
        assert_eq!(get_last_four("sk_123"), "_123");
    }
}
