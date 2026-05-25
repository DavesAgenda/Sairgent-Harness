use crate::error::{KernelError, Result};
use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use sha2::Sha256;
use std::env;

const VAULT_BLOB_VERSION: u8 = 0x01;

fn derive_aes_key(master_key: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, master_key);
    let mut okm = [0u8; 32];
    hk.expand(b"sairgent-vault-v1", &mut okm)
        .expect("HKDF expand failed");
    okm
}

/// The Vault manages secure encryption and decryption of sensitive strings
/// (such as API keys), ensuring they are kept at rest safely.
pub struct Vault {
    cipher: Aes256Gcm,
    pub key_version: u8,
}

impl Vault {
    /// Initialize a new Vault using a raw key (any length); derives a 32-byte
    /// AES key via HKDF-SHA256 with a fixed info string.
    pub fn new(key_str: &str) -> Result<Self> {
        let raw_key = key_str.as_bytes();
        let derived = derive_aes_key(raw_key);
        let key = Key::<Aes256Gcm>::from_slice(&derived);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
            key_version: 1,
        })
    }

    /// Load the vault from the `SAIRGENT_VAULT_KEY` environment variable.
    pub fn from_env() -> Result<Self> {
        let key_str = env::var("SAIRGENT_VAULT_KEY")
            .map_err(|_| KernelError::VaultError("SAIRGENT_VAULT_KEY not set".to_string()))?;
        Self::new(&key_str)
    }

    /// Encrypt a plaintext string into a base64 (url-safe) string.
    /// The raw blob is: [version_byte (1)] || [nonce (12)] || [ciphertext].
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bits; unique per message
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| KernelError::VaultError(format!("Encryption failed: {:?}", e)))?;

        // Pack version byte, nonce, and ciphertext together
        let mut combined = Vec::with_capacity(1 + 12 + ciphertext.len());
        combined.push(VAULT_BLOB_VERSION);
        combined.extend_from_slice(&nonce);
        combined.extend_from_slice(&ciphertext);

        Ok(URL_SAFE_NO_PAD.encode(combined))
    }

    /// Decrypt a base64 (url-safe) string back into plaintext.
    /// Expects a blob with a leading version byte.
    pub fn decrypt(&self, encrypted_b64: &str) -> Result<String> {
        let combined = URL_SAFE_NO_PAD
            .decode(encrypted_b64)
            .map_err(|e| KernelError::VaultError(format!("Base64 decode failed: {:?}", e)))?;

        // Minimum: 1 (version) + 12 (nonce) + 1 (at least one ciphertext byte)
        if combined.len() < 14 {
            return Err(KernelError::VaultError(
                "Invalid ciphertext payload length".to_string(),
            ));
        }

        let version = combined[0];
        if version != VAULT_BLOB_VERSION {
            return Err(KernelError::VaultError(format!(
                "Unsupported vault blob version: {:#04x}",
                version
            )));
        }

        let (nonce_bytes, ciphertext_bytes) = combined[1..].split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext_bytes = self
            .cipher
            .decrypt(nonce, ciphertext_bytes)
            .map_err(|e| KernelError::VaultError(format!("Decryption failed: {:?}", e)))?;

        String::from_utf8(plaintext_bytes).map_err(|e| {
            KernelError::VaultError(format!("Invalid UTF-8 in decrypted string: {:?}", e))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_encryption_decryption() {
        let key = "12345678901234567890123456789012"; // 32 bytes
        let vault = Vault::new(key).unwrap();

        let secret = "TEST_SECRET_NOT_A_REAL_KEY_SAIRGENT";
        let encrypted = vault.encrypt(secret).unwrap();
        let decrypted = vault.decrypt(&encrypted).unwrap();

        assert_eq!(secret, decrypted);
        assert_ne!(secret, encrypted);
    }
}
