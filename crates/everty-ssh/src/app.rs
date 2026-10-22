use std::collections::HashMap;
use std::path::PathBuf;
use crossbeam_channel::{Receiver, Sender};
use winit::window::CursorIcon;

use crate::config::{
    hash_password, load_hosts, save_hosts, verify_password, AuthConfig, ExportBundle, HostConfig,
    HostsDocument,
};
use crate::db::{CommandRecord, HistoryDb, SessionRecord, SnippetRecord};
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

    pub fn move_left(&mut self) {
        self.selection = None;
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_right(&mut self) {
        self.selection = None;
        let len = self.text.chars().count();
        if self.cursor < len {
            self.cursor += 1;
        }
    }

    pub fn move_home(&mut self) {
        self.selection = None;
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.selection = None;
        self.cursor = self.text.chars().count();
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
        if self.start.row < self.end.row || (self.start.row == self.end.row && self.start.col <= self.end.col) {
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

pub fn open_google_search(query: &str) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }
    let encoded = percent_encode(trimmed);
    let url = format!("https://www.google.com/search?q={}", encoded);

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(&url)
            .spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn();
    }
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
        .arg("-t").arg(algo_flag)
        .arg("-C").arg(comment.trim())
        .arg("-N").arg(passphrase)
        .arg("-f").arg(&priv_path);

    if algo == KeyAlgo::Rsa4096 {
        cmd.arg("-b").arg("4096");
    }

    let output = cmd.output().map_err(|e| format!("Не удалось запустить ssh-keygen: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh-keygen завершился с ошибкой: {err}"));
    }

    let pub_content = std::fs::read_to_string(&pub_path)
        .map_err(|e| format!("Не удалось прочитать сгенерированный публичный ключ: {e}"))?;

    Ok((priv_path.to_string_lossy().to_string(), pub_content.trim().to_string()))
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

#[derive(Debug, Clone)]
pub struct ConfirmRunCommandModal {
    pub is_open: bool,
    pub command: String,
    pub source_title: String,
    pub is_dangerous: bool,
    pub danger_reason: Option<String>,
    pub target_tab_title: String,
    pub can_execute: bool,
    pub copied_feedback: bool,
}

pub fn check_dangerous_command(cmd: &str) -> (bool, Option<String>) {
    let lower = cmd.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    if lower.contains("rm -rf") || lower.contains("rm -fr") || lower.contains("rm -r") {
        return (true, Some("Рекурсивное удаление файлов и каталогов (rm -r)".into()));
    }
    if words.contains(&"rm") || words.contains(&"rmdir") {
        return (true, Some("Команда содержит операцию удаления файлов/папок".into()));
    }
    if words.contains(&"reboot") || words.contains(&"shutdown") || lower.contains("init 0") || lower.contains("init 6") {
        return (true, Some("Команда перезагрузит или выключит сервер".into()));
    }
    if words.contains(&"kill") || words.contains(&"killall") || words.contains(&"pkill") {
        return (true, Some("Команда принудительно завершит системные процессы".into()));
    }
    if lower.contains("dd if=") || words.contains(&"mkfs") || words.contains(&"fdisk") || words.contains(&"parted") {
        return (true, Some("Низкоуровневая операция с дисками и разделами".into()));
    }
    if lower.contains("drop database") || lower.contains("drop table") || lower.contains("truncate ") {
        return (true, Some("Удаление таблиц или базы данных".into()));
    }
    if lower.contains("chmod -r 777") || lower.contains("chmod 777") {
        return (true, Some("Открытие полного небезопасного доступа ко всем файлам (777)".into()));
    }
    if lower.contains("> /dev/") {
        return (true, Some("Прямая перезапись системного устройства".into()));
    }
    (false, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
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

    // General fields
    pub default_user: TextInput,
    pub scrollback_lines: TextInput,
    pub confirm_delete: bool,
    pub active_general_field: usize,
    pub general_msg: Option<(String, bool)>,
}

#[derive(Debug, Clone, Default)]
pub struct AddSnippetModal {
    pub is_open: bool,
    pub title: TextInput,
    pub command: TextInput,
    pub category: TextInput,
    pub description: TextInput,
    pub active_field: usize, // 0=title, 1=command, 2=category, 3=description
    pub error_msg: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    None,
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
    SnippetDelete(usize),
    // Add Snippet Modal
    ModalSnippetField(usize),
    ModalSnippetSave,
    ModalSnippetClose,
    // Settings & Security
    HeaderSettingsBtn,
    ConfirmDeleteConfirm,
    ConfirmDeleteCancel,
    SettingsClose,
    SettingsTab(SettingsTab),
    SettingsSavePassword,
    SettingsRemovePassword,
    SettingsExportJson,
    SettingsImportJson,
    SettingsSaveGeneral,
    SettingsToggleConfirmDelete,
    SettingsSecurityField(usize),
    SettingsGeneralField(usize),
    LockUnlockBtn,
    LockPasswordField,
    // Confirm Run Command Modal
    ConfirmRunExecute,
    ConfirmRunCancel,
    ConfirmRunCopy,
    // Terminal Reconnect & Quick Actions
    TerminalReconnect,
    TerminalCloseTab,
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
            badge_color: "#2dd4bf".to_string(),
            active_field: 0,
            error_msg: None,
        }
    }
}

pub enum AppEvent {
    SshOutput { session_id: String, data: Vec<u8> },
    SshConnected { session_id: String, handle: SshSessionHandle },
    SshDisconnected { session_id: String, error: Option<String> },
}

pub struct SessionTab {
    pub id: String,
    pub title: String,
    pub host_address: String,
    pub username: String,
    pub terminal: Terminal,
    pub handle: Option<SshSessionHandle>,
    pub current_input_buffer: String,
    pub status: String,
    pub host_id: Option<String>,
    pub host_config: Option<HostConfig>,
}

pub struct AppState {
    pub hosts_doc: HostsDocument,
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
    pub terminal_rows: usize,

    // Modals
    pub key_tool_modal: KeyToolModal,
    pub add_snippet_modal: AddSnippetModal,
    pub session_dashboard_modal: Option<SessionDashboardModal>,
    pub confirm_delete_modal: Option<ConfirmDeleteHostModal>,
    pub confirm_run_modal: Option<ConfirmRunCommandModal>,
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

    // Cached lists
    pub recent_sessions: Vec<SessionRecord>,
    pub snippets: Vec<SnippetRecord>,
    pub selected_sidebar_item: usize,

    // Async Bridge
    pub event_tx: Sender<AppEvent>,
    pub event_rx: Receiver<AppEvent>,
    pub tokio_handle: tokio::runtime::Handle,
}

impl AppState {
    pub fn new(tokio_handle: tokio::runtime::Handle) -> Self {
        let hosts_doc = load_hosts().unwrap_or_default();
        let is_app_locked = hosts_doc.settings.master_password_hash.is_some();
        let db = HistoryDb::open_default().unwrap_or_else(|_| HistoryDb::open_in_memory().unwrap());

        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        let recent_sessions = db.list_recent_sessions(30).unwrap_or_default();
        let snippets = db.list_snippets().unwrap_or_default();
        let initial_cmds = db.list_recent_commands(50).unwrap_or_default();

        Self {
            hosts_doc,
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
            terminal_rows: 30,
            key_tool_modal: KeyToolModal::default(),
            add_snippet_modal: AddSnippetModal::default(),
            session_dashboard_modal: None,
            confirm_delete_modal: None,
            confirm_run_modal: None,
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
            recent_sessions,
            snippets,
            selected_sidebar_item: 0,
            event_tx,
            event_rx,
            tokio_handle,
        }
    }

    pub fn resize_terminals(&mut self, logical_w: f64, logical_h: f64) {
        let term_w = (logical_w - self.sidebar_width).max(0.0);
        let term_h = (logical_h - self.header_height - self.footer_height).max(0.0);
        let cols = ((term_w - 24.0) / 8.2).floor().max(20.0) as usize;
        let rows = ((term_h - 20.0) / 18.0).floor().max(5.0) as usize;

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

    pub fn update_hover(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> (bool, CursorIcon) {
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

        // 0. Confirm Delete Host Modal
        if let Some(confirm) = &self.confirm_delete_modal {
            if confirm.is_open {
                let modal_w = 480.0f64.min(w - 40.0);
                let modal_h = 210.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;
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

        // 0.005 Confirm Run Command Modal
        if let Some(confirm_run) = &self.confirm_run_modal {
            if confirm_run.is_open {
                let modal_w = 540.0f64.min(w - 40.0);
                let modal_h = (if confirm_run.is_dangerous { 310.0 } else { 270.0 } as f64).min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;
                let btn_y = modal_y + modal_h - 52.0;

                if y >= btn_y && y <= btn_y + 36.0 {
                    // Cancel button
                    if x >= modal_x + modal_w - 360.0 && x <= modal_x + modal_w - 260.0 {
                        new_hover = HoverTarget::ConfirmRunCancel;
                        cursor = CursorIcon::Pointer;
                    }
                    // Copy button
                    else if x >= modal_x + modal_w - 250.0 && x <= modal_x + modal_w - 140.0 {
                        new_hover = HoverTarget::ConfirmRunCopy;
                        cursor = CursorIcon::Pointer;
                    }
                    // Execute button
                    else if x >= modal_x + modal_w - 130.0 && x <= modal_x + modal_w - 20.0 {
                        new_hover = HoverTarget::ConfirmRunExecute;
                        cursor = if confirm_run.can_execute { CursorIcon::Pointer } else { CursorIcon::NotAllowed };
                    }
                }
                let changed = old_hover != new_hover;
                self.hover = new_hover;
                return (changed, cursor);
            }
        }

        // 0.01 Settings Modal
        if let Some(settings) = &self.settings_modal {
            if settings.is_open {
                let modal_w = 620.0f64.min(w - 40.0);
                let modal_h = 490.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;

                // Close button
                if x >= modal_x + modal_w - 38.0 && x <= modal_x + modal_w - 14.0 && y >= modal_y + 14.0 && y <= modal_y + 38.0 {
                    new_hover = HoverTarget::SettingsClose;
                    cursor = CursorIcon::Pointer;
                }
                // Tabs
                else if y >= modal_y + 55.0 && y <= modal_y + 88.0 {
                    if x >= modal_x + 24.0 && x <= modal_x + 160.0 {
                        new_hover = HoverTarget::SettingsTab(SettingsTab::Security);
                        cursor = CursorIcon::Pointer;
                    } else if x >= modal_x + 165.0 && x <= modal_x + 310.0 {
                        new_hover = HoverTarget::SettingsTab(SettingsTab::Backup);
                        cursor = CursorIcon::Pointer;
                    } else if x >= modal_x + 315.0 && x <= modal_x + 450.0 {
                        new_hover = HoverTarget::SettingsTab(SettingsTab::General);
                        cursor = CursorIcon::Pointer;
                    }
                }
                // Tab content
                else {
                    match settings.active_tab {
                        SettingsTab::Security => {
                            let f_y_base = modal_y + 110.0;
                            let has_existing = self.hosts_doc.settings.master_password_hash.is_some();
                            let start_field = if has_existing { 0 } else { 1 };
                            let end_field = 3;

                            let mut cy = f_y_base;
                            for f_idx in start_field..end_field {
                                if x >= modal_x + 190.0 && x <= modal_x + modal_w - 30.0 && y >= cy && y <= cy + 32.0 {
                                    new_hover = HoverTarget::SettingsSecurityField(f_idx);
                                    cursor = CursorIcon::Text;
                                    break;
                                }
                                cy += 44.0;
                            }

                            if new_hover == HoverTarget::None {
                                let btn_y = modal_y + modal_h - 52.0;
                                if y >= btn_y && y <= btn_y + 34.0 {
                                    if has_existing {
                                        if x >= modal_x + modal_w - 320.0 && x <= modal_x + modal_w - 170.0 {
                                            new_hover = HoverTarget::SettingsRemovePassword;
                                            cursor = CursorIcon::Pointer;
                                        } else if x >= modal_x + modal_w - 160.0 && x <= modal_x + modal_w - 24.0 {
                                            new_hover = HoverTarget::SettingsSavePassword;
                                            cursor = CursorIcon::Pointer;
                                        }
                                    } else {
                                        if x >= modal_x + modal_w - 180.0 && x <= modal_x + modal_w - 24.0 {
                                            new_hover = HoverTarget::SettingsSavePassword;
                                            cursor = CursorIcon::Pointer;
                                        }
                                    }
                                }
                            }
                        }
                        SettingsTab::Backup => {
                            let exp_y = modal_y + 130.0;
                            let imp_y = modal_y + 240.0;
                            if x >= modal_x + 30.0 && x <= modal_x + 230.0 && y >= exp_y && y <= exp_y + 36.0 {
                                new_hover = HoverTarget::SettingsExportJson;
                                cursor = CursorIcon::Pointer;
                            } else if x >= modal_x + 30.0 && x <= modal_x + 230.0 && y >= imp_y && y <= imp_y + 36.0 {
                                new_hover = HoverTarget::SettingsImportJson;
                                cursor = CursorIcon::Pointer;
                            }
                        }
                        SettingsTab::General => {
                            let mut cy = modal_y + 110.0;
                            for f_idx in 0..2 {
                                if x >= modal_x + 240.0 && x <= modal_x + modal_w - 30.0 && y >= cy && y <= cy + 32.0 {
                                    new_hover = HoverTarget::SettingsGeneralField(f_idx);
                                    cursor = CursorIcon::Text;
                                    break;
                                }
                                cy += 48.0;
                            }
                            if new_hover == HoverTarget::None {
                                // Checkbox confirm delete
                                if x >= modal_x + 30.0 && x <= modal_x + 360.0 && y >= cy && y <= cy + 28.0 {
                                    new_hover = HoverTarget::SettingsToggleConfirmDelete;
                                    cursor = CursorIcon::Pointer;
                                }
                                let btn_y = modal_y + modal_h - 52.0;
                                if x >= modal_x + modal_w - 190.0 && x <= modal_x + modal_w - 24.0 && y >= btn_y && y <= btn_y + 34.0 {
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

                if x >= modal_x && x <= modal_x + modal_w && y >= modal_y && y <= modal_y + modal_h {
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
                                } else if x >= modal_x + modal_w - 96.0 && x < modal_x + modal_w - 60.0 {
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
                if x >= modal_x + modal_w - 40.0 && y <= modal_y + 40.0 {
                    new_hover = HoverTarget::ModalSnippetClose;
                    cursor = CursorIcon::Pointer;
                } else {
                    let mut f_y = modal_y + 60.0;
                    for idx in 0..4 {
                        if x >= modal_x + 24.0 && x <= modal_x + modal_w - 24.0 && y >= f_y && y <= f_y + 34.0 {
                            new_hover = HoverTarget::ModalSnippetField(idx);
                            cursor = CursorIcon::Text;
                            break;
                        }
                        f_y += 62.0;
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
                        if x >= modal_x + 160.0 && x <= modal_x + modal_w - 24.0 && y >= f_y && y <= f_y + 32.0 {
                            new_hover = HoverTarget::KeyToolField(idx);
                            cursor = CursorIcon::Text;
                        }
                        f_y += 38.0;
                    }

                    let gen_y = modal_y + 210.0;
                    if x >= modal_x + 160.0 && x <= modal_x + modal_w - 24.0 && y >= gen_y && y <= gen_y + 34.0 {
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
                // Close button
                if x >= modal_x + modal_w - 40.0 && y <= modal_y + 40.0 {
                    new_hover = HoverTarget::ModalClose;
                    cursor = CursorIcon::Pointer;
                } else {
                    // Fields
                    let mut f_y = modal_y + 60.0;
                    for idx in 0..5 {
                        if x >= modal_x + 160.0 && x <= modal_x + modal_w - 24.0 && y >= f_y && y <= f_y + 32.0 {
                            new_hover = HoverTarget::ModalField(idx);
                            cursor = CursorIcon::Text;
                        }
                        f_y += 38.0;
                    }

                    // Auth buttons
                    let mut btn_x = modal_x + 160.0;
                    let modes = [AuthModeChoice::Agent, AuthModeChoice::Key, AuthModeChoice::Password];
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
                            if x >= modal_x + 160.0 && x <= modal_x + 160.0 + input_w && y >= f_y && y <= f_y + 32.0 {
                                new_hover = HoverTarget::ModalField(5);
                                cursor = CursorIcon::Text;
                            } else if x >= modal_x + modal_w - 170.0 && x <= modal_x + modal_w - 96.0 && y >= f_y && y <= f_y + 32.0 {
                                new_hover = HoverTarget::ModalBrowseKey;
                                cursor = CursorIcon::Pointer;
                            } else if x >= modal_x + modal_w - 90.0 && x <= modal_x + modal_w - 24.0 && y >= f_y && y <= f_y + 32.0 {
                                new_hover = HoverTarget::ModalGenerateKey;
                                cursor = CursorIcon::Pointer;
                            }
                        } else {
                            if x >= modal_x + 160.0 && x <= modal_x + modal_w - 24.0 && y >= f_y && y <= f_y + 32.0 {
                                new_hover = HoverTarget::ModalField(5);
                                cursor = CursorIcon::Text;
                            }
                        }
                    }

                    // Save / Connect buttons
                    let save_btn_x = modal_x + modal_w - 280.0;
                    let btn_y = modal_y + modal_h - 48.0;
                    if x >= save_btn_x && x <= save_btn_x + 120.0 && y >= btn_y && y <= btn_y + 34.0 {
                        new_hover = HoverTarget::ModalSave;
                        cursor = CursorIcon::Pointer;
                    }
                    let conn_btn_x = modal_x + modal_w - 146.0;
                    if x >= conn_btn_x && x <= conn_btn_x + 126.0 && y >= btn_y && y <= btn_y + 34.0 {
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
            } else if x >= w - 46.0 - 160.0 - 8.0 && x < w - 46.0 {
                new_hover = HoverTarget::HeaderSearchBtn;
                cursor = CursorIcon::Pointer;
            } else {
                let mut tab_x = 160.0;
                for (i, _) in self.tabs.iter().enumerate() {
                    let tab_w = 175.0;
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

                let add_btn_w = 125.0;
                if x >= tab_x && x <= tab_x + add_btn_w {
                    new_hover = HoverTarget::HeaderAddBtn;
                    cursor = CursorIcon::Pointer;
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
                                            } else if x >= sidebar_w - 62.0 && x < sidebar_w - 34.0 {
                                                new_hover = HoverTarget::HostEdit(real_idx);
                                            } else if x >= sidebar_w - 90.0 && x < sidebar_w - 62.0 {
                                                new_hover = HoverTarget::HostDeployKey(real_idx);
                                            } else if x >= sidebar_w - 118.0 && x < sidebar_w - 90.0 {
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
                                            } else if x >= sidebar_w - 62.0 && x < sidebar_w - 34.0 {
                                                new_hover = HoverTarget::SnippetCopy(item_idx);
                                            } else if x >= sidebar_w - 90.0 && x < sidebar_w - 62.0 {
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

            if x >= center_x + 90.0 && x <= center_x + 250.0 && y >= title_y + 36.0 && y <= title_y + 72.0 {
                new_hover = HoverTarget::WelcomeAdd;
                cursor = CursorIcon::Pointer;
            } else if x >= center_x + 260.0 && x <= center_x + 410.0 && y >= title_y + 36.0 && y <= title_y + 72.0 {
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

            // 5.2 Split View Close button and Panel Switch
            if let Some(_s_tab) = self.split_tab {
                let split_x = sidebar_w + (w - sidebar_w) / 2.0;
                if (x - split_x).abs() <= 16.0 && y >= header_h + 4.0 && y <= header_h + 30.0 {
                    new_hover = HoverTarget::TerminalSplitClose;
                    cursor = CursorIcon::Pointer;
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                } else if x > split_x && y >= header_h && y <= h - self.footer_height {
                    new_hover = HoverTarget::SplitTerminalArea;
                    cursor = CursorIcon::Text;
                    let changed = old_hover != new_hover;
                    self.hover = new_hover;
                    return (changed, cursor);
                }
            }

            // 5.3 Quick Actions Toolbar or Reconnect Banner
            if let Some(tab) = self.tabs.get(self.active_tab) {
                if tab.handle.is_some() {
                    let bar_y = header_h + 3.0;
                    let bar_h = 28.0;
                    if y >= bar_y && y <= bar_y + bar_h {
                        let mut px = sidebar_w + 12.0;
                        let pills = [
                            (QuickActionCmd::Htop, 78.0),
                            (QuickActionCmd::Df, 74.0),
                            (QuickActionCmd::Free, 80.0),
                            (QuickActionCmd::Uptime, 82.0),
                            (QuickActionCmd::Docker, 82.0),
                            (QuickActionCmd::Ports, 92.0),
                        ];
                        for (cmd, pw) in pills {
                            if x >= px && x <= px + pw {
                                new_hover = HoverTarget::QuickAction(cmd);
                                cursor = CursorIcon::Pointer;
                                break;
                            }
                            px += pw + 6.0;
                        }

                        // Right-aligned toolbar buttons: Search & Split
                        if new_hover == HoverTarget::None {
                            if x >= w - 185.0 && x <= w - 95.0 {
                                new_hover = HoverTarget::TerminalSearchOpenBtn;
                                cursor = CursorIcon::Pointer;
                            } else if x >= w - 90.0 && x <= w - 16.0 {
                                new_hover = HoverTarget::TerminalSplitBtn;
                                cursor = CursorIcon::Pointer;
                            }
                        }
                    }
                } else {
                    // Reconnect Banner at bottom of disconnected terminal
                    let banner_y = h - self.footer_height - 46.0;
                    let banner_h = 40.0;
                    if y >= banner_y && y <= banner_y + banner_h {
                        if x >= w - 370.0 && x <= w - 130.0 {
                            new_hover = HoverTarget::TerminalReconnect;
                            cursor = CursorIcon::Pointer;
                        } else if x >= w - 120.0 && x <= w - 20.0 {
                            new_hover = HoverTarget::TerminalCloseTab;
                            cursor = CursorIcon::Pointer;
                        }
                    }
                }
            }

            // 5.4 Scrollbar or Terminal Area
            if new_hover == HoverTarget::None {
                let sb_w = 8.0;
                let sb_margin_right = 4.0;
                let sb_x = w - sb_w - sb_margin_right;
                let sb_y = header_h + 6.0;
                let sb_h = (h - header_h - self.footer_height - 12.0).max(20.0);

                if let Some(tab) = self.tabs.get(self.active_tab) {
                    let scrollback_len = tab.terminal.scrollback_len();
                    let scroll_offset = tab.terminal.scroll_offset();
                    let rows = self.terminal_rows;

                    let (thumb_y, thumb_h) = if scrollback_len > 0 {
                        let total_lines = scrollback_len + rows;
                        let visible_ratio = (rows as f64) / (total_lines as f64);
                        let th = (sb_h * visible_ratio).clamp(24.0, sb_h);
                        let max_scroll_offset = scrollback_len as f64;
                        let scroll_ratio = (scroll_offset as f64) / max_scroll_offset;
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
                    } else if x >= sidebar_w && y >= header_h && y <= h - self.footer_height {
                        new_hover = HoverTarget::TerminalArea;
                        cursor = CursorIcon::Text;
                    }
                }
            }
        }

        let changed = old_hover != new_hover;
        self.hover = new_hover;

        if changed {
            if new_hover != HoverTarget::None {
                if let Some(text) = Self::get_hover_tooltip_text(new_hover) {
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
        let pad_x = self.sidebar_width + 12.0;
        let pad_y = self.header_height + 10.0;
        let char_w = 8.2;
        let line_h = 18.0;

        if x < pad_x || y < pad_y {
            return None;
        }

        let col = ((x - pad_x) / char_w).floor().max(0.0) as usize;
        let row = ((y - pad_y) / line_h).floor().max(0.0) as usize;

        if row < self.terminal_rows {
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
        let text = tab.terminal.get_text_range((p1.col, p1.row), (p2.col, p2.row));
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
            || self.add_host_modal.is_open
            || self.key_tool_modal.is_open
            || self.add_snippet_modal.is_open
            || self.session_dashboard_modal.as_ref().map_or(false, |m| m.is_open)
            || self.confirm_delete_modal.is_some()
            || self.confirm_run_modal.is_some()
            || self.settings_modal.as_ref().map_or(false, |m| m.is_open)
            || self.palette_open
            || self.search_state.is_open
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
            return Some(InputFieldId::AddSnippet(self.add_snippet_modal.active_field));
        }
        if let Some(m) = &self.settings_modal {
            if m.is_open {
                return match m.active_tab {
                    SettingsTab::General => Some(InputFieldId::SettingsGeneral(m.active_general_field)),
                    SettingsTab::Security => Some(InputFieldId::SettingsSecurity(m.active_security_field)),
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
                        }
                        if fid == InputFieldId::CommandSearch {
                            self.refresh_command_search();
                        } else if fid == InputFieldId::SnippetSearch {
                            self.refresh_snippets();
                        } else if fid == InputFieldId::TerminalSearch {
                            self.update_terminal_search();
                        }
                    } else {
                        self.send_to_active_terminal(&text);
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
        while let Ok(event) = self.event_rx.try_recv() {
            updated = true;
            match event {
                AppEvent::SshOutput { session_id, data } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == session_id) {
                        tab.terminal.write_bytes(&data);
                    }
                }
                AppEvent::SshConnected { session_id, handle } => {
                    let c = self.terminal_cols as u32;
                    let r = self.terminal_rows as u32;
                    let r_tx = handle.resize_tx.clone();
                    self.tokio_handle.spawn(async move {
                        let _ = r_tx.send((c, r)).await;
                    });
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == session_id) {
                        tab.status = "🟢 Connected".into();
                        tab.handle = Some(handle);
                    }
                    self.refresh_recent_sessions();
                }
                AppEvent::SshDisconnected { session_id, error } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == session_id) {
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
                                tab.terminal.write_bytes("\r\n\x1b[38;2;148;163;184m[Сессия завершена]\x1b[0m\r\n".as_bytes());
                            }
                        }
                        let _ = self.db.record_session_end(&session_id, "closed", error.as_deref());
                    }
                    self.refresh_recent_sessions();
                }
            }
        }
        updated
    }

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
                AuthConfig::Agent => (AuthModeChoice::Agent, "~/.ssh/id_ed25519".into(), String::new()),
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

        if self.add_host_modal.is_editing {
            if let Some(idx) = self.hosts_doc.hosts.iter().position(|h| h.id == id) {
                self.hosts_doc.hosts[idx] = new_host.clone();
            }
        } else {
            self.hosts_doc.hosts.push(new_host.clone());
        }

        let _ = save_hosts(&self.hosts_doc);
        self.add_host_modal.is_open = false;

        if connect_immediately {
            self.connect_to_host(new_host);
        }

        true
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
            let is_connected = tab.handle.as_ref().map_or(false, |h| *h.is_connected.read());
            if is_connected {
                (format!("{} ({})", tab.title, tab.host_address), true)
            } else {
                (format!("{} (отключен)", tab.title), false)
            }
        } else {
            ("Нет активных сессий терминала".into(), false)
        };

        self.confirm_run_modal = Some(ConfirmRunCommandModal {
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
        if let Some(m) = self.confirm_run_modal.take() {
            if m.can_execute {
                let to_send = format!("{}\r", m.command);
                self.send_to_active_terminal(&to_send);
            }
        }
    }

    pub fn delete_host(&mut self, host_idx: usize) {
        if host_idx < self.hosts_doc.hosts.len() {
            self.hosts_doc.hosts.remove(host_idx);
            let _ = save_hosts(&self.hosts_doc);
        }
    }

    pub fn open_settings_modal(&mut self) {
        let default_user = self.hosts_doc.settings.default_user.clone();
        let scrollback = self.hosts_doc.settings.scrollback_lines.to_string();
        let confirm_delete = self.hosts_doc.settings.confirm_delete;
        self.settings_modal = Some(SettingsModal {
            is_open: true,
            active_tab: SettingsTab::Security,
            current_password: TextInput::default(),
            new_password: TextInput::default(),
            confirm_password: TextInput::default(),
            active_security_field: 0,
            security_msg: None,
            backup_msg: None,
            default_user: TextInput::new(default_user),
            scrollback_lines: TextInput::new(scrollback),
            confirm_delete,
            active_general_field: 0,
            general_msg: None,
        });
        self.focused_input = Some(InputFieldId::SettingsSecurity(0));
    }

    pub fn save_master_password(&mut self) {
        let Some(modal) = &mut self.settings_modal else { return; };
        let has_existing = self.hosts_doc.settings.master_password_hash.is_some();
        if has_existing {
            let current = modal.current_password.text.trim();
            if let Some(existing_hash) = &self.hosts_doc.settings.master_password_hash {
                if !verify_password(current, existing_hash) {
                    modal.security_msg = Some(("Текущий пароль введен неверно".to_string(), true));
                    return;
                }
            }
        }

        let new_pass = modal.new_password.text.trim();
        let confirm_pass = modal.confirm_password.text.trim();
        if new_pass.is_empty() {
            modal.security_msg = Some(("Новый пароль не может быть пустым".to_string(), true));
            return;
        }
        if new_pass != confirm_pass {
            modal.security_msg = Some(("Пароли не совпадают".to_string(), true));
            return;
        }
        if new_pass.len() < 4 {
            modal.security_msg = Some(("Пароль должен содержать минимум 4 символа".to_string(), true));
            return;
        }

        self.hosts_doc.settings.master_password_hash = Some(hash_password(new_pass));
        let _ = save_hosts(&self.hosts_doc);
        modal.current_password = TextInput::default();
        modal.new_password = TextInput::default();
        modal.confirm_password = TextInput::default();
        modal.security_msg = Some(("Мастер-пароль успешно сохранен!".to_string(), false));
    }

    pub fn remove_master_password(&mut self) {
        let Some(modal) = &mut self.settings_modal else { return; };
        if let Some(existing_hash) = &self.hosts_doc.settings.master_password_hash {
            let current = modal.current_password.text.trim();
            if !verify_password(current, existing_hash) {
                modal.security_msg = Some(("Для удаления пароля введите верный текущий пароль".to_string(), true));
                return;
            }
        }
        self.hosts_doc.settings.master_password_hash = None;
        let _ = save_hosts(&self.hosts_doc);
        modal.current_password = TextInput::default();
        modal.new_password = TextInput::default();
        modal.confirm_password = TextInput::default();
        modal.security_msg = Some(("Мастер-пароль успешно удален".to_string(), false));
    }

    pub fn try_unlock_app(&mut self) -> bool {
        if let Some(hash) = &self.hosts_doc.settings.master_password_hash {
            let entered = self.lock_password_input.text.trim();
            if verify_password(entered, hash) {
                self.is_app_locked = false;
                self.lock_password_input = TextInput::default();
                self.lock_error_msg = None;
                return true;
            } else {
                self.lock_error_msg = Some("Неверный мастер-пароль. Попробуйте еще раз.".to_string());
                return false;
            }
        }
        self.is_app_locked = false;
        true
    }

    pub fn export_backup_dialog(&mut self) {
        let bundle = ExportBundle {
            version: 1,
            exported_at: chrono::Local::now().to_rfc3339(),
            hosts: self.hosts_doc.hosts.clone(),
            settings: self.hosts_doc.settings.clone(),
        };

        if let Some(path) = rfd::FileDialog::new()
            .set_title("Экспорт конфигурации EvertySSH")
            .set_file_name("evertyssh_backup.json")
            .add_filter("EvertySSH Backup (*.json)", &["json"])
            .save_file()
        {
            match serde_json::to_string_pretty(&bundle) {
                Ok(json_str) => match std::fs::write(&path, json_str) {
                    Ok(_) => {
                        if let Some(m) = &mut self.settings_modal {
                            m.backup_msg = Some((format!("Успешно экспортировано {} серверов", bundle.hosts.len()), false));
                        }
                    }
                    Err(e) => {
                        if let Some(m) = &mut self.settings_modal {
                            m.backup_msg = Some((format!("Ошибка записи: {e}"), true));
                        }
                    }
                },
                Err(e) => {
                    if let Some(m) = &mut self.settings_modal {
                        m.backup_msg = Some((format!("Ошибка сериализации: {e}"), true));
                    }
                }
            }
        }
    }

    pub fn import_backup_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Импорт конфигурации EvertySSH")
            .add_filter("EvertySSH Backup (*.json)", &["json"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<ExportBundle>(&content) {
                    Ok(bundle) => {
                        let count = bundle.hosts.len();
                        for h in bundle.hosts {
                            if !self.hosts_doc.hosts.iter().any(|existing| existing.id == h.id) {
                                self.hosts_doc.hosts.push(h);
                            }
                        }
                        let _ = save_hosts(&self.hosts_doc);
                        if let Some(m) = &mut self.settings_modal {
                            m.backup_msg = Some((format!("Импортировано и объединено {} серверов", count), false));
                        }
                    }
                    Err(e) => {
                        if let Some(m) = &mut self.settings_modal {
                            m.backup_msg = Some((format!("Неверный формат резервной копии: {e}"), true));
                        }
                    }
                },
                Err(e) => {
                    if let Some(m) = &mut self.settings_modal {
                        m.backup_msg = Some((format!("Не удалось прочитать файл: {e}"), true));
                    }
                }
            }
        }
    }

    pub fn save_general_settings(&mut self) {
        let Some(modal) = &mut self.settings_modal else { return; };
        let def_u = modal.default_user.text.trim();
        if !def_u.is_empty() {
            self.hosts_doc.settings.default_user = def_u.to_string();
        }
        if let Ok(sb) = modal.scrollback_lines.text.trim().parse::<usize>() {
            if sb >= 100 && sb <= 100_000 {
                self.hosts_doc.settings.scrollback_lines = sb;
            }
        }
        self.hosts_doc.settings.confirm_delete = modal.confirm_delete;
        let _ = save_hosts(&self.hosts_doc);
        modal.general_msg = Some(("Параметры успешно сохранены!".to_string(), false));
    }

    pub fn refresh_recent_sessions(&mut self) {
        if let Ok(sessions) = self.db.list_recent_sessions(30) {
            self.recent_sessions = sessions;
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

    pub fn connect_to_host(&mut self, host: HostConfig) {
        let session_id = uuid::Uuid::new_v4().to_string();
        let title = host.name.clone();
        let host_address = format!("{}:{}", host.hostname, host.port);
        let username = host.username.clone();

        let _ = self.db.record_session_start(
            &session_id,
            Some(&host.id),
            &host_address,
            &username,
        );

        let mut terminal = Terminal::new(self.terminal_cols, self.terminal_rows, 5000);
        let welcome = format!(
            "\x1b[38;2;45;212;191m⚡ EvertySSH v0.1.0\x1b[0m\r\nConnecting to \x1b[38;2;52;211;153m{}@{}\x1b[0m...\r\n",
            username, host_address
        );
        terminal.write_bytes(welcome.as_bytes());

        let tab = SessionTab {
            id: session_id.clone(),
            title,
            host_address: host_address.clone(),
            username,
            terminal,
            handle: None,
            current_input_buffer: String::new(),
            status: "🟡 Connecting...".into(),
            host_id: Some(host.id.clone()),
            host_config: Some(host.clone()),
        };

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;

        let _s_id = session_id.clone();
        let event_tx = self.event_tx.clone();
        let cols = self.terminal_cols as u32;
        let rows = self.terminal_rows as u32;

        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

        let s_id_bridge = session_id.clone();
        let event_tx_bridge = event_tx.clone();
        self.tokio_handle.spawn(async move {
            while let Some(bytes) = out_rx.recv().await {
                if event_tx_bridge
                    .send(AppEvent::SshOutput {
                        session_id: s_id_bridge.clone(),
                        data: bytes,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let s_id_task = session_id.clone();
        self.tokio_handle.spawn(async move {
            match connect_ssh(host, s_id_task.clone(), cols, rows, out_tx).await {
                Ok(handle) => {
                    let is_conn = handle.is_connected.clone();
                    let s_id_mon = s_id_task.clone();
                    let event_tx_mon = event_tx.clone();

                    let _ = event_tx.send(AppEvent::SshConnected {
                        session_id: s_id_task.clone(),
                        handle,
                    });

                    tokio::spawn(async move {
                        while *is_conn.read() {
                            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        }
                        let _ = event_tx_mon.send(AppEvent::SshDisconnected {
                            session_id: s_id_mon,
                            error: None,
                        });
                    });
                }
                Err(err) => {
                    let _ = event_tx.send(AppEvent::SshDisconnected {
                        session_id: s_id_task,
                        error: Some(err),
                    });
                }
            }
        });

        self.refresh_recent_sessions();
    }

    pub fn reconnect_tab(&mut self, tab_idx: usize) {
        if tab_idx >= self.tabs.len() {
            return;
        }
        let (host_cfg, host_id, host_address, username) = {
            let tab = &mut self.tabs[tab_idx];
            let cfg = tab.host_config.clone().or_else(|| {
                tab.host_id.as_ref().and_then(|hid| self.hosts_doc.hosts.iter().find(|h| &h.id == hid).cloned())
            });
            (cfg, tab.host_id.clone(), tab.host_address.clone(), tab.username.clone())
        };

        if let Some(host) = host_cfg {
            let new_session_id = uuid::Uuid::new_v4().to_string();
            let tab = &mut self.tabs[tab_idx];
            tab.id = new_session_id.clone();
            tab.status = "🟡 Подключение...".into();
            tab.handle = None;
            tab.host_config = Some(host.clone());

            let _ = self.db.record_session_start(
                &new_session_id,
                host_id.as_deref().or(Some(&host.id)),
                &host_address,
                &username,
            );

            let msg = format!(
                "\r\n\x1b[38;2;45;212;191m🔄 Повторное подключение к {}@{}...\x1b[0m\r\n",
                username, host_address
            );
            tab.terminal.write_bytes(msg.as_bytes());

            let event_tx = self.event_tx.clone();
            let cols = self.terminal_cols as u32;
            let rows = self.terminal_rows as u32;

            let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

            let s_id_bridge = new_session_id.clone();
            let event_tx_bridge = event_tx.clone();
            self.tokio_handle.spawn(async move {
                while let Some(bytes) = out_rx.recv().await {
                    if event_tx_bridge
                        .send(AppEvent::SshOutput {
                            session_id: s_id_bridge.clone(),
                            data: bytes,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });

            let s_id_task = new_session_id.clone();
            self.tokio_handle.spawn(async move {
                match connect_ssh(host, s_id_task.clone(), cols, rows, out_tx).await {
                    Ok(handle) => {
                        let is_conn = handle.is_connected.clone();
                        let s_id_mon = s_id_task.clone();
                        let event_tx_mon = event_tx.clone();

                        let _ = event_tx.send(AppEvent::SshConnected {
                            session_id: s_id_task.clone(),
                            handle,
                        });

                        tokio::spawn(async move {
                            while *is_conn.read() {
                                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                            }
                            let _ = event_tx_mon.send(AppEvent::SshDisconnected {
                                session_id: s_id_mon,
                                error: None,
                            });
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::SshDisconnected {
                            session_id: s_id_task,
                            error: Some(err),
                        });
                    }
                }
            });

            self.refresh_recent_sessions();
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
    }

    pub fn close_split_view(&mut self) {
        self.split_tab = None;
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
            let matches = tab.terminal.search(&query_text, self.search_state.case_sensitive);
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

        // 3. Regular Terminal Input
        let mut recorded_cmd = None;
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            for ch in text.chars() {
                if ch == '\r' || ch == '\n' {
                    let cmd = tab.current_input_buffer.trim().to_string();
                    if !cmd.is_empty() {
                        recorded_cmd = Some((tab.id.clone(), tab.host_address.clone(), cmd));
                    }
                    tab.current_input_buffer.clear();
                } else if ch == '\x08' {
                    tab.current_input_buffer.pop();
                } else if !ch.is_control() {
                    tab.current_input_buffer.push(ch);
                }
            }

            if let Some(handle) = &tab.handle {
                let data = text.as_bytes().to_vec();
                let handle_clone = handle.input_tx.clone();
                self.tokio_handle.spawn(async move {
                    let _ = handle_clone.send(data).await;
                });
            } else {
                tab.terminal.write_bytes(text.as_bytes());
            }
        }

        if let Some((tab_id, host_addr, cmd)) = recorded_cmd {
            let _ = self.db.record_command(
                Some(&tab_id),
                &host_addr,
                &cmd,
            );
            self.refresh_command_search();
        }
    }

    pub fn execute_selected_palette_command(&mut self) {
        if let Some(record) = self.command_search_results.get(self.selected_search_idx) {
            let cmd = format!("{}\r", record.command);
            self.send_to_active_terminal(&cmd);
        }
    }

    pub fn send_to_active_terminal(&mut self, text: &str) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            if let Some(handle) = &tab.handle {
                let data = text.as_bytes().to_vec();
                let handle_clone = handle.input_tx.clone();
                self.tokio_handle.spawn(async move {
                    let _ = handle_clone.send(data).await;
                });
            } else {
                tab.terminal.write_bytes(text.as_bytes());
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
        let (suggested_name, suggested_comment) = if let Some(h) = self.hosts_doc.hosts.get(host_idx) {
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
        if let (Some(host_idx), Some(priv_path)) = (
            self.key_tool_modal.target_host_idx,
            &self.key_tool_modal.generated_priv_path,
        ) {
            let host_name = if let Some(host) = self.hosts_doc.hosts.get_mut(host_idx) {
                host.auth = AuthConfig::Key {
                    path: PathBuf::from(priv_path),
                    passphrase: if self.key_tool_modal.passphrase.text.is_empty() {
                        None
                    } else {
                        Some(self.key_tool_modal.passphrase.text.clone())
                    },
                };
                Some(host.name.clone())
            } else {
                None
            };

            if let Some(name) = host_name {
                let _ = save_hosts(&self.hosts_doc);
                self.key_tool_modal.status_msg =
                    Some(format!("✓ Ключ привязан к серверу '{name}'"));
                return true;
            }
        }
        false
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
            let _ = self.db.record_session_end(&removed.id, "closed", None);
            if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                self.active_tab = self.tabs.len() - 1;
            }
            self.refresh_recent_sessions();
        }
    }

    pub fn get_hover_tooltip_text(target: HoverTarget) -> Option<&'static str> {
        match target {
            HoverTarget::HeaderAddBtn => Some("Добавить новый хост (Ctrl+N)"),
            HoverTarget::HeaderSearchBtn => Some("Поиск по истории команд (Ctrl+R)"),
            HoverTarget::SidebarSplitter => Some("Потяните мышью для изменения ширины панели"),
            HoverTarget::SidebarAddHostBtn => Some("Добавить новый сервер"),
            HoverTarget::SidebarAddSnippetBtn => Some("Создать новый сниппет"),
            HoverTarget::HostDeployKey(_) => Some("Сгенерировать и установить SSH ключ на сервер"),
            HoverTarget::HostConnect(_) => Some("Подключиться к серверу"),
            HoverTarget::HostEdit(_) => Some("Редактировать параметры сервера"),
            HoverTarget::HostDelete(_) => Some("Удалить сервер из списка"),
            HoverTarget::SessionCard(_) => Some("Открыть подробный дашборд сессии и журнал команд"),
            HoverTarget::SessionDashboardReconnect => Some("Переподключиться к серверу в новой вкладке"),
            HoverTarget::SessionDashboardDelete => Some("Удалить эту сессию из истории"),
            HoverTarget::SessionDashboardRunCmd(_) => Some("Запустить команду в активной консоли"),
            HoverTarget::SessionDashboardCopyCmd(_) => Some("Скопировать команду в буфер"),
            HoverTarget::SnippetRun(_) => Some("Запустить сниппет в активном терминале"),
            HoverTarget::SnippetCopy(_) => Some("Скопировать текст сниппета"),
            HoverTarget::SnippetDelete(_) => Some("Удалить сниппет"),
            HoverTarget::KeyToolGenerateBtn => Some("Сгенерировать ключевую пару OpenSSH"),
            HoverTarget::KeyToolSendTermBtn => Some("Отправить ключ и настроить authorized_keys"),
            HoverTarget::KeyToolBindHostBtn => Some("Привязать ключ к профилю сервера"),
            HoverTarget::KeyToolCopyPubBtn => Some("Скопировать публичный ключ"),
            HoverTarget::ModalBrowseKey => Some("Выбрать файл ключа через проводник"),
            HoverTarget::ModalGenerateKey => Some("Сгенерировать новый SSH ключ"),
            HoverTarget::SidebarSearchHost => Some("Поиск серверов по имени, адресу, логину или группе (Esc для сброса)"),
            HoverTarget::SidebarSearchSnippet => Some("Поиск сохраненных сниппетов (Esc для сброса)"),
            HoverTarget::SidebarSearchCommand => Some("Поиск в истории команд"),
            HoverTarget::HeaderSettingsBtn => Some("Настройки: мастер-пароль, резервная копия, параметры"),
            HoverTarget::ConfirmDeleteConfirm => Some("Подтвердить безвозвратное удаление сервера"),
            HoverTarget::ConfirmDeleteCancel => Some("Отменить удаление сервера (Esc)"),
            HoverTarget::SettingsClose => Some("Закрыть настройки (Esc)"),
            HoverTarget::SettingsSavePassword => Some("Сохранить настройки мастер-пароля"),
            HoverTarget::SettingsRemovePassword => Some("Отключить защиту паролем"),
            HoverTarget::SettingsExportJson => Some("Экспортировать серверы и настройки в JSON файл"),
            HoverTarget::SettingsImportJson => Some("Импортировать серверы из файла резервной копии"),
            HoverTarget::SettingsSaveGeneral => Some("Сохранить общие настройки"),
            HoverTarget::SettingsToggleConfirmDelete => Some("Включить/выключить подтверждение удаления"),
            HoverTarget::LockUnlockBtn => Some("Разблокировать приложение (Enter)"),
            HoverTarget::ConfirmRunExecute => Some("Выполнить команду в консоли (Enter)"),
            HoverTarget::ConfirmRunCancel => Some("Отменить запуск команды (Esc)"),
            HoverTarget::ConfirmRunCopy => Some("Скопировать текст команды в буфер"),
            HoverTarget::TerminalReconnect => Some("Повторно подключиться к серверу в этой вкладке"),
            HoverTarget::TerminalCloseTab => Some("Закрыть эту вкладку терминала"),
            HoverTarget::QuickAction(cmd) => Some(cmd.tooltip()),
            HoverTarget::TerminalSearchOpenBtn => Some("Поиск текста в консоли (Ctrl+F)"),
            HoverTarget::TerminalSearchCloseBtn => Some("Закрыть поиск (Esc)"),
            HoverTarget::TerminalSearchPrevBtn => Some("Предыдущее совпадение (Shift+Enter)"),
            HoverTarget::TerminalSearchNextBtn => Some("Следующее совпадение (Enter)"),
            HoverTarget::TerminalSearchCaseToggle => Some("Учитывать регистр символов (Aa)"),
            HoverTarget::TerminalSearchInput => Some("Поле поиска текста в терминале"),
            HoverTarget::TerminalSplitBtn => Some("Разделить экран терминала (Ctrl+Alt+S)"),
            HoverTarget::TerminalSplitClose => Some("Закрыть разделение терминала"),
            HoverTarget::SplitTerminalArea => Some("Активировать этот терминал"),
            _ => None,
        }
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
            InputFieldId::SettingsSecurity(0) => self.settings_modal.as_mut().map(|m| &mut m.current_password),
            InputFieldId::SettingsSecurity(1) => self.settings_modal.as_mut().map(|m| &mut m.new_password),
            InputFieldId::SettingsSecurity(2) => self.settings_modal.as_mut().map(|m| &mut m.confirm_password),
            InputFieldId::SettingsSecurity(_) => None,
            InputFieldId::SettingsGeneral(0) => self.settings_modal.as_mut().map(|m| &mut m.default_user),
            InputFieldId::SettingsGeneral(1) => self.settings_modal.as_mut().map(|m| &mut m.scrollback_lines),
            InputFieldId::SettingsGeneral(_) => None,
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
            InputFieldId::SettingsSecurity(0) => self.settings_modal.as_ref().map(|m| &m.current_password),
            InputFieldId::SettingsSecurity(1) => self.settings_modal.as_ref().map(|m| &m.new_password),
            InputFieldId::SettingsSecurity(2) => self.settings_modal.as_ref().map(|m| &m.confirm_password),
            InputFieldId::SettingsSecurity(_) => None,
            InputFieldId::SettingsGeneral(0) => self.settings_modal.as_ref().map(|m| &m.default_user),
            InputFieldId::SettingsGeneral(1) => self.settings_modal.as_ref().map(|m| &m.scrollback_lines),
            InputFieldId::SettingsGeneral(_) => None,
            InputFieldId::LockPassword => Some(&self.lock_password_input),
            InputFieldId::TerminalSearch => Some(&self.search_state.query),
        }
    }

    pub fn open_input_context_menu(&mut self, x: f64, y: f64, field_id: InputFieldId, win_w: f64, win_h: f64) {
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

    pub fn open_session_dashboard(&mut self, sess_idx: usize) {
        if let Some(sess) = self.recent_sessions.get(sess_idx) {
            let commands = self.db.list_session_commands(&sess.id).unwrap_or_default();
            let host = sess.host_id.as_ref().and_then(|hid| {
                self.hosts_doc.hosts.iter().find(|h| &h.id == hid).cloned()
            });
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

    pub fn open_add_snippet_dialog(&mut self) {
        self.add_snippet_modal = AddSnippetModal {
            is_open: true,
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

        let cat_opt = if category.is_empty() { None } else { Some(category.as_str()) };
        let desc_opt = if description.is_empty() { None } else { Some(description.as_str()) };

        if let Ok(_) = self.db.add_snippet(&title, &command, cat_opt, desc_opt) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(percent_encode("ssh-keygen -t ed25519"), "ssh-keygen+-t+ed25519");
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
        assert!(sel.contains(0, 2));  // entire middle line
        assert!(sel.contains(50, 2)); // entire middle line
        assert!(sel.contains(0, 3));  // start of last line
        assert!(sel.contains(10, 3)); // up to col 10
        assert!(!sel.contains(11, 3)); // beyond col 10 on last line
        assert!(!sel.contains(5, 4));  // next line
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
}
