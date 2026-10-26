mod dialogs;
mod history;
#[cfg(test)]
mod regression_tests;
mod sessions;
mod settings;
use crate::config_store::ConfigStore;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use tokio::sync::mpsc::{Receiver, Sender};
use winit::window::CursorIcon;

use crate::config::{
    hash_password, verify_password, AppLanguage, AuthConfig, ExportBundle, HostConfig,
    HostsDocument,
};
use crate::db::{CommandRecord, HistoryDb, SessionRecord, SnippetRecord};
use crate::i18n::tr;
use crate::ssh::{connect_ssh, SshSessionHandle};
use crate::terminal::Terminal;

#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub text: String,
    pub cursor: usize,
    pub selection: Option<(usize, usize)>, // (start, end)
    pub scroll_offset: usize,              // character offset to prevent overflow
}

impl TextInput {
    pub fn new(text: impl Into<String>) -> Self {
        let t = text.into();
        let len = t.chars().count();
        Self {
            text: t,
            cursor: len,
            selection: None,
            scroll_offset: 0,
        }
    }

    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
        self.selection = None;
        self.scroll_offset = 0;
    }

    pub fn select_all(&mut self) {
        let len = self.text.chars().count();
        if len > 0 {
            self.selection = Some((0, len));
            self.cursor = len;
        }
    }

    pub fn select_word_at_cursor(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        if chars.is_empty() {
            self.selection = None;
            self.cursor = 0;
            return;
        }
        let pos = self.cursor.min(chars.len());
        let anchor = if pos > 0 && (pos == chars.len() || chars[pos].is_whitespace()) {
            pos - 1
        } else {
            pos
        };
        let mut start = anchor;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let mut end = anchor;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        if start < end {
            self.selection = Some((start, end));
            self.cursor = end;
        } else {
            self.select_all();
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn selected_range(&self) -> Option<(usize, usize)> {
        let (a, b) = self.selection?;
        if a == b {
            None
        } else if a < b {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        let chars: Vec<char> = self.text.chars().collect();
        if start < chars.len() && end <= chars.len() && start < end {
            Some(chars[start..end].iter().collect())
        } else {
            None
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selected_range() {
            let mut chars: Vec<char> = self.text.chars().collect();
            if start < chars.len() && end <= chars.len() && start < end {
                chars.drain(start..end);
                self.text = chars.into_iter().collect();
                self.cursor = start;
                self.selection = None;
                return true;
            }
        }
        false
    }

    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.cursor > chars.len() {
            self.cursor = chars.len();
        }
        chars.insert(self.cursor, ch);
        self.cursor += 1;
        self.text = chars.into_iter().collect();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        for ch in s.chars() {
            if !ch.is_control() {
                self.insert_char(ch);
            }
        }
    }

    pub fn delete_back(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor > 0 {
            let mut chars: Vec<char> = self.text.chars().collect();
            if self.cursor <= chars.len() {
                chars.remove(self.cursor - 1);
                self.cursor -= 1;
                self.text = chars.into_iter().collect();
            }
        }
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.cursor < chars.len() {
            chars.remove(self.cursor);
            self.text = chars.into_iter().collect();
        }
    }

    fn char_category(c: char) -> u8 {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    }

    pub fn calc_word_left(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        if chars.is_empty() || self.cursor == 0 {
            return 0;
        }
        let mut pos = self.cursor.min(chars.len());
        // 1. Skip whitespace backwards
        while pos > 0 && Self::char_category(chars[pos - 1]) == 0 {
            pos -= 1;
        }
        // 2. Skip word/punct characters backwards of the same category
        if pos > 0 {
            let cat = Self::char_category(chars[pos - 1]);
            while pos > 0 && Self::char_category(chars[pos - 1]) == cat {
                pos -= 1;
            }
        }
        pos
    }

    pub fn calc_word_right(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        if chars.is_empty() || self.cursor >= len {
            return len;
        }
        let mut pos = self.cursor;
        // 1. If currently on whitespace, skip it
        if Self::char_category(chars[pos]) == 0 {
            while pos < len && Self::char_category(chars[pos]) == 0 {
                pos += 1;
            }
        } else {
            // Skip current word / punct
            let cat = Self::char_category(chars[pos]);
            while pos < len && Self::char_category(chars[pos]) == cat {
                pos += 1;
            }
            // Also skip trailing whitespace
            while pos < len && Self::char_category(chars[pos]) == 0 {
                pos += 1;
            }
        }
        pos
    }

    pub fn move_left(&mut self) {
        if let Some((start, _)) = self.selected_range() {
            self.cursor = start;
        } else if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.selection = None;
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
    }

    pub fn move_right(&mut self) {
        let len = self.text.chars().count();
        if let Some((_, end)) = self.selected_range() {
            self.cursor = end.min(len);
        } else if self.cursor < len {
            self.cursor += 1;
        }
        self.selection = None;
    }

    pub fn move_home(&mut self) {
        self.selection = None;
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn move_end(&mut self) {
        self.selection = None;
        self.cursor = self.text.chars().count();
    }

    pub fn move_word_left(&mut self) {
        self.cursor = self.calc_word_left();
        self.selection = None;
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
    }

    pub fn move_word_right(&mut self) {
        self.cursor = self.calc_word_right();
        self.selection = None;
    }

    pub fn move_left_select(&mut self) {
        let anchor = match self.selection {
            Some((a, _)) => a,
            None => self.cursor,
        };
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        if anchor == self.cursor {
            self.selection = None;
        } else {
            self.selection = Some((anchor, self.cursor));
        }
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
    }

    pub fn move_right_select(&mut self) {
        let len = self.text.chars().count();
        let anchor = match self.selection {
            Some((a, _)) => a,
            None => self.cursor,
        };
        if self.cursor < len {
            self.cursor += 1;
        }
        if anchor == self.cursor {
            self.selection = None;
        } else {
            self.selection = Some((anchor, self.cursor));
        }
    }

    pub fn move_home_select(&mut self) {
        let anchor = match self.selection {
            Some((a, _)) => a,
            None => self.cursor,
        };
        self.cursor = 0;
        self.scroll_offset = 0;
        if anchor == 0 {
            self.selection = None;
        } else {
            self.selection = Some((anchor, 0));
        }
    }

    pub fn move_end_select(&mut self) {
        let len = self.text.chars().count();
        let anchor = match self.selection {
            Some((a, _)) => a,
            None => self.cursor,
        };
        self.cursor = len;
        if anchor == len {
            self.selection = None;
        } else {
            self.selection = Some((anchor, len));
        }
    }

    pub fn move_word_left_select(&mut self) {
        let anchor = match self.selection {
            Some((a, _)) => a,
            None => self.cursor,
        };
        self.cursor = self.calc_word_left();
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
        if anchor == self.cursor {
            self.selection = None;
        } else {
            self.selection = Some((anchor, self.cursor));
        }
    }

    pub fn move_word_right_select(&mut self) {
        let anchor = match self.selection {
            Some((a, _)) => a,
            None => self.cursor,
        };
        self.cursor = self.calc_word_right();
        if anchor == self.cursor {
            self.selection = None;
        } else {
            self.selection = Some((anchor, self.cursor));
        }
    }

    pub fn delete_word_back(&mut self) {
        if self.delete_selection() {
            return;
        }
        let target = self.calc_word_left();
        if target < self.cursor {
            let mut chars: Vec<char> = self.text.chars().collect();
            chars.drain(target..self.cursor);
            self.text = chars.into_iter().collect();
            self.cursor = target;
            if self.cursor < self.scroll_offset {
                self.scroll_offset = self.cursor;
            }
        }
    }

    pub fn delete_word_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let target = self.calc_word_right();
        if target > self.cursor {
            let mut chars: Vec<char> = self.text.chars().collect();
            chars.drain(self.cursor..target);
            self.text = chars.into_iter().collect();
        }
    }

    pub fn scroll_delta(&mut self, delta_chars: i32, max_visible_chars: usize) {
        let total = self.text.chars().count();
        if delta_chars > 0 {
            // Scroll towards beginning (left)
            self.scroll_offset = self.scroll_offset.saturating_sub(delta_chars as usize);
        } else if delta_chars < 0 {
            // Scroll towards end (right)
            let max_scroll = total.saturating_sub(max_visible_chars.min(total));
            self.scroll_offset =
                (self.scroll_offset + delta_chars.unsigned_abs() as usize).min(max_scroll);
        }
        // Keep cursor in view when scrolling so user can immediately edit where they scrolled
        if self.cursor < self.scroll_offset {
            self.cursor = self.scroll_offset;
        } else if self.cursor > self.scroll_offset + max_visible_chars {
            self.cursor = self.scroll_offset + max_visible_chars;
        }
    }

    pub fn set_cursor_from_click(&mut self, click_offset_px: f64, approx_char_w: f64) {
        let total_chars = self.text.chars().count();
        let clicked_char = (click_offset_px / approx_char_w).round().max(0.0) as usize;
        self.cursor = (self.scroll_offset + clicked_char).min(total_chars);
        self.clear_selection();
    }

    pub fn prefix_to_cursor(&self) -> String {
        self.text.chars().take(self.cursor).collect()
    }

    pub fn ensure_cursor_visible(&mut self, max_visible_chars: usize) {
        if max_visible_chars == 0 {
            return;
        }
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor > self.scroll_offset + max_visible_chars {
            self.scroll_offset = self.cursor.saturating_sub(max_visible_chars);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    Hosts,
    Sessions,
    Commands,
    Snippets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPos {
    pub col: usize,
    pub row: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSelection {
    pub start: TerminalPos,
    pub end: TerminalPos,
}

impl TerminalSelection {
    pub fn new(start: TerminalPos, end: TerminalPos) -> Self {
        Self { start, end }
    }

    pub fn normalized(&self) -> (TerminalPos, TerminalPos) {
        if self.start.row < self.end.row
            || (self.start.row == self.end.row && self.start.col <= self.end.col)
        {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    pub fn contains(&self, col: usize, row: usize) -> bool {
        let (p1, p2) = self.normalized();
        if row < p1.row || row > p2.row {
            return false;
        }
        if p1.row == p2.row {
            return col >= p1.col && col <= p2.col;
        }
        if row == p1.row {
            return col >= p1.col;
        }
        if row == p2.row {
            return col <= p2.col;
        }
        true
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    Copy,
    SearchGoogle,
    Paste,
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub is_open: bool,
    pub x: f64,
    pub y: f64,
    pub selected_text: String,
}

pub fn percent_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match *b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*b as char);
            }
            b' ' => encoded.push('+'),
            other => {
                encoded.push_str(&format!("%{:02X}", other));
            }
        }
    }
    encoded
}

pub fn open_browser_url(url: &str) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", trimmed])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(trimmed).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(trimmed).spawn();
    }
}

pub fn open_google_search(query: &str) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }
    let encoded = percent_encode(trimmed);
    let url = format!("https://www.google.com/search?q={}", encoded);
    open_browser_url(&url);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthModeChoice {
    Agent,
    Key,
    Password,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAlgo {
    Ed25519,
    Rsa4096,
}

#[derive(Debug, Clone)]
pub struct KeyToolModal {
    pub is_open: bool,
    pub target_host_idx: Option<usize>,
    pub algo: KeyAlgo,
    pub key_name: TextInput,
    pub comment: TextInput,
    pub passphrase: TextInput,
    pub generated_pub_key: Option<String>,
    pub generated_priv_path: Option<String>,
    pub status_msg: Option<String>,
    pub error_msg: Option<String>,
    pub active_field: usize, // 0=key_name, 1=comment, 2=passphrase
}

impl Default for KeyToolModal {
    fn default() -> Self {
        Self {
            is_open: false,
            target_host_idx: None,
            algo: KeyAlgo::Ed25519,
            key_name: TextInput::new("id_ed25519"),
            comment: TextInput::new("user@evertyssh"),
            passphrase: TextInput::default(),
            generated_pub_key: None,
            generated_priv_path: None,
            status_msg: None,
            error_msg: None,
            active_field: 0,
        }
    }
}

pub fn generate_ssh_key(
    algo: KeyAlgo,
    key_name: &str,
    comment: &str,
    passphrase: &str,
) -> Result<(String, String), String> {
    let ssh_dir = dirs::home_dir()
        .map(|h| h.join(".ssh"))
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&ssh_dir);

    let clean_name = key_name.trim();
    if clean_name.is_empty() {
        return Err("Имя файла ключа не может быть пустым".into());
    }

    let priv_path = ssh_dir.join(clean_name);
    let pub_path = ssh_dir.join(format!("{clean_name}.pub"));

    if priv_path.exists() {
        let _ = std::fs::remove_file(&priv_path);
        let _ = std::fs::remove_file(&pub_path);
    }

    let algo_flag = match algo {
        KeyAlgo::Ed25519 => "ed25519",
        KeyAlgo::Rsa4096 => "rsa",
    };

    let mut cmd = std::process::Command::new("ssh-keygen");
    cmd.arg("-q")
        .arg("-t")
        .arg(algo_flag)
        .arg("-C")
        .arg(comment.trim())
        .arg("-N")
        .arg(passphrase)
        .arg("-f")
        .arg(&priv_path);

    if algo == KeyAlgo::Rsa4096 {
        cmd.arg("-b").arg("4096");
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить ssh-keygen: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh-keygen завершился с ошибкой: {err}"));
    }

    let pub_content = std::fs::read_to_string(&pub_path)
        .map_err(|e| format!("Не удалось прочитать сгенерированный публичный ключ: {e}"))?;

    Ok((
        priv_path.to_string_lossy().to_string(),
        pub_content.trim().to_string(),
    ))
}

pub fn make_authorized_keys_cmd(pub_key: &str) -> String {
    let clean = pub_key.trim();
    format!("mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '{clean}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFieldId {
    AddHost(usize),
    KeyTool(usize),
    AddSnippet(usize),
    CommandSearch,
    SnippetSearch,
    HostSearch,
    SettingsSecurity(usize),
    SettingsGeneral(usize),
    SettingsBackup(usize),
    LockPassword,
    TerminalSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickActionCmd {
    Htop,
    Df,
    Free,
    Uptime,
    Docker,
    Ports,
}

impl QuickActionCmd {
    pub fn label(&self) -> &'static str {
        match self {
            QuickActionCmd::Htop => "📊 htop",
            QuickActionCmd::Df => "💾 df -h",
            QuickActionCmd::Free => "⚡ free -m",
            QuickActionCmd::Uptime => "⏱ uptime",
            QuickActionCmd::Docker => "🐳 docker",
            QuickActionCmd::Ports => "🌐 ss -tulpn",
        }
    }

    pub fn command_str(&self) -> &'static str {
        match self {
            QuickActionCmd::Htop => "htop\r",
            QuickActionCmd::Df => "df -h\r",
            QuickActionCmd::Free => "free -m\r",
            QuickActionCmd::Uptime => "uptime\r",
            QuickActionCmd::Docker => "docker ps\r",
            QuickActionCmd::Ports => "ss -tulpn 2>/dev/null || netstat -tulpn\r",
        }
    }

    pub fn tooltip(&self) -> &'static str {
        match self {
            QuickActionCmd::Htop => "Мониторинг процессов и нагрузки CPU (htop)",
            QuickActionCmd::Df => "Свободное место на дисковых накопителях (df -h)",
            QuickActionCmd::Free => "Использование оперативной памяти RAM (free -m)",
            QuickActionCmd::Uptime => "Время непрерывной работы и средняя нагрузка (uptime)",
            QuickActionCmd::Docker => "Список активных Docker контейнеров (docker ps)",
            QuickActionCmd::Ports => "Открытые сетевые порты и сокеты (ss -tulpn)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalSearchState {
    pub is_open: bool,
    pub query: TextInput,
    pub case_sensitive: bool,
    pub matches: Vec<crate::terminal::SearchMatch>,
    pub selected_match_idx: usize,
}

impl Default for TerminalSearchState {
    fn default() -> Self {
        Self {
            is_open: false,
            query: TextInput::default(),
            case_sensitive: false,
            matches: Vec::new(),
            selected_match_idx: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMenuAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[derive(Debug, Clone)]
pub struct InputContextMenu {
    pub is_open: bool,
    pub x: f64,
    pub y: f64,
    pub field_id: InputFieldId,
}

#[derive(Debug, Clone)]
pub struct TooltipState {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub hover_start: std::time::Instant,
}

#[derive(Debug, Clone)]
pub struct SessionDashboardModal {
    pub is_open: bool,
    pub session: SessionRecord,
    pub commands: Vec<CommandRecord>,
    pub host: Option<HostConfig>,
}

#[derive(Debug, Clone)]
pub struct ConfirmDeleteHostModal {
    pub is_open: bool,
    pub host_idx: usize,
    pub host_name: String,
    pub host_address: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfirmRunModalLayout {
    pub modal_x: f64,
    pub modal_y: f64,
    pub modal_w: f64,
    pub modal_h: f64,
    pub cmd_box_h: f64,
    pub btn_y: f64,
    pub cancel_rect: (f64, f64, f64, f64),
    pub copy_rect: (f64, f64, f64, f64),
    pub exec_rect: (f64, f64, f64, f64),
}

impl ConfirmRunModalLayout {
    #[inline]
    pub fn cancel_contains(&self, x: f64, y: f64) -> bool {
        x >= self.cancel_rect.0
            && x <= self.cancel_rect.0 + self.cancel_rect.2
            && y >= self.cancel_rect.1
            && y <= self.cancel_rect.1 + self.cancel_rect.3
    }

    #[inline]
    pub fn copy_contains(&self, x: f64, y: f64) -> bool {
        x >= self.copy_rect.0
            && x <= self.copy_rect.0 + self.copy_rect.2
            && y >= self.copy_rect.1
            && y <= self.copy_rect.1 + self.copy_rect.3
    }

    #[inline]
    pub fn exec_contains(&self, x: f64, y: f64) -> bool {
        x >= self.exec_rect.0
            && x <= self.exec_rect.0 + self.exec_rect.2
            && y >= self.exec_rect.1
            && y <= self.exec_rect.1 + self.exec_rect.3
    }
}

#[derive(Debug, Clone)]
pub struct ConfirmRunCommandModal {
    pub paste: bool,
    pub target_session_id: Option<String>,
    pub is_open: bool,
    pub command: String,
    pub source_title: String,
    pub is_dangerous: bool,
    pub danger_reason: Option<String>,
    pub target_tab_title: String,
    pub can_execute: bool,
    pub copied_feedback: bool,
}

impl ConfirmRunCommandModal {
    pub fn layout(&self, window_w: f64, window_h: f64) -> ConfirmRunModalLayout {
        let modal_w = 600.0f64.min(window_w - 40.0);
        let line_count = self.command.lines().count().max(1);
        let char_len = self.command.len();
        let est_lines = line_count.max((char_len / 55) + 1).min(8);
        let cmd_box_h = (est_lines as f64 * 20.0 + 24.0).max(48.0);
        let base_h = if self.is_dangerous { 280.0 } else { 240.0 };
        let modal_h = (base_h + cmd_box_h).min(window_h - 40.0);
        let modal_x = (window_w - modal_w) / 2.0;
        let modal_y = (window_h - modal_h) / 2.0;
        let btn_y = modal_y + modal_h - 52.0;

        let cancel_rect = (modal_x + modal_w - 360.0, btn_y, 96.0, 34.0);
        let copy_rect = (modal_x + modal_w - 252.0, btn_y, 108.0, 34.0);
        let exec_rect = (modal_x + modal_w - 132.0, btn_y, 110.0, 34.0);

        ConfirmRunModalLayout {
            modal_x,
            modal_y,
            modal_w,
            modal_h,
            cmd_box_h,
            btn_y,
            cancel_rect,
            copy_rect,
            exec_rect,
        }
    }
}

pub fn check_dangerous_command(cmd: &str) -> (bool, Option<String>) {
    let lower = cmd.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    if lower.contains("rm -rf") || lower.contains("rm -fr") || lower.contains("rm -r") {
        return (
            true,
            Some("Рекурсивное удаление файлов и каталогов (rm -r)".into()),
        );
    }
    if words.contains(&"rm") || words.contains(&"rmdir") {
        return (
            true,
            Some("Команда содержит операцию удаления файлов/папок".into()),
        );
    }
    if words.contains(&"reboot")
        || words.contains(&"shutdown")
        || lower.contains("init 0")
        || lower.contains("init 6")
    {
        return (
            true,
            Some("Команда перезагрузит или выключит сервер".into()),
        );
    }
    if words.contains(&"kill") || words.contains(&"killall") || words.contains(&"pkill") {
        return (
            true,
            Some("Команда принудительно завершит системные процессы".into()),
        );
    }
    if lower.contains("dd if=")
        || words.contains(&"mkfs")
        || words.contains(&"fdisk")
        || words.contains(&"parted")
    {
        return (
            true,
            Some("Низкоуровневая операция с дисками и разделами".into()),
        );
    }
    if lower.contains("drop database")
        || lower.contains("drop table")
        || lower.contains("truncate ")
    {
        return (true, Some("Удаление таблиц или базы данных".into()));
    }
    if lower.contains("chmod -r 777") || lower.contains("chmod 777") {
        return (
            true,
            Some("Открытие полного небезопасного доступа ко всем файлам (777)".into()),
        );
    }
    if lower.contains("> /dev/") {
        return (true, Some("Прямая перезапись системного устройства".into()));
    }
    (false, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    About,
    Security,
    Backup,
    General,
}

#[derive(Debug, Clone)]
pub struct SettingsModal {
    pub is_open: bool,
    pub active_tab: SettingsTab,
    // Security fields
    pub current_password: TextInput,
    pub new_password: TextInput,
    pub confirm_password: TextInput,
    pub active_security_field: usize,
    pub security_msg: Option<(String, bool)>, // (msg, is_error)

    // Backup state
    pub backup_msg: Option<(String, bool)>,
    pub backup_password: TextInput,
    pub backup_confirm: TextInput,

    // General fields
    pub default_user: TextInput,
    pub scrollback_lines: TextInput,
    pub confirm_delete: bool,
    pub record_command_history: bool,
    pub active_general_field: usize,
    pub general_msg: Option<(String, bool)>,
    pub show_passwords: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AddSnippetModal {
    pub is_open: bool,
    pub is_editing: bool,
    pub editing_id: Option<i64>,
    pub title: TextInput,
    pub command: TextInput,
    pub category: TextInput,
    pub description: TextInput,
    pub active_field: usize, // 0=title, 1=command, 2=category, 3=description
    pub error_msg: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfirmDeleteSnippetModal {
    pub is_open: bool,
    pub snippet_id: i64,
    pub snippet_title: String,
    pub snippet_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    None,
    ModalDialogBody,
    HeaderTab(usize),
    HeaderTabClose(usize),
    HeaderAddBtn,
    HeaderSearchBtn,
    SidebarNav(SidebarTab),
    SidebarAddHostBtn,
    SidebarSplitter,
    SidebarSearchHost,
    SidebarSearchSnippet,
    SidebarSearchCommand,
    HostCard(usize),
    HostConnect(usize),
    HostEdit(usize),
    HostDelete(usize),
    HostDeployKey(usize),
    SessionCard(usize),
    CommandItem(usize),
    SnippetItem(usize),
    ModalField(usize),
    ModalAuthMode(AuthModeChoice),
    ModalBrowseKey,
    ModalGenerateKey,
    ModalTogglePassword,
    ModalSave,
    ModalConnect,
    ModalClose,
    PaletteItem(usize),
    WelcomeAdd,
    WelcomeConnect,
    TerminalScrollbarThumb,
    TerminalScrollbarTrack,
    ContextMenu(ContextMenuItem),
    TerminalArea,
    // KeyToolModal
    KeyToolAlgo(KeyAlgo),
    KeyToolField(usize),
    KeyToolGenerateBtn,
    KeyToolCopyPubBtn,
    KeyToolSendTermBtn,
    KeyToolCopyCmdBtn,
    KeyToolBindHostBtn,
    KeyToolCloseBtn,
    // Input Context Menu
    InputContextMenu(InputMenuAction),
    // Session Dashboard Modal
    SessionDashboardClose,
    SessionDashboardReconnect,
    SessionDashboardDelete,
    SessionDashboardRunCmd(usize),
    SessionDashboardCopyCmd(usize),
    // Snippets Actions
    SidebarAddSnippetBtn,
    SnippetRun(usize),
    SnippetCopy(usize),
    SnippetEdit(usize),
    SnippetDelete(usize),
    ConfirmDeleteSnippetConfirm,
    ConfirmDeleteSnippetCancel,
    // Add Snippet Modal
    ModalSnippetField(usize),
    ModalSnippetSave,
    ModalSnippetClose,
    // Host Key Modal
    HostKeyModalAccept,
    HostKeyModalReject,
    HostKeyModalCopy,
    // Settings & Security
    HeaderSettingsBtn,
    HeaderLangBtn,
    SettingsSetLanguage(crate::config::AppLanguage),
    ConfirmDeleteConfirm,
    ConfirmDeleteCancel,
    SettingsClose,
    SettingsTab(SettingsTab),
    SettingsAboutLinkDesk,
    SettingsAboutLinkEmail,
    SettingsAboutLinkTelegram,
    SettingsAboutLinkGithub,
    SettingsSavePassword,
    SettingsRemovePassword,
    SettingsExportJson,
    SettingsImportJson,
    SettingsSaveGeneral,
    SettingsToggleConfirmDelete,
    SettingsToggleHistory,
    SettingsBackupField(usize),
    SettingsSecurityField(usize),
    SettingsGeneralField(usize),
    SettingsTogglePassword,
    LockUnlockBtn,
    LockPasswordField,
    // Confirm Run Command Modal
    ConfirmRunExecute,
    ConfirmRunCancel,
    ConfirmRunCopy,
    // Terminal Reconnect & Quick Actions
    TerminalReconnect(usize),
    TerminalCloseTab(usize),
    TerminalStatusBanner(usize),
    QuickAction(QuickActionCmd),
    TerminalSearchOpenBtn,
    TerminalSearchCloseBtn,
    TerminalSearchPrevBtn,
    TerminalSearchNextBtn,
    TerminalSearchCaseToggle,
    TerminalSearchInput,
    TerminalSplitBtn,
    TerminalSplitClose,
    SplitTerminalArea,
}

#[derive(Debug, Clone)]
pub struct AddHostModal {
    pub is_open: bool,
    pub is_editing: bool,
    pub editing_id: Option<String>,
    pub name: TextInput,
    pub hostname: TextInput,
    pub port: TextInput,
    pub username: TextInput,
    pub group: TextInput,
    pub auth_mode: AuthModeChoice,
    pub key_path: TextInput,
    pub password: TextInput,
    pub show_password: bool,
    pub badge_color: String,
    pub active_field: usize, // 0=Name, 1=Host, 2=Port, 3=User, 4=Group, 5=Key/Pass
    pub error_msg: Option<String>,
}

impl Default for AddHostModal {
    fn default() -> Self {
        Self {
            is_open: false,
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
            badge_color: "#2dd4bf".to_string(),
            active_field: 0,
            error_msg: None,
        }
    }
}

pub struct HostKeyPromptModal {
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub is_changed: bool,
    pub old_line: Option<usize>,
    pub response_tx: Option<tokio::sync::oneshot::Sender<bool>>,
    pub copied_feedback: bool,
}

pub enum AppEvent {
    SshOutput {
        session_id: String,
        data: Vec<u8>,
    },
    SshConnected {
        session_id: String,
        handle: SshSessionHandle,
    },
    SshDisconnected {
        session_id: String,
        error: Option<String>,
    },
    HostKeyPrompt {
        session_id: String,
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        is_changed: bool,
        old_line: Option<usize>,
        response_tx: tokio::sync::oneshot::Sender<bool>,
    },
}

pub struct SessionTab {
    pub id: String,
    pub title: String,
    pub host_address: String,
    pub username: String,
    pub terminal: Terminal,
    pub handle: Option<SshSessionHandle>,
    pub status: String,
    pub host_id: Option<String>,
    pub host_config: Option<HostConfig>,
}

pub struct AppState {
    pub hosts_doc: HostsDocument,
    config_store: ConfigStore,
    pub pending_host_keys: VecDeque<HostKeyPromptModal>,
    pub pending_connections: HashMap<String, tokio::task::AbortHandle>,
    pub db: HistoryDb,
    pub sidebar_tab: SidebarTab,
    pub tabs: Vec<SessionTab>,
    pub active_tab: usize,

    // Modal dialogs
    pub add_host_modal: AddHostModal,
    pub command_search_input: TextInput,
    pub command_search_results: Vec<CommandRecord>,
    pub selected_search_idx: usize,
    pub palette_open: bool,

    // Hover state
    pub hover: HoverTarget,

    // Caret blink state
    pub caret_blink_time: std::time::Instant,
    pub caret_visible: bool,

    // UI Metrics
    pub sidebar_width: f64,
    pub is_dragging_sidebar: bool,
    pub sidebar_drag_start_x: f64,
    pub sidebar_drag_start_width: f64,
    pub header_height: f64,
    pub footer_height: f64,
    pub terminal_cols: usize,
    pub terminal_char_width: f64,
    pub terminal_rows: usize,

    // Modals
    pub key_tool_modal: KeyToolModal,
    pub add_snippet_modal: AddSnippetModal,
    pub session_dashboard_modal: Option<SessionDashboardModal>,
    pub confirm_delete_modal: Option<ConfirmDeleteHostModal>,
    pub confirm_delete_snippet_modal: Option<ConfirmDeleteSnippetModal>,
    pub confirm_run_modal: Option<ConfirmRunCommandModal>,
    pub host_key_modal: Option<HostKeyPromptModal>,
    pub settings_modal: Option<SettingsModal>,
    pub input_context_menu: Option<InputContextMenu>,
    pub tooltip: Option<TooltipState>,

    // App Lock
    pub is_app_locked: bool,
    pub lock_password_input: TextInput,
    pub lock_error_msg: Option<String>,

    // Search and filters
    pub host_search_input: TextInput,
    pub snippet_search_input: TextInput,
    pub focused_input: Option<InputFieldId>,
    pub search_state: TerminalSearchState,
    pub split_tab: Option<usize>,
    pub split_vertical: bool,

    // Terminal Interaction
    pub selection: Option<TerminalSelection>,
    pub is_mouse_selecting: bool,
    pub context_menu: Option<ContextMenu>,
    pub is_dragging_scrollbar: bool,
    pub scrollbar_drag_start_y: f64,
    pub scrollbar_drag_start_offset: usize,

    // Input Drag Selection
    pub is_input_selecting: bool,
    pub input_select_anchor: usize,
    pub input_select_field: Option<InputFieldId>,
    pub input_select_origin_x: f64,

    // Cached lists
    pub recent_sessions: Vec<SessionRecord>,
    pub snippets: Vec<SnippetRecord>,
    pub selected_sidebar_item: usize,

    // Window Size Cache
    pub last_window_w: f64,
    pub last_window_h: f64,

    // Async Bridge
    pub event_tx: Sender<AppEvent>,
    pub event_rx: Receiver<AppEvent>,
    pub tokio_handle: tokio::runtime::Handle,
}

pub const TERMINAL_CHAR_WIDTH: f64 = 7.6171875;
pub const TERMINAL_LINE_HEIGHT: f64 = 18.0;

impl AppState {
    pub fn new(tokio_handle: tokio::runtime::Handle) -> Result<Self, String> {
        #[cfg(not(test))]
        let mut config_store = ConfigStore::new(
            crate::config::hosts_file_path(),
            std::sync::Arc::new(crate::secrets::SystemSecrets),
        );
        #[cfg(test)]
        let mut config_store = ConfigStore::temporary();
        let hosts_doc = config_store.load()?;
        let is_app_locked = hosts_doc.settings.master_password_hash.is_some();
        #[cfg(not(test))]
        let db =
            HistoryDb::open_default().map_err(|e| format!("Cannot open history database: {e}"))?;
        #[cfg(test)]
        let db = HistoryDb::open_in_memory().map_err(|e| e.to_string())?;

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(128);

        let recent_sessions = db.list_recent_sessions(30).unwrap_or_default();
        let snippets = db.list_snippets().unwrap_or_default();
        let initial_cmds = db.list_recent_commands(50).unwrap_or_default();

        Ok(Self {
            hosts_doc,
            config_store,
            pending_host_keys: VecDeque::new(),
            pending_connections: HashMap::new(),
            db,
            sidebar_tab: SidebarTab::Hosts,
            tabs: Vec::new(),
            active_tab: 0,
            add_host_modal: AddHostModal::default(),
            command_search_input: TextInput::default(),
            command_search_results: initial_cmds,
            selected_search_idx: 0,
            palette_open: false,
            hover: HoverTarget::None,
            caret_blink_time: std::time::Instant::now(),
            caret_visible: true,
            sidebar_width: 280.0,
            is_dragging_sidebar: false,
            sidebar_drag_start_x: 0.0,
            sidebar_drag_start_width: 280.0,
            header_height: 44.0,
            footer_height: 28.0,
            terminal_cols: 100,
            terminal_char_width: TERMINAL_CHAR_WIDTH,
            terminal_rows: 30,
            key_tool_modal: KeyToolModal::default(),
            add_snippet_modal: AddSnippetModal::default(),
            session_dashboard_modal: None,
            confirm_delete_modal: None,
            confirm_delete_snippet_modal: None,
            confirm_run_modal: None,
            host_key_modal: None,
            settings_modal: None,
            input_context_menu: None,
            tooltip: None,
            is_app_locked,
            lock_password_input: TextInput::default(),
            lock_error_msg: None,
            host_search_input: TextInput::default(),
            snippet_search_input: TextInput::default(),
            focused_input: None,
            search_state: TerminalSearchState::default(),
            split_tab: None,
            split_vertical: true,
            selection: None,
            is_mouse_selecting: false,
            context_menu: None,
            is_dragging_scrollbar: false,
            scrollbar_drag_start_y: 0.0,
            scrollbar_drag_start_offset: 0,
            is_input_selecting: false,
            input_select_anchor: 0,
            input_select_field: None,
            input_select_origin_x: 0.0,
            recent_sessions,
            snippets,
            selected_sidebar_item: 0,
            last_window_w: 1120.0,
            last_window_h: 720.0,
            event_tx,
            event_rx,
            tokio_handle,
        })
    }

    pub fn resize_terminals(&mut self, logical_w: f64, logical_h: f64) {
        self.last_window_w = logical_w;
        self.last_window_h = logical_h;

        let term_w = (logical_w - self.sidebar_width).max(0.0);
        let term_h = (logical_h - self.header_height - self.footer_height).max(0.0);

        let eff_term_w = if self.split_tab.is_some() {
            ((term_w / 2.0).floor() - 2.0).max(0.0)
        } else {
            term_w
        };

        let cols = ((eff_term_w - 24.0) / self.terminal_char_width)
            .floor()
            .max(20.0) as usize;
        // Available vertical height inside terminal viewport:
        // Subtract toolbar (30.0) and top+bottom padding (6.0 + 6.0 = 12.0) -> 42.0.
        // This ensures the PTY row count exactly matches the visible lines in UI,
        // so the active prompt at the bottom line is always visible.
        let rows = ((term_h - 42.0) / TERMINAL_LINE_HEIGHT).floor().max(2.0) as usize;

        if self.terminal_cols != cols || self.terminal_rows != rows {
            self.terminal_cols = cols;
            self.terminal_rows = rows;

            for tab in &mut self.tabs {
                tab.terminal.resize(cols, rows);
                if let Some(handle) = &tab.handle {
                    let handle_clone = handle.resize_tx.clone();
                    let c = cols as u32;
                    let r = rows as u32;
                    self.tokio_handle.spawn(async move {
                        let _ = handle_clone.send((c, r)).await;
                    });
                }
            }
        }
    }

    pub fn filtered_hosts(&self) -> Vec<(usize, &HostConfig)> {
        let q = self.host_search_input.text.trim().to_lowercase();
        if q.is_empty() {
            self.hosts_doc.hosts.iter().enumerate().collect()
        } else {
            self.hosts_doc
                .hosts
                .iter()
                .enumerate()
                .filter(|(_, h)| {
                    h.name.to_lowercase().contains(&q)
                        || h.hostname.to_lowercase().contains(&q)
                        || h.username.to_lowercase().contains(&q)
                        || h.group.as_deref().unwrap_or("").to_lowercase().contains(&q)
                })
                .collect()
        }
    }

    pub fn update_hover(&mut self, x: f64, y: f64, w: f64, h: f64) -> (bool, CursorIcon) {
        let old_hover = self.hover;
        let mut new_hover = HoverTarget::None;
        let mut cursor = CursorIcon::Default;

        // -0.1 App Lock Screen (blocks all other interactions if locked)
        if self.is_app_locked {
            let modal_w = 400.0f64.min(w - 40.0);
            let modal_h = 290.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;

            if x >= modal_x + 30.0 && x <= modal_x + modal_w - 30.0 {
                if y >= modal_y + 130.0 && y <= modal_y + 168.0 {
                    new_hover = HoverTarget::LockPasswordField;
                    cursor = CursorIcon::Text;
                } else if y >= modal_y + 180.0 && y <= modal_y + 220.0 {
                    new_hover = HoverTarget::LockUnlockBtn;
                    cursor = CursorIcon::Pointer;
                }
            }
            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // -0.05 Host Key Prompt Modal
        if let Some(host_key) = &self.host_key_modal {
            let modal_w = 560.0f64.min(w - 40.0);
            let modal_h = (if host_key.is_changed { 320.0 } else { 280.0 } as f64).min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;

            if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h {
                new_hover = HoverTarget::ModalDialogBody;
                cursor = CursorIcon::Default;

                let fp_y = modal_y + 110.0;
                if y >= fp_y
                    && y <= fp_y + 34.0
                    && x >= modal_x + modal_w - 110.0
                    && x <= modal_x + modal_w - 24.0
                {
                    new_hover = HoverTarget::HostKeyModalCopy;
                    cursor = CursorIcon::Pointer;
                }

                let btn_y = modal_y + modal_h - 52.0;
                if y >= btn_y && y <= btn_y + 36.0 {
                    if host_key.is_changed {
                        if x >= modal_x + modal_w - 220.0 && x <= modal_x + modal_w - 20.0 {
                            new_hover = HoverTarget::HostKeyModalAccept;
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + modal_w - 380.0 && x <= modal_x + modal_w - 230.0 {
                            new_hover = HoverTarget::HostKeyModalReject;
                            cursor = CursorIcon::Pointer;
                        }
                    } else {
                        if x >= modal_x + modal_w - 230.0 && x <= modal_x + modal_w - 20.0 {
                            new_hover = HoverTarget::HostKeyModalAccept;
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + modal_w - 360.0 && x <= modal_x + modal_w - 240.0 {
                            new_hover = HoverTarget::HostKeyModalReject;
                            cursor = CursorIcon::Pointer;
                        }
                    }
                }
                let changed = old_hover != new_hover;
                self.hover = new_hover;
                return (changed, cursor);
            }
            self.hover = HoverTarget::None;
            return (old_hover != HoverTarget::None, CursorIcon::Default);
        }

        // 0. Confirm Delete Host Modal
        if let Some(confirm) = &self.confirm_delete_modal {
            if confirm.is_open {
                let modal_w = 480.0f64.min(w - 40.0);
                let modal_h = 210.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;

                if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h
                {
                    new_hover = HoverTarget::ModalDialogBody;
                    cursor = CursorIcon::Default;
                    let btn_y = modal_y + modal_h - 52.0;

                    if y >= btn_y && y <= btn_y + 36.0 {
                        if x >= modal_x + modal_w - 240.0 && x <= modal_x + modal_w - 130.0 {
                            new_hover = HoverTarget::ConfirmDeleteCancel;
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + modal_w - 120.0 && x <= modal_x + modal_w - 20.0 {
                            new_hover = HoverTarget::ConfirmDeleteConfirm;
                            cursor = CursorIcon::Pointer;
                        }
                    }
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0.002 Confirm Delete Snippet Modal
        if let Some(confirm) = &self.confirm_delete_snippet_modal {
            if confirm.is_open {
                let modal_w = 480.0f64.min(w - 40.0);
                let modal_h = 220.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;

                if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h
                {
                    new_hover = HoverTarget::ModalDialogBody;
                    cursor = CursorIcon::Default;
                    let btn_y = modal_y + modal_h - 52.0;

                    if y >= btn_y && y <= btn_y + 36.0 {
                        if x >= modal_x + modal_w - 240.0 && x <= modal_x + modal_w - 130.0 {
                            new_hover = HoverTarget::ConfirmDeleteSnippetCancel;
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + modal_w - 120.0 && x <= modal_x + modal_w - 20.0 {
                            new_hover = HoverTarget::ConfirmDeleteSnippetConfirm;
                            cursor = CursorIcon::Pointer;
                        }
                    }
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0.005 Confirm Run Command Modal
        if let Some(confirm_run) = &self.confirm_run_modal {
            if confirm_run.is_open {
                let layout = confirm_run.layout(w, h);

                if x >= layout.modal_x
                    && x <= layout.modal_x + layout.modal_w
                    && y >= layout.modal_y
                    && y <= layout.modal_y + layout.modal_h
                {
                    new_hover = HoverTarget::ModalDialogBody;
                    cursor = CursorIcon::Default;

                    if layout.cancel_contains(x, y) {
                        new_hover = HoverTarget::ConfirmRunCancel;
                        cursor = CursorIcon::Pointer;
                    } else if layout.copy_contains(x, y) {
                        new_hover = HoverTarget::ConfirmRunCopy;
                        cursor = CursorIcon::Pointer;
                    } else if layout.exec_contains(x, y) {
                        new_hover = HoverTarget::ConfirmRunExecute;
                        cursor = if confirm_run.can_execute {
                            CursorIcon::Pointer
                        } else {
                            CursorIcon::NotAllowed
                        };
                    }

                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0.01 Settings Modal
        if let Some(settings) = &self.settings_modal {
            if settings.is_open {
                let modal_w = 620.0f64.min(w - 40.0);
                let modal_h = 490.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;

                if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h
                {
                    new_hover = HoverTarget::ModalDialogBody;
                    cursor = CursorIcon::Default;

                    // Close button
                    if x >= modal_x + modal_w - 38.0
                        && x <= modal_x + modal_w - 14.0
                        && y >= modal_y + 14.0
                        && y <= modal_y + 38.0
                    {
                        new_hover = HoverTarget::SettingsClose;
                        cursor = CursorIcon::Pointer;
                    }
                    // Tabs
                    else if y >= modal_y + 55.0 && y <= modal_y + 88.0 {
                        if x >= modal_x + 24.0 && x <= modal_x + 134.0 {
                            new_hover = HoverTarget::SettingsTab(SettingsTab::About);
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + 140.0 && x <= modal_x + 275.0 {
                            new_hover = HoverTarget::SettingsTab(SettingsTab::Security);
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + 281.0 && x <= modal_x + 426.0 {
                            new_hover = HoverTarget::SettingsTab(SettingsTab::Backup);
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + 432.0 && x <= modal_x + 562.0 {
                            new_hover = HoverTarget::SettingsTab(SettingsTab::General);
                            cursor = CursorIcon::Pointer;
                        }
                    }
                    // Tab content
                    else {
                        match settings.active_tab {
                            SettingsTab::About => {
                                let c1_x = modal_x + 24.0;
                                let c2_x = modal_x + 316.0;
                                let c_w = 276.0;
                                let r1_y = modal_y + 244.0;
                                let r2_y = modal_y + 308.0;
                                let c_h = 54.0;

                                if y >= r1_y && y <= r1_y + c_h {
                                    if x >= c1_x && x <= c1_x + c_w {
                                        new_hover = HoverTarget::SettingsAboutLinkDesk;
                                        cursor = CursorIcon::Pointer;
                                    } else if x >= c2_x && x <= c2_x + c_w {
                                        new_hover = HoverTarget::SettingsAboutLinkTelegram;
                                        cursor = CursorIcon::Pointer;
                                    }
                                } else if y >= r2_y && y <= r2_y + c_h {
                                    if x >= c1_x && x <= c1_x + c_w {
                                        new_hover = HoverTarget::SettingsAboutLinkEmail;
                                        cursor = CursorIcon::Pointer;
                                    } else if x >= c2_x && x <= c2_x + c_w {
                                        new_hover = HoverTarget::SettingsAboutLinkGithub;
                                        cursor = CursorIcon::Pointer;
                                    }
                                }
                            }
                            SettingsTab::Security => {
                                let f_y_base = modal_y + 155.0;
                                let has_existing =
                                    self.hosts_doc.settings.master_password_hash.is_some();
                                let start_field = if has_existing { 0 } else { 1 };
                                let end_field = 3;

                                let mut cy = f_y_base;
                                for f_idx in start_field..end_field {
                                    let inp_w = modal_w - 220.0 - 42.0;
                                    if x >= modal_x + 190.0
                                        && x <= modal_x + 190.0 + inp_w
                                        && y >= cy
                                        && y <= cy + 32.0
                                    {
                                        new_hover = HoverTarget::SettingsSecurityField(f_idx);
                                        cursor = CursorIcon::Text;
                                        break;
                                    } else if x >= modal_x + 190.0 + inp_w + 8.0
                                        && x <= modal_x + modal_w - 30.0
                                        && y >= cy
                                        && y <= cy + 32.0
                                    {
                                        new_hover = HoverTarget::SettingsTogglePassword;
                                        cursor = CursorIcon::Pointer;
                                        break;
                                    }
                                    cy += 44.0;
                                }

                                if new_hover == HoverTarget::ModalDialogBody {
                                    let btn_y = modal_y + modal_h - 52.0;
                                    if y >= btn_y && y <= btn_y + 34.0 {
                                        if has_existing {
                                            if x >= modal_x + modal_w - 320.0
                                                && x <= modal_x + modal_w - 170.0
                                            {
                                                new_hover = HoverTarget::SettingsRemovePassword;
                                                cursor = CursorIcon::Pointer;
                                            } else if x >= modal_x + modal_w - 160.0
                                                && x <= modal_x + modal_w - 24.0
                                            {
                                                new_hover = HoverTarget::SettingsSavePassword;
                                                cursor = CursorIcon::Pointer;
                                            }
                                        } else {
                                            if x >= modal_x + modal_w - 180.0
                                                && x <= modal_x + modal_w - 24.0
                                            {
                                                new_hover = HoverTarget::SettingsSavePassword;
                                                cursor = CursorIcon::Pointer;
                                            }
                                        }
                                    }
                                }
                            }
                            SettingsTab::Backup => {
                                for index in 0..2 {
                                    let cy = modal_y + 330.0 + index as f64 * 40.0;
                                    if x >= modal_x + 200.0
                                        && x <= modal_x + modal_w - 30.0
                                        && y >= cy
                                        && y <= cy + 32.0
                                    {
                                        new_hover = HoverTarget::SettingsBackupField(index);
                                        cursor = CursorIcon::Text;
                                    }
                                }
                                let exp_y = modal_y + 164.0;
                                let imp_y = modal_y + 274.0;
                                if x >= modal_x + 38.0
                                    && x <= modal_x + 238.0
                                    && y >= exp_y
                                    && y <= exp_y + 30.0
                                {
                                    new_hover = HoverTarget::SettingsExportJson;
                                    cursor = CursorIcon::Pointer;
                                } else if x >= modal_x + 38.0
                                    && x <= modal_x + 238.0
                                    && y >= imp_y
                                    && y <= imp_y + 30.0
                                {
                                    new_hover = HoverTarget::SettingsImportJson;
                                    cursor = CursorIcon::Pointer;
                                }
                            }
                            SettingsTab::General => {
                                let lang_y = modal_y + 105.0;
                                if y >= lang_y && y <= lang_y + 30.0 {
                                    if x >= modal_x + 240.0 && x <= modal_x + 340.0 {
                                        new_hover = HoverTarget::SettingsSetLanguage(
                                            crate::config::AppLanguage::Ru,
                                        );
                                        cursor = CursorIcon::Pointer;
                                    } else if x >= modal_x + 350.0 && x <= modal_x + 450.0 {
                                        new_hover = HoverTarget::SettingsSetLanguage(
                                            crate::config::AppLanguage::En,
                                        );
                                        cursor = CursorIcon::Pointer;
                                    }
                                }

                                let mut cy = modal_y + 150.0;
                                for f_idx in 0..2 {
                                    if x >= modal_x + 240.0
                                        && x <= modal_x + modal_w - 30.0
                                        && y >= cy
                                        && y <= cy + 32.0
                                    {
                                        new_hover = HoverTarget::SettingsGeneralField(f_idx);
                                        cursor = CursorIcon::Text;
                                        break;
                                    }
                                    cy += 45.0;
                                }
                                if new_hover == HoverTarget::ModalDialogBody {
                                    if x >= modal_x + 24.0
                                        && x <= modal_x + modal_w - 24.0
                                        && y >= modal_y + 282.0
                                        && y <= modal_y + 310.0
                                    {
                                        new_hover = HoverTarget::SettingsToggleHistory;
                                        cursor = CursorIcon::Pointer;
                                    }
                                    // Checkbox confirm delete
                                    if x >= modal_x + 24.0
                                        && x <= modal_x + 420.0
                                        && y >= cy
                                        && y <= cy + 28.0
                                    {
                                        new_hover = HoverTarget::SettingsToggleConfirmDelete;
                                        cursor = CursorIcon::Pointer;
                                    }
                                    let btn_y = modal_y + modal_h - 52.0;
                                    if x >= modal_x + modal_w - 190.0
                                        && x <= modal_x + modal_w - 24.0
                                        && y >= btn_y
                                        && y <= btn_y + 34.0
                                    {
                                        new_hover = HoverTarget::SettingsSaveGeneral;
                                        cursor = CursorIcon::Pointer;
                                    }
                                }
                            }
                        }
                    }

                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0. Input Context Menu (highest z-order)
        if let Some(menu) = &self.input_context_menu {
            if menu.is_open {
                let menu_w = 170.0;
                let menu_h = 132.0;
                if x >= menu.x && x <= menu.x + menu_w && y >= menu.y && y <= menu.y + menu_h {
                    let rel_y = y - (menu.y + 6.0);
                    if rel_y >= 0.0 && rel_y < 30.0 {
                        new_hover = HoverTarget::InputContextMenu(InputMenuAction::Cut);
                    } else if rel_y >= 30.0 && rel_y < 60.0 {
                        new_hover = HoverTarget::InputContextMenu(InputMenuAction::Copy);
                    } else if rel_y >= 60.0 && rel_y < 90.0 {
                        new_hover = HoverTarget::InputContextMenu(InputMenuAction::Paste);
                    } else if rel_y >= 90.0 && rel_y < 120.0 {
                        new_hover = HoverTarget::InputContextMenu(InputMenuAction::SelectAll);
                    }
                    cursor = CursorIcon::Pointer;
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0.05 Terminal Context Menu
        if let Some(menu) = &self.context_menu {
            if menu.is_open {
                let menu_w = 180.0;
                let menu_h = 108.0;
                if x >= menu.x && x <= menu.x + menu_w && y >= menu.y && y <= menu.y + menu_h {
                    let rel_y = y - (menu.y + 6.0);
                    if rel_y >= 0.0 && rel_y < 32.0 {
                        new_hover = HoverTarget::ContextMenu(ContextMenuItem::Copy);
                    } else if rel_y >= 32.0 && rel_y < 64.0 {
                        new_hover = HoverTarget::ContextMenu(ContextMenuItem::SearchGoogle);
                    } else if rel_y >= 64.0 && rel_y < 96.0 {
                        new_hover = HoverTarget::ContextMenu(ContextMenuItem::Paste);
                    }
                    cursor = CursorIcon::Pointer;
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0.1 Session Dashboard Modal
        if let Some(dash) = &self.session_dashboard_modal {
            if dash.is_open {
                let modal_w = 640.0f64.min(w - 40.0);
                let modal_h = 520.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;

                if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h
                {
                    new_hover = HoverTarget::ModalDialogBody;
                    cursor = CursorIcon::Default;

                    if x >= modal_x + modal_w - 40.0 && y <= modal_y + 40.0 {
                        new_hover = HoverTarget::SessionDashboardClose;
                        cursor = CursorIcon::Pointer;
                    } else if y >= modal_y + modal_h - 48.0 && y <= modal_y + modal_h - 14.0 {
                        if x >= modal_x + 24.0 && x <= modal_x + 140.0 {
                            new_hover = HoverTarget::SessionDashboardDelete;
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + modal_w - 234.0 && x <= modal_x + modal_w - 24.0 {
                            new_hover = HoverTarget::SessionDashboardReconnect;
                            cursor = CursorIcon::Pointer;
                        }
                    } else {
                        let cmd_start_y = modal_y + 170.0;
                        for (cmd_idx, _) in dash.commands.iter().enumerate().take(8) {
                            let item_y = cmd_start_y + (cmd_idx as f64) * 38.0;
                            if y >= item_y && y <= item_y + 32.0 {
                                if x >= modal_x + modal_w - 60.0 && x <= modal_x + modal_w - 30.0 {
                                    new_hover = HoverTarget::SessionDashboardRunCmd(cmd_idx);
                                    cursor = CursorIcon::Pointer;
                                    break;
                                } else if x >= modal_x + modal_w - 96.0
                                    && x < modal_x + modal_w - 60.0
                                {
                                    new_hover = HoverTarget::SessionDashboardCopyCmd(cmd_idx);
                                    cursor = CursorIcon::Pointer;
                                    break;
                                }
                            }
                        }
                    }
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }
        }

        // 0.2 Add Snippet Modal
        if self.add_snippet_modal.is_open {
            let modal_w = 520.0f64.min(w - 40.0);
            let modal_h = 420.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;

            if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h {
                new_hover = HoverTarget::ModalDialogBody;
                cursor = CursorIcon::Default;

                if x >= modal_x + modal_w - 40.0 && y <= modal_y + 40.0 {
                    new_hover = HoverTarget::ModalSnippetClose;
                    cursor = CursorIcon::Pointer;
                } else {
                    let mut f_y = modal_y + 55.0;
                    for idx in 0..4 {
                        if x >= modal_x + 24.0
                            && x <= modal_x + modal_w - 24.0
                            && y >= f_y
                            && y <= f_y + 52.0
                        {
                            new_hover = HoverTarget::ModalSnippetField(idx);
                            cursor = CursorIcon::Text;
                            break;
                        }
                        f_y += 58.0;
                    }

                    let btn_y = modal_y + modal_h - 48.0;
                    if y >= btn_y && y <= btn_y + 34.0 {
                        if x >= modal_x + modal_w - 140.0 && x <= modal_x + modal_w - 24.0 {
                            new_hover = HoverTarget::ModalSnippetSave;
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + modal_w - 240.0 && x <= modal_x + modal_w - 150.0 {
                            new_hover = HoverTarget::ModalSnippetClose;
                            cursor = CursorIcon::Pointer;
                        }
                    }
                }
                let changed = old_hover != new_hover;
                self.hover = new_hover;
                return (changed, cursor);
            }
        }

        // 0.1 Key Tool Modal (SSH Key Generator & Deployer)
        if self.key_tool_modal.is_open {
            let modal_w = 580.0f64.min(w - 40.0);
            let modal_h = 520.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;

            if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h {
                new_hover = HoverTarget::ModalDialogBody;
                cursor = CursorIcon::Default;

                if x >= modal_x + modal_w - 40.0 && y <= modal_y + 40.0 {
                    new_hover = HoverTarget::KeyToolCloseBtn;
                    cursor = CursorIcon::Pointer;
                } else {
                    let algo_y = modal_y + 55.0;
                    if y >= algo_y && y <= algo_y + 28.0 {
                        if x >= modal_x + 160.0 && x <= modal_x + 300.0 {
                            new_hover = HoverTarget::KeyToolAlgo(KeyAlgo::Ed25519);
                            cursor = CursorIcon::Pointer;
                        } else if x >= modal_x + 310.0 && x <= modal_x + 420.0 {
                            new_hover = HoverTarget::KeyToolAlgo(KeyAlgo::Rsa4096);
                            cursor = CursorIcon::Pointer;
                        }
                    }

                    let mut f_y = modal_y + 92.0;
                    for idx in 0..3 {
                        if x >= modal_x + 160.0
                            && x <= modal_x + modal_w - 24.0
                            && y >= f_y
                            && y <= f_y + 32.0
                        {
                            new_hover = HoverTarget::KeyToolField(idx);
                            cursor = CursorIcon::Text;
                        }
                        f_y += 38.0;
                    }

                    let gen_y = modal_y + 210.0;
                    if x >= modal_x + 160.0
                        && x <= modal_x + modal_w - 24.0
                        && y >= gen_y
                        && y <= gen_y + 34.0
                    {
                        new_hover = HoverTarget::KeyToolGenerateBtn;
                        cursor = CursorIcon::Pointer;
                    }

                    if self.key_tool_modal.generated_pub_key.is_some() {
                        let btn_row_y = modal_y + modal_h - 48.0;
                        if y >= btn_row_y && y <= btn_row_y + 34.0 {
                            let mut bx = modal_x + 24.0;
                            if x >= bx && x <= bx + 160.0 {
                                new_hover = HoverTarget::KeyToolCopyPubBtn;
                                cursor = CursorIcon::Pointer;
                            }
                            bx += 168.0;
                            if self.key_tool_modal.target_host_idx.is_some() {
                                if x >= bx && x <= bx + 150.0 {
                                    new_hover = HoverTarget::KeyToolSendTermBtn;
                                    cursor = CursorIcon::Pointer;
                                }
                                bx += 158.0;
                                if x >= bx && x <= bx + 130.0 {
                                    new_hover = HoverTarget::KeyToolBindHostBtn;
                                    cursor = CursorIcon::Pointer;
                                }
                            }
                        }
                    }
                }
            }
            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // 1. Modal Add Host
        if self.add_host_modal.is_open {
            let modal_w = 540.0f64.min(w - 40.0);
            let modal_h = 480.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;

            if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h {
                new_hover = HoverTarget::ModalDialogBody;
                cursor = CursorIcon::Default;

                // Close button
                if x >= modal_x + modal_w - 40.0 && y <= modal_y + 40.0 {
                    new_hover = HoverTarget::ModalClose;
                    cursor = CursorIcon::Pointer;
                } else {
                    // Fields
                    let mut f_y = modal_y + 60.0;
                    for idx in 0..5 {
                        if x >= modal_x + 24.0
                            && x <= modal_x + modal_w - 24.0
                            && y >= f_y
                            && y <= f_y + 32.0
                        {
                            new_hover = HoverTarget::ModalField(idx);
                            cursor = CursorIcon::Text;
                        }
                        f_y += 38.0;
                    }

                    // Auth buttons
                    let mut btn_x = modal_x + 160.0;
                    let modes = [
                        AuthModeChoice::Agent,
                        AuthModeChoice::Key,
                        AuthModeChoice::Password,
                    ];
                    for mode in modes {
                        if x >= btn_x && x <= btn_x + 90.0 && y >= f_y && y <= f_y + 28.0 {
                            new_hover = HoverTarget::ModalAuthMode(mode);
                            cursor = CursorIcon::Pointer;
                        }
                        btn_x += 98.0;
                    }
                    f_y += 38.0;

                    // Key/Password Field
                    if self.add_host_modal.auth_mode != AuthModeChoice::Agent {
                        if self.add_host_modal.auth_mode == AuthModeChoice::Key {
                            let input_w = modal_w - 160.0 - 180.0;
                            if x >= modal_x + 24.0
                                && x <= modal_x + 160.0 + input_w
                                && y >= f_y
                                && y <= f_y + 32.0
                            {
                                new_hover = HoverTarget::ModalField(5);
                                cursor = CursorIcon::Text;
                            } else if x >= modal_x + modal_w - 170.0
                                && x <= modal_x + modal_w - 96.0
                                && y >= f_y
                                && y <= f_y + 32.0
                            {
                                new_hover = HoverTarget::ModalBrowseKey;
                                cursor = CursorIcon::Pointer;
                            } else if x >= modal_x + modal_w - 90.0
                                && x <= modal_x + modal_w - 24.0
                                && y >= f_y
                                && y <= f_y + 32.0
                            {
                                new_hover = HoverTarget::ModalGenerateKey;
                                cursor = CursorIcon::Pointer;
                            }
                        } else {
                            let eye_btn_w = 34.0;
                            let inp_w = modal_w - 184.0 - eye_btn_w - 8.0;
                            if x >= modal_x + 160.0
                                && x <= modal_x + 160.0 + inp_w
                                && y >= f_y
                                && y <= f_y + 32.0
                            {
                                new_hover = HoverTarget::ModalField(5);
                                cursor = CursorIcon::Text;
                            } else if x >= modal_x + 160.0 + inp_w + 8.0
                                && x <= modal_x + 160.0 + inp_w + 8.0 + eye_btn_w
                                && y >= f_y
                                && y <= f_y + 32.0
                            {
                                new_hover = HoverTarget::ModalTogglePassword;
                                cursor = CursorIcon::Pointer;
                            } else if x >= modal_x + 24.0
                                && x <= modal_x + modal_w - 24.0
                                && y >= f_y
                                && y <= f_y + 32.0
                            {
                                new_hover = HoverTarget::ModalField(5);
                                cursor = CursorIcon::Text;
                            }
                        }
                    }

                    // Save / Connect buttons
                    let save_btn_x = modal_x + modal_w - 280.0;
                    let btn_y = modal_y + modal_h - 48.0;
                    if x >= save_btn_x && x <= save_btn_x + 120.0 && y >= btn_y && y <= btn_y + 34.0
                    {
                        new_hover = HoverTarget::ModalSave;
                        cursor = CursorIcon::Pointer;
                    }
                    let conn_btn_x = modal_x + modal_w - 146.0;
                    if x >= conn_btn_x && x <= conn_btn_x + 126.0 && y >= btn_y && y <= btn_y + 34.0
                    {
                        new_hover = HoverTarget::ModalConnect;
                        cursor = CursorIcon::Pointer;
                    }
                }
            }

            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // 2. Command Palette Modal
        if self.palette_open {
            let pal_w = 600.0f64.min(w - 40.0);
            let pal_h = 360.0f64.min(h - 80.0);
            let pal_x = (w - pal_w) / 2.0;
            let pal_y = 70.0;

            if x >= pal_x + 16.0 && x <= pal_x + pal_w - 16.0 {
                if y >= pal_y + 16.0 && y <= pal_y + 52.0 {
                    cursor = CursorIcon::Text;
                } else if y >= pal_y + 64.0 && y <= pal_y + pal_h - 16.0 {
                    let mut row_y = pal_y + 64.0;
                    for idx in 0..self.command_search_results.len().min(7) {
                        if y >= row_y && y <= row_y + 36.0 {
                            new_hover = HoverTarget::PaletteItem(idx);
                            cursor = CursorIcon::Pointer;
                            break;
                        }
                        row_y += 42.0;
                    }
                }
            }
            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // 3. Header
        let header_h = self.header_height;
        if y < header_h {
            if x >= w - 46.0 && x <= w - 14.0 {
                new_hover = HoverTarget::HeaderSettingsBtn;
                cursor = CursorIcon::Pointer;
            } else if x >= w - 94.0 && x <= w - 52.0 {
                new_hover = HoverTarget::HeaderLangBtn;
                cursor = CursorIcon::Pointer;
            } else {
                let compact_search = w < 880.0 || (self.tabs.len() >= 3 && w < 1050.0);
                let search_btn_w = if compact_search { 32.0 } else { 150.0 };
                let search_btn_x = w - 100.0 - search_btn_w;

                let tab_start_x = 150.0;
                let compact_add = w < 720.0 || (self.tabs.len() >= 3 && w < 920.0);
                let add_btn_w = if compact_add { 32.0 } else { 110.0 };
                let max_tabs_w = (search_btn_x - 10.0 - tab_start_x - add_btn_w - 6.0).max(60.0);
                let num_tabs = self.tabs.len().max(1);
                let tab_w = ((max_tabs_w - (num_tabs - 1) as f64 * 6.0) / num_tabs as f64)
                    .clamp(60.0, 175.0);

                let mut tab_x = tab_start_x;
                for (i, _) in self.tabs.iter().enumerate() {
                    if tab_x + tab_w > search_btn_x - 4.0 {
                        break;
                    }
                    if x >= tab_x && x <= tab_x + tab_w {
                        if x >= tab_x + tab_w - 24.0 {
                            new_hover = HoverTarget::HeaderTabClose(i);
                        } else {
                            new_hover = HoverTarget::HeaderTab(i);
                        }
                        cursor = CursorIcon::Pointer;
                        break;
                    }
                    tab_x += tab_w + 6.0;
                }

                if new_hover == HoverTarget::None {
                    if tab_x + add_btn_w <= search_btn_x - 4.0
                        && x >= tab_x
                        && x <= tab_x + add_btn_w
                    {
                        new_hover = HoverTarget::HeaderAddBtn;
                        cursor = CursorIcon::Pointer;
                    } else if x >= search_btn_x && x <= search_btn_x + search_btn_w {
                        new_hover = HoverTarget::HeaderSearchBtn;
                        cursor = CursorIcon::Pointer;
                    }
                }
            }

            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // 3.5 Sidebar Splitter (Resize handle)
        let sidebar_w = self.sidebar_width;
        if y >= header_h && y <= h - self.footer_height && (x - sidebar_w).abs() <= 5.0 {
            new_hover = HoverTarget::SidebarSplitter;
            cursor = CursorIcon::ColResize;
            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // 4. Sidebar
        if x < sidebar_w {
            let nav_h = 38.0;
            if y >= header_h && y <= header_h + nav_h {
                let btn_w = sidebar_w / 4.0;
                let idx = (x / btn_w) as usize;
                match idx {
                    0 => new_hover = HoverTarget::SidebarNav(SidebarTab::Hosts),
                    1 => new_hover = HoverTarget::SidebarNav(SidebarTab::Sessions),
                    2 => new_hover = HoverTarget::SidebarNav(SidebarTab::Commands),
                    3 => new_hover = HoverTarget::SidebarNav(SidebarTab::Snippets),
                    _ => {}
                }
                cursor = CursorIcon::Pointer;
            } else {
                let content_y = header_h + nav_h + 8.0;
                if y >= content_y {
                    let rel_y = y - content_y;
                    match self.sidebar_tab {
                        SidebarTab::Hosts => {
                            if rel_y <= 36.0 {
                                if x >= sidebar_w - 84.0 && x <= sidebar_w - 10.0 {
                                    new_hover = HoverTarget::SidebarAddHostBtn;
                                    cursor = CursorIcon::Pointer;
                                } else {
                                    new_hover = HoverTarget::SidebarSearchHost;
                                    cursor = CursorIcon::Text;
                                }
                            } else {
                                let item_rel_y = rel_y - 42.0;
                                if item_rel_y >= 0.0 {
                                    let item_idx = (item_rel_y / 64.0) as usize;
                                    let card_y = item_rel_y % 64.0;
                                    let filtered = self.filtered_hosts();
                                    if item_idx < filtered.len() && card_y <= 58.0 {
                                        let real_idx = filtered[item_idx].0;
                                        if card_y >= 14.0 && card_y <= 42.0 {
                                            if x >= sidebar_w - 34.0 && x <= sidebar_w - 10.0 {
                                                new_hover = HoverTarget::HostDelete(real_idx);
                                            } else if x >= sidebar_w - 62.0 && x < sidebar_w - 34.0
                                            {
                                                new_hover = HoverTarget::HostEdit(real_idx);
                                            } else if x >= sidebar_w - 90.0 && x < sidebar_w - 62.0
                                            {
                                                new_hover = HoverTarget::HostDeployKey(real_idx);
                                            } else if x >= sidebar_w - 118.0 && x < sidebar_w - 90.0
                                            {
                                                new_hover = HoverTarget::HostConnect(real_idx);
                                            } else {
                                                new_hover = HoverTarget::HostCard(real_idx);
                                            }
                                        } else {
                                            new_hover = HoverTarget::HostCard(real_idx);
                                        }
                                        cursor = CursorIcon::Pointer;
                                    }
                                }
                            }
                        }
                        SidebarTab::Sessions => {
                            let item_idx = (rel_y / 54.0) as usize;
                            if item_idx < self.recent_sessions.len() {
                                new_hover = HoverTarget::SessionCard(item_idx);
                                cursor = CursorIcon::Pointer;
                            }
                        }
                        SidebarTab::Commands => {
                            let search_box_h = 40.0;
                            if rel_y <= search_box_h {
                                new_hover = HoverTarget::SidebarSearchCommand;
                                cursor = CursorIcon::Text;
                            } else {
                                let item_idx = ((rel_y - search_box_h) / 44.0) as usize;
                                if item_idx < self.command_search_results.len() {
                                    new_hover = HoverTarget::CommandItem(item_idx);
                                    cursor = CursorIcon::Pointer;
                                }
                            }
                        }
                        SidebarTab::Snippets => {
                            if rel_y <= 36.0 {
                                if x >= sidebar_w - 84.0 && x <= sidebar_w - 10.0 {
                                    new_hover = HoverTarget::SidebarAddSnippetBtn;
                                    cursor = CursorIcon::Pointer;
                                } else {
                                    new_hover = HoverTarget::SidebarSearchSnippet;
                                    cursor = CursorIcon::Text;
                                }
                            } else {
                                let item_rel_y = rel_y - 42.0;
                                if item_rel_y >= 0.0 {
                                    let item_idx = (item_rel_y / 62.0) as usize;
                                    let card_y = item_rel_y % 62.0;
                                    if item_idx < self.snippets.len() && card_y <= 56.0 {
                                        if card_y >= 14.0 && card_y <= 42.0 {
                                            if x >= sidebar_w - 34.0 && x <= sidebar_w - 10.0 {
                                                new_hover = HoverTarget::SnippetDelete(item_idx);
                                            } else if x >= sidebar_w - 62.0 && x < sidebar_w - 34.0
                                            {
                                                new_hover = HoverTarget::SnippetEdit(item_idx);
                                            } else if x >= sidebar_w - 90.0 && x < sidebar_w - 62.0
                                            {
                                                new_hover = HoverTarget::SnippetCopy(item_idx);
                                            } else if x >= sidebar_w - 118.0 && x < sidebar_w - 90.0
                                            {
                                                new_hover = HoverTarget::SnippetRun(item_idx);
                                            } else {
                                                new_hover = HoverTarget::SnippetItem(item_idx);
                                            }
                                        } else {
                                            new_hover = HoverTarget::SnippetItem(item_idx);
                                        }
                                        cursor = CursorIcon::Pointer;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let changed = old_hover != new_hover;
            self.hover = new_hover;
            return (changed, cursor);
        }

        // 5. Main Area (Welcome Screen actions or Terminal)
        if self.tabs.is_empty() {
            let center_x = sidebar_w + (w - sidebar_w) / 2.0 - 220.0;
            let center_y = header_h + (h - header_h - self.footer_height) / 2.0 - 140.0;
            let title_y = center_y + 84.0;

            if x >= center_x + 90.0
                && x <= center_x + 250.0
                && y >= title_y + 36.0
                && y <= title_y + 72.0
            {
                new_hover = HoverTarget::WelcomeAdd;
                cursor = CursorIcon::Pointer;
            } else if x >= center_x + 260.0
                && x <= center_x + 410.0
                && y >= title_y + 36.0
                && y <= title_y + 72.0
            {
                new_hover = HoverTarget::WelcomeConnect;
                cursor = CursorIcon::Pointer;
            }
        } else {
            // 5.1 Floating Terminal Search Widget (if open)
            if self.search_state.is_open {
                let sw_w = 360.0;
                let sw_h = 36.0;
                let sw_x = w - sw_w - 24.0;
                let sw_y = header_h + 8.0;

                if x >= sw_x && x <= sw_x + sw_w && y >= sw_y && y <= sw_y + sw_h {
                    if x >= sw_x + sw_w - 30.0 && x <= sw_x + sw_w - 6.0 {
                        new_hover = HoverTarget::TerminalSearchCloseBtn;
                        cursor = CursorIcon::Pointer;
                    } else if x >= sw_x + sw_w - 56.0 && x < sw_x + sw_w - 30.0 {
                        new_hover = HoverTarget::TerminalSearchNextBtn;
                        cursor = CursorIcon::Pointer;
                    } else if x >= sw_x + sw_w - 82.0 && x < sw_x + sw_w - 56.0 {
                        new_hover = HoverTarget::TerminalSearchPrevBtn;
                        cursor = CursorIcon::Pointer;
                    } else if x >= sw_x + sw_w - 112.0 && x < sw_x + sw_w - 82.0 {
                        new_hover = HoverTarget::TerminalSearchCaseToggle;
                        cursor = CursorIcon::Pointer;
                    } else {
                        new_hover = HoverTarget::TerminalSearchInput;
                        cursor = CursorIcon::Text;
                    }
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }

            // 5.2 Split View Close button on top-right of split panel
            let is_split = self.split_tab.is_some()
                && self.split_tab.unwrap() < self.tabs.len()
                && self.split_tab.unwrap() != self.active_tab;

            let total_term_w = (w - sidebar_w).max(0.0);

            if is_split {
                let right_w = (total_term_w - (total_term_w / 2.0).floor() - 2.0).max(0.0);
                let right_x = sidebar_w + (total_term_w / 2.0).floor() + 2.0;
                let sc_x = right_x + right_w - 30.0;
                if x >= sc_x && x <= sc_x + 24.0 && y >= header_h + 4.0 && y <= header_h + 26.0 {
                    new_hover = HoverTarget::TerminalSplitClose;
                    cursor = CursorIcon::Pointer;
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }

            // 5.3 Terminal Panels (Left & Right if split, otherwise Single)
            let panels: Vec<(usize, bool, f64, f64)> = if is_split {
                let left_w = ((total_term_w / 2.0).floor() - 2.0).max(0.0);
                let right_x = sidebar_w + (total_term_w / 2.0).floor() + 2.0;
                let right_w = (total_term_w - (total_term_w / 2.0).floor() - 2.0).max(0.0);
                vec![
                    (self.active_tab, true, sidebar_w, left_w),
                    (self.split_tab.unwrap(), false, right_x, right_w),
                ]
            } else {
                vec![(self.active_tab, true, sidebar_w, total_term_w)]
            };

            for (tab_idx, is_active, px_start, pw) in panels {
                if x < px_start || x > px_start + pw || y < header_h || y > h - self.footer_height {
                    continue;
                }

                let tab = match self.tabs.get(tab_idx) {
                    Some(t) => t,
                    None => continue,
                };

                // A. Disconnected Tab -> Reconnect / Close Banner at bottom
                if tab.handle.is_none() {
                    let banner_y = h - self.footer_height - 44.0;
                    let banner_h = 38.0;
                    if y >= banner_y && y <= banner_y + banner_h {
                        let close_btn_w = if pw >= 480.0 { 92.0 } else { 32.0 };
                        let close_x = px_start + pw - 12.0 - close_btn_w;

                        let recon_btn_w = if pw >= 540.0 {
                            180.0
                        } else if pw >= 360.0 {
                            105.0
                        } else {
                            32.0
                        };
                        let recon_x = close_x - 6.0 - recon_btn_w;

                        if x >= close_x && x <= close_x + close_btn_w {
                            new_hover = HoverTarget::TerminalCloseTab(tab_idx);
                            cursor = CursorIcon::Pointer;
                            break;
                        } else if x >= recon_x && x <= recon_x + recon_btn_w {
                            new_hover = HoverTarget::TerminalReconnect(tab_idx);
                            cursor = CursorIcon::Pointer;
                            break;
                        } else {
                            new_hover = HoverTarget::TerminalStatusBanner(tab_idx);
                            cursor = CursorIcon::Default;
                            break;
                        }
                    }
                } else {
                    // B. Connected Tab -> Diagnostics Toolbar (Htop, Df, Free, Search, Split)
                    let bar_y = header_h;
                    let bar_h = 30.0;
                    if y >= bar_y && y <= bar_y + bar_h {
                        if is_active && pw >= 320.0 {
                            let search_rect_x = px_start + pw - 170.0;
                            let search_rect_w = 80.0;
                            let split_rect_x = px_start + pw - 84.0;
                            let split_rect_w = 76.0;

                            if x >= search_rect_x && x <= search_rect_x + search_rect_w {
                                new_hover = HoverTarget::TerminalSearchOpenBtn;
                                cursor = CursorIcon::Pointer;
                                break;
                            } else if x >= split_rect_x && x <= split_rect_x + split_rect_w {
                                new_hover = HoverTarget::TerminalSplitBtn;
                                cursor = CursorIcon::Pointer;
                                break;
                            }
                        }

                        let right_reserved = if is_active && pw >= 320.0 {
                            180.0
                        } else {
                            12.0
                        };
                        let mut pill_x = px_start + 8.0;
                        let pills = [
                            (QuickActionCmd::Htop, 78.0),
                            (QuickActionCmd::Df, 74.0),
                            (QuickActionCmd::Free, 80.0),
                            (QuickActionCmd::Uptime, 82.0),
                            (QuickActionCmd::Docker, 82.0),
                            (QuickActionCmd::Ports, 92.0),
                        ];
                        for (cmd, pill_w) in pills {
                            if pill_x + pill_w + 6.0 > px_start + pw - right_reserved {
                                break;
                            }
                            if x >= pill_x && x <= pill_x + pill_w {
                                if is_active {
                                    new_hover = HoverTarget::QuickAction(cmd);
                                    cursor = CursorIcon::Pointer;
                                }
                                break;
                            }
                            pill_x += pill_w + 6.0;
                        }
                        if new_hover != HoverTarget::None {
                            break;
                        }
                    }
                }

                // C. Scrollbar (only on active panel)
                if new_hover == HoverTarget::None && is_active {
                    let sb_w = 8.0;
                    let sb_margin_right = 4.0;
                    let sb_x = px_start + pw - sb_w - sb_margin_right;
                    let bar_h = if tab.handle.is_some() { 30.0 } else { 0.0 };
                    let sb_y = header_h + bar_h + 6.0;
                    let banner_h = if tab.handle.is_none() { 46.0 } else { 0.0 };
                    let sb_h =
                        (h - header_h - self.footer_height - bar_h - banner_h - 12.0).max(10.0);

                    let scrollback_len = tab.terminal.scrollback_len();
                    let scroll_offset = tab.terminal.scroll_offset();
                    let rows = tab.terminal.rows();

                    let (thumb_y, thumb_h) = if scrollback_len > 0 && sb_h > 0.0 {
                        let total_lines = scrollback_len + rows;
                        let visible_ratio = if total_lines > 0 {
                            (rows as f64) / (total_lines as f64)
                        } else {
                            1.0
                        };
                        let min_th = 14.0f64.min(sb_h);
                        let th = (sb_h * visible_ratio).max(min_th).min(sb_h);
                        let max_scroll_offset = scrollback_len as f64;
                        let scroll_ratio = if max_scroll_offset > 0.0 {
                            (scroll_offset as f64) / max_scroll_offset
                        } else {
                            0.0
                        };
                        let ty = sb_y + (sb_h - th) * (1.0 - scroll_ratio);
                        (ty, th)
                    } else {
                        (sb_y, sb_h)
                    };

                    if x >= sb_x - 3.0 && x <= sb_x + sb_w + 3.0 && y >= sb_y && y <= sb_y + sb_h {
                        if y >= thumb_y && y <= thumb_y + thumb_h {
                            new_hover = HoverTarget::TerminalScrollbarThumb;
                            cursor = CursorIcon::Default;
                        } else {
                            new_hover = HoverTarget::TerminalScrollbarTrack;
                            cursor = CursorIcon::Pointer;
                        }
                        break;
                    }
                }

                // D. Terminal Area
                if new_hover == HoverTarget::None {
                    if !is_active {
                        new_hover = HoverTarget::SplitTerminalArea;
                        cursor = CursorIcon::Pointer;
                    } else {
                        new_hover = HoverTarget::TerminalArea;
                        cursor = CursorIcon::Text;
                    }
                    break;
                }
            }
        }

        let changed = old_hover != new_hover;
        self.hover = new_hover;

        if changed {
            if new_hover != HoverTarget::None {
                if let Some(text) = self.get_hover_tooltip_text(new_hover) {
                    self.tooltip = Some(TooltipState {
                        text: text.to_string(),
                        x: (x + 12.0).min(w - 240.0),
                        y: (y + 18.0).min(h - 40.0),
                        hover_start: std::time::Instant::now(),
                    });
                } else {
                    self.tooltip = None;
                }
            } else {
                self.tooltip = None;
            }
        } else if let Some(tt) = &mut self.tooltip {
            tt.x = (x + 12.0).min(w - 240.0);
            tt.y = (y + 18.0).min(h - 40.0);
        }

        (changed, cursor)
    }

    pub fn pos_from_coords(&self, x: f64, y: f64) -> Option<TerminalPos> {
        let is_connected = self
            .tabs
            .get(self.active_tab)
            .map_or(false, |t| t.handle.is_some());
        let bar_h = if is_connected { 30.0 } else { 0.0 };

        let pad_x = self.sidebar_width + 10.0;
        let pad_y = self.header_height + bar_h + 6.0;
        let char_w = self.terminal_char_width;
        let line_h = TERMINAL_LINE_HEIGHT;

        if !self.is_mouse_selecting && (x < pad_x || y < pad_y) {
            return None;
        }

        let col = if x < pad_x {
            0
        } else {
            ((x - pad_x) / char_w).floor().max(0.0) as usize
        };

        let row = if y < pad_y {
            0
        } else {
            ((y - pad_y) / line_h).floor().max(0.0) as usize
        };

        if self.is_mouse_selecting {
            Some(TerminalPos {
                col: col.min(self.terminal_cols.saturating_sub(1)),
                row: row.min(self.terminal_rows.saturating_sub(1)),
            })
        } else if row < self.terminal_rows {
            Some(TerminalPos {
                col: col.min(self.terminal_cols.saturating_sub(1)),
                row,
            })
        } else {
            None
        }
    }

    pub fn get_selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?;
        if sel.is_empty() {
            return None;
        }
        let tab = self.tabs.get(self.active_tab)?;
        let (p1, p2) = sel.normalized();
        let text = tab
            .terminal
            .get_text_range((p1.col, p1.row), (p2.col, p2.row));
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn copy_selection_to_clipboard(&self) -> bool {
        if let Some(text) = self.get_selected_text() {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(text);
                return true;
            }
        }
        false
    }

    pub fn open_context_menu_at(&mut self, x: f64, y: f64, win_w: f64, win_h: f64) {
        let selected_text = self.get_selected_text().unwrap_or_default();
        let menu_w = 180.0;
        let menu_h = 108.0;
        let clamped_x = x.min(win_w - menu_w - 8.0).max(8.0);
        let clamped_y = y.min(win_h - menu_h - 8.0).max(8.0);
        self.context_menu = Some(ContextMenu {
            is_open: true,
            x: clamped_x,
            y: clamped_y,
            selected_text,
        });
    }

    pub fn is_modal_open(&self) -> bool {
        self.is_app_locked
            || self.host_key_modal.is_some()
            || self.add_host_modal.is_open
            || self.key_tool_modal.is_open
            || self.add_snippet_modal.is_open
            || self
                .session_dashboard_modal
                .as_ref()
                .map_or(false, |m| m.is_open)
            || self.confirm_delete_modal.is_some()
            || self.confirm_delete_snippet_modal.is_some()
            || self.confirm_run_modal.is_some()
            || self.settings_modal.as_ref().map_or(false, |m| m.is_open)
            || self.palette_open
            || self.search_state.is_open
    }

    pub fn start_input_selection(&mut self, fid: InputFieldId, anchor: usize, origin_x: f64) {
        self.is_input_selecting = true;
        self.input_select_anchor = anchor;
        self.input_select_field = Some(fid);
        self.input_select_origin_x = origin_x;
    }

    pub fn update_input_selection(&mut self, current_x: f64) -> bool {
        if !self.is_input_selecting {
            return false;
        }
        let fid = match self.input_select_field {
            Some(f) => f,
            None => return false,
        };
        let anchor = self.input_select_anchor;
        let origin_x = self.input_select_origin_x;
        if let Some(inp) = self.get_text_input_mut(fid) {
            let total_chars = inp.text.chars().count();
            let mut changed = false;

            let current_cursor = if current_x <= origin_x {
                // Dragging to/past left edge -> select to 0 and auto-scroll left
                if inp.scroll_offset > 0 {
                    inp.scroll_offset = inp.scroll_offset.saturating_sub(2);
                    changed = true;
                }
                0
            } else {
                let click_offset_px = current_x - origin_x;
                // If dragging past 420px (towards/past the right edge of modal inputs)
                if click_offset_px > 430.0 {
                    // Auto-scroll towards end and select to total_chars
                    if inp.scroll_offset + 2 < total_chars {
                        inp.scroll_offset = (inp.scroll_offset + 2).min(total_chars);
                        changed = true;
                    }
                    total_chars
                } else {
                    let approx_char_w = 7.0;
                    let clicked_char = (click_offset_px / approx_char_w).round() as usize;
                    (inp.scroll_offset + clicked_char).min(total_chars)
                }
            };

            if inp.cursor != current_cursor {
                inp.cursor = current_cursor;
                changed = true;
            }
            let new_sel = if current_cursor != anchor {
                Some((anchor, current_cursor))
            } else {
                None
            };
            if inp.selection != new_sel {
                inp.selection = new_sel;
                changed = true;
            }
            if changed {
                self.caret_visible = true;
            }
            changed
        } else {
            false
        }
    }

    pub fn end_input_selection(&mut self) -> bool {
        if self.is_input_selecting {
            self.is_input_selecting = false;
            true
        } else {
            false
        }
    }

    pub fn handle_input_click(
        &mut self,
        fid: InputFieldId,
        logical_x: f64,
        input_start_x: f64,
        click_count: usize,
    ) {
        let anchor = if let Some(inp) = self.get_text_input_mut(fid) {
            if click_count >= 3 {
                inp.select_all();
                None
            } else if click_count == 2 {
                inp.select_word_at_cursor();
                None
            } else {
                let click_offset_px = (logical_x - input_start_x).max(0.0);
                inp.set_cursor_from_click(click_offset_px, 7.0);
                Some(inp.cursor)
            }
        } else {
            None
        };
        if let Some(cur) = anchor {
            self.start_input_selection(fid, cur, input_start_x);
        }
    }

    pub fn handle_snippet_field_click(
        &mut self,
        idx: usize,
        logical_x: f64,
        logical_y: f64,
        label_y: f64,
        input_start_x: f64,
        click_count: usize,
    ) {
        self.add_snippet_modal.active_field = idx;
        let fid = InputFieldId::AddSnippet(idx);
        let anchor = if let Some(inp) = self.get_text_input_mut(fid) {
            if click_count >= 3 {
                inp.select_all();
                None
            } else if click_count == 2 {
                inp.select_word_at_cursor();
                None
            } else if logical_y < label_y + 18.0 {
                inp.select_all();
                None
            } else {
                let click_offset_px = (logical_x - input_start_x).max(0.0);
                inp.set_cursor_from_click(click_offset_px, 7.0);
                Some(inp.cursor)
            }
        } else {
            None
        };
        if let Some(cur) = anchor {
            self.start_input_selection(fid, cur, input_start_x);
        }
    }

    pub fn active_field_id(&self) -> Option<InputFieldId> {
        if self.is_app_locked {
            return Some(InputFieldId::LockPassword);
        }
        if self.search_state.is_open {
            return Some(InputFieldId::TerminalSearch);
        }
        if self.palette_open {
            return Some(InputFieldId::CommandSearch);
        }
        if self.add_host_modal.is_open {
            return Some(InputFieldId::AddHost(self.add_host_modal.active_field));
        }
        if self.key_tool_modal.is_open {
            return Some(InputFieldId::KeyTool(self.key_tool_modal.active_field));
        }
        if self.add_snippet_modal.is_open {
            return Some(InputFieldId::AddSnippet(
                self.add_snippet_modal.active_field,
            ));
        }
        if let Some(m) = &self.settings_modal {
            if m.is_open {
                return match m.active_tab {
                    SettingsTab::General => {
                        Some(InputFieldId::SettingsGeneral(m.active_general_field))
                    }
                    SettingsTab::Security => {
                        Some(InputFieldId::SettingsSecurity(m.active_security_field))
                    }
                    SettingsTab::Backup => self.focused_input,
                    _ => None,
                };
            }
        }
        self.focused_input
    }

    pub fn paste_clipboard_text(&mut self) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Ok(text) = clipboard.get_text() {
                if !text.is_empty() {
                    if let Some(fid) = self.active_field_id() {
                        if let Some(inp) = self.get_text_input_mut(fid) {
                            inp.delete_selection();
                            inp.insert_str(&text);
                            inp.ensure_cursor_visible(50);
                        }
                        if fid == InputFieldId::CommandSearch {
                            self.refresh_command_search();
                        } else if fid == InputFieldId::SnippetSearch {
                            self.refresh_snippets();
                        } else if fid == InputFieldId::TerminalSearch {
                            self.update_terminal_search();
                        }
                    } else {
                        self.paste_to_active_terminal(&text);
                    }
                }
            }
        }
    }

    pub fn copy_active_selection_or_terminal(&mut self) -> bool {
        if let Some(fid) = self.active_field_id() {
            if let Some(inp) = self.get_text_input(fid) {
                if let Some(sel) = inp.selected_text() {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(sel.to_string());
                        return true;
                    }
                }
            }
        }
        self.copy_selection_to_clipboard()
    }

    pub fn cut_active_selection(&mut self) -> bool {
        if let Some(fid) = self.active_field_id() {
            if let Some(inp) = self.get_text_input_mut(fid) {
                if let Some(sel) = inp.selected_text().map(|s| s.to_string()) {
                    inp.delete_selection();
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(sel);
                    }
                    if fid == InputFieldId::CommandSearch {
                        self.refresh_command_search();
                    } else if fid == InputFieldId::SnippetSearch {
                        self.refresh_snippets();
                    } else if fid == InputFieldId::TerminalSearch {
                        self.update_terminal_search();
                    }
                    return true;
                }
            }
        }
        false
    }

    pub fn select_all_active_input(&mut self) -> bool {
        if let Some(fid) = self.active_field_id() {
            if let Some(inp) = self.get_text_input_mut(fid) {
                inp.select_all();
                return true;
            }
        }
        false
    }

    pub fn handle_text_navigation_key(
        &mut self,
        logical_key: &winit::keyboard::Key,
        is_ctrl: bool,
        is_shift: bool,
    ) -> bool {
        let fid = match self.active_field_id() {
            Some(f) => f,
            None => return false,
        };

        let handled = match logical_key {
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowLeft) => {
                if let Some(inp) = self.get_text_input_mut(fid) {
                    if is_ctrl && is_shift {
                        inp.move_word_left_select();
                    } else if is_ctrl {
                        inp.move_word_left();
                    } else if is_shift {
                        inp.move_left_select();
                    } else {
                        inp.move_left();
                    }
                    inp.ensure_cursor_visible(45);
                    true
                } else {
                    false
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowRight) => {
                if let Some(inp) = self.get_text_input_mut(fid) {
                    if is_ctrl && is_shift {
                        inp.move_word_right_select();
                    } else if is_ctrl {
                        inp.move_word_right();
                    } else if is_shift {
                        inp.move_right_select();
                    } else {
                        inp.move_right();
                    }
                    inp.ensure_cursor_visible(45);
                    true
                } else {
                    false
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => {
                if let Some(inp) = self.get_text_input_mut(fid) {
                    if is_shift {
                        inp.move_home_select();
                    } else {
                        inp.move_home();
                    }
                    inp.ensure_cursor_visible(45);
                    true
                } else {
                    false
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::End) => {
                if let Some(inp) = self.get_text_input_mut(fid) {
                    if is_shift {
                        inp.move_end_select();
                    } else {
                        inp.move_end();
                    }
                    inp.ensure_cursor_visible(45);
                    true
                } else {
                    false
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                if let Some(inp) = self.get_text_input_mut(fid) {
                    if is_ctrl {
                        inp.delete_word_back();
                    } else {
                        inp.delete_back();
                    }
                    inp.ensure_cursor_visible(45);
                    true
                } else {
                    false
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Delete) => {
                if let Some(inp) = self.get_text_input_mut(fid) {
                    if is_ctrl {
                        inp.delete_word_forward();
                    } else {
                        inp.delete_forward();
                    }
                    inp.ensure_cursor_visible(45);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };

        if handled {
            self.caret_visible = true;
            self.caret_blink_time = std::time::Instant::now();
            if fid == InputFieldId::SnippetSearch {
                self.refresh_snippets();
            } else if fid == InputFieldId::CommandSearch {
                self.refresh_command_search();
            } else if fid == InputFieldId::TerminalSearch {
                self.update_terminal_search();
            }
        }

        handled
    }

    pub fn update_caret(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now.duration_since(self.caret_blink_time).as_millis() >= 500 {
            self.caret_blink_time = now;
            self.caret_visible = !self.caret_visible;
            true
        } else {
            false
        }
    }

    pub fn poll_events(&mut self) -> bool {
        let mut updated = false;
        let mut history_changed = false;
        let mut history_error = None;
        let started = std::time::Instant::now();
        for _ in 0..64 {
            if started.elapsed() >= std::time::Duration::from_millis(4) {
                break;
            }
            let Ok(event) = self.event_rx.try_recv() else {
                break;
            };
            updated = true;
            match event {
                AppEvent::SshOutput { session_id, data } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == session_id) {
                        tab.terminal.write_bytes(&data);
                        let commands = tab.terminal.take_executed_commands();
                        if self.hosts_doc.settings.record_command_history {
                            for command in commands {
                                match self.db.record_command(
                                    Some(&tab.id),
                                    &tab.host_address,
                                    &command,
                                ) {
                                    Ok(_) => history_changed = true,
                                    Err(e) => {
                                        history_error = Some(format!("Cannot record command: {e}"))
                                    }
                                }
                            }
                        }
                    }
                }
                AppEvent::SshConnected { session_id, handle } => {
                    self.pending_connections.remove(&session_id);
                    let c = self.terminal_cols as u32;
                    let r = self.terminal_rows as u32;
                    let r_tx = handle.resize_tx.clone();
                    self.tokio_handle.spawn(async move {
                        let _ = r_tx.send((c, r)).await;
                    });
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == session_id) {
                        if *handle.is_connected.read() {
                            tab.status = "🟢 Connected".into();
                            tab.handle = Some(handle);
                        }
                    }
                    self.refresh_recent_sessions();
                }
                AppEvent::SshDisconnected { session_id, error } => {
                    self.pending_connections.remove(&session_id);
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == session_id) {
                        tab.handle = None;
                        match error {
                            Some(ref err) => {
                                tab.status = format!("🔴 Ошибка: {err}");
                                let msg = format!(
                                    "\r\n\x1b[1;31m✖ Ошибка подключения:\x1b[0m \x1b[38;2;252;165;165m{err}\x1b[0m\r\n\x1b[38;2;148;163;184mПопытка остановлена. Проверьте правильность адреса, порта, логина или пароля/ключа.\x1b[0m\r\n"
                                );
                                tab.terminal.write_bytes(msg.as_bytes());
                            }
                            None => {
                                tab.status = "⚪ Отключено".into();
                                tab.terminal.write_bytes(
                                    "\r\n\x1b[38;2;148;163;184m[Сессия завершена]\x1b[0m\r\n"
                                        .as_bytes(),
                                );
                            }
                        }
                        let _ = self.db.record_session_end(
                            &session_id,
                            if error.is_some() { "error" } else { "closed" },
                            error.as_deref(),
                        );
                    }
                    self.refresh_recent_sessions();
                }
                AppEvent::HostKeyPrompt {
                    session_id,
                    host,
                    port,
                    key_type,
                    fingerprint,
                    is_changed,
                    old_line,
                    response_tx,
                } => {
                    self.pending_host_keys.push_back(HostKeyPromptModal {
                        session_id,
                        host,
                        port,
                        key_type,
                        fingerprint,
                        is_changed,
                        old_line,
                        response_tx: Some(response_tx),
                        copied_feedback: false,
                    });
                }
            }
        }
        self.advance_host_key_prompt();
        if history_changed {
            self.refresh_command_search();
        }
        if let Some(error) = history_error {
            self.show_error(&error);
        }
        updated
    }

    pub fn paste_to_active_terminal(&mut self, text: &str) {
        if self.is_modal_open() {
            return;
        }
        if text.len() > 1024 * 1024 {
            self.show_error("Paste exceeds 1 MiB");
            return;
        }
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        if tab.handle.is_none() {
            return;
        }
        if crate::paste::needs_preview(text) {
            self.open_confirm_run_modal(
                crate::paste::encode(text, false),
                "Вставка в терминал / Paste into terminal".into(),
            );
            if let Some(modal) = &mut self.confirm_run_modal {
                modal.paste = true;
            }
        } else {
            let text = crate::paste::encode(text, tab.terminal.screen.bracketed_paste);
            self.send_to_terminal(self.active_tab, &text);
        }
    }

    pub fn reconnect_active_tab(&mut self) {
        let active = self.active_tab;
        self.reconnect_tab(active);
    }

    pub fn execute_quick_action(&mut self, cmd: QuickActionCmd) {
        self.send_to_active_terminal(cmd.command_str());
    }

    pub fn toggle_split_view(&mut self) {
        if self.split_tab.is_some() {
            self.split_tab = None;
        } else if self.tabs.len() >= 2 {
            let next_tab = (self.active_tab + 1) % self.tabs.len();
            self.split_tab = Some(next_tab);
        }
        self.resize_terminals(self.last_window_w, self.last_window_h);
    }

    pub fn close_split_view(&mut self) {
        self.split_tab = None;
        self.resize_terminals(self.last_window_w, self.last_window_h);
    }

    pub fn open_terminal_search(&mut self) {
        if let Some(selected) = self.get_selected_text() {
            let trimmed = selected.trim();
            if !trimmed.is_empty() && !trimmed.contains('\n') {
                self.search_state.query.set(trimmed);
            }
        }
        self.search_state.is_open = true;
        self.focused_input = None;
        self.update_terminal_search();
    }

    pub fn close_terminal_search(&mut self) {
        self.search_state.is_open = false;
        self.search_state.matches.clear();
        self.search_state.selected_match_idx = 0;
    }

    pub fn toggle_search_case(&mut self) {
        self.search_state.case_sensitive = !self.search_state.case_sensitive;
        self.update_terminal_search();
    }

    pub fn update_terminal_search(&mut self) {
        if !self.search_state.is_open {
            return;
        }
        let query_text = self.search_state.query.text.clone();
        if query_text.trim().is_empty() {
            self.search_state.matches.clear();
            self.search_state.selected_match_idx = 0;
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            let matches = tab
                .terminal
                .search(&query_text, self.search_state.case_sensitive);
            self.search_state.matches = matches;
            if self.search_state.matches.is_empty() {
                self.search_state.selected_match_idx = 0;
            } else {
                if self.search_state.selected_match_idx >= self.search_state.matches.len() {
                    self.search_state.selected_match_idx = 0;
                }
                let target_match = self.search_state.matches[self.search_state.selected_match_idx];
                tab.terminal.scroll_to_abs_row(target_match.abs_row);
            }
        }
    }

    pub fn next_search_match(&mut self) {
        if self.search_state.matches.is_empty() {
            return;
        }
        self.search_state.selected_match_idx =
            (self.search_state.selected_match_idx + 1) % self.search_state.matches.len();
        let target_match = self.search_state.matches[self.search_state.selected_match_idx];
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.terminal.scroll_to_abs_row(target_match.abs_row);
        }
    }

    pub fn prev_search_match(&mut self) {
        if self.search_state.matches.is_empty() {
            return;
        }
        self.search_state.selected_match_idx = if self.search_state.selected_match_idx == 0 {
            self.search_state.matches.len() - 1
        } else {
            self.search_state.selected_match_idx - 1
        };
        let target_match = self.search_state.matches[self.search_state.selected_match_idx];
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.terminal.scroll_to_abs_row(target_match.abs_row);
        }
    }

    pub fn handle_key_input(&mut self, text: &str) {
        self.caret_visible = true;
        self.caret_blink_time = std::time::Instant::now();

        // -1. If Terminal Search is Open
        if self.search_state.is_open {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    self.next_search_match();
                    return;
                } else if ch == '\x08' {
                    self.search_state.query.delete_back();
                    self.update_terminal_search();
                } else if !ch.is_control() {
                    self.search_state.query.insert_char(ch);
                    self.update_terminal_search();
                }
            }
            return;
        }

        // 0. If Key Tool Modal is Open
        if self.key_tool_modal.is_open {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    self.execute_key_generation();
                    return;
                } else if ch == '\x08' {
                    match self.key_tool_modal.active_field {
                        0 => self.key_tool_modal.key_name.delete_back(),
                        1 => self.key_tool_modal.comment.delete_back(),
                        2 => self.key_tool_modal.passphrase.delete_back(),
                        _ => {}
                    }
                } else if !ch.is_control() {
                    match self.key_tool_modal.active_field {
                        0 => self.key_tool_modal.key_name.insert_char(ch),
                        1 => self.key_tool_modal.comment.insert_char(ch),
                        2 => self.key_tool_modal.passphrase.insert_char(ch),
                        _ => {}
                    }
                }
            }
            return;
        }

        // 0.1 If Add Snippet Modal is Open
        if self.add_snippet_modal.is_open {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    self.save_snippet_from_modal();
                    return;
                } else if ch == '\x08' {
                    match self.add_snippet_modal.active_field {
                        0 => self.add_snippet_modal.title.delete_back(),
                        1 => self.add_snippet_modal.command.delete_back(),
                        2 => self.add_snippet_modal.category.delete_back(),
                        3 => self.add_snippet_modal.description.delete_back(),
                        _ => {}
                    }
                } else if !ch.is_control() {
                    match self.add_snippet_modal.active_field {
                        0 => self.add_snippet_modal.title.insert_char(ch),
                        1 => self.add_snippet_modal.command.insert_char(ch),
                        2 => self.add_snippet_modal.category.insert_char(ch),
                        3 => self.add_snippet_modal.description.insert_char(ch),
                        _ => {}
                    }
                }
            }
            return;
        }

        // 1. If Modal is Open
        if self.add_host_modal.is_open {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    self.save_host_from_modal(false);
                    return;
                } else if ch == '\x08' {
                    match self.add_host_modal.active_field {
                        0 => self.add_host_modal.name.delete_back(),
                        1 => self.add_host_modal.hostname.delete_back(),
                        2 => self.add_host_modal.port.delete_back(),
                        3 => self.add_host_modal.username.delete_back(),
                        4 => self.add_host_modal.group.delete_back(),
                        5 => {
                            if self.add_host_modal.auth_mode == AuthModeChoice::Key {
                                self.add_host_modal.key_path.delete_back();
                            } else if self.add_host_modal.auth_mode == AuthModeChoice::Password {
                                self.add_host_modal.password.delete_back();
                            }
                        }
                        _ => {}
                    }
                } else if !ch.is_control() {
                    match self.add_host_modal.active_field {
                        0 => self.add_host_modal.name.insert_char(ch),
                        1 => self.add_host_modal.hostname.insert_char(ch),
                        2 => {
                            if ch.is_ascii_digit() {
                                self.add_host_modal.port.insert_char(ch);
                            }
                        }
                        3 => self.add_host_modal.username.insert_char(ch),
                        4 => self.add_host_modal.group.insert_char(ch),
                        5 => {
                            if self.add_host_modal.auth_mode == AuthModeChoice::Key {
                                self.add_host_modal.key_path.insert_char(ch);
                            } else if self.add_host_modal.auth_mode == AuthModeChoice::Password {
                                self.add_host_modal.password.insert_char(ch);
                            }
                        }
                        _ => {}
                    }
                }
            }
            return;
        }

        // 2. If Palette is Open
        if self.palette_open {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    self.execute_selected_palette_command();
                    self.palette_open = false;
                    return;
                } else if ch == '\x08' {
                    self.command_search_input.delete_back();
                    self.refresh_command_search();
                } else if !ch.is_control() {
                    self.command_search_input.insert_char(ch);
                    self.refresh_command_search();
                }
            }
            return;
        }

        // 2.5 If a sidebar search input is focused
        if let Some(field_id) = self.focused_input {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    if field_id == InputFieldId::HostSearch {
                        let filtered = self.filtered_hosts();
                        if let Some(&(first_idx, _)) = filtered.first() {
                            if let Some(host) = self.hosts_doc.hosts.get(first_idx).cloned() {
                                self.focused_input = None;
                                self.connect_to_host(host);
                                return;
                            }
                        }
                    } else if field_id == InputFieldId::CommandSearch {
                        if let Some(cmd) = self.command_search_results.first() {
                            let to_send = format!("{}\r", cmd.command);
                            self.focused_input = None;
                            self.send_to_active_terminal(&to_send);
                            return;
                        }
                    }
                    self.focused_input = None;
                    return;
                } else if ch == '\x08' {
                    if let Some(inp) = self.get_text_input_mut(field_id) {
                        inp.delete_back();
                    }
                    if field_id == InputFieldId::SnippetSearch {
                        self.refresh_snippets();
                    } else if field_id == InputFieldId::CommandSearch {
                        self.refresh_command_search();
                    }
                } else if !ch.is_control() {
                    if let Some(inp) = self.get_text_input_mut(field_id) {
                        inp.insert_char(ch);
                    }
                    if field_id == InputFieldId::SnippetSearch {
                        self.refresh_snippets();
                    } else if field_id == InputFieldId::CommandSearch {
                        self.refresh_command_search();
                    }
                }
            }
            return;
        }

        // Raw input includes passwords and editor keystrokes. Only shell integration
        // can report the command actually executed after readline editing.
        self.send_to_active_terminal(text);
    }

    pub fn execute_selected_palette_command(&mut self) {
        if let Some(record) = self.command_search_results.get(self.selected_search_idx) {
            let cmd = format!("{}\r", record.command);
            self.send_to_active_terminal(&cmd);
        }
    }

    pub fn send_to_active_terminal(&mut self, text: &str) {
        self.send_to_terminal(self.active_tab, text);
    }

    fn send_to_terminal(&mut self, index: usize, text: &str) {
        if self.is_app_locked {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.terminal.scroll_to_bottom();
            if let Some(handle) = &tab.handle {
                if *handle.is_connected.read() {
                    if let Err(e) = handle.input_tx.try_send(text.as_bytes().to_vec()) {
                        self.show_error(&format!("SSH input was not sent: {e}"));
                    }
                }
            }
        }
    }

    pub fn open_key_tool_generator(&mut self) {
        self.key_tool_modal = KeyToolModal {
            is_open: true,
            target_host_idx: None,
            algo: KeyAlgo::Ed25519,
            key_name: TextInput::new("id_ed25519"),
            comment: TextInput::new("user@evertyssh"),
            passphrase: TextInput::default(),
            generated_pub_key: None,
            generated_priv_path: None,
            status_msg: None,
            error_msg: None,
            active_field: 0,
        };
        self.caret_visible = true;
        self.caret_blink_time = std::time::Instant::now();
    }

    pub fn open_key_tool_for_host(&mut self, host_idx: usize) {
        let (suggested_name, suggested_comment) =
            if let Some(h) = self.hosts_doc.hosts.get(host_idx) {
                let safe_name: String = h
                    .name
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                let safe_name = if safe_name.is_empty() {
                    "server".into()
                } else {
                    safe_name.to_lowercase()
                };
                (
                    format!("id_ed25519_{safe_name}"),
                    format!("{}@{}", h.username, h.hostname),
                )
            } else {
                ("id_ed25519".into(), "user@evertyssh".into())
            };

        self.key_tool_modal = KeyToolModal {
            is_open: true,
            target_host_idx: Some(host_idx),
            algo: KeyAlgo::Ed25519,
            key_name: TextInput::new(suggested_name),
            comment: TextInput::new(suggested_comment),
            passphrase: TextInput::default(),
            generated_pub_key: None,
            generated_priv_path: None,
            status_msg: None,
            error_msg: None,
            active_field: 0,
        };
        self.caret_visible = true;
        self.caret_blink_time = std::time::Instant::now();
    }

    pub fn execute_key_generation(&mut self) -> bool {
        let key_name = self.key_tool_modal.key_name.text.trim();
        let comment = self.key_tool_modal.comment.text.trim();
        let pass = self.key_tool_modal.passphrase.text.clone();

        match generate_ssh_key(self.key_tool_modal.algo, key_name, comment, &pass) {
            Ok((priv_path, pub_key)) => {
                self.key_tool_modal.generated_priv_path = Some(priv_path);
                self.key_tool_modal.generated_pub_key = Some(pub_key);
                self.key_tool_modal.status_msg =
                    Some("✓ Ключ успешно сгенерирован и сохранен в ~/.ssh/".into());
                self.key_tool_modal.error_msg = None;
                true
            }
            Err(e) => {
                self.key_tool_modal.error_msg = Some(e);
                false
            }
        }
    }

    pub fn bind_generated_key_to_host(&mut self) -> bool {
        let (Some(index), Some(path)) = (
            self.key_tool_modal.target_host_idx,
            self.key_tool_modal.generated_priv_path.clone(),
        ) else {
            return false;
        };
        let mut proposed = self.hosts_doc.clone();
        let Some(host) = proposed.hosts.get_mut(index) else {
            return false;
        };
        host.auth = AuthConfig::Key {
            path: path.into(),
            passphrase: if self.key_tool_modal.passphrase.text.is_empty() {
                None
            } else {
                Some(self.key_tool_modal.passphrase.text.clone())
            },
        };
        let name = host.name.clone();
        match self.commit_document(proposed) {
            Ok(()) => {
                self.key_tool_modal.status_msg =
                    Some(format!("Ключ привязан к серверу / Key bound to '{name}'"));
                true
            }
            Err(e) => {
                self.key_tool_modal.error_msg = Some(e);
                false
            }
        }
    }

    pub fn send_deploy_cmd_to_active_terminal(&mut self) -> bool {
        if let Some(pub_key) = &self.key_tool_modal.generated_pub_key {
            let cmd = make_authorized_keys_cmd(pub_key);
            self.send_to_active_terminal(&cmd);
            self.key_tool_modal.status_msg = Some("⚡ Команда отправлена в консоль!".into());
            return true;
        }
        false
    }

    pub fn browse_key_file_for_add_modal(&mut self) {
        let ssh_dir = dirs::home_dir().map(|h| h.join(".ssh"));
        let mut dialog = rfd::FileDialog::new().set_title("Выберите SSH приватный ключ");
        if let Some(dir) = ssh_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.add_host_modal
                .key_path
                .set(path.to_string_lossy().to_string());
        }
    }

    pub fn close_active_tab(&mut self) {
        if self.active_tab < self.tabs.len() {
            let removed = self.tabs.remove(self.active_tab);
            if let Some(task) = self.pending_connections.remove(&removed.id) {
                task.abort();
            }
            self.advance_host_key_prompt();
            let _ = self.db.record_session_end(&removed.id, "closed", None);
            if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab = self.tabs.len() - 1;
            }
            self.refresh_recent_sessions();
        }
    }

    pub fn get_hover_tooltip_text<'a>(
        &'a self,
        target: HoverTarget,
    ) -> Option<std::borrow::Cow<'a, str>> {
        let lang = self.hosts_doc.settings.language;
        let s = match target {
            HoverTarget::HeaderAddBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Добавить новый хост (Ctrl+N)",
                "Add new host (Ctrl+N)",
            )),
            HoverTarget::HeaderSearchBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Поиск по истории команд (Ctrl+R)",
                "Search command history (Ctrl+R)",
            )),
            HoverTarget::HeaderLangBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Переключить язык (RU/EN)",
                "Switch language (RU/EN)",
            )),
            HoverTarget::HeaderSettingsBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Настройки: мастер-пароль, язык, бэкап",
                "Settings: master password, language, backup",
            )),
            HoverTarget::SidebarSplitter => std::borrow::Cow::Borrowed(tr(
                lang,
                "Потяните мышью для изменения ширины панели",
                "Drag to resize sidebar",
            )),
            HoverTarget::SidebarAddHostBtn => {
                std::borrow::Cow::Borrowed(tr(lang, "Добавить новый сервер", "Add new server"))
            }
            HoverTarget::SidebarAddSnippetBtn => {
                std::borrow::Cow::Borrowed(tr(lang, "Создать новый сниппет", "Create new snippet"))
            }
            HoverTarget::HostDeployKey(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Сгенерировать и установить SSH ключ на сервер",
                "Generate & deploy SSH key to server",
            )),
            HoverTarget::HostConnect(_) => {
                std::borrow::Cow::Borrowed(tr(lang, "Подключиться к серверу", "Connect to server"))
            }
            HoverTarget::HostEdit(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Редактировать параметры сервера",
                "Edit server configuration",
            )),
            HoverTarget::HostDelete(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Удалить сервер из списка",
                "Delete server from list",
            )),
            HoverTarget::SessionCard(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Открыть подробный дашборд сессии и журнал команд",
                "Open session dashboard & command log",
            )),
            HoverTarget::SessionDashboardReconnect => std::borrow::Cow::Borrowed(tr(
                lang,
                "Переподключиться к серверу в новой вкладке",
                "Reconnect to server in new tab",
            )),
            HoverTarget::SessionDashboardDelete => std::borrow::Cow::Borrowed(tr(
                lang,
                "Удалить эту сессию из истории",
                "Delete this session from history",
            )),
            HoverTarget::SessionDashboardRunCmd(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Запустить команду в активной консоли",
                "Run command in active terminal",
            )),
            HoverTarget::SessionDashboardCopyCmd(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Скопировать команду в буфер",
                "Copy command to clipboard",
            )),
            HoverTarget::SnippetRun(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Запустить сниппет в активном терминале",
                "Run snippet in active terminal",
            )),
            HoverTarget::SnippetCopy(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Скопировать текст сниппета",
                "Copy snippet text",
            )),
            HoverTarget::SnippetEdit(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Редактировать параметры сниппета",
                "Edit snippet configuration",
            )),
            HoverTarget::SnippetDelete(_) => {
                std::borrow::Cow::Borrowed(tr(lang, "Удалить сниппет", "Delete snippet"))
            }
            HoverTarget::ConfirmDeleteSnippetConfirm => std::borrow::Cow::Borrowed(tr(
                lang,
                "Подтвердить удаление сниппета",
                "Confirm snippet deletion",
            )),
            HoverTarget::ConfirmDeleteSnippetCancel => std::borrow::Cow::Borrowed(tr(
                lang,
                "Отменить удаление сниппета (Esc)",
                "Cancel snippet deletion (Esc)",
            )),
            HoverTarget::KeyToolGenerateBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Сгенерировать ключевую пару OpenSSH",
                "Generate OpenSSH key pair",
            )),
            HoverTarget::KeyToolSendTermBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Отправить ключ и настроить authorized_keys",
                "Deploy key to authorized_keys",
            )),
            HoverTarget::KeyToolBindHostBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Привязать ключ к профилю сервера",
                "Attach key to server profile",
            )),
            HoverTarget::KeyToolCopyPubBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Скопировать публичный ключ",
                "Copy public key",
            )),
            HoverTarget::ModalBrowseKey => std::borrow::Cow::Borrowed(tr(
                lang,
                "Выбрать файл ключа через проводник",
                "Browse key file via explorer",
            )),
            HoverTarget::ModalGenerateKey => std::borrow::Cow::Borrowed(tr(
                lang,
                "Сгенерировать новый SSH ключ",
                "Generate new SSH key",
            )),
            HoverTarget::SidebarSearchHost => std::borrow::Cow::Borrowed(tr(
                lang,
                "Поиск серверов по имени, адресу, логину или группе (Esc для сброса)",
                "Search servers by name, host, user or group (Esc to clear)",
            )),
            HoverTarget::SidebarSearchSnippet => std::borrow::Cow::Borrowed(tr(
                lang,
                "Поиск сохраненных сниппетов (Esc для сброса)",
                "Search saved snippets (Esc to clear)",
            )),
            HoverTarget::SidebarSearchCommand => std::borrow::Cow::Borrowed(tr(
                lang,
                "Поиск в истории команд",
                "Search command history",
            )),
            HoverTarget::HostKeyModalAccept => std::borrow::Cow::Borrowed(tr(
                lang,
                "Подтвердить ключ хоста и продолжить подключение",
                "Accept host key and continue connection",
            )),
            HoverTarget::HostKeyModalReject => std::borrow::Cow::Borrowed(tr(
                lang,
                "Отклонить ключ и разорвать соединение (Esc)",
                "Reject host key and abort connection (Esc)",
            )),
            HoverTarget::HostKeyModalCopy => std::borrow::Cow::Borrowed(tr(
                lang,
                "Скопировать отпечаток (fingerprint) в буфер",
                "Copy fingerprint to clipboard",
            )),
            HoverTarget::ConfirmDeleteConfirm => std::borrow::Cow::Borrowed(tr(
                lang,
                "Подтвердить безвозвратное удаление сервера",
                "Confirm permanent server deletion",
            )),
            HoverTarget::ConfirmDeleteCancel => std::borrow::Cow::Borrowed(tr(
                lang,
                "Отменить удаление сервера (Esc)",
                "Cancel server deletion (Esc)",
            )),
            HoverTarget::HeaderTab(idx) => {
                if let Some(tab) = self.tabs.get(idx) {
                    std::borrow::Cow::Owned(format!("{}: {}", tab.title, tab.status))
                } else {
                    return None;
                }
            }
            HoverTarget::SettingsClose => std::borrow::Cow::Borrowed(tr(
                lang,
                "Закрыть настройки (Esc)",
                "Close settings (Esc)",
            )),
            HoverTarget::SettingsTab(SettingsTab::About) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Информация о программе и разработчике",
                "About program and developer",
            )),
            HoverTarget::SettingsTab(SettingsTab::Security) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Настройки безопасности и мастер-пароля",
                "Security & master password settings",
            )),
            HoverTarget::SettingsTab(SettingsTab::Backup) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Резервное копирование и экспорт хостов",
                "Backup & export hosts",
            )),
            HoverTarget::SettingsTab(SettingsTab::General) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Общие параметры и настройки интерфейса",
                "General options and interface settings",
            )),
            HoverTarget::SettingsAboutLinkDesk => std::borrow::Cow::Borrowed(tr(
                lang,
                "Открыть сайт desk.everty.ru",
                "Open desk.everty.ru website",
            )),
            HoverTarget::SettingsAboutLinkEmail => std::borrow::Cow::Borrowed(tr(
                lang,
                "Написать на info@everty.ru (копировать адрес)",
                "Write to info@everty.ru (copy address)",
            )),
            HoverTarget::SettingsAboutLinkTelegram => std::borrow::Cow::Borrowed(tr(
                lang,
                "Открыть Telegram канал: @Everty_web",
                "Open Telegram channel: @Everty_web",
            )),
            HoverTarget::SettingsAboutLinkGithub => std::borrow::Cow::Borrowed(tr(
                lang,
                "Репозиторий EvertySSH на GitHub",
                "EvertySSH GitHub repository",
            )),
            HoverTarget::SettingsSavePassword => std::borrow::Cow::Borrowed(tr(
                lang,
                "Сохранить настройки мастер-пароля",
                "Save master password settings",
            )),
            HoverTarget::SettingsRemovePassword => std::borrow::Cow::Borrowed(tr(
                lang,
                "Отключить защиту паролем",
                "Disable password protection",
            )),
            HoverTarget::SettingsExportJson => std::borrow::Cow::Borrowed(tr(
                lang,
                "Экспортировать серверы и настройки в JSON файл",
                "Export servers & settings to JSON",
            )),
            HoverTarget::SettingsImportJson => std::borrow::Cow::Borrowed(tr(
                lang,
                "Импортировать серверы из файла резервной копии",
                "Import servers from JSON backup",
            )),
            HoverTarget::SettingsSaveGeneral => std::borrow::Cow::Borrowed(tr(
                lang,
                "Сохранить общие настройки",
                "Save general settings",
            )),
            HoverTarget::SettingsToggleConfirmDelete => std::borrow::Cow::Borrowed(tr(
                lang,
                "Включить/выключить подтверждение удаления",
                "Toggle delete confirmation dialog",
            )),
            HoverTarget::SettingsSetLanguage(AppLanguage::Ru) => {
                std::borrow::Cow::Borrowed("Переключить интерфейс на Русский язык")
            }
            HoverTarget::SettingsSetLanguage(AppLanguage::En) => {
                std::borrow::Cow::Borrowed("Switch interface to English language")
            }
            HoverTarget::LockUnlockBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Разблокировать приложение (Enter)",
                "Unlock application (Enter)",
            )),
            HoverTarget::ConfirmRunExecute => std::borrow::Cow::Borrowed(tr(
                lang,
                "Выполнить команду в консоли (Enter)",
                "Execute command in terminal (Enter)",
            )),
            HoverTarget::ConfirmRunCancel => std::borrow::Cow::Borrowed(tr(
                lang,
                "Отменить запуск команды (Esc)",
                "Cancel command execution (Esc)",
            )),
            HoverTarget::ConfirmRunCopy => std::borrow::Cow::Borrowed(tr(
                lang,
                "Скопировать текст команды в буфер",
                "Copy command text",
            )),
            HoverTarget::TerminalReconnect(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Повторно подключиться к серверу в этой вкладке",
                "Reconnect session in this tab",
            )),
            HoverTarget::TerminalCloseTab(_) => std::borrow::Cow::Borrowed(tr(
                lang,
                "Закрыть эту вкладку терминала",
                "Close this terminal tab",
            )),
            HoverTarget::TerminalStatusBanner(tab_idx) => {
                if let Some(tab) = self.tabs.get(tab_idx) {
                    std::borrow::Cow::Borrowed(tab.status.as_str())
                } else {
                    return None;
                }
            }
            HoverTarget::QuickAction(cmd) => std::borrow::Cow::Borrowed(cmd.tooltip()),
            HoverTarget::TerminalSearchOpenBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Поиск текста в консоли (Ctrl+F)",
                "Search text in terminal (Ctrl+F)",
            )),
            HoverTarget::TerminalSearchCloseBtn => {
                std::borrow::Cow::Borrowed(tr(lang, "Закрыть поиск (Esc)", "Close search (Esc)"))
            }
            HoverTarget::TerminalSearchPrevBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Предыдущее совпадение (Shift+Enter)",
                "Previous match (Shift+Enter)",
            )),
            HoverTarget::TerminalSearchNextBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Следующее совпадение (Enter)",
                "Next match (Enter)",
            )),
            HoverTarget::TerminalSearchCaseToggle => std::borrow::Cow::Borrowed(tr(
                lang,
                "Учитывать регистр символов (Aa)",
                "Match case (Aa)",
            )),
            HoverTarget::TerminalSearchInput => std::borrow::Cow::Borrowed(tr(
                lang,
                "Поле поиска текста в терминале",
                "Terminal search input",
            )),
            HoverTarget::TerminalSplitBtn => std::borrow::Cow::Borrowed(tr(
                lang,
                "Разделить экран терминала (Ctrl+Alt+S)",
                "Split terminal screen (Ctrl+Alt+S)",
            )),
            HoverTarget::TerminalSplitClose => std::borrow::Cow::Borrowed(tr(
                lang,
                "Закрыть разделение терминала",
                "Close terminal split",
            )),
            HoverTarget::SplitTerminalArea => std::borrow::Cow::Borrowed(tr(
                lang,
                "Активировать этот терминал",
                "Activate this terminal",
            )),
            _ => return None,
        };
        Some(s)
    }

    pub fn get_text_input_mut(&mut self, id: InputFieldId) -> Option<&mut TextInput> {
        match id {
            InputFieldId::AddHost(0) => Some(&mut self.add_host_modal.name),
            InputFieldId::AddHost(1) => Some(&mut self.add_host_modal.hostname),
            InputFieldId::AddHost(2) => Some(&mut self.add_host_modal.port),
            InputFieldId::AddHost(3) => Some(&mut self.add_host_modal.username),
            InputFieldId::AddHost(4) => Some(&mut self.add_host_modal.group),
            InputFieldId::AddHost(5) => {
                if self.add_host_modal.auth_mode == AuthModeChoice::Key {
                    Some(&mut self.add_host_modal.key_path)
                } else {
                    Some(&mut self.add_host_modal.password)
                }
            }
            InputFieldId::AddHost(_) => None,
            InputFieldId::KeyTool(0) => Some(&mut self.key_tool_modal.key_name),
            InputFieldId::KeyTool(1) => Some(&mut self.key_tool_modal.comment),
            InputFieldId::KeyTool(2) => Some(&mut self.key_tool_modal.passphrase),
            InputFieldId::KeyTool(_) => None,
            InputFieldId::AddSnippet(0) => Some(&mut self.add_snippet_modal.title),
            InputFieldId::AddSnippet(1) => Some(&mut self.add_snippet_modal.command),
            InputFieldId::AddSnippet(2) => Some(&mut self.add_snippet_modal.category),
            InputFieldId::AddSnippet(3) => Some(&mut self.add_snippet_modal.description),
            InputFieldId::AddSnippet(_) => None,
            InputFieldId::CommandSearch => Some(&mut self.command_search_input),
            InputFieldId::SnippetSearch => Some(&mut self.snippet_search_input),
            InputFieldId::HostSearch => Some(&mut self.host_search_input),
            InputFieldId::SettingsSecurity(0) => self
                .settings_modal
                .as_mut()
                .map(|m| &mut m.current_password),
            InputFieldId::SettingsSecurity(1) => {
                self.settings_modal.as_mut().map(|m| &mut m.new_password)
            }
            InputFieldId::SettingsSecurity(2) => self
                .settings_modal
                .as_mut()
                .map(|m| &mut m.confirm_password),
            InputFieldId::SettingsSecurity(_) => None,
            InputFieldId::SettingsGeneral(0) => {
                self.settings_modal.as_mut().map(|m| &mut m.default_user)
            }
            InputFieldId::SettingsGeneral(1) => self
                .settings_modal
                .as_mut()
                .map(|m| &mut m.scrollback_lines),
            InputFieldId::SettingsGeneral(_) => None,
            InputFieldId::SettingsBackup(0) => {
                self.settings_modal.as_mut().map(|m| &mut m.backup_password)
            }
            InputFieldId::SettingsBackup(1) => {
                self.settings_modal.as_mut().map(|m| &mut m.backup_confirm)
            }
            InputFieldId::SettingsBackup(_) => None,
            InputFieldId::LockPassword => Some(&mut self.lock_password_input),
            InputFieldId::TerminalSearch => Some(&mut self.search_state.query),
        }
    }

    pub fn get_text_input(&self, id: InputFieldId) -> Option<&TextInput> {
        match id {
            InputFieldId::AddHost(0) => Some(&self.add_host_modal.name),
            InputFieldId::AddHost(1) => Some(&self.add_host_modal.hostname),
            InputFieldId::AddHost(2) => Some(&self.add_host_modal.port),
            InputFieldId::AddHost(3) => Some(&self.add_host_modal.username),
            InputFieldId::AddHost(4) => Some(&self.add_host_modal.group),
            InputFieldId::AddHost(5) => {
                if self.add_host_modal.auth_mode == AuthModeChoice::Key {
                    Some(&self.add_host_modal.key_path)
                } else {
                    Some(&self.add_host_modal.password)
                }
            }
            InputFieldId::AddHost(_) => None,
            InputFieldId::KeyTool(0) => Some(&self.key_tool_modal.key_name),
            InputFieldId::KeyTool(1) => Some(&self.key_tool_modal.comment),
            InputFieldId::KeyTool(2) => Some(&self.key_tool_modal.passphrase),
            InputFieldId::KeyTool(_) => None,
            InputFieldId::AddSnippet(0) => Some(&self.add_snippet_modal.title),
            InputFieldId::AddSnippet(1) => Some(&self.add_snippet_modal.command),
            InputFieldId::AddSnippet(2) => Some(&self.add_snippet_modal.category),
            InputFieldId::AddSnippet(3) => Some(&self.add_snippet_modal.description),
            InputFieldId::AddSnippet(_) => None,
            InputFieldId::CommandSearch => Some(&self.command_search_input),
            InputFieldId::SnippetSearch => Some(&self.snippet_search_input),
            InputFieldId::HostSearch => Some(&self.host_search_input),
            InputFieldId::SettingsSecurity(0) => {
                self.settings_modal.as_ref().map(|m| &m.current_password)
            }
            InputFieldId::SettingsSecurity(1) => {
                self.settings_modal.as_ref().map(|m| &m.new_password)
            }
            InputFieldId::SettingsSecurity(2) => {
                self.settings_modal.as_ref().map(|m| &m.confirm_password)
            }
            InputFieldId::SettingsSecurity(_) => None,
            InputFieldId::SettingsGeneral(0) => {
                self.settings_modal.as_ref().map(|m| &m.default_user)
            }
            InputFieldId::SettingsGeneral(1) => {
                self.settings_modal.as_ref().map(|m| &m.scrollback_lines)
            }
            InputFieldId::SettingsGeneral(_) => None,
            InputFieldId::SettingsBackup(0) => {
                self.settings_modal.as_ref().map(|m| &m.backup_password)
            }
            InputFieldId::SettingsBackup(1) => {
                self.settings_modal.as_ref().map(|m| &m.backup_confirm)
            }
            InputFieldId::SettingsBackup(_) => None,
            InputFieldId::LockPassword => Some(&self.lock_password_input),
            InputFieldId::TerminalSearch => Some(&self.search_state.query),
        }
    }

    pub fn open_input_context_menu(
        &mut self,
        x: f64,
        y: f64,
        field_id: InputFieldId,
        win_w: f64,
        win_h: f64,
    ) {
        let menu_w = 170.0;
        let menu_h = 132.0;
        let clamped_x = x.min(win_w - menu_w - 8.0).max(8.0);
        let clamped_y = y.min(win_h - menu_h - 8.0).max(8.0);
        self.input_context_menu = Some(InputContextMenu {
            is_open: true,
            x: clamped_x,
            y: clamped_y,
            field_id,
        });
    }

    pub fn handle_input_menu_action(&mut self, action: InputMenuAction) {
        let field_id = match &self.input_context_menu {
            Some(m) if m.is_open => m.field_id,
            _ => return,
        };
        self.input_context_menu = None;

        match action {
            InputMenuAction::Cut => {
                if let Some(inp) = self.get_text_input_mut(field_id) {
                    if let Some(sel) = inp.selected_text() {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(sel);
                        }
                        inp.delete_selection();
                    }
                }
            }
            InputMenuAction::Copy => {
                if let Some(inp) = self.get_text_input(field_id) {
                    if let Some(sel) = inp.selected_text() {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(sel);
                        }
                    } else if !inp.text.is_empty() {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(inp.text.clone());
                        }
                    }
                }
            }
            InputMenuAction::Paste => {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    if let Ok(text) = cb.get_text() {
                        if !text.is_empty() {
                            if let Some(inp) = self.get_text_input_mut(field_id) {
                                inp.delete_selection();
                                inp.insert_str(&text);
                            }
                        }
                    }
                }
            }
            InputMenuAction::SelectAll => {
                if let Some(inp) = self.get_text_input_mut(field_id) {
                    inp.select_all();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(
            percent_encode("ssh-keygen -t ed25519"),
            "ssh-keygen+-t+ed25519"
        );
        let cyrillic_encoded = percent_encode("привет");
        assert!(cyrillic_encoded.contains('%'));
    }

    #[test]
    fn test_terminal_selection_contains() {
        let sel = TerminalSelection::new(
            TerminalPos { col: 5, row: 1 },
            TerminalPos { col: 10, row: 3 },
        );
        assert!(!sel.contains(4, 1));
        assert!(sel.contains(5, 1));
        assert!(sel.contains(20, 1)); // rest of first line
        assert!(sel.contains(0, 2)); // entire middle line
        assert!(sel.contains(50, 2)); // entire middle line
        assert!(sel.contains(0, 3)); // start of last line
        assert!(sel.contains(10, 3)); // up to col 10
        assert!(!sel.contains(11, 3)); // beyond col 10 on last line
        assert!(!sel.contains(5, 4)); // next line
    }

    #[test]
    fn test_mono_metrics() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        let mut ui = crate::ui::UiRenderer::new();
        app.terminal_char_width = ui.mono_char_w;
        let mono_style = everty_ui::TextStyle {
            family: everty_ui::FontFamily::Monospace,
            weight: everty_ui::FontWeight::Regular,
            font_size: everty_ui::LogicalPx(13.0),
            line_height: everty_ui::LogicalPx(18.0),
        };
        let l1 = ui.text_system.shape("a", &mono_style, None);
        assert!(app.terminal_char_width.is_finite() && app.terminal_char_width > 0.0);
        assert!((l1.metrics.size.width.0 - app.terminal_char_width).abs() < 0.001);
        app.resize_terminals(1120.0, 720.0);
        let expected_cols = ((1120.0 - app.sidebar_width - 24.0) / ui.mono_char_w).floor() as usize;
        assert_eq!(app.terminal_cols, expected_cols);
        let x = app.sidebar_width + 10.0 + 3.5 * ui.mono_char_w;
        assert_eq!(
            app.pos_from_coords(x, app.header_height + 7.0),
            Some(TerminalPos { col: 3, row: 0 })
        );
    }

    #[test]
    fn test_make_authorized_keys_cmd() {
        let pub_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGeneratedKey123 user@evertyssh";
        let cmd = make_authorized_keys_cmd(pub_key);
        assert!(cmd.contains("mkdir -p ~/.ssh"));
        assert!(cmd.contains("echo 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGeneratedKey123 user@evertyssh' >> ~/.ssh/authorized_keys"));
        assert!(cmd.contains("chmod 600 ~/.ssh/authorized_keys"));
        assert!(cmd.ends_with('\n'));
    }

    #[test]
    fn test_pos_from_coords_calculation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.sidebar_width = 240.0;
        app.header_height = 42.0;
        app.terminal_cols = 80;
        app.terminal_rows = 24;

        // Disconnected tab: bar_h = 0.0 -> pad_y = 42 + 0 + 6 = 48.0, pad_x = 240 + 10 = 250.0
        let pos0 = app.pos_from_coords(254.0, 50.0);
        assert_eq!(pos0, Some(TerminalPos { col: 0, row: 0 }));

        let pos1 = app.pos_from_coords(254.0, 50.0 + 18.0);
        assert_eq!(pos1, Some(TerminalPos { col: 0, row: 1 }));
    }

    #[test]
    fn test_text_input_horizontal_scrolling() {
        let mut inp = TextInput::new("docker run -d --name nginx-server -p 80:80 -v /var/www:/usr/share/nginx/html:ro nginx:latest");
        assert_eq!(inp.cursor, inp.text.chars().count());
        assert_eq!(inp.scroll_offset, 0);

        // Home key jumps to start and resets scroll
        inp.move_home();
        assert_eq!(inp.cursor, 0);
        assert_eq!(inp.scroll_offset, 0);

        // Scroll delta towards end (lines < 0)
        inp.scroll_delta(-10, 40);
        assert_eq!(inp.scroll_offset, 10);
        assert_eq!(inp.cursor, 10);

        // Scroll delta towards start (lines > 0)
        inp.scroll_delta(6, 40);
        assert_eq!(inp.scroll_offset, 4);
        assert_eq!(inp.cursor, 10); // cursor remains within visible range [4, 44]

        // Clicking at 100px from left
        inp.set_cursor_from_click(100.0, 10.0);
        assert_eq!(inp.cursor, 4 + 10); // scroll_offset (4) + 10 chars
    }

    #[test]
    fn test_text_input_selection_and_drag() {
        let mut inp = TextInput::new("systemctl restart nginx");
        inp.cursor = 10;
        inp.select_word_at_cursor();
        assert_eq!(inp.selected_text(), Some("restart".to_string()));

        // Test drag selection simulation
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.add_snippet_modal.is_open = true;
        app.add_snippet_modal
            .command
            .set("sudo systemctl status docker");

        // Click at character index 5 ("systemctl...")
        let input_origin_x = 100.0;
        app.handle_input_click(
            InputFieldId::AddSnippet(1),
            input_origin_x + 5.0 * 7.0,
            input_origin_x,
            1,
        );
        assert!(app.is_input_selecting);
        assert_eq!(app.add_snippet_modal.command.cursor, 5);

        // Drag to character index 14
        let changed = app.update_input_selection(input_origin_x + 14.0 * 7.0);
        assert!(changed);
        assert_eq!(app.add_snippet_modal.command.cursor, 14);
        assert_eq!(
            app.add_snippet_modal.command.selected_range(),
            Some((5, 14))
        );

        // End drag selection
        let ended = app.end_input_selection();
        assert!(ended);
        assert!(!app.is_input_selecting);
        assert_eq!(
            app.add_snippet_modal.command.selected_range(),
            Some((5, 14))
        );

        // Test triple click selects whole line
        app.handle_input_click(
            InputFieldId::AddSnippet(1),
            input_origin_x + 10.0,
            input_origin_x,
            3,
        );
        assert_eq!(
            app.add_snippet_modal.command.selected_range(),
            Some((0, 28))
        );
    }

    #[test]
    fn test_modal_snippet_hit_testing_stability() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.add_snippet_modal.is_open = true;

        let w = 1120.0;
        let h = 720.0;
        let modal_w = 520.0;
        let modal_h = 420.0;
        let modal_x = (w - modal_w) / 2.0; // 300.0
        let modal_y = (h - modal_h) / 2.0; // 150.0

        // 1. Hover on first field label ("Название:" at modal_y + 55 = 205.0)
        let (_, _) = app.update_hover(modal_x + 30.0, modal_y + 60.0, w, h);
        assert_eq!(app.hover, HoverTarget::ModalSnippetField(0));

        // 2. Hover inside first field input box (modal_y + 75 = 225.0)
        let (_, _) = app.update_hover(modal_x + 30.0, modal_y + 80.0, w, h);
        assert_eq!(app.hover, HoverTarget::ModalSnippetField(0));

        // 3. Hover on modal title or card background (not a field or button)
        let (_, _) = app.update_hover(modal_x + 30.0, modal_y + 20.0, w, h);
        assert_eq!(app.hover, HoverTarget::ModalDialogBody);

        // 4. Hover outside modal entirely
        let (_, _) = app.update_hover(10.0, 10.0, w, h);
        assert_eq!(app.hover, HoverTarget::None);
    }

    #[test]
    fn test_text_input_word_navigation_and_deletion() {
        let mut inp = TextInput::new("git commit -m initial");
        assert_eq!(inp.cursor, 21);

        // Word navigation left
        inp.move_word_left(); // skips "initial" -> 14
        assert_eq!(inp.cursor, 14);
        inp.move_word_left(); // skips "m" -> 12
        assert_eq!(inp.cursor, 12);
        inp.move_word_left(); // skips "-" -> 11
        assert_eq!(inp.cursor, 11);
        inp.move_word_left(); // skips "commit " -> 4
        assert_eq!(inp.cursor, 4);

        // Word navigation right
        inp.move_word_right(); // skips "commit " -> 11
        assert_eq!(inp.cursor, 11);

        // Shift selection
        inp.move_left_select();
        assert_eq!(inp.selected_range(), Some((10, 11)));

        // Word deletion back
        inp.set("git commit -m initial");
        assert_eq!(inp.cursor, 21);
        inp.delete_word_back(); // deletes "initial"
        assert_eq!(inp.text, "git commit -m ");
        assert_eq!(inp.cursor, 14);

        // Word deletion forward
        inp.cursor = 4; // at "commit -m "
        inp.delete_word_forward(); // deletes "commit "
        assert_eq!(inp.text, "git -m ");
    }

    #[test]
    fn test_handle_text_navigation_key() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.add_host_modal.is_open = true;
        app.add_host_modal.name.set("prod-server-01");

        // Navigate left with Ctrl
        let left_key = winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowLeft);
        let handled = app.handle_text_navigation_key(&left_key, true, false);
        assert!(handled);
        assert_eq!(app.add_host_modal.name.cursor, 12); // jumps before "01"

        // Select with Shift+Left
        let handled_sel = app.handle_text_navigation_key(&left_key, false, true);
        assert!(handled_sel);
        assert_eq!(app.add_host_modal.name.selected_range(), Some((11, 12)));

        // Jump to Home
        let home_key = winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home);
        app.handle_text_navigation_key(&home_key, false, false);
        assert_eq!(app.add_host_modal.name.cursor, 0);

        // Jump to End
        let end_key = winit::keyboard::Key::Named(winit::keyboard::NamedKey::End);
        app.handle_text_navigation_key(&end_key, false, false);
        assert_eq!(app.add_host_modal.name.cursor, 14);
    }

    #[test]
    fn test_header_tabs_hit_testing_stability() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        // Create 2 tabs (index 0 and index 1)
        app.tabs.push(SessionTab {
            id: "tab-01".into(),
            title: "server-alpha".into(),
            host_address: "192.168.1.1".into(),
            username: "root".into(),
            status: "🟢 Connected".into(),
            terminal: Terminal::new(80, 24, 100),
            handle: None,
            host_id: None,
            host_config: None,
        });
        app.tabs.push(SessionTab {
            id: "tab-02".into(),
            title: "server-beta".into(),
            host_address: "192.168.1.10".into(),
            username: "root".into(),
            status: "🟢 Connected".into(),
            terminal: Terminal::new(80, 24, 100),
            handle: None,
            host_id: None,
            host_config: None,
        });

        // Test on narrow/compressed window (e.g. w = 700.0)
        let w = 700.0;
        let h = 500.0;

        // Hover on Tab 0 body (near middle of tab 0)
        let (_, _) = app.update_hover(200.0, 15.0, w, h);
        assert_eq!(app.hover, HoverTarget::HeaderTab(0));

        // Hover on Tab 1 body (on tab name/title)
        let (_, _) = app.update_hover(370.0, 15.0, w, h);
        assert_eq!(app.hover, HoverTarget::HeaderTab(1));

        // Hover on Settings button
        let (_, _) = app.update_hover(w - 30.0, 15.0, w, h);
        assert_eq!(app.hover, HoverTarget::HeaderSettingsBtn);
    }

    #[test]
    fn test_split_banner_hit_testing() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.tabs.push(SessionTab {
            id: "tab-01".into(),
            title: "server-left".into(),
            host_address: "192.168.1.1".into(),
            username: "root".into(),
            status: "🟢 Connected".into(),
            terminal: Terminal::new(80, 24, 100),
            handle: None,
            host_id: None,
            host_config: None,
        });
        app.tabs.push(SessionTab {
            id: "tab-split".into(),
            title: "server-split".into(),
            host_address: "10.0.0.1".into(),
            username: "root".into(),
            status: "🔴 Ошибка: Connection refused".into(),
            terminal: Terminal::new(80, 24, 100),
            handle: None,
            host_id: None,
            host_config: None,
        });
        app.active_tab = 0;
        app.split_tab = Some(1);

        let w = 1000.0;
        let h = 600.0;

        // Right panel starts around x = 240 + (1000 - 240)/2 + 2 = 622.0
        // Test hovering near the right panel bottom banner close button
        let banner_y = h - app.footer_height - 25.0;
        let (_, _) = app.update_hover(w - 20.0, banner_y, w, h);
        assert_eq!(app.hover, HoverTarget::TerminalCloseTab(1));

        // Test hovering on reconnect button in right panel
        let (_, _) = app.update_hover(w - 80.0, banner_y, w, h);
        assert_eq!(app.hover, HoverTarget::TerminalReconnect(1));
    }

    #[test]
    fn test_confirm_run_modal_hit_testing() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.open_confirm_run_modal("rm -rf /var/log/*".into(), "Сниппет «Очистить логи»".into());
        assert!(app.confirm_run_modal.is_some());

        let w = 1000.0;
        let h = 700.0;
        let modal = app.confirm_run_modal.as_ref().unwrap();
        let layout = modal.layout(w, h);

        // Cancel button center
        let (cx, cy, cw, ch) = layout.cancel_rect;
        let (_, _) = app.update_hover(cx + cw / 2.0, cy + ch / 2.0, w, h);
        assert_eq!(app.hover, HoverTarget::ConfirmRunCancel);

        // Copy button center
        let (k_x, k_y, k_w, k_h) = layout.copy_rect;
        let (_, _) = app.update_hover(k_x + k_w / 2.0, k_y + k_h / 2.0, w, h);
        assert_eq!(app.hover, HoverTarget::ConfirmRunCopy);

        // Execute button center
        let (ex, ey, ew, eh) = layout.exec_rect;
        let (_, _) = app.update_hover(ex + ew / 2.0, ey + eh / 2.0, w, h);
        assert_eq!(app.hover, HoverTarget::ConfirmRunExecute);

        // Outside buttons but inside modal
        let (_, _) = app.update_hover(layout.modal_x + 10.0, layout.modal_y + 10.0, w, h);
        assert_eq!(app.hover, HoverTarget::ModalDialogBody);
    }

    #[test]
    fn test_settings_about_tab_hit_testing() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        app.open_settings_modal();
        assert!(app.settings_modal.is_some());
        let modal = app.settings_modal.as_ref().unwrap();
        assert_eq!(modal.active_tab, SettingsTab::About);

        let w = 1000.0;
        let h = 700.0;
        let modal_w = 620.0f64.min(w - 40.0);
        let modal_h = 490.0f64.min(h - 40.0);
        let modal_x = (w - modal_w) / 2.0;
        let modal_y = (h - modal_h) / 2.0;

        // Hover About tab
        let (_, _) = app.update_hover(modal_x + 50.0, modal_y + 70.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsTab(SettingsTab::About));

        // Hover Desk link card
        let (_, _) = app.update_hover(modal_x + 50.0, modal_y + 260.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsAboutLinkDesk);

        // Hover Telegram link card
        let (_, _) = app.update_hover(modal_x + 350.0, modal_y + 260.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsAboutLinkTelegram);

        // Hover Email link card
        let (_, _) = app.update_hover(modal_x + 50.0, modal_y + 320.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsAboutLinkEmail);

        // Hover GitHub link card
        let (_, _) = app.update_hover(modal_x + 350.0, modal_y + 320.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsAboutLinkGithub);

        // Switch to Backup tab
        if let Some(m) = &mut app.settings_modal {
            m.active_tab = SettingsTab::Backup;
        }

        // Hover Export JSON button (modal_x + 38..238, modal_y + 164..194)
        let (_, _) = app.update_hover(modal_x + 100.0, modal_y + 175.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsExportJson);

        // Hover Import JSON button (modal_x + 38..238, modal_y + 274..304)
        let (_, _) = app.update_hover(modal_x + 100.0, modal_y + 285.0, w, h);
        assert_eq!(app.hover, HoverTarget::SettingsImportJson);
    }

    #[test]
    fn test_scrollbar_compressed_height_no_panic() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = AppState::new(rt.handle().clone()).unwrap();
        let mut term = Terminal::new(80, 24, 100);
        // Fill scrollback buffer so scrollback_len > 0
        for i in 0..50 {
            term.write_bytes(format!("line {i}\r\n").as_bytes());
        }
        app.tabs.push(SessionTab {
            id: "tab-test".into(),
            title: "server".into(),
            host_address: "127.0.0.1".into(),
            username: "root".into(),
            status: "🟢 Connected".into(),
            terminal: term,
            handle: None,
            host_id: None,
            host_config: None,
        });
        app.active_tab = 0;

        // Test with compressed heights where sb_h <= 24.0 (previously caused panic min = 24.0, max = 20.0)
        for compressed_h in [50.0, 60.0, 70.0, 80.0, 100.0, 150.0] {
            let (_, _) = app.update_hover(995.0, 45.0, 1000.0, compressed_h);
        }
    }
}
