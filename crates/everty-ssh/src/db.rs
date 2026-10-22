use std::path::Path;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub host_id: Option<String>,
    pub host_address: String,
    pub username: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: String, // "active", "closed", "error"
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    pub id: i64,
    pub session_id: Option<String>,
    pub host_address: String,
    pub command: String,
    pub executed_at: DateTime<Utc>,
    pub is_favorite: bool,
    pub snippet_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetRecord {
    pub id: i64,
    pub title: String,
    pub command: String,
    pub category: Option<String>,
    pub description: Option<String>,
}

pub struct HistoryDb {
    conn: Connection,
}

impl HistoryDb {
    pub fn open_default() -> SqlResult<Self> {
        let dir = crate::config::config_dir();
        std::fs::create_dir_all(&dir).ok();
        Self::open(dir.join("history.db"))
    }

    pub fn open<P: AsRef<Path>>(path: P) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> SqlResult<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                host_id TEXT,
                host_address TEXT NOT NULL,
                username TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                status TEXT NOT NULL,
                error_message TEXT
            );

            CREATE TABLE IF NOT EXISTS command_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT,
                host_address TEXT NOT NULL,
                command TEXT NOT NULL,
                executed_at TEXT NOT NULL,
                is_favorite INTEGER DEFAULT 0,
                snippet_title TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );

            CREATE TABLE IF NOT EXISTS snippets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                command TEXT NOT NULL,
                category TEXT,
                description TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_cmd_history_executed ON command_history(executed_at DESC);
            CREATE INDEX IF NOT EXISTS idx_cmd_history_cmd ON command_history(command);
            CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC);
            "#,
        )?;

        // Populate default snippets if table is empty
        let count: i64 = self.conn.query_row("SELECT count(*) FROM snippets", [], |r| r.get(0))?;
        if count == 0 {
            self.seed_default_snippets()?;
        }

        Ok(())
    }

    fn seed_default_snippets(&self) -> SqlResult<()> {
        let defaults = [
            ("System Overview", "uname -a && uptime && free -h && df -h", "System", "Display OS, uptime, RAM, and disk usage"),
            ("Active Network Ports", "ss -tulpn || netstat -tulpn", "Network", "List all listening TCP/UDP ports and processes"),
            ("Docker Containers", "docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'", "Docker", "Compact list of running containers"),
            ("Follow Systemd Logs", "journalctl -u nginx -f --no-tail", "Logs", "Follow logs of a systemd service"),
            ("Top Memory Processes", "ps aux --sort=-%mem | head -n 15", "System", "List top 15 processes by memory consumption"),
            ("Top CPU Processes", "ps aux --sort=-%cpu | head -n 15", "System", "List top 15 processes by CPU consumption"),
        ];

        for (title, cmd, cat, desc) in defaults {
            self.add_snippet(title, cmd, Some(cat), Some(desc))?;
        }
        Ok(())
    }

    // Sessions API
    pub fn record_session_start(
        &self,
        id: &str,
        host_id: Option<&str>,
        host_address: &str,
        username: &str,
    ) -> SqlResult<()> {
        let started_at = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            INSERT INTO sessions (id, host_id, host_address, username, started_at, status)
            VALUES (?1, ?2, ?3, ?4, ?5, 'active')
            "#,
            params![id, host_id, host_address, username, started_at],
        )?;
        Ok(())
    }

    pub fn record_session_end(
        &self,
        id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> SqlResult<()> {
        let ended_at = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            UPDATE sessions
            SET ended_at = ?1, status = ?2, error_message = ?3
            WHERE id = ?4
            "#,
            params![ended_at, status, error_message, id],
        )?;
        Ok(())
    }

    pub fn list_recent_sessions(&self, limit: usize) -> SqlResult<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, host_id, host_address, username, started_at, ended_at, status, error_message
            FROM sessions
            ORDER BY started_at DESC
            LIMIT ?1
            "#,
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let started_str: String = row.get(4)?;
            let ended_str: Option<String> = row.get(5)?;

            let started_at = DateTime::parse_from_rfc3339(&started_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let ended_at = ended_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            });

            Ok(SessionRecord {
                id: row.get(0)?,
                host_id: row.get(1)?,
                host_address: row.get(2)?,
                username: row.get(3)?,
                started_at,
                ended_at,
                status: row.get(6)?,
                error_message: row.get(7)?,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn delete_session(&self, session_id: &str) -> SqlResult<()> {
        self.conn.execute(
            "DELETE FROM command_history WHERE session_id = ?1",
            params![session_id],
        )?;
        self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn list_session_commands(&self, session_id: &str) -> SqlResult<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, session_id, host_address, command, executed_at, is_favorite, snippet_title
            FROM command_history
            WHERE session_id = ?1
            ORDER BY executed_at ASC
            "#,
        )?;
        let rows = stmt.query_map(params![session_id], Self::map_command_row)?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    // Commands API
    pub fn record_command(
        &self,
        session_id: Option<&str>,
        host_address: &str,
        command: &str,
    ) -> SqlResult<i64> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }

        let executed_at = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            INSERT INTO command_history (session_id, host_address, command, executed_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![session_id, host_address, trimmed, executed_at],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_command_history(
        &self,
        query: &str,
        host_filter: Option<&str>,
        limit: usize,
    ) -> SqlResult<Vec<CommandRecord>> {
        let pattern = format!("%{}%", query.trim());
        let sql = match host_filter {
            Some(_) => {
                r#"
                SELECT id, session_id, host_address, command, executed_at, is_favorite, snippet_title
                FROM command_history
                WHERE command LIKE ?1 AND host_address = ?2
                ORDER BY executed_at DESC
                LIMIT ?3
                "#
            }
            None => {
                r#"
                SELECT id, session_id, host_address, command, executed_at, is_favorite, snippet_title
                FROM command_history
                WHERE command LIKE ?1
                ORDER BY executed_at DESC
                LIMIT ?2
                "#
            }
        };

        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(hf) = host_filter {
            stmt.query_map(params![pattern, hf, limit as i64], Self::map_command_row)?
        } else {
            stmt.query_map(params![pattern, limit as i64], Self::map_command_row)?
        };

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn list_recent_commands(&self, limit: usize) -> SqlResult<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, session_id, host_address, command, executed_at, is_favorite, snippet_title
            FROM command_history
            ORDER BY executed_at DESC
            LIMIT ?1
            "#,
        )?;

        let rows = stmt.query_map(params![limit as i64], Self::map_command_row)?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn toggle_favorite_command(&self, id: i64) -> SqlResult<bool> {
        let current: i64 = self.conn.query_row(
            "SELECT is_favorite FROM command_history WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let new_val = if current == 1 { 0 } else { 1 };
        self.conn.execute(
            "UPDATE command_history SET is_favorite = ?1 WHERE id = ?2",
            params![new_val, id],
        )?;
        Ok(new_val == 1)
    }

    pub fn list_favorites(&self) -> SqlResult<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, session_id, host_address, command, executed_at, is_favorite, snippet_title
            FROM command_history
            WHERE is_favorite = 1
            ORDER BY executed_at DESC
            "#,
        )?;
        let rows = stmt.query_map([], Self::map_command_row)?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    // Snippets API
    pub fn add_snippet(
        &self,
        title: &str,
        command: &str,
        category: Option<&str>,
        description: Option<&str>,
    ) -> SqlResult<i64> {
        self.conn.execute(
            r#"
            INSERT INTO snippets (title, command, category, description)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![title, command, category, description],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_snippets(&self) -> SqlResult<Vec<SnippetRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, title, command, category, description
            FROM snippets
            ORDER BY category ASC, title ASC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(SnippetRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                command: row.get(2)?,
                category: row.get(3)?,
                description: row.get(4)?,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn search_snippets(&self, query: &str) -> SqlResult<Vec<SnippetRecord>> {
        let pattern = format!("%{}%", query.trim());
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, title, command, category, description
            FROM snippets
            WHERE title LIKE ?1 OR command LIKE ?1 OR description LIKE ?1 OR category LIKE ?1
            ORDER BY category ASC, title ASC
            "#,
        )?;

        let rows = stmt.query_map(params![pattern], |row| {
            Ok(SnippetRecord {
                id: row.get(0)?,
                title: row.get(1)?,
                command: row.get(2)?,
                category: row.get(3)?,
                description: row.get(4)?,
            })
        })?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn delete_snippet(&self, id: i64) -> SqlResult<()> {
        self.conn.execute("DELETE FROM snippets WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn map_command_row(row: &rusqlite::Row) -> SqlResult<CommandRecord> {
        let executed_str: String = row.get(4)?;
        let executed_at = DateTime::parse_from_rfc3339(&executed_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let fav: i64 = row.get(5)?;
        Ok(CommandRecord {
            id: row.get(0)?,
            session_id: row.get(1)?,
            host_address: row.get(2)?,
            command: row.get(3)?,
            executed_at,
            is_favorite: fav == 1,
            snippet_title: row.get(6)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_session_and_commands() {
        let db = HistoryDb::open_in_memory().unwrap();

        // Check seeded snippets
        let snippets = db.list_snippets().unwrap();
        assert!(!snippets.is_empty());

        // Record session
        let s_id = "test-session-1";
        db.record_session_start(s_id, Some("host-1"), "192.168.1.10", "root").unwrap();

        // Record commands
        let c1 = db.record_command(Some(s_id), "192.168.1.10", "ls -la /var/log").unwrap();
        let _c2 = db.record_command(Some(s_id), "192.168.1.10", "docker compose up -d").unwrap();

        // Toggle favorite
        assert!(db.toggle_favorite_command(c1).unwrap());
        let favs = db.list_favorites().unwrap();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].command, "ls -la /var/log");

        // Search commands
        let search = db.search_command_history("docker", None, 10).unwrap();
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].command, "docker compose up -d");

        // Session commands
        let session_cmds = db.list_session_commands(s_id).unwrap();
        assert_eq!(session_cmds.len(), 2);

        // Close session
        db.record_session_end(s_id, "closed", None).unwrap();
        let sessions = db.list_recent_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "closed");
        assert!(sessions[0].ended_at.is_some());
    }
}
