use super::*;

impl AppState {
    pub fn open_add_host_dialog(&mut self) {
        self.add_host_modal = AddHostModal {
            is_open: true,
            is_editing: false,
            editing_id: None,
            name: TextInput::default(),
            hostname: TextInput::default(),
            port: TextInput::new("22"),
            username: TextInput::new("root"),
            group: TextInput::new("Production"),
            auth_mode: AuthModeChoice::Agent,
            key_path: TextInput::new("~/.ssh/id_ed25519"),
            password: TextInput::default(),
            show_password: false,
            badge_color: "#2dd4bf".into(),
            active_field: 0,
            error_msg: None,
        };
        self.caret_visible = true;
        self.caret_blink_time = std::time::Instant::now();
    }

    pub fn open_edit_host_dialog(&mut self, host_idx: usize) {
        if let Some(host) = self.hosts_doc.hosts.get(host_idx) {
            let (auth_mode, key_path, password) = match &host.auth {
                AuthConfig::Agent => (
                    AuthModeChoice::Agent,
                    "~/.ssh/id_ed25519".into(),
                    String::new(),
                ),
                AuthConfig::Key { path, .. } => (
                    AuthModeChoice::Key,
                    path.to_string_lossy().to_string(),
                    String::new(),
                ),
                AuthConfig::Password { password } => (
                    AuthModeChoice::Password,
                    String::new(),
                    password.clone().unwrap_or_default(),
                ),
            };

            self.add_host_modal = AddHostModal {
                is_open: true,
                is_editing: true,
                editing_id: Some(host.id.clone()),
                name: TextInput::new(&host.name),
                hostname: TextInput::new(&host.hostname),
                port: TextInput::new(host.port.to_string()),
                username: TextInput::new(&host.username),
                group: TextInput::new(host.group.clone().unwrap_or_else(|| "General".into())),
                auth_mode,
                key_path: TextInput::new(key_path),
                password: TextInput::new(password),
                show_password: false,
                badge_color: host.badge_color.clone().unwrap_or_else(|| "#2dd4bf".into()),
                active_field: 0,
                error_msg: None,
            };
            self.caret_visible = true;
            self.caret_blink_time = std::time::Instant::now();
        }
    }

    pub fn save_host_from_modal(&mut self, connect_immediately: bool) -> bool {
        let name = self.add_host_modal.name.text.trim().to_string();
        let hostname = self.add_host_modal.hostname.text.trim().to_string();

        if name.is_empty() {
            self.add_host_modal.error_msg = Some("Укажите имя сервера".into());
            return false;
        }
        if hostname.is_empty() {
            self.add_host_modal.error_msg = Some("Укажите адрес хоста/IP".into());
            return false;
        }

        let port: u16 = self.add_host_modal.port.text.trim().parse().unwrap_or(22);
        let username = if self.add_host_modal.username.text.trim().is_empty() {
            "root".to_string()
        } else {
            self.add_host_modal.username.text.trim().to_string()
        };

        let auth = match self.add_host_modal.auth_mode {
            AuthModeChoice::Agent => AuthConfig::Agent,
            AuthModeChoice::Key => {
                let kp = self.add_host_modal.key_path.text.trim();
                let p = if kp.starts_with("~/") {
                    if let Some(h) = dirs::home_dir() {
                        h.join(&kp[2..])
                    } else {
                        PathBuf::from(kp)
                    }
                } else {
                    PathBuf::from(kp)
                };
                AuthConfig::Key {
                    path: p,
                    passphrase: None,
                }
            }
            AuthModeChoice::Password => AuthConfig::Password {
                password: Some(self.add_host_modal.password.text.clone()),
            },
        };

        let id = self
            .add_host_modal
            .editing_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let new_host = HostConfig {
            secret_ref: None,
            id: id.clone(),
            name,
            hostname,
            port,
            username,
            group: Some(self.add_host_modal.group.text.trim().to_string()),
            tags: vec![],
            badge_color: Some(self.add_host_modal.badge_color.clone()),
            auth,
            default_command: None,
            env: HashMap::new(),
        };

        let mut proposed = self.hosts_doc.clone();
        if self.add_host_modal.is_editing {
            if let Some(idx) = proposed.hosts.iter().position(|h| h.id == id) {
                let mut edited = new_host.clone();
                edited
                    .secret_ref
                    .clone_from(&proposed.hosts[idx].secret_ref);
                edited.env.clone_from(&proposed.hosts[idx].env);
                edited
                    .default_command
                    .clone_from(&proposed.hosts[idx].default_command);
                edited.tags.clone_from(&proposed.hosts[idx].tags);
                if let (
                    AuthConfig::Key {
                        path: old_path,
                        passphrase,
                    },
                    AuthConfig::Key {
                        path,
                        passphrase: edited_pass,
                    },
                ) = (&proposed.hosts[idx].auth, &mut edited.auth)
                {
                    if path == old_path {
                        edited_pass.clone_from(passphrase);
                    }
                }
                proposed.hosts[idx] = edited;
            }
        } else {
            proposed.hosts.push(new_host.clone());
        }
        if let Err(e) = self.commit_document(proposed) {
            self.add_host_modal.error_msg = Some(e);
            return false;
        }
        self.add_host_modal.password = TextInput::default();
        self.add_host_modal.is_open = false;

        if connect_immediately {
            if let Some(saved) = self
                .hosts_doc
                .hosts
                .iter()
                .find(|host| host.id == id)
                .cloned()
            {
                self.connect_to_host(saved);
            }
        }

        true
    }

    pub fn accept_host_key(&mut self) {
        if let Some(mut modal) = self.host_key_modal.take() {
            if let Some(tx) = modal.response_tx.take() {
                let _ = tx.send(true);
            }
        }
        self.advance_host_key_prompt();
    }

    pub fn reject_host_key(&mut self) {
        if let Some(mut modal) = self.host_key_modal.take() {
            if let Some(tx) = modal.response_tx.take() {
                let _ = tx.send(false);
            }
        }
        self.advance_host_key_prompt();
    }

    pub fn open_confirm_delete_host(&mut self, host_idx: usize) {
        if !self.hosts_doc.settings.confirm_delete {
            self.delete_host(host_idx);
            return;
        }
        if let Some(h) = self.hosts_doc.hosts.get(host_idx) {
            self.confirm_delete_modal = Some(ConfirmDeleteHostModal {
                is_open: true,
                host_idx,
                host_name: h.name.clone(),
                host_address: format!("{}@{}:{}", h.username, h.hostname, h.port),
            });
        }
    }

    pub fn execute_confirm_delete_host(&mut self) {
        if let Some(m) = self.confirm_delete_modal.take() {
            self.delete_host(m.host_idx);
        }
    }

    pub fn open_confirm_run_modal(&mut self, command: String, source_title: String) {
        let (is_dangerous, danger_reason) = check_dangerous_command(&command);
        let (target_tab_title, can_execute) = if let Some(tab) = self.tabs.get(self.active_tab) {
            let is_connected = tab
                .handle
                .as_ref()
                .map_or(false, |h| *h.is_connected.read());
            if is_connected {
                (format!("{} ({})", tab.title, tab.host_address), true)
            } else {
                (format!("{} (отключен)", tab.title), false)
            }
        } else {
            ("Нет активных сессий терминала".into(), false)
        };

        self.confirm_run_modal = Some(ConfirmRunCommandModal {
            paste: false,
            target_session_id: self.tabs.get(self.active_tab).map(|t| t.id.clone()),
            is_open: true,
            command,
            source_title,
            is_dangerous,
            danger_reason,
            target_tab_title,
            can_execute,
            copied_feedback: false,
        });
    }

    pub fn execute_confirm_run_command(&mut self) {
        if let Some(modal) = self.confirm_run_modal.take() {
            let Some(index) = self
                .tabs
                .iter()
                .position(|tab| Some(&tab.id) == modal.target_session_id.as_ref())
            else {
                return;
            };
            if !modal.can_execute
                || !self.tabs[index]
                    .handle
                    .as_ref()
                    .is_some_and(|h| *h.is_connected.read())
            {
                return;
            }
            let text = if modal.paste {
                crate::paste::encode(
                    &modal.command,
                    self.tabs[index].terminal.screen.bracketed_paste,
                )
            } else {
                format!("{}\r", modal.command)
            };
            self.send_to_terminal(index, &text);
        }
    }

    pub fn delete_host(&mut self, host_idx: usize) {
        if host_idx < self.hosts_doc.hosts.len() {
            let mut proposed = self.hosts_doc.clone();
            proposed.hosts.remove(host_idx);
            if let Err(e) = self.commit_document(proposed) {
                self.show_error(&e);
            }
        }
    }

    pub fn open_add_snippet_dialog(&mut self) {
        self.add_snippet_modal = AddSnippetModal {
            is_open: true,
            is_editing: false,
            editing_id: None,
            title: TextInput::default(),
            command: TextInput::default(),
            category: TextInput::new("General"),
            description: TextInput::default(),
            active_field: 0,
            error_msg: None,
        };
        self.caret_visible = true;
        self.caret_blink_time = std::time::Instant::now();
    }

    pub fn open_edit_snippet_dialog(&mut self, snip_idx: usize) {
        if let Some(snip) = self.snippets.get(snip_idx) {
            self.add_snippet_modal = AddSnippetModal {
                is_open: true,
                is_editing: true,
                editing_id: Some(snip.id),
                title: TextInput::new(&snip.title),
                command: TextInput::new(&snip.command),
                category: TextInput::new(snip.category.clone().unwrap_or_else(|| "General".into())),
                description: TextInput::new(snip.description.clone().unwrap_or_default()),
                active_field: 0,
                error_msg: None,
            };
            self.caret_visible = true;
            self.caret_blink_time = std::time::Instant::now();
        }
    }

    pub fn open_confirm_delete_snippet_modal(&mut self, snip_idx: usize) {
        if let Some(snip) = self.snippets.get(snip_idx) {
            self.confirm_delete_snippet_modal = Some(ConfirmDeleteSnippetModal {
                is_open: true,
                snippet_id: snip.id,
                snippet_title: snip.title.clone(),
                snippet_command: snip.command.clone(),
            });
        }
    }

    pub fn execute_confirm_delete_snippet(&mut self) {
        if let Some(m) = self.confirm_delete_snippet_modal.take() {
            self.delete_snippet_by_id(m.snippet_id);
        }
    }

    pub fn save_snippet_from_modal(&mut self) -> bool {
        let title = self.add_snippet_modal.title.text.trim().to_string();
        let command = self.add_snippet_modal.command.text.trim().to_string();
        let category = self.add_snippet_modal.category.text.trim().to_string();
        let description = self.add_snippet_modal.description.text.trim().to_string();

        if title.is_empty() {
            self.add_snippet_modal.error_msg = Some("Укажите название сниппета".into());
            return false;
        }
        if command.is_empty() {
            self.add_snippet_modal.error_msg = Some("Укажите команду".into());
            return false;
        }

        let cat_opt = if category.is_empty() {
            None
        } else {
            Some(category.as_str())
        };
        let desc_opt = if description.is_empty() {
            None
        } else {
            Some(description.as_str())
        };

        let result = if self.add_snippet_modal.is_editing {
            if let Some(id) = self.add_snippet_modal.editing_id {
                self.db
                    .update_snippet(id, &title, &command, cat_opt, desc_opt)
            } else {
                self.db
                    .add_snippet(&title, &command, cat_opt, desc_opt)
                    .map(|_| ())
            }
        } else {
            self.db
                .add_snippet(&title, &command, cat_opt, desc_opt)
                .map(|_| ())
        };

        if result.is_ok() {
            self.add_snippet_modal.is_open = false;
            self.refresh_snippets();
            true
        } else {
            self.add_snippet_modal.error_msg = Some("Ошибка при сохранении сниппета".into());
            false
        }
    }

    pub fn delete_snippet_by_id(&mut self, id: i64) {
        let _ = self.db.delete_snippet(id);
        self.refresh_snippets();
    }
}
