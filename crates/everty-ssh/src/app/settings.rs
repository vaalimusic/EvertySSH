use super::*;

impl AppState {
    pub(super) fn commit_document(&mut self, mut proposed: HostsDocument) -> Result<(), String> {
        self.config_store.save(&mut proposed)?;
        self.hosts_doc = proposed;
        Ok(())
    }

    pub(super) fn show_error(&self, error: &str) {
        #[cfg(not(test))]
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("EvertySSH")
            .set_description(error)
            .show();
        #[cfg(test)]
        eprintln!("{error}");
    }

    pub fn open_settings_modal(&mut self) {
        let has_existing = self.hosts_doc.settings.master_password_hash.is_some();
        self.settings_modal = Some(SettingsModal {
            is_open: true,
            active_tab: SettingsTab::About,
            current_password: TextInput::default(),
            new_password: TextInput::default(),
            confirm_password: TextInput::default(),
            active_security_field: if has_existing { 0 } else { 1 },
            security_msg: None,
            backup_msg: None,
            backup_password: TextInput::default(),
            backup_confirm: TextInput::default(),
            default_user: TextInput::new(&self.hosts_doc.settings.default_user),
            scrollback_lines: TextInput::new(self.hosts_doc.settings.scrollback_lines.to_string()),
            confirm_delete: self.hosts_doc.settings.confirm_delete,
            record_command_history: self.hosts_doc.settings.record_command_history,
            active_general_field: 0,
            general_msg: None,
            show_passwords: false,
        });
        self.focused_input = None;
    }

    pub fn toggle_language(&mut self) {
        self.set_language(self.hosts_doc.settings.language.toggle());
    }

    pub fn set_language(&mut self, lang: AppLanguage) {
        let mut proposed = self.hosts_doc.clone();
        proposed.settings.language = lang;
        if let Err(e) = self.commit_document(proposed) {
            self.show_error(&e);
        }
    }

    pub fn save_master_password(&mut self) {
        let Some(modal) = &self.settings_modal else {
            return;
        };
        let result = (|| {
            if let Some(existing) = &self.hosts_doc.settings.master_password_hash {
                if !verify_password(&modal.current_password.text, existing) {
                    return Err("Текущий пароль введён неверно / Incorrect current password".into());
                }
            }
            let password = &modal.new_password.text;
            if password.chars().count() < 8 {
                return Err("Минимум 8 символов / Use at least 8 characters".into());
            }
            if password != &modal.confirm_password.text {
                return Err("Пароли не совпадают / Passwords do not match".into());
            }
            let mut proposed = self.hosts_doc.clone();
            proposed.settings.master_password_hash = Some(hash_password(password)?);
            Ok(proposed)
        })()
        .and_then(|proposed| self.commit_document(proposed));
        if let Some(modal) = &mut self.settings_modal {
            match result {
                Ok(()) => {
                    modal.current_password = TextInput::default();
                    modal.new_password = TextInput::default();
                    modal.confirm_password = TextInput::default();
                    modal.security_msg = Some(("Пароль сохранён / Password saved".into(), false));
                }
                Err(e) => modal.security_msg = Some((e, true)),
            }
        }
    }

    pub fn remove_master_password(&mut self) {
        let Some(modal) = &self.settings_modal else {
            return;
        };
        if self
            .hosts_doc
            .settings
            .master_password_hash
            .as_ref()
            .is_some_and(|hash| !verify_password(&modal.current_password.text, hash))
        {
            self.settings_modal.as_mut().unwrap().security_msg = Some((
                "Неверный текущий пароль / Incorrect current password".into(),
                true,
            ));
            return;
        }
        let mut proposed = self.hosts_doc.clone();
        proposed.settings.master_password_hash = None;
        let result = self.commit_document(proposed);
        if let Some(modal) = &mut self.settings_modal {
            match result {
                Ok(()) => {
                    modal.current_password = TextInput::default();
                    modal.new_password = TextInput::default();
                    modal.confirm_password = TextInput::default();
                    modal.security_msg =
                        Some(("Блокировка отключена / Lock disabled".into(), false));
                }
                Err(e) => modal.security_msg = Some((e, true)),
            }
        }
    }

    pub fn try_unlock_app(&mut self) -> bool {
        if let Some(hash) = &self.hosts_doc.settings.master_password_hash {
            // Older versions trimmed passwords before hashing.
            let legacy = !hash.starts_with("$argon2id$");
            let entered = if legacy {
                self.lock_password_input.text.trim()
            } else {
                &self.lock_password_input.text
            };
            if !verify_password(entered, hash) {
                self.lock_error_msg = Some("Неверный пароль / Incorrect password".into());
                return false;
            }
            if legacy {
                let result = hash_password(entered).and_then(|hash| {
                    let mut proposed = self.hosts_doc.clone();
                    proposed.settings.master_password_hash = Some(hash);
                    self.commit_document(proposed)
                });
                if let Err(e) = result {
                    self.lock_error_msg = Some(e);
                    return false;
                }
            }
        }
        self.is_app_locked = false;
        self.lock_password_input = TextInput::default();
        self.lock_error_msg = None;
        true
    }

    pub fn export_backup_dialog(&mut self) {
        let Some(modal) = &self.settings_modal else {
            return;
        };
        if modal.backup_password.text != modal.backup_confirm.text {
            self.settings_modal.as_mut().unwrap().backup_msg =
                Some(("Пароли не совпадают / Passwords do not match".into(), true));
            return;
        }
        let mut bundle = ExportBundle {
            version: 2,
            exported_at: chrono::Utc::now().to_rfc3339(),
            hosts: self.hosts_doc.hosts.clone(),
            settings: self.hosts_doc.settings.clone(),
        };
        for host in &mut bundle.hosts {
            host.secret_ref = None;
        }
        let bytes = match crate::backup::encrypt(&bundle, &modal.backup_password.text) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.settings_modal.as_mut().unwrap().backup_msg = Some((e, true));
                return;
            }
        };
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Encrypted EvertySSH backup")
            .set_file_name("evertyssh-backup.evertybackup")
            .add_filter("EvertySSH", &["evertybackup"])
            .save_file()
        {
            let result = crate::config_store::atomic_write(&path, &bytes);
            if let Some(modal) = &mut self.settings_modal {
                modal.backup_msg = Some(match result {
                    Ok(()) => (
                        "Зашифрованная копия сохранена / Encrypted backup saved".into(),
                        false,
                    ),
                    Err(e) => (e, true),
                });
                modal.backup_password = TextInput::default();
                modal.backup_confirm = TextInput::default();
            }
        }
    }

    pub fn import_backup_dialog(&mut self) {
        let Some(modal) = &self.settings_modal else {
            return;
        };
        let password = zeroize::Zeroizing::new(modal.backup_password.text.clone());
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import EvertySSH backup")
            .add_filter("EvertySSH", &["evertybackup", "json"])
            .pick_file()
        else {
            return;
        };
        let result = (|| {
            use std::io::{Read, Take};
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut reader: Take<std::fs::File> = file.take(crate::backup::MAX_BACKUP_BYTES + 1);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            let bundle = crate::backup::import(&bytes, &password)?;
            self.merge_backup(bundle)
        })();
        if let Some(modal) = &mut self.settings_modal {
            modal.backup_password = TextInput::default();
            modal.backup_confirm = TextInput::default();
            modal.backup_msg = Some(match result {
                Ok(count) => (
                    format!("Добавлено серверов / Servers added: {count}"),
                    false,
                ),
                Err(e) => (e, true),
            });
        }
    }

    pub(super) fn merge_backup(&mut self, bundle: ExportBundle) -> Result<usize, String> {
        if bundle.version != 2 {
            return Err("Unsupported backup version".into());
        }
        crate::config_store::validate(&HostsDocument {
            hosts: bundle.hosts.clone(),
            settings: bundle.settings.clone(),
        })?;
        let mut proposed = self.hosts_doc.clone();
        let mut added = 0;
        for mut host in bundle.hosts {
            host.secret_ref = None;
            if !proposed.hosts.iter().any(|existing| existing.id == host.id) {
                proposed.hosts.push(host);
                added += 1;
            }
        }
        let local_lock = proposed.settings.master_password_hash.clone();
        proposed.settings = bundle.settings;
        proposed.settings.master_password_hash = local_lock;
        self.commit_document(proposed)?;
        if let Some(modal) = &mut self.settings_modal {
            modal
                .default_user
                .set(&self.hosts_doc.settings.default_user);
            modal
                .scrollback_lines
                .set(self.hosts_doc.settings.scrollback_lines.to_string());
            modal.confirm_delete = self.hosts_doc.settings.confirm_delete;
            modal.record_command_history = self.hosts_doc.settings.record_command_history;
        }
        Ok(added)
    }

    pub fn save_general_settings(&mut self) {
        let Some(modal) = &self.settings_modal else {
            return;
        };
        let mut proposed = self.hosts_doc.clone();
        let result = (|| {
            if modal.default_user.text.trim().is_empty() {
                return Err("Username is required".into());
            }
            proposed.settings.default_user = modal.default_user.text.trim().into();
            proposed.settings.scrollback_lines = modal
                .scrollback_lines
                .text
                .trim()
                .parse()
                .map_err(|_| "Invalid scrollback size")?;
            proposed.settings.confirm_delete = modal.confirm_delete;
            proposed.settings.record_command_history = modal.record_command_history;
            Ok(proposed)
        })()
        .and_then(|proposed| self.commit_document(proposed));
        if let Some(modal) = &mut self.settings_modal {
            modal.general_msg = Some(match result {
                Ok(()) => ("Настройки сохранены / Settings saved".into(), false),
                Err(e) => (e, true),
            });
        }
    }
}
