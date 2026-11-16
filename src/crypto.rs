use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine as _};

const NONCE_LEN: usize = 12;

#[derive(Debug, PartialEq, Eq)]
pub enum CryptoError {
    BadKey,
    BadCiphertext,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::BadKey => write!(f, "encryption key must be 64 hex characters"),
            CryptoError::BadCiphertext => write!(f, "ciphertext could not be decrypted"),
        }
    }
}

impl std::error::Error for CryptoError {}

#[derive(Clone)]
pub struct SecretBox {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox")
    }
}

impl SecretBox {
    pub fn from_hex(hex_key: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(hex_key.trim()).map_err(|_| CryptoError::BadKey)?;
        if bytes.len() != 32 {
            return Err(CryptoError::BadKey);
        }
        Ok(Self { cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&bytes)) })
    }

    pub fn encrypt(&self, plaintext: &str) -> String {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        out.extend(self.cipher.encrypt(&nonce, plaintext.as_bytes()).expect("aes-gcm encrypt"));
        STANDARD.encode(out)
    }

    pub fn decrypt(&self, encoded: &str) -> Result<String, CryptoError> {
        let raw = STANDARD.decode(encoded).map_err(|_| CryptoError::BadCiphertext)?;
        if raw.len() <= NONCE_LEN {
            return Err(CryptoError::BadCiphertext);
        }
        let (nonce, body) = raw.split_at(NONCE_LEN);
        let plain = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), body)
            .map_err(|_| CryptoError::BadCiphertext)?;
        String::from_utf8(plain).map_err(|_| CryptoError::BadCiphertext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f000102030405060708090a0b0c0d0e0f";

    #[test]
    fn round_trips_and_never_stores_plaintext() {
        let secret = SecretBox::from_hex(KEY).unwrap();
        let sealed = secret.encrypt("cust_123");
        assert!(!sealed.contains("cust_123"));
        assert_ne!(sealed, secret.encrypt("cust_123"));
        assert_eq!(secret.decrypt(&sealed).unwrap(), "cust_123");
    }

    #[test]
    fn a_different_key_or_tampering_fails() {
        let secret = SecretBox::from_hex(KEY).unwrap();
        let other = SecretBox::from_hex(&KEY.replace('0', "1")).unwrap();
        let sealed = secret.encrypt("cust_123");
        assert_eq!(other.decrypt(&sealed), Err(CryptoError::BadCiphertext));
        assert_eq!(secret.decrypt("AAAA"), Err(CryptoError::BadCiphertext));
        assert_eq!(SecretBox::from_hex("abcd").unwrap_err(), CryptoError::BadKey);
    }
}
