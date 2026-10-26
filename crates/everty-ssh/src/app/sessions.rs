use super::*;

impl AppState {
    pub fn shutdown_sessions(&mut self) {
        for (_, task) in self.pending_connections.drain() {
            task.abort();
        }
        self.host_key_modal = None;
        self.pending_host_keys.clear();
        for tab in &mut self.tabs {
            tab.handle = None;
            let _ = self.db.record_session_end(&tab.id, "closed", None);
        }
    }

    pub fn connect_to_host(&mut self, host: HostConfig) {
        if self.is_app_locked {
            return;
        }
        let session_id = uuid::Uuid::new_v4().to_string();
        let host_address = format!("{}:{}", host.hostname, host.port);
        let mut terminal = Terminal::new(
            self.terminal_cols,
            self.terminal_rows,
            self.hosts_doc.settings.scrollback_lines,
        );
        terminal.write_bytes(
            format!(
                "\x1b[38;2;45;212;191mEvertySSH v{}\x1b[0m\r\nConnecting to {}@{}...\r\n",
                env!("CARGO_PKG_VERSION"),
                host.username,
                host_address
            )
            .as_bytes(),
        );
        self.tabs.push(SessionTab {
            id: session_id.clone(),
            title: host.name.clone(),
            host_address,
            username: host.username.clone(),
            terminal,
            handle: None,
            status: "Connecting...".into(),
            host_id: Some(host.id.clone()),
            host_config: Some(host.clone()),
        });
        self.active_tab = self.tabs.len() - 1;
        self.start_connection(host, session_id);
    }

    fn start_connection(&mut self, host: HostConfig, session_id: String) {
        if let Err(e) = self.db.record_session_start(
            &session_id,
            Some(&host.id),
            &format!("{}:{}", host.hostname, host.port),
            &host.username,
        ) {
            self.show_error(&format!("Cannot save session history: {e}"));
        }
        let event_tx = self.event_tx.clone();
        let cols = self.terminal_cols as u32;
        let rows = self.terminal_rows as u32;
        let task_id = session_id.clone();
        let task = self.tokio_handle.spawn(async move {
            match connect_ssh(host, task_id.clone(), cols, rows, event_tx.clone()).await {
                Ok(handle) => {
                    let _ = event_tx
                        .send(AppEvent::SshConnected {
                            session_id: task_id,
                            handle,
                        })
                        .await;
                }
                Err(error) => {
                    let _ = event_tx
                        .send(AppEvent::SshDisconnected {
                            session_id: task_id,
                            error: Some(error),
                        })
                        .await;
                }
            }
        });
        self.pending_connections
            .insert(session_id, task.abort_handle());
        self.refresh_recent_sessions();
    }

    pub fn reconnect_tab(&mut self, index: usize) {
        if self.is_app_locked {
            return;
        }
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if let Some(task) = self.pending_connections.remove(&tab.id) {
            task.abort();
        }
        let Some(host) = tab.host_config.clone() else {
            return;
        };
        let _ = self.db.record_session_end(&tab.id, "closed", None);
        tab.handle = None;
        tab.id = uuid::Uuid::new_v4().to_string();
        tab.status = "Connecting...".into();
        tab.terminal = Terminal::new(
            self.terminal_cols,
            self.terminal_rows,
            self.hosts_doc.settings.scrollback_lines,
        );
        let id = tab.id.clone();
        self.advance_host_key_prompt();
        self.start_connection(host, id);
    }

    pub(super) fn advance_host_key_prompt(&mut self) {
        if self.host_key_modal.as_ref().is_some_and(|m| {
            !self.tabs.iter().any(|t| t.id == m.session_id)
                || m.response_tx.as_ref().is_none_or(|tx| tx.is_closed())
        }) {
            self.host_key_modal = None;
        }
        self.pending_host_keys.retain(|m| {
            self.tabs.iter().any(|t| t.id == m.session_id)
                && m.response_tx.as_ref().is_some_and(|tx| !tx.is_closed())
        });
        if self.host_key_modal.is_none() {
            self.host_key_modal = self.pending_host_keys.pop_front();
        }
    }
}
