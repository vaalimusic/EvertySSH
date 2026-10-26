use super::*;
use crate::{config::sample_hosts_document, secrets::MemorySecrets};

#[test]
fn migration_moves_passwords_and_passphrases_to_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.toml");
    let secrets = Arc::new(MemorySecrets::default());
    let mut doc = sample_hosts_document();
    doc.hosts[0].auth = AuthConfig::Password {
        password: Some("secret-password".into()),
    };
    if let AuthConfig::Key { passphrase, .. } = &mut doc.hosts[1].auth {
        *passphrase = Some("secret-passphrase".into());
    }
    fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
    let mut store = ConfigStore::new(path.clone(), secrets.clone());
    let mut loaded = store.load().unwrap();
    let disk = fs::read_to_string(&path).unwrap();
    assert!(!disk.contains("secret-password"));
    assert!(!disk.contains("secret-passphrase"));
    assert!(disk.contains("secret_ref"));
    assert_eq!(secrets.0.lock().unwrap().len(), 2);
    store.save(&mut loaded).unwrap();
    assert_eq!(
        secrets.0.lock().unwrap().len(),
        2,
        "Unchanged saves reuse credentials"
    );
    let reloaded = ConfigStore::new(path, secrets).load().unwrap();
    assert_eq!(secret(&reloaded.hosts[0].auth), Some("secret-password"));
    assert_eq!(secret(&reloaded.hosts[1].auth), Some("secret-passphrase"));
}

#[test]
fn malformed_config_cannot_be_replaced_by_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.toml");
    fs::write(&path, "broken = [").unwrap();
    let mut store = ConfigStore::new(path.clone(), Arc::new(MemorySecrets::default()));
    assert!(store.load().is_err());
    assert!(store.save(&mut sample_hosts_document()).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "broken = [");
}

#[test]
fn external_edits_are_preserved() {
    let mut store = ConfigStore::temporary();
    let mut doc = store.load().unwrap();
    store.save(&mut doc).unwrap();
    let external = format!(
        "{}\n# edited elsewhere\n",
        fs::read_to_string(&store.path).unwrap()
    );
    fs::write(&store.path, &external).unwrap();
    doc.hosts.clear();
    assert!(store
        .save(&mut doc)
        .unwrap_err()
        .contains("changed on disk"));
    assert_eq!(fs::read_to_string(&store.path).unwrap(), external);
}

struct UnavailableSecrets;
impl SecretStore for UnavailableSecrets {
    fn get(&self, _: &str) -> Result<String, String> {
        Err("Keychain locked".into())
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("Keychain locked".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn credential_failure_preserves_legacy_file_and_does_not_fallback_to_plaintext() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.toml");
    let mut doc = sample_hosts_document();
    doc.hosts[0].auth = AuthConfig::Password {
        password: Some("legacy-secret".into()),
    };
    let original = toml::to_string(&doc).unwrap();
    fs::write(&path, &original).unwrap();
    assert!(ConfigStore::new(path.clone(), Arc::new(UnavailableSecrets))
        .load()
        .is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn missing_referenced_credential_stops_loading() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.toml");
    let mut doc = sample_hosts_document();
    doc.hosts[0].auth = AuthConfig::Password { password: None };
    doc.hosts[0].secret_ref = Some("missing".into());
    fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
    assert!(ConfigStore::new(path, Arc::new(MemorySecrets::default()))
        .load()
        .is_err());
}

#[test]
fn atomic_replacement_leaves_complete_file_and_no_temporary_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.toml");
    atomic_write(&path, b"old").unwrap();
    atomic_write(&path, b"new complete contents").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"new complete contents");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

struct EditingSecrets {
    path: PathBuf,
    inner: MemorySecrets,
}

impl SecretStore for EditingSecrets {
    fn get(&self, id: &str) -> Result<String, String> {
        self.inner.get(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.inner.set(id, value)?;
        fs::write(&self.path, "# edited while keychain was open\n").map_err(|e| e.to_string())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        self.inner.delete(id)
    }
}

#[test]
fn edits_during_credential_write_abort_save_and_rollback_new_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hosts.toml");
    let secrets = Arc::new(EditingSecrets {
        path: path.clone(),
        inner: MemorySecrets::default(),
    });
    let mut store = ConfigStore::new(path.clone(), secrets.clone());
    let mut doc = store.load().unwrap();
    doc.hosts[0].auth = AuthConfig::Password {
        password: Some("synthetic secret".into()),
    };
    assert!(store.save(&mut doc).is_err());
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "# edited while keychain was open\n"
    );
    assert!(secrets.inner.0.lock().unwrap().is_empty());
    assert!(doc.hosts[0].secret_ref.is_none());
}
