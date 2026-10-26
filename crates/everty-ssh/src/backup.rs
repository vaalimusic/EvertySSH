use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::ExportBundle;

const AAD: &[u8] = b"EvertySSH backup v2; Argon2id v19 m=19456,t=2,p=1; XChaCha20Poly1305";
pub const MAX_BACKUP_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

pub fn encrypt(bundle: &ExportBundle, password: &str) -> Result<Vec<u8>, String> {
    if password.chars().count() < 8 {
        return Err("Use at least 8 characters for the backup password".into());
    }
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    OsRng.try_fill_bytes(&mut salt).map_err(|e| e.to_string())?;
    OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|e| e.to_string())?;
    let key = derive_key(password, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|e| e.to_string())?;
    let plain = Zeroizing::new(serde_json::to_vec(bundle).map_err(|e| e.to_string())?);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: AAD,
            },
        )
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&Envelope {
        version: 2,
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err("Backup exceeds 16 MiB".into());
    }
    Ok(bytes)
}

pub fn decrypt(bytes: &[u8], password: &str) -> Result<ExportBundle, String> {
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err("Backup exceeds 16 MiB".into());
    }
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| "Invalid encrypted backup")?;
    if envelope.version != 2 {
        return Err("Unsupported backup version".into());
    }
    let salt = STANDARD.decode(envelope.salt).map_err(|e| e.to_string())?;
    let nonce = STANDARD.decode(envelope.nonce).map_err(|e| e.to_string())?;
    if salt.len() != 16 || nonce.len() != 24 {
        return Err("Invalid backup salt or nonce".into());
    }
    let key = derive_key(password, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|e| e.to_string())?;
    let ciphertext = STANDARD
        .decode(envelope.ciphertext)
        .map_err(|e| e.to_string())?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: AAD,
                },
            )
            .map_err(|_| "Wrong backup password or damaged backup")?,
    );
    let bundle: ExportBundle = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;
    if bundle.version != 2 {
        return Err("Unsupported backup contents".into());
    }
    Ok(bundle)
}

/// Import old explicitly selected v1 JSON files without treating a damaged v2
/// envelope as plaintext. New exports are always encrypted v2 envelopes.
pub fn import(bytes: &[u8], password: &str) -> Result<ExportBundle, String> {
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err("Backup exceeds 16 MiB".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if value.get("version").and_then(|v| v.as_u64()) == Some(1) && value.get("ciphertext").is_none()
    {
        let mut bundle: ExportBundle = serde_json::from_value(value).map_err(|e| e.to_string())?;
        bundle.version = 2;
        Ok(bundle)
    } else {
        decrypt(bytes, password)
    }
}

fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let mut key = Zeroizing::new([0u8; 32]);
    let params = Params::new(19456, 2, 1, Some(32)).map_err(|e| e.to_string())?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_backup_authenticates_password_and_contents() {
        let mut bundle = ExportBundle {
            version: 2,
            exported_at: "now".into(),
            hosts: crate::config::sample_hosts_document().hosts,
            settings: Default::default(),
        };
        bundle.hosts[0].auth = crate::config::AuthConfig::Password {
            password: Some("synthetic secret".into()),
        };
        let a = encrypt(&bundle, "backup password").unwrap();
        let b = encrypt(&bundle, "backup password").unwrap();
        assert_ne!(a, b);
        assert!(!String::from_utf8_lossy(&a).contains("web01.example.com"));
        assert!(!String::from_utf8_lossy(&a).contains("synthetic secret"));
        let restored = decrypt(&a, "backup password").unwrap();
        assert!(
            matches!(&restored.hosts[0].auth, crate::config::AuthConfig::Password { password: Some(value) } if value == "synthetic secret")
        );
        assert_eq!(decrypt(&a, "backup password").unwrap().hosts.len(), 2);
        assert!(decrypt(&a, "wrong password").is_err());
        let mut envelope: Envelope = serde_json::from_slice(&a).unwrap();
        let mut ciphertext = STANDARD.decode(&envelope.ciphertext).unwrap();
        ciphertext[0] ^= 1;
        envelope.ciphertext = STANDARD.encode(ciphertext);
        assert!(decrypt(&serde_json::to_vec(&envelope).unwrap(), "backup password").is_err());
        assert!(decrypt(br#"{"version":1,"hosts":[]}"#, "backup password").is_err());
    }

    #[test]
    fn legacy_import_is_explicitly_versioned_and_does_not_downgrade_v2() {
        let legacy = ExportBundle {
            version: 1,
            exported_at: "now".into(),
            hosts: crate::config::sample_hosts_document().hosts,
            settings: Default::default(),
        };
        let restored = import(&serde_json::to_vec(&legacy).unwrap(), "").unwrap();
        assert_eq!(restored.version, 2);
        assert_eq!(restored.hosts.len(), 2);
        assert!(import(br#"{"version":2,"hosts":[]}"#, "").is_err());
        assert!(import(br#"{"version":1,"ciphertext":"broken"}"#, "").is_err());
    }
}
