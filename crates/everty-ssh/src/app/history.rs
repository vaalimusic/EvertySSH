use super::*;

impl AppState {
    pub fn refresh_recent_sessions(&mut self) {
        match self.db.list_recent_sessions(30) {
            Ok(sessions) => self.recent_sessions = sessions,
            Err(e) => self.show_error(&format!("Cannot read session history: {e}")),
        }
    }

    pub fn refresh_command_search(&mut self) {
        let q = self.command_search_input.text.trim();
        let res = if q.is_empty() {
            self.db.list_recent_commands(50)
        } else {
            self.db.search_command_history(q, None, 50)
        };
        if let Ok(records) = res {
            self.command_search_results = records;
            self.selected_search_idx = 0;
        }
    }

    pub fn open_session_dashboard(&mut self, sess_idx: usize) {
        if let Some(sess) = self.recent_sessions.get(sess_idx) {
            let commands = self.db.list_session_commands(&sess.id).unwrap_or_default();
            let host = sess
                .host_id
                .as_ref()
                .and_then(|hid| self.hosts_doc.hosts.iter().find(|h| &h.id == hid).cloned());
            self.session_dashboard_modal = Some(SessionDashboardModal {
                is_open: true,
                session: sess.clone(),
                commands,
                host,
            });
        }
    }

    pub fn delete_session_from_dashboard(&mut self) {
        if let Some(dash) = self.session_dashboard_modal.take() {
            let _ = self.db.delete_session(&dash.session.id);
            self.refresh_recent_sessions();
        }
    }

    pub fn refresh_snippets(&mut self) {
        let q = self.snippet_search_input.text.trim();
        let res = if q.is_empty() {
            self.db.list_snippets()
        } else {
            self.db.search_snippets(q)
        };
        if let Ok(snips) = res {
            self.snippets = snips;
        }
    }
}
