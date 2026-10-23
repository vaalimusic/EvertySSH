#![windows_subsystem = "windows"]

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use everty_ui::{DpiScale, PhysicalSize};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize as WinitPhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

pub mod app;
pub mod config;
pub mod db;
pub mod i18n;
pub mod ssh;
pub mod terminal;
pub mod ui;

use app::{
    open_google_search, AppState, AuthModeChoice, ContextMenuItem, HoverTarget, InputFieldId,
    KeyAlgo, SettingsTab, TerminalSelection,
};
use ui::UiRenderer;

struct EvertySshApp {
    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    state: AppState,
    ui: UiRenderer,
    modifiers: ModifiersState,
    cursor_pos: (f64, f64),
    last_click_time: Instant,
    last_render_size: (u32, u32),
}

impl EvertySshApp {
    fn new(tokio_handle: tokio::runtime::Handle) -> Self {
        Self {
            window: None,
            context: None,
            surface: None,
            state: AppState::new(tokio_handle),
            ui: UiRenderer::new(),
            modifiers: ModifiersState::default(),
            cursor_pos: (0.0, 0.0),
            last_click_time: Instant::now(),
            last_render_size: (0, 0),
        }
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.last_render_size == (width, height) {
            return;
        }
        self.last_render_size = (width, height);

        if let Some(surface) = &mut self.surface {
            if let (Some(nz_w), Some(nz_h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
                let _ = surface.resize(nz_w, nz_h);
            }
        }
    }

    fn render(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        if self.last_render_size != (width, height) {
            self.resize_surface(width, height);
        }

        let Some(surface) = &mut self.surface else {
            return;
        };

        let scale_factor = window.scale_factor();
        let scale = DpiScale::new(scale_factor).unwrap_or_else(|_| DpiScale::new(1.0).unwrap());
        let phys_size = PhysicalSize::new(width as f64, height as f64);

        let logical_w = width as f64 / scale_factor;
        let logical_h = height as f64 / scale_factor;
        self.state.resize_terminals(logical_w, logical_h);

        if let Ok(pixels) = self.ui.render_frame(&self.state, scale, phys_size) {
            if let Ok(mut buffer) = surface.buffer_mut() {
                let dst_slice = buffer.as_mut();
                let count = dst_slice.len().min(pixels.len() / 4);
                let src_slice = &pixels[..count * 4];
                let dst_slice = &mut dst_slice[..count];

                let mut i = 0;
                let mut j = 0;
                while i + 16 <= src_slice.len() {
                    dst_slice[j] = ((src_slice[i] as u32) << 16) | ((src_slice[i + 1] as u32) << 8) | (src_slice[i + 2] as u32);
                    dst_slice[j + 1] = ((src_slice[i + 4] as u32) << 16) | ((src_slice[i + 5] as u32) << 8) | (src_slice[i + 6] as u32);
                    dst_slice[j + 2] = ((src_slice[i + 8] as u32) << 16) | ((src_slice[i + 9] as u32) << 8) | (src_slice[i + 10] as u32);
                    dst_slice[j + 3] = ((src_slice[i + 12] as u32) << 16) | ((src_slice[i + 13] as u32) << 8) | (src_slice[i + 14] as u32);
                    i += 16;
                    j += 4;
                }
                while i + 4 <= src_slice.len() {
                    dst_slice[j] = ((src_slice[i] as u32) << 16) | ((src_slice[i + 1] as u32) << 8) | (src_slice[i + 2] as u32);
                    i += 4;
                    j += 1;
                }
                let _ = buffer.present();
            }
        }
    }

    fn handle_mouse_click(&mut self, x: f64, y: f64) {
        let scale_factor = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
        let logical_x = x / scale_factor;
        let logical_y = y / scale_factor;
        let win_size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or(WinitPhysicalSize::new(1120, 720));
        let w = (win_size.width as f64) / scale_factor;
        let h = (win_size.height as f64) / scale_factor;

        // Ensure hover state is updated right at click coordinates
        self.state.update_hover(logical_x, logical_y, w, h);

        if !matches!(self.state.hover, HoverTarget::SidebarSearchHost | HoverTarget::SidebarSearchSnippet | HoverTarget::SidebarSearchCommand | HoverTarget::InputContextMenu(_)) {
            self.state.focused_input = None;
        }

        // If context menu is open, handle clicks on it or dismiss
        if let HoverTarget::ContextMenu(item) = self.state.hover {
            match item {
                ContextMenuItem::Copy => {
                    self.state.copy_selection_to_clipboard();
                }
                ContextMenuItem::SearchGoogle => {
                    if let Some(text) = self.state.get_selected_text() {
                        open_google_search(&text);
                    }
                }
                ContextMenuItem::Paste => {
                    self.state.paste_clipboard_text();
                }
            }
            self.state.context_menu = None;
            self.request_redraw();
            return;
        }

        // Dismiss context menu if clicked elsewhere
        if self.state.context_menu.is_some() {
            self.state.context_menu = None;
            self.request_redraw();
        }

        match self.state.hover {
            HoverTarget::TerminalScrollbarThumb => {
                self.state.is_dragging_scrollbar = true;
                self.state.scrollbar_drag_start_y = logical_y;
                if let Some(tab) = self.state.tabs.get(self.state.active_tab) {
                    self.state.scrollbar_drag_start_offset = tab.terminal.scroll_offset();
                }
                self.request_redraw();
            }
            HoverTarget::TerminalScrollbarTrack => {
                if let Some(tab) = self.state.tabs.get_mut(self.state.active_tab) {
                    let scrollback_len = tab.terminal.scrollback_len();
                    if scrollback_len > 0 {
                        let header_h = self.state.header_height;
                        let sb_y = header_h + 6.0;
                        let sb_h = (h - header_h - self.state.footer_height - 12.0).max(20.0);
                        let rel_ratio = ((logical_y - sb_y) / sb_h).clamp(0.0, 1.0);
                        let target_offset = ((1.0 - rel_ratio) * (scrollback_len as f64)).round() as usize;
                        tab.terminal.set_scroll_offset(target_offset);
                        self.request_redraw();
                    }
                }
            }
            HoverTarget::ContextMenu(_) => {}
            HoverTarget::SidebarSplitter => {
                self.state.is_dragging_sidebar = true;
                self.state.sidebar_drag_start_x = logical_x;
                self.state.sidebar_drag_start_width = self.state.sidebar_width;
            }
            HoverTarget::HostDeployKey(idx) => {
                self.state.open_key_tool_for_host(idx);
                self.request_redraw();
            }
            HoverTarget::ModalBrowseKey => {
                self.state.browse_key_file_for_add_modal();
                self.request_redraw();
            }
            HoverTarget::ModalGenerateKey => {
                self.state.open_key_tool_generator();
                self.request_redraw();
            }
            HoverTarget::KeyToolAlgo(algo) => {
                self.state.key_tool_modal.algo = algo;
                let def_name = match algo {
                    KeyAlgo::Ed25519 => "id_ed25519",
                    KeyAlgo::Rsa4096 => "id_rsa",
                };
                self.state.key_tool_modal.key_name.set(def_name);
                self.request_redraw();
            }
            HoverTarget::KeyToolField(idx) => {
                self.state.key_tool_modal.active_field = idx;
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::KeyToolGenerateBtn => {
                self.state.execute_key_generation();
                self.request_redraw();
            }
            HoverTarget::KeyToolCopyPubBtn => {
                if let Some(pub_key) = &self.state.key_tool_modal.generated_pub_key {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(pub_key.clone());
                    }
                    self.state.key_tool_modal.status_msg = Some("✓ Публичный ключ скопирован в буфер!".into());
                }
                self.request_redraw();
            }
            HoverTarget::KeyToolSendTermBtn => {
                self.state.send_deploy_cmd_to_active_terminal();
                self.request_redraw();
            }
            HoverTarget::KeyToolCopyCmdBtn => {
                if let Some(pub_key) = &self.state.key_tool_modal.generated_pub_key {
                    let cmd = app::make_authorized_keys_cmd(pub_key);
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(cmd);
                    }
                    self.state.key_tool_modal.status_msg = Some("✓ Команда скопирована в буфер!".into());
                }
                self.request_redraw();
            }
            HoverTarget::KeyToolBindHostBtn => {
                self.state.bind_generated_key_to_host();
                self.request_redraw();
            }
            HoverTarget::KeyToolCloseBtn => {
                self.state.key_tool_modal.is_open = false;
                self.request_redraw();
            }
            HoverTarget::TerminalArea => {
                if let Some(pos) = self.state.pos_from_coords(logical_x, logical_y) {
                    self.state.selection = Some(TerminalSelection::new(pos, pos));
                    self.state.is_mouse_selecting = true;
                    self.request_redraw();
                }
            }
            HoverTarget::ModalClose => {
                self.state.add_host_modal.is_open = false;
                self.request_redraw();
            }
            HoverTarget::ModalField(idx) => {
                self.state.add_host_modal.active_field = idx;
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::ModalAuthMode(mode) => {
                self.state.add_host_modal.auth_mode = mode;
                self.request_redraw();
            }
            HoverTarget::ModalSave => {
                self.state.save_host_from_modal(false);
                self.request_redraw();
            }
            HoverTarget::ModalConnect => {
                self.state.save_host_from_modal(true);
                self.request_redraw();
            }
            HoverTarget::PaletteItem(idx) => {
                self.state.selected_search_idx = idx;
                if let Some(cmd) = self.state.command_search_results.get(idx) {
                    let cmd_text = cmd.command.clone();
                    self.state.palette_open = false;
                    self.state.open_confirm_run_modal(cmd_text, "Палитра команд (Ctrl+R)".into());
                }
                self.request_redraw();
            }
            HoverTarget::HeaderTab(i) => {
                self.state.active_tab = i;
                self.request_redraw();
            }
            HoverTarget::HeaderTabClose(i) => {
                self.state.active_tab = i;
                self.state.close_active_tab();
                self.request_redraw();
            }
            HoverTarget::HeaderAddBtn | HoverTarget::SidebarAddHostBtn | HoverTarget::WelcomeAdd => {
                self.state.open_add_host_dialog();
                self.request_redraw();
            }
            HoverTarget::HeaderSearchBtn => {
                self.state.palette_open = !self.state.palette_open;
                if self.state.palette_open {
                    self.state.refresh_command_search();
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();
                }
                self.request_redraw();
            }
            HoverTarget::SidebarNav(tab) => {
                self.state.sidebar_tab = tab;
                self.request_redraw();
            }
            HoverTarget::SidebarSearchHost => {
                self.state.focused_input = Some(InputFieldId::HostSearch);
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::SidebarSearchSnippet => {
                self.state.focused_input = Some(InputFieldId::SnippetSearch);
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::SidebarSearchCommand => {
                self.state.focused_input = Some(InputFieldId::CommandSearch);
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::HostConnect(idx) => {
                if let Some(host) = self.state.hosts_doc.hosts.get(idx).cloned() {
                    self.state.connect_to_host(host);
                    self.request_redraw();
                }
            }
            HoverTarget::HostEdit(idx) => {
                self.state.open_edit_host_dialog(idx);
                self.request_redraw();
            }
            HoverTarget::HostDelete(idx) => {
                self.state.open_confirm_delete_host(idx);
                self.request_redraw();
            }
            HoverTarget::ConfirmDeleteConfirm => {
                self.state.execute_confirm_delete_host();
                self.request_redraw();
            }
            HoverTarget::ConfirmDeleteCancel => {
                self.state.confirm_delete_modal = None;
                self.request_redraw();
            }
            HoverTarget::HeaderSettingsBtn => {
                self.state.open_settings_modal();
                self.request_redraw();
            }
            HoverTarget::HeaderLangBtn => {
                self.state.toggle_language();
                self.request_redraw();
            }
            HoverTarget::SettingsSetLanguage(lang) => {
                self.state.set_language(lang);
                self.request_redraw();
            }
            HoverTarget::SettingsClose => {
                self.state.settings_modal = None;
                self.request_redraw();
            }
            HoverTarget::SettingsTab(tab) => {
                if let Some(m) = &mut self.state.settings_modal {
                    m.active_tab = tab;
                }
                self.request_redraw();
            }
            HoverTarget::SettingsSecurityField(idx) => {
                if let Some(m) = &mut self.state.settings_modal {
                    m.active_security_field = idx;
                    self.state.focused_input = Some(InputFieldId::SettingsSecurity(idx));
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();
                }
                self.request_redraw();
            }
            HoverTarget::SettingsGeneralField(idx) => {
                if let Some(m) = &mut self.state.settings_modal {
                    m.active_general_field = idx;
                    self.state.focused_input = Some(InputFieldId::SettingsGeneral(idx));
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();
                }
                self.request_redraw();
            }
            HoverTarget::SettingsSavePassword => {
                self.state.save_master_password();
                self.request_redraw();
            }
            HoverTarget::SettingsRemovePassword => {
                self.state.remove_master_password();
                self.request_redraw();
            }
            HoverTarget::SettingsExportJson => {
                self.state.export_backup_dialog();
                self.request_redraw();
            }
            HoverTarget::SettingsImportJson => {
                self.state.import_backup_dialog();
                self.request_redraw();
            }
            HoverTarget::SettingsSaveGeneral => {
                self.state.save_general_settings();
                self.request_redraw();
            }
            HoverTarget::SettingsToggleConfirmDelete => {
                if let Some(m) = &mut self.state.settings_modal {
                    m.confirm_delete = !m.confirm_delete;
                }
                self.request_redraw();
            }
            HoverTarget::LockPasswordField => {
                self.state.focused_input = Some(InputFieldId::LockPassword);
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::LockUnlockBtn => {
                self.state.try_unlock_app();
                self.request_redraw();
            }
            HoverTarget::HostCard(idx) => {
                self.state.selected_sidebar_item = idx;
                let now = Instant::now();
                let is_double_click = now.duration_since(self.last_click_time).as_millis() < 400;
                self.last_click_time = now;
                if is_double_click {
                    if let Some(host) = self.state.hosts_doc.hosts.get(idx).cloned() {
                        self.state.connect_to_host(host);
                    }
                }
                self.request_redraw();
            }
            HoverTarget::SessionCard(idx) => {
                self.state.open_session_dashboard(idx);
                self.request_redraw();
            }
            HoverTarget::SessionDashboardClose => {
                self.state.session_dashboard_modal = None;
                self.request_redraw();
            }
            HoverTarget::SessionDashboardReconnect => {
                if let Some(dash) = &self.state.session_dashboard_modal {
                    let host_id = dash.session.host_id.clone();
                    let username = dash.session.username.clone();
                    let host_address = dash.session.host_address.clone();
                    if let Some(host) = dash.host.clone() {
                        self.state.connect_to_host(host);
                    } else if let Some(hid) = host_id {
                        if let Some(host) = self.state.hosts_doc.hosts.iter().find(|h| h.id == hid).cloned() {
                            self.state.connect_to_host(host);
                        }
                    } else {
                        let parts: Vec<&str> = host_address.split(':').collect();
                        let hostname = parts.first().copied().unwrap_or("localhost").to_string();
                        let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(22);
                        let host = crate::config::HostConfig {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: format!("{username}@{hostname}"),
                            hostname,
                            port,
                            username,
                            auth: crate::config::AuthConfig::Agent,
                            group: Some("Default".into()),
                            tags: Vec::new(),
                            badge_color: Some("#2dd4bf".into()),
                            default_command: None,
                            env: std::collections::HashMap::new(),
                        };
                        self.state.connect_to_host(host);
                    }
                }
                self.state.session_dashboard_modal = None;
                self.request_redraw();
            }
            HoverTarget::SessionDashboardDelete => {
                self.state.delete_session_from_dashboard();
                self.request_redraw();
            }
            HoverTarget::SessionDashboardRunCmd(cmd_idx) => {
                if let Some(dash) = &self.state.session_dashboard_modal {
                    if let Some(cmd) = dash.commands.get(cmd_idx) {
                        let cmd_text = cmd.command.clone();
                        let src = format!("История ({})", dash.session.host_address);
                        self.state.open_confirm_run_modal(cmd_text, src);
                    }
                }
                self.request_redraw();
            }
            HoverTarget::SessionDashboardCopyCmd(cmd_idx) => {
                if let Some(dash) = &self.state.session_dashboard_modal {
                    if let Some(cmd) = dash.commands.get(cmd_idx) {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(cmd.command.clone());
                        }
                    }
                }
                self.request_redraw();
            }
            HoverTarget::CommandItem(idx) => {
                if let Some(cmd) = self.state.command_search_results.get(idx) {
                    let cmd_text = cmd.command.clone();
                    self.state.open_confirm_run_modal(cmd_text, "Журнал команд".into());
                }
                self.request_redraw();
            }
            HoverTarget::SidebarAddSnippetBtn => {
                self.state.open_add_snippet_dialog();
                self.request_redraw();
            }
            HoverTarget::SnippetItem(idx) | HoverTarget::SnippetRun(idx) => {
                if let Some(snip) = self.state.snippets.get(idx) {
                    let cmd_text = snip.command.clone();
                    let src = format!("Сниппет «{}»", snip.title);
                    self.state.open_confirm_run_modal(cmd_text, src);
                }
                self.request_redraw();
            }
            HoverTarget::ConfirmRunCancel => {
                self.state.confirm_run_modal = None;
                self.request_redraw();
            }
            HoverTarget::ConfirmRunCopy => {
                if let Some(m) = &mut self.state.confirm_run_modal {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(m.command.clone());
                        m.copied_feedback = true;
                    }
                }
                self.request_redraw();
            }
            HoverTarget::ConfirmRunExecute => {
                self.state.execute_confirm_run_command();
                self.request_redraw();
            }
            HoverTarget::SnippetCopy(idx) => {
                if let Some(snip) = self.state.snippets.get(idx) {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(snip.command.clone());
                    }
                }
                self.request_redraw();
            }
            HoverTarget::SnippetDelete(idx) => {
                if let Some(snip) = self.state.snippets.get(idx) {
                    let id = snip.id;
                    self.state.delete_snippet_by_id(id);
                }
                self.request_redraw();
            }
            HoverTarget::ModalSnippetField(idx) => {
                self.state.add_snippet_modal.active_field = idx;
                self.state.caret_visible = true;
                self.state.caret_blink_time = Instant::now();
                self.request_redraw();
            }
            HoverTarget::ModalSnippetSave => {
                self.state.save_snippet_from_modal();
                self.request_redraw();
            }
            HoverTarget::ModalSnippetClose => {
                self.state.add_snippet_modal.is_open = false;
                self.request_redraw();
            }
            HoverTarget::InputContextMenu(action) => {
                self.state.handle_input_menu_action(action);
                self.request_redraw();
            }
            HoverTarget::WelcomeConnect => {
                if let Some(host) = self.state.hosts_doc.hosts.first().cloned() {
                    self.state.connect_to_host(host);
                    self.request_redraw();
                }
            }
            HoverTarget::TerminalReconnect => {
                self.state.reconnect_active_tab();
                self.request_redraw();
            }
            HoverTarget::TerminalCloseTab => {
                self.state.close_active_tab();
                self.request_redraw();
            }
            HoverTarget::QuickAction(cmd) => {
                self.state.execute_quick_action(cmd);
                self.request_redraw();
            }
            HoverTarget::TerminalSearchOpenBtn => {
                self.state.open_terminal_search();
                self.request_redraw();
            }
            HoverTarget::TerminalSearchCloseBtn => {
                self.state.close_terminal_search();
                self.request_redraw();
            }
            HoverTarget::TerminalSearchPrevBtn => {
                self.state.prev_search_match();
                self.request_redraw();
            }
            HoverTarget::TerminalSearchNextBtn => {
                self.state.next_search_match();
                self.request_redraw();
            }
            HoverTarget::TerminalSearchCaseToggle => {
                self.state.toggle_search_case();
                self.request_redraw();
            }
            HoverTarget::TerminalSearchInput => {
                self.state.focused_input = Some(InputFieldId::TerminalSearch);
                self.request_redraw();
            }
            HoverTarget::TerminalSplitBtn => {
                self.state.toggle_split_view();
                self.request_redraw();
            }
            HoverTarget::TerminalSplitClose => {
                self.state.close_split_view();
                self.request_redraw();
            }
            HoverTarget::SplitTerminalArea => {
                if let Some(s_tab) = self.state.split_tab {
                    self.state.active_tab = s_tab;
                    self.request_redraw();
                }
            }
            HoverTarget::None => {
                if self.state.input_context_menu.is_some() {
                    self.state.input_context_menu = None;
                    self.request_redraw();
                } else if self.state.session_dashboard_modal.is_some() {
                    self.state.session_dashboard_modal = None;
                    self.request_redraw();
                } else if self.state.add_snippet_modal.is_open {
                    self.state.add_snippet_modal.is_open = false;
                    self.request_redraw();
                } else if self.state.add_host_modal.is_open {
                    self.state.add_host_modal.is_open = false;
                    self.request_redraw();
                } else if self.state.palette_open {
                    self.state.palette_open = false;
                    self.request_redraw();
                }
            }
        }
    }
}

fn matches_shortcut(
    physical_key: &PhysicalKey,
    logical_key: &Key,
    target_code: KeyCode,
    en_char: &str,
    ru_chars: &[&str],
) -> bool {
    if let PhysicalKey::Code(code) = physical_key {
        if *code == target_code {
            return true;
        }
    }
    if let Key::Character(c) = logical_key {
        if c.eq_ignore_ascii_case(en_char) {
            return true;
        }
        for ru in ru_chars {
            if c.eq_ignore_ascii_case(ru) {
                return true;
            }
        }
    }
    false
}

impl ApplicationHandler for EvertySshApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attr = WindowAttributes::default()
            .with_title("EvertySSH — Универсальный SSH клиент")
            .with_inner_size(WinitPhysicalSize::new(1120, 720));

        let window = Arc::new(event_loop.create_window(attr).expect("Failed to create window"));

        // Set sharp window icon from embedded Logo.png
        const LOGO_BYTES: &[u8] = include_bytes!("../Logo.png");
        if let Ok(img) = image::load_from_memory(LOGO_BYTES) {
            let resized = image::imageops::resize(&img, 64, 64, image::imageops::FilterType::Lanczos3);
            let (w, h) = resized.dimensions();
            if let Ok(icon) = winit::window::Icon::from_rgba(resized.into_raw(), w, h) {
                window.set_window_icon(Some(icon));
            }
        }

        let context = Context::new(window.clone()).expect("Failed to create softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("Failed to create surface");

        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);

        let size = self.window.as_ref().unwrap().inner_size();
        let scale_factor = self.window.as_ref().unwrap().scale_factor();
        let logical_w = size.width as f64 / scale_factor;
        let logical_h = size.height as f64 / scale_factor;
        self.state.resize_terminals(logical_w, logical_h);
        self.resize_surface(size.width, size.height);

        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                let scale_factor = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let logical_w = new_size.width as f64 / scale_factor;
                let logical_h = new_size.height as f64 / scale_factor;
                self.state.resize_terminals(logical_w, logical_h);
                self.resize_surface(new_size.width, new_size.height);
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.render();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x, position.y);
                let scale_factor = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let logical_x = position.x / scale_factor;
                let logical_y = position.y / scale_factor;
                let win_size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or(WinitPhysicalSize::new(1120, 720));
                let w = (win_size.width as f64) / scale_factor;
                let h = (win_size.height as f64) / scale_factor;

                if self.state.is_dragging_sidebar {
                    let delta_x = logical_x - self.state.sidebar_drag_start_x;
                    let min_w = 200.0;
                    let max_w = (w * 0.55).max(250.0).min(550.0);
                    let new_w = (self.state.sidebar_drag_start_width + delta_x).clamp(min_w, max_w);
                    if (new_w - self.state.sidebar_width).abs() >= 1.0 {
                        self.state.sidebar_width = new_w;
                        self.state.resize_terminals(w, h);
                        self.request_redraw();
                    }
                } else if self.state.is_dragging_scrollbar {
                    let delta_y = logical_y - self.state.scrollbar_drag_start_y;
                    let header_h = self.state.header_height;
                    let sb_h = (h - header_h - self.state.footer_height - 12.0).max(20.0);
                    if let Some(tab) = self.state.tabs.get_mut(self.state.active_tab) {
                        let scrollback_len = tab.terminal.scrollback_len();
                        if scrollback_len > 0 {
                            let rows = self.state.terminal_rows;
                            let total_lines = scrollback_len + rows;
                            let visible_ratio = (rows as f64) / (total_lines as f64);
                            let th = (sb_h * visible_ratio).clamp(24.0, sb_h);
                            let track_travel = sb_h - th;
                            if track_travel > 0.0 {
                                let offset_delta = -(delta_y / track_travel) * (scrollback_len as f64);
                                let new_offset = (self.state.scrollbar_drag_start_offset as f64 + offset_delta)
                                    .clamp(0.0, scrollback_len as f64)
                                    .round() as usize;
                                tab.terminal.set_scroll_offset(new_offset);
                                self.request_redraw();
                            }
                        }
                    }
                } else if self.state.is_mouse_selecting {
                    if let Some(pos) = self.state.pos_from_coords(logical_x, logical_y) {
                        if let Some(sel) = &mut self.state.selection {
                            if sel.end != pos {
                                sel.end = pos;
                                self.request_redraw();
                            }
                        }
                    }
                }

                let (changed, cursor_icon) = self.state.update_hover(logical_x, logical_y, w, h);
                if let Some(win) = &self.window {
                    win.set_cursor(cursor_icon);
                }
                if changed {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let scale_factor = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let logical_x = self.cursor_pos.0 / scale_factor;
                let logical_y = self.cursor_pos.1 / scale_factor;
                let win_size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or(WinitPhysicalSize::new(1120, 720));
                let w = (win_size.width as f64) / scale_factor;
                let h = (win_size.height as f64) / scale_factor;

                if state == ElementState::Pressed {
                    match button {
                        MouseButton::Left => {
                            self.handle_mouse_click(self.cursor_pos.0, self.cursor_pos.1);
                        }
                        MouseButton::Right => {
                            // Right click on text input opens InputContextMenu (Cut, Copy, Paste, Select All)
                            let target_field = match self.state.hover {
                                HoverTarget::ModalField(idx) => Some(InputFieldId::AddHost(idx)),
                                HoverTarget::KeyToolField(idx) => Some(InputFieldId::KeyTool(idx)),
                                HoverTarget::ModalSnippetField(idx) => Some(InputFieldId::AddSnippet(idx)),
                                HoverTarget::SidebarSearchHost => Some(InputFieldId::HostSearch),
                                HoverTarget::SidebarSearchSnippet => Some(InputFieldId::SnippetSearch),
                                HoverTarget::SidebarSearchCommand => Some(InputFieldId::CommandSearch),
                                _ => None,
                            };

                            if let Some(f_id) = target_field {
                                self.state.open_input_context_menu(logical_x, logical_y, f_id, w, h);
                                self.request_redraw();
                            } else if let Some(pos) = self.state.pos_from_coords(logical_x, logical_y) {
                                // PuTTY style right-click:
                                // If user clicked on existing selection, show Context Menu (Copy, Search in Google, Paste)
                                // Otherwise, directly paste clipboard text into the terminal!
                                let has_sel = self.state.selection.as_ref().map_or(false, |s| !s.is_empty());
                                let clicked_on_sel = self.state.selection.as_ref().map_or(false, |s| s.contains(pos.col, pos.row));

                                if has_sel && clicked_on_sel {
                                    self.state.open_context_menu_at(logical_x, logical_y, w, h);
                                    self.request_redraw();
                                } else {
                                    self.state.selection = None;
                                    self.state.context_menu = None;
                                    self.state.paste_clipboard_text();
                                    self.request_redraw();
                                }
                            }
                        }
                        _ => {}
                    }
                } else if state == ElementState::Released && button == MouseButton::Left {
                    if self.state.is_dragging_sidebar {
                        self.state.is_dragging_sidebar = false;
                        self.request_redraw();
                    }
                    if self.state.is_dragging_scrollbar {
                        self.state.is_dragging_scrollbar = false;
                        self.request_redraw();
                    }
                    if self.state.is_mouse_selecting {
                        self.state.is_mouse_selecting = false;
                        if let Some(sel) = &self.state.selection {
                            if sel.is_empty() {
                                self.state.selection = None;
                            } else {
                                // Auto-copy selected text (PuTTY behavior)
                                self.state.copy_selection_to_clipboard();
                            }
                        }
                        self.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 20.0) as i32,
                };
                if let Some(tab) = self.state.tabs.get_mut(self.state.active_tab) {
                    if lines > 0 {
                        tab.terminal.scroll_up(lines.abs() as usize * 3);
                    } else if lines < 0 {
                        tab.terminal.scroll_down(lines.abs() as usize * 3);
                    }
                    self.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        physical_key,
                        logical_key,
                        text,
                        ..
                    },
                ..
            } => {
                // Hotkeys
                if self.modifiers.control_key() && self.modifiers.alt_key() {
                    // Ctrl+Alt+S (Split Screen Toggle)
                    if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyS, "s", &["ы", "Ы"]) {
                        self.state.toggle_split_view();
                        self.request_redraw();
                        return;
                    }
                } else if self.modifiers.control_key() {
                    if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyV, "v", &["м", "М"]) {
                        self.state.paste_clipboard_text();
                        self.request_redraw();
                        return;
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyC, "c", &["с", "С"]) {
                        let copied = self.state.copy_active_selection_or_terminal();
                        if !copied && !self.state.is_modal_open() {
                            self.state.send_to_active_terminal("\x03");
                        }
                        self.request_redraw();
                        return;
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyX, "x", &["ч", "Ч"]) {
                        if self.state.cut_active_selection() {
                            self.request_redraw();
                            return;
                        }
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyA, "a", &["ф", "Ф"]) {
                        if self.state.select_all_active_input() {
                            self.request_redraw();
                            return;
                        }
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyD, "d", &["в", "В"]) {
                        if !self.state.is_modal_open() {
                            self.state.send_to_active_terminal("\x04");
                            self.request_redraw();
                            return;
                        }
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyL, "l", &["д", "Д"]) {
                        if !self.state.is_modal_open() {
                            self.state.send_to_active_terminal("\x0c");
                            self.request_redraw();
                            return;
                        }
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyN, "n", &["т", "Т"]) {
                        self.state.open_add_host_dialog();
                        self.request_redraw();
                        return;
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyR, "r", &["к", "К"]) {
                        self.state.palette_open = !self.state.palette_open;
                        if self.state.palette_open {
                            self.state.refresh_command_search();
                            self.state.caret_visible = true;
                            self.state.caret_blink_time = Instant::now();
                        }
                        self.request_redraw();
                        return;
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyT, "t", &["е", "Е"]) {
                        if let Some(host) = self.state.hosts_doc.hosts.first().cloned() {
                            self.state.connect_to_host(host);
                            self.request_redraw();
                        }
                        return;
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyW, "w", &["ц", "Ц"]) {
                        self.state.close_active_tab();
                        self.request_redraw();
                        return;
                    } else if matches_shortcut(&physical_key, &logical_key, KeyCode::KeyF, "f", &["а", "А"]) {
                        self.state.open_terminal_search();
                        self.request_redraw();
                        return;
                    }
                }

                // Shift+Insert paste
                if self.modifiers.shift_key() && matches!(&logical_key, Key::Named(NamedKey::Insert)) {
                    self.state.paste_clipboard_text();
                    self.request_redraw();
                    return;
                }

                // If App is Locked
                if self.state.is_app_locked {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();
                    match &logical_key {
                        Key::Named(NamedKey::Enter) => {
                            self.state.try_unlock_app();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.lock_password_input.delete_back();
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if let Some(t) = text {
                                self.state.lock_password_input.insert_str(&t);
                                self.request_redraw();
                                return;
                            }
                        }
                    }
                    return;
                }

                // If Terminal Search is Open
                if self.state.search_state.is_open {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();
                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.close_terminal_search();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            if self.modifiers.shift_key() {
                                self.state.prev_search_match();
                            } else {
                                self.state.next_search_match();
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.state.search_state.query.move_left();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.state.search_state.query.move_right();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Delete) => {
                            self.state.search_state.query.delete_forward();
                            self.state.update_terminal_search();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.search_state.query.delete_back();
                            self.state.update_terminal_search();
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if let Some(t) = text {
                                self.state.search_state.query.insert_str(&t);
                                self.state.update_terminal_search();
                                self.request_redraw();
                                return;
                            }
                        }
                    }
                    return;
                }

                // If Confirm Delete Modal is Open
                if self.state.confirm_delete_modal.is_some() {
                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.confirm_delete_modal = None;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.state.execute_confirm_delete_host();
                            self.request_redraw();
                            return;
                        }
                        _ => return,
                    }
                }

                // If Confirm Run Command Modal is Open
                if let Some(confirm_run) = &mut self.state.confirm_run_modal {
                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.confirm_run_modal = None;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.state.execute_confirm_run_command();
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if self.modifiers.control_key() {
                                if let Key::Character(c) = &logical_key {
                                    if c.eq_ignore_ascii_case("c") {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            let _ = cb.set_text(confirm_run.command.clone());
                                            confirm_run.copied_feedback = true;
                                            self.request_redraw();
                                        }
                                        return;
                                    }
                                }
                            }
                            return;
                        }
                    }
                }

                // If Settings Modal is Open
                if let Some(settings) = &mut self.state.settings_modal {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();
                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.settings_modal = None;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Tab) => {
                            match settings.active_tab {
                                SettingsTab::Security => {
                                    let has_existing = self.state.hosts_doc.settings.master_password_hash.is_some();
                                    if has_existing {
                                        settings.active_security_field = (settings.active_security_field + 1) % 3;
                                    } else {
                                        settings.active_security_field = if settings.active_security_field == 1 { 2 } else { 1 };
                                    }
                                    self.state.focused_input = Some(InputFieldId::SettingsSecurity(settings.active_security_field));
                                }
                                SettingsTab::General => {
                                    settings.active_general_field = (settings.active_general_field + 1) % 2;
                                    self.state.focused_input = Some(InputFieldId::SettingsGeneral(settings.active_general_field));
                                }
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            match settings.active_tab {
                                SettingsTab::Security => self.state.save_master_password(),
                                SettingsTab::General => self.state.save_general_settings(),
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.handle_key_input("\x08");
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if let Some(t) = text {
                                self.state.handle_key_input(&t);
                                self.request_redraw();
                                return;
                            }
                        }
                    }
                    return;
                }

                // If Input Context Menu is Open
                if self.state.input_context_menu.is_some() {
                    if matches!(&logical_key, Key::Named(NamedKey::Escape)) {
                        self.state.input_context_menu = None;
                        self.request_redraw();
                        return;
                    }
                }

                // If Session Dashboard Modal is Open
                if self.state.session_dashboard_modal.is_some() {
                    if matches!(&logical_key, Key::Named(NamedKey::Escape)) {
                        self.state.session_dashboard_modal = None;
                        self.request_redraw();
                        return;
                    }
                }

                // If a sidebar search box is focused
                if let Some(f_id) = self.state.focused_input {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();

                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            if let Some(inp) = self.state.get_text_input_mut(f_id) {
                                if !inp.text.is_empty() {
                                    inp.text.clear();
                                    inp.cursor = 0;
                                    inp.selection = None;
                                    if f_id == InputFieldId::SnippetSearch {
                                        self.state.refresh_snippets();
                                    } else if f_id == InputFieldId::CommandSearch {
                                        self.state.refresh_command_search();
                                    }
                                }
                            }
                            self.state.focused_input = None;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            if let Some(inp) = self.state.get_text_input_mut(f_id) {
                                inp.move_left();
                                self.request_redraw();
                                return;
                            }
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            if let Some(inp) = self.state.get_text_input_mut(f_id) {
                                inp.move_right();
                                self.request_redraw();
                                return;
                            }
                        }
                        Key::Named(NamedKey::Home) => {
                            if let Some(inp) = self.state.get_text_input_mut(f_id) {
                                inp.move_home();
                                self.request_redraw();
                                return;
                            }
                        }
                        Key::Named(NamedKey::End) => {
                            if let Some(inp) = self.state.get_text_input_mut(f_id) {
                                inp.move_end();
                                self.request_redraw();
                                return;
                            }
                        }
                        Key::Named(NamedKey::Delete) => {
                            if let Some(inp) = self.state.get_text_input_mut(f_id) {
                                inp.delete_forward();
                                if f_id == InputFieldId::SnippetSearch {
                                    self.state.refresh_snippets();
                                } else if f_id == InputFieldId::CommandSearch {
                                    self.state.refresh_command_search();
                                }
                                self.request_redraw();
                                return;
                            }
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.state.handle_key_input("\r");
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.handle_key_input("\x08");
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if let Some(t) = text {
                                self.state.handle_key_input(&t);
                                self.request_redraw();
                                return;
                            }
                        }
                    }
                }

                // If Add Snippet Modal is Open
                if self.state.add_snippet_modal.is_open {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();

                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.add_snippet_modal.is_open = false;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Tab) => {
                            if self.modifiers.shift_key() {
                                if self.state.add_snippet_modal.active_field == 0 {
                                    self.state.add_snippet_modal.active_field = 3;
                                } else {
                                    self.state.add_snippet_modal.active_field -= 1;
                                }
                            } else {
                                self.state.add_snippet_modal.active_field =
                                    (self.state.add_snippet_modal.active_field + 1) % 4;
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.state.save_snippet_from_modal();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.handle_key_input("\x08");
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if let Some(t) = text {
                                self.state.handle_key_input(&t);
                                self.request_redraw();
                                return;
                            }
                        }
                    }
                }

                // If Key Tool Modal is Open
                if self.state.key_tool_modal.is_open {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();

                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.key_tool_modal.is_open = false;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Tab) => {
                            self.state.key_tool_modal.active_field =
                                (self.state.key_tool_modal.active_field + 1) % 3;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.state.execute_key_generation();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.handle_key_input("\x08");
                            self.request_redraw();
                            return;
                        }
                        _ => {
                            if let Some(t) = text {
                                self.state.handle_key_input(&t);
                                self.request_redraw();
                                return;
                            }
                        }
                    }
                }

                // If Modal Add Host is Open
                if self.state.add_host_modal.is_open {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();

                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.add_host_modal.is_open = false;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Tab) => {
                            let max_fields = if self.state.add_host_modal.auth_mode == AuthModeChoice::Agent { 5 } else { 6 };
                            if self.modifiers.shift_key() {
                                if self.state.add_host_modal.active_field == 0 {
                                    self.state.add_host_modal.active_field = max_fields - 1;
                                } else {
                                    self.state.add_host_modal.active_field -= 1;
                                }
                            } else {
                                self.state.add_host_modal.active_field = (self.state.add_host_modal.active_field + 1) % max_fields;
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.state.save_host_from_modal(false);
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            match self.state.add_host_modal.active_field {
                                0 => self.state.add_host_modal.name.move_left(),
                                1 => self.state.add_host_modal.hostname.move_left(),
                                2 => self.state.add_host_modal.port.move_left(),
                                3 => self.state.add_host_modal.username.move_left(),
                                4 => self.state.add_host_modal.group.move_left(),
                                5 => {
                                    if self.state.add_host_modal.auth_mode == AuthModeChoice::Key {
                                        self.state.add_host_modal.key_path.move_left();
                                    } else if self.state.add_host_modal.auth_mode == AuthModeChoice::Password {
                                        self.state.add_host_modal.password.move_left();
                                    }
                                }
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            match self.state.add_host_modal.active_field {
                                0 => self.state.add_host_modal.name.move_right(),
                                1 => self.state.add_host_modal.hostname.move_right(),
                                2 => self.state.add_host_modal.port.move_right(),
                                3 => self.state.add_host_modal.username.move_right(),
                                4 => self.state.add_host_modal.group.move_right(),
                                5 => {
                                    if self.state.add_host_modal.auth_mode == AuthModeChoice::Key {
                                        self.state.add_host_modal.key_path.move_right();
                                    } else if self.state.add_host_modal.auth_mode == AuthModeChoice::Password {
                                        self.state.add_host_modal.password.move_right();
                                    }
                                }
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Home) => {
                            match self.state.add_host_modal.active_field {
                                0 => self.state.add_host_modal.name.move_home(),
                                1 => self.state.add_host_modal.hostname.move_home(),
                                2 => self.state.add_host_modal.port.move_home(),
                                3 => self.state.add_host_modal.username.move_home(),
                                4 => self.state.add_host_modal.group.move_home(),
                                5 => {
                                    if self.state.add_host_modal.auth_mode == AuthModeChoice::Key {
                                        self.state.add_host_modal.key_path.move_home();
                                    } else if self.state.add_host_modal.auth_mode == AuthModeChoice::Password {
                                        self.state.add_host_modal.password.move_home();
                                    }
                                }
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::End) => {
                            match self.state.add_host_modal.active_field {
                                0 => self.state.add_host_modal.name.move_end(),
                                1 => self.state.add_host_modal.hostname.move_end(),
                                2 => self.state.add_host_modal.port.move_end(),
                                3 => self.state.add_host_modal.username.move_end(),
                                4 => self.state.add_host_modal.group.move_end(),
                                5 => {
                                    if self.state.add_host_modal.auth_mode == AuthModeChoice::Key {
                                        self.state.add_host_modal.key_path.move_end();
                                    } else if self.state.add_host_modal.auth_mode == AuthModeChoice::Password {
                                        self.state.add_host_modal.password.move_end();
                                    }
                                }
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Delete) => {
                            match self.state.add_host_modal.active_field {
                                0 => self.state.add_host_modal.name.delete_forward(),
                                1 => self.state.add_host_modal.hostname.delete_forward(),
                                2 => self.state.add_host_modal.port.delete_forward(),
                                3 => self.state.add_host_modal.username.delete_forward(),
                                4 => self.state.add_host_modal.group.delete_forward(),
                                5 => {
                                    if self.state.add_host_modal.auth_mode == AuthModeChoice::Key {
                                        self.state.add_host_modal.key_path.delete_forward();
                                    } else if self.state.add_host_modal.auth_mode == AuthModeChoice::Password {
                                        self.state.add_host_modal.password.delete_forward();
                                    }
                                }
                                _ => {}
                            }
                            self.request_redraw();
                            return;
                        }
                        _ => {}
                    }
                }

                // Modal Palette Navigation
                if self.state.palette_open {
                    self.state.caret_visible = true;
                    self.state.caret_blink_time = Instant::now();

                    match &logical_key {
                        Key::Named(NamedKey::Escape) => {
                            self.state.palette_open = false;
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            if !self.state.command_search_results.is_empty() {
                                self.state.selected_search_idx = (self.state.selected_search_idx + 1)
                                    .min(self.state.command_search_results.len().saturating_sub(1));
                                self.request_redraw();
                            }
                            return;
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            if self.state.selected_search_idx > 0 {
                                self.state.selected_search_idx -= 1;
                                self.request_redraw();
                            }
                            return;
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.state.command_search_input.move_left();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.state.command_search_input.move_right();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Delete) => {
                            self.state.command_search_input.delete_forward();
                            self.state.refresh_command_search();
                            self.request_redraw();
                            return;
                        }
                        Key::Named(NamedKey::Enter) => {
                            if let Some(cmd) = self.state.command_search_results.get(self.state.selected_search_idx) {
                                let cmd_text = cmd.command.clone();
                                self.state.palette_open = false;
                                self.state.open_confirm_run_modal(cmd_text, "Палитра команд (Ctrl+R)".into());
                            } else {
                                self.state.palette_open = false;
                            }
                            self.request_redraw();
                            return;
                        }
                        _ => {}
                    }
                }

                // Regular key text input
                if let Some(txt) = text {
                    self.state.handle_key_input(txt.as_str());
                    self.request_redraw();
                } else {
                    match &logical_key {
                        Key::Named(NamedKey::Enter) => {
                            self.state.handle_key_input("\r");
                            self.request_redraw();
                        }
                        Key::Named(NamedKey::Backspace) => {
                            self.state.handle_key_input("\x08");
                            self.request_redraw();
                        }
                        Key::Named(NamedKey::Tab) => {
                            self.state.handle_key_input("\t");
                            self.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            self.state.send_to_active_terminal("\x1b[A");
                            self.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            self.state.send_to_active_terminal("\x1b[B");
                            self.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.state.send_to_active_terminal("\x1b[C");
                            self.request_redraw();
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.state.send_to_active_terminal("\x1b[D");
                            self.request_redraw();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut need_redraw = false;

        if self.state.poll_events() {
            need_redraw = true;
        }

        if self.state.update_caret() {
            // Only redraw if a caret is actually visible on screen (search focused, modal open, palette open, or terminal open)
            if self.state.focused_input.is_some()
                || self.state.add_host_modal.is_open
                || self.state.key_tool_modal.is_open
                || self.state.add_snippet_modal.is_open
                || self.state.palette_open
                || !self.state.tabs.is_empty()
            {
                need_redraw = true;
            }
        }

        // Automatic tooltip display after 350ms hover delay
        if let Some(tt) = &self.state.tooltip {
            let el = tt.hover_start.elapsed();
            if el >= Duration::from_millis(350) && el < Duration::from_millis(420) {
                need_redraw = true;
            }
        }

        if need_redraw {
            self.request_redraw();
        }

        // Set efficient wait duration instead of busy-poll!
        event_loop.set_control_flow(ControlFlow::wait_duration(Duration::from_millis(30)));
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".into());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic".to_string()
        };
        let backtrace = std::backtrace::Backtrace::capture();
        let log_msg = format!(
            "=== EvertySSH CRASH REPORT ===\nTime: {:?}\nLocation: {}\nError: {}\nBacktrace:\n{}\n",
            std::time::SystemTime::now(),
            location,
            payload,
            backtrace
        );
        eprintln!("{}", log_msg);
        let _ = std::fs::write("crash.log", &log_msg);
        if let Some(h) = dirs::home_dir() {
            let _ = std::fs::create_dir_all(h.join(".evertyssh"));
            let _ = std::fs::write(h.join(".evertyssh").join("crash.log"), &log_msg);
        }
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("EvertySSH - Ошибка")
            .set_description(&format!("Приложение аварийно завершило работу:\n{}\n\nПодробности записаны в crash.log", payload))
            .show();
    }));

    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio runtime");

    let tokio_handle = tokio_runtime.handle().clone();

    let _ = config::load_hosts();

    let event_loop = EventLoop::new().expect("Failed to create winit event loop");
    let mut app = EvertySshApp::new(tokio_handle);
    event_loop.run_app(&mut app).expect("Application run error");
}
