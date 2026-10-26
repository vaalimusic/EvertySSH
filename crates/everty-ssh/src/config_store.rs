use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    config::{AuthConfig, HostsDocument},
    secrets::SecretStore,
};

pub struct ConfigStore {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    expected: Option<Vec<u8>>,
    loaded: bool,
    #[cfg(test)]
    _temp: Option<tempfile::TempDir>,
}

impl ConfigStore {
    pub fn new(path: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            path,
            secrets,
            expected: None,
            loaded: false,
            #[cfg(test)]
            _temp: None,
        }
    }

    #[cfg(test)]
    pub fn temporary() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Self::new(
            temp.path().join("hosts.toml"),
            Arc::new(crate::secrets::MemorySecrets::default()),
        );
        store._temp = Some(temp);
        store
    }

    pub fn load(&mut self) -> Result<HostsDocument, String> {
        self.loaded = false;
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Cannot read {}: {e}", self.path.display())),
        };
        let mut doc = match &bytes {
            Some(bytes) => toml::from_str::<HostsDocument>(
                std::str::from_utf8(bytes).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("Invalid configuration {}: {e}", self.path.display()))?,
            None => crate::config::sample_hosts_document(),
        };
        validate(&doc)?;
        let migrate = doc.hosts.iter().any(|host| secret(&host.auth).is_some());
        for host in &mut doc.hosts {
            if secret(&host.auth).is_none() {
                if let Some(id) = &host.secret_ref {
                    set_secret(&mut host.auth, Some(self.secrets.get(id)?));
                }
            }
        }
        self.expected = bytes;
        self.loaded = true;
        if migrate {
            self.save(&mut doc)?;
        }
        Ok(doc)
    }

    pub fn save(&mut self, doc: &mut HostsDocument) -> Result<(), String> {
        if !self.loaded {
            return Err("Configuration was not loaded successfully; writing is disabled".into());
        }
        validate(doc)?;
        let parent = self
            .path
            .parent()
            .ok_or("Missing configuration directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // Serialize writers from separate app instances. The snapshot checks
        // additionally protect edits made by tools that do not use this lock.
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|e| format!("Configuration is being saved by another process: {e}"))?;
        let current = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if current != self.expected {
            return Err(
                "Configuration changed on disk. Restart before saving to avoid overwriting it."
                    .into(),
            );
        }
        let mut persisted = doc.clone();
        let mut created = Vec::new();
        let result = (|| {
            for host in &mut persisted.hosts {
                if let Some(value) = secret(&host.auth) {
                    let existing_matches = host
                        .secret_ref
                        .as_ref()
                        .is_some_and(|id| self.secrets.get(id).is_ok_and(|stored| stored == value));
                    if !existing_matches {
                        let id = uuid::Uuid::new_v4().to_string();
                        self.secrets.set(&id, value)?;
                        created.push(id.clone());
                        host.secret_ref = Some(id);
                    }
                } else {
                    host.secret_ref = None;
                }
                set_secret(&mut host.auth, None);
            }
            let bytes = toml::to_string_pretty(&persisted)
                .map_err(|e| e.to_string())?
                .into_bytes();
            let latest = match fs::read(&self.path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            };
            if latest != self.expected {
                return Err("Configuration changed while credentials were being saved. Restart before saving.".into());
            }
            atomic_write(&self.path, &bytes)?;
            self.expected = Some(bytes);
            Ok::<(), String>(())
        })();
        if result.is_err() {
            for id in created {
                let _ = self.secrets.delete(&id);
            }
            return result;
        }
        for (host, saved) in doc.hosts.iter_mut().zip(&persisted.hosts) {
            host.secret_ref.clone_from(&saved.secret_ref);
        }
        Ok(())
    }
}

pub fn validate(doc: &HostsDocument) -> Result<(), String> {
    let mut ids = HashSet::new();
    if !(100..=100_000).contains(&doc.settings.scrollback_lines)
        || !doc.settings.font_size.is_finite()
        || !(6.0..=72.0).contains(&doc.settings.font_size)
    {
        return Err("Invalid terminal settings".into());
    }
    for host in &doc.hosts {
        if host.id.is_empty()
            || !ids.insert(&host.id)
            || host.port == 0
            || host.hostname.trim().is_empty()
            || host.username.trim().is_empty()
        {
            return Err("Invalid or duplicate host in configuration".into());
        }
    }
    Ok(())
}

fn secret(auth: &AuthConfig) -> Option<&str> {
    match auth {
        AuthConfig::Key { passphrase, .. } => passphrase.as_deref(),
        AuthConfig::Password { password } => password.as_deref(),
        AuthConfig::Agent => None,
    }
}

fn set_secret(auth: &mut AuthConfig, value: Option<String>) {
    match auth {
        AuthConfig::Key { passphrase, .. } => *passphrase = value,
        AuthConfig::Password { password } => *password = value,
        AuthConfig::Agent => {}
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("Missing parent directory")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    // Once rename succeeds the new file is committed. Never report a rollback
    // after this point: callers would otherwise delete its new credentials.
    {
        let _ = fs::File::open(dir).and_then(|file| file.sync_all());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
