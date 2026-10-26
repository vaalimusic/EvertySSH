//! Credential storage is injectable so tests never access the user's keychain.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<String, String>;
    fn set(&self, id: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

pub struct SystemSecrets;

impl SecretStore for SystemSecrets {
    fn get(&self, id: &str) -> Result<String, String> {
        keyring::Entry::new("EvertySSH", id)
            .and_then(|entry| entry.get_password())
            .map_err(|e| format!("Credential store: {e}"))
    }

    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new("EvertySSH", id)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| format!("Credential store: {e}"))
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        match keyring::Entry::new("EvertySSH", id).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Credential store: {e}")),
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct MemorySecrets(pub std::sync::Mutex<std::collections::HashMap<String, String>>);

#[cfg(test)]
impl SecretStore for MemorySecrets {
    fn get(&self, id: &str) -> Result<String, String> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or("Missing credential".into())
    }
    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(id.into(), secret.into());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires an unlocked OS credential store; uses and removes a synthetic credential"]
    fn system_credential_roundtrip() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = SystemSecrets.delete(&self.0);
            }
        }
        let id = Cleanup(format!("test-{}", uuid::Uuid::new_v4()));
        SystemSecrets
            .set(&id.0, "evertyssh synthetic credential")
            .unwrap();
        assert_eq!(
            SystemSecrets.get(&id.0).unwrap(),
            "evertyssh synthetic credential"
        );
        SystemSecrets.delete(&id.0).unwrap();
        assert!(SystemSecrets.get(&id.0).is_err());
    }
}
