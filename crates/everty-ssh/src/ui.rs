use everty_ui::{
    BitmapIcon, Color, DpiScale, FontFamily, FontWeight, IconId, LogicalPoint, LogicalPx,
    LogicalRect, PaintBuilder, PhysicalSize, Renderer, Rgba8Renderer, TextLayout, TextStyle,
    TextSystem,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::app::{
    AppState, AuthModeChoice, ContextMenuItem, HoverTarget, InputFieldId, InputMenuAction, KeyAlgo,
    QuickActionCmd, SessionTab, SettingsTab, SidebarTab, TextInput,
};
use crate::config::AppLanguage;
use crate::i18n::tr;
use crate::terminal::{DEFAULT_TERM_BG, DEFAULT_TERM_FG};

const LOGO_ICON_ID: IconId = IconId(1);
const LOGO_SMALL_ICON_ID: IconId = IconId(2);
const ICON_CONNECT_ID: IconId = IconId(10);
const ICON_KEY_ID: IconId = IconId(11);
const ICON_EDIT_ID: IconId = IconId(12);
const ICON_CLOSE_ID: IconId = IconId(13);
const ICON_COPY_ID: IconId = IconId(14);
const ICON_SEARCH_ID: IconId = IconId(15);
const ICON_PLUS_ID: IconId = IconId(16);
const ICON_SETTINGS_ID: IconId = IconId(17);
const ICON_LOCK_ID: IconId = IconId(18);
const ICON_ALERT_ID: IconId = IconId(19);
const ICON_REFRESH_ID: IconId = IconId(20);
const ICON_SPLIT_ID: IconId = IconId(21);
const ICON_UP_ID: IconId = IconId(22);
const ICON_DOWN_ID: IconId = IconId(23);
const ICON_EYE_ID: IconId = IconId(24);
const ICON_EYE_OFF_ID: IconId = IconId(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleId {
    Sans,
    SansBold,
    Mono,
    Small,
    Title,
}

pub struct UiRenderer {
    pub renderer: Rgba8Renderer,
    pub text_system: TextSystem,
    pub mono_char_w: f64,
    sans_style: TextStyle,
    sans_bold: TextStyle,
    mono_style: TextStyle,
    small_style: TextStyle,
    title_style: TextStyle,
    layout_cache: HashMap<(String, StyleId, u32), Arc<TextLayout>>,
    logo_loaded: bool,
}

impl UiRenderer {
    pub fn new() -> Self {
        let mut renderer = Rgba8Renderer::new();
        let logo_loaded = Self::try_load_logo(&mut renderer);
        Self::register_vector_icons(&mut renderer);

        let mut text_system = TextSystem::new();
        let mono_style = TextStyle {
            family: FontFamily::Monospace,
            weight: FontWeight::Regular,
            font_size: LogicalPx(13.0),
            line_height: LogicalPx(18.0),
        };
        let mono_char_w = text_system
            .shape("M", &mono_style, None)
            .metrics
            .size
            .width
            .0;

        Self {
            renderer,
            text_system,
            mono_char_w,
            sans_style: TextStyle {
                family: FontFamily::SansSerif,
                weight: FontWeight::Regular,
                font_size: LogicalPx(13.0),
                line_height: LogicalPx(18.0),
            },
            sans_bold: TextStyle {
                family: FontFamily::SansSerif,
                weight: FontWeight::Bold,
                font_size: LogicalPx(13.0),
                line_height: LogicalPx(18.0),
            },
            mono_style: TextStyle {
                family: FontFamily::Monospace,
                weight: FontWeight::Regular,
                font_size: LogicalPx(13.0),
                line_height: LogicalPx(18.0),
            },
            small_style: TextStyle {
                family: FontFamily::SansSerif,
                weight: FontWeight::Regular,
                font_size: LogicalPx(11.0),
                line_height: LogicalPx(15.0),
            },
            title_style: TextStyle {
                family: FontFamily::SansSerif,
                weight: FontWeight::Bold,
                font_size: LogicalPx(15.0),
                line_height: LogicalPx(21.0),
            },
            layout_cache: HashMap::with_capacity(256),
            logo_loaded,
        }
    }

    pub fn shape_cached(
        &mut self,
        text: &str,
        style_id: StyleId,
        max_width: Option<LogicalPx>,
    ) -> Arc<TextLayout> {
        let mw_key = max_width.map(|w| (w.0 * 10.0) as u32).unwrap_or(0);
        let key = (text.to_string(), style_id, mw_key);
        if let Some(layout) = self.layout_cache.get(&key) {
            return Arc::clone(layout);
        }

        let style = match style_id {
            StyleId::Sans => &self.sans_style,
            StyleId::SansBold => &self.sans_bold,
            StyleId::Mono => &self.mono_style,
            StyleId::Small => &self.small_style,
            StyleId::Title => &self.title_style,
        };

        let layout = Arc::new(self.text_system.shape(text, style, max_width));
        if self.layout_cache.len() >= 1200 {
            self.layout_cache.clear();
        }
        self.layout_cache.insert(key, Arc::clone(&layout));
        layout
    }

    pub fn shape_clamped_cached(
        &mut self,
        text: &str,
        style_id: StyleId,
        max_width: LogicalPx,
        max_lines: usize,
    ) -> Arc<TextLayout> {
        let mw_key = (max_width.0 * 10.0) as u32;
        let key = (format!("{text}#clamped#{max_lines}"), style_id, mw_key);
        if let Some(layout) = self.layout_cache.get(&key) {
            return Arc::clone(layout);
        }

        let style = match style_id {
            StyleId::Sans => &self.sans_style,
            StyleId::SansBold => &self.sans_bold,
            StyleId::Mono => &self.mono_style,
            StyleId::Small => &self.small_style,
            StyleId::Title => &self.title_style,
        };

        let layout = Arc::new(
            self.text_system
                .shape_clamped(text, style, max_width, max_lines),
        );
        if self.layout_cache.len() >= 1200 {
            self.layout_cache.clear();
        }
        self.layout_cache.insert(key, Arc::clone(&layout));
        layout
    }

    pub fn draw_input_box(
        &mut self,
        builder: &mut PaintBuilder,
        inp: &TextInput,
        rect: LogicalRect,
        is_focused: bool,
        is_hover: bool,
        placeholder: &str,
        is_password: bool,
        caret_visible: bool,
        accent_mint: Color,
        card_border: Color,
        text_main: Color,
        text_muted: Color,
    ) {
        builder.fill_rect(rect, Color::rgba(20, 32, 58, 255), 4.0);
        if is_focused {
            builder.stroke_rect(rect, accent_mint, 1.5, 4.0);
        } else if is_hover {
            builder.stroke_rect(rect, Color::rgba(45, 212, 191, 140), 1.0, 4.0);
        } else {
            builder.stroke_rect(rect, card_border, 1.0, 4.0);
        }

        let inner_w = (rect.size.width.0 - 20.0).max(10.0);

        if inp.text.is_empty() {
            let p_layout = self.shape_cached(placeholder, StyleId::Sans, None);
            builder.text(
                LogicalPoint::new(rect.origin.x.0 + 10.0, rect.origin.y.0 + 6.0),
                p_layout,
                text_muted,
            );
            if is_focused && caret_visible {
                builder.fill_rect(
                    LogicalRect::from_xywh(
                        rect.origin.x.0 + 10.0,
                        rect.origin.y.0 + 5.0,
                        2.0,
                        20.0,
                    ),
                    accent_mint,
                    1.0,
                );
            }
            return;
        }

        let display_chars: Vec<char> = if is_password {
            inp.text.chars().map(|_| '•').collect()
        } else {
            inp.text.chars().collect()
        };

        let total_chars = display_chars.len();
        let mut offset = inp.scroll_offset.min(total_chars);
        if is_focused {
            if inp.cursor < offset {
                offset = inp.cursor;
            } else {
                // Ensure text from offset to cursor fits in inner_w
                while offset < inp.cursor {
                    let sub: String = display_chars
                        .iter()
                        .skip(offset)
                        .take(inp.cursor - offset)
                        .collect();
                    let w = self
                        .shape_cached(&sub, StyleId::Sans, None)
                        .metrics
                        .size
                        .width
                        .0;
                    if w <= inner_w {
                        break;
                    }
                    offset += 1;
                }
            }
            // Expand offset to the left if the remaining text to the end easily fits in inner_w
            while offset > 0 {
                let sub: String = display_chars.iter().skip(offset - 1).collect();
                let w = self
                    .shape_cached(&sub, StyleId::Sans, None)
                    .metrics
                    .size
                    .width
                    .0;
                if w <= inner_w {
                    offset -= 1;
                } else {
                    break;
                }
            }
        }

        // Compute take_count: greedily take all characters that physically fit in inner_w
        let remaining = total_chars.saturating_sub(offset);
        let rem_str: String = display_chars.iter().skip(offset).collect();
        let take_count = if self
            .shape_cached(&rem_str, StyleId::Sans, None)
            .metrics
            .size
            .width
            .0
            <= inner_w
        {
            remaining
        } else {
            let mut low = 1;
            let mut high = remaining;
            let mut best = 1;
            while low <= high {
                let mid = (low + high) / 2;
                let sub: String = display_chars.iter().skip(offset).take(mid).collect();
                let w = self
                    .shape_cached(&sub, StyleId::Sans, None)
                    .metrics
                    .size
                    .width
                    .0;
                if w <= inner_w + 2.0 {
                    best = mid;
                    low = mid + 1;
                } else {
                    if mid == 0 {
                        break;
                    }
                    high = mid - 1;
                }
            }
            best
        };
        let visible_str: String = display_chars.iter().skip(offset).take(take_count).collect();

        // Render selection highlight if any
        if is_focused {
            if let Some((sel_start, sel_end)) = inp.selected_range() {
                if sel_end > offset && sel_start < offset + take_count {
                    let rel_start = sel_start.saturating_sub(offset);
                    let rel_end = (sel_end - offset).min(take_count);
                    if rel_end > rel_start {
                        let prefix: String =
                            display_chars.iter().skip(offset).take(rel_start).collect();
                        let sel_part: String = display_chars
                            .iter()
                            .skip(offset + rel_start)
                            .take(rel_end - rel_start)
                            .collect();
                        let p_w = if prefix.is_empty() {
                            0.0
                        } else {
                            self.shape_cached(&prefix, StyleId::Sans, None)
                                .metrics
                                .size
                                .width
                                .0
                        };
                        let s_w = self
                            .shape_cached(&sel_part, StyleId::Sans, None)
                            .metrics
                            .size
                            .width
                            .0;
                        let fill_w = s_w.min(inner_w - p_w).max(4.0);
                        let sel_rect = LogicalRect::from_xywh(
                            rect.origin.x.0 + 10.0 + p_w,
                            rect.origin.y.0 + 4.0,
                            fill_w,
                            22.0,
                        );
                        // Selection background fill
                        builder.fill_rect(sel_rect, Color::rgba(45, 212, 191, 75), 3.0);
                        // Crisp selection outline border
                        builder.stroke_rect(sel_rect, Color::rgba(45, 212, 191, 180), 1.0, 3.0);
                    }
                }
            }
        }

        let txt_layout = self.shape_cached(&visible_str, StyleId::Sans, None);
        builder.text(
            LogicalPoint::new(rect.origin.x.0 + 10.0, rect.origin.y.0 + 6.0),
            txt_layout,
            text_main,
        );

        if is_focused && caret_visible {
            let rel_cursor = inp.cursor.saturating_sub(offset);
            if rel_cursor <= take_count {
                let cursor_prefix: String =
                    display_chars.iter().skip(offset).take(rel_cursor).collect();
                let prefix_w = if cursor_prefix.is_empty() {
                    0.0
                } else {
                    self.shape_cached(&cursor_prefix, StyleId::Sans, None)
                        .metrics
                        .size
                        .width
                        .0
                };
                let caret_x = rect.origin.x.0 + 10.0 + prefix_w;
                if caret_x <= rect.origin.x.0 + rect.size.width.0 - 6.0 {
                    builder.fill_rect(
                        LogicalRect::from_xywh(caret_x, rect.origin.y.0 + 5.0, 2.0, 20.0),
                        accent_mint,
                        1.0,
                    );
                }
            }
        }

        // Draw subtle bottom scroll track & thumb and edge indicators when text overflows
        let max_visible_chars = take_count;
        if total_chars > max_visible_chars {
            let track_x = rect.origin.x.0 + 8.0;
            let track_y = rect.origin.y.0 + rect.size.height.0 - 2.5;
            let track_w = (rect.size.width.0 - 16.0).max(10.0);
            builder.fill_rect(
                LogicalRect::from_xywh(track_x, track_y, track_w, 2.0),
                Color::rgba(255, 255, 255, 30),
                1.0,
            );

            let visible_ratio = (max_visible_chars as f64 / total_chars as f64).clamp(0.08, 1.0);
            let thumb_w = (track_w * visible_ratio).max(18.0);
            let max_scroll = total_chars.saturating_sub(max_visible_chars);
            let scroll_ratio = if max_scroll > 0 {
                (offset as f64 / max_scroll as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let thumb_x = track_x + (track_w - thumb_w) * scroll_ratio;
            builder.fill_rect(
                LogicalRect::from_xywh(thumb_x, track_y, thumb_w, 2.0),
                accent_mint,
                1.0,
            );

            // Left hint: small indicator if scrolled forward
            if offset > 0 {
                let left_arrow = self.shape_cached("‹", StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(rect.origin.x.0 + 2.0, rect.origin.y.0 + 7.0),
                    left_arrow,
                    accent_mint,
                );
            }
            // Right hint: small indicator if more text to the right
            if offset + take_count < total_chars {
                let right_arrow = self.shape_cached("›", StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(
                        rect.origin.x.0 + rect.size.width.0 - 9.0,
                        rect.origin.y.0 + 7.0,
                    ),
                    right_arrow,
                    accent_mint,
                );
            }
        }
    }

    fn render_single_terminal(
        &mut self,
        builder: &mut PaintBuilder,
        app: &AppState,
        tab_idx: usize,
        tab: &SessionTab,
        is_active_panel: bool,
        term_rect: LogicalRect,
        accent_mint: Color,
        card_border: Color,
        text_main: Color,
        text_muted: Color,
        bg_app: Color,
    ) {
        builder.fill_rect(term_rect, bg_app, 0.0);

        let term_x = term_rect.origin.x.0;
        let term_y = term_rect.origin.y.0;
        let term_w = term_rect.size.width.0;
        let term_h = term_rect.size.height.0;

        let char_w = self.mono_char_w;
        let line_h = 18.0;

        // 1. Diagnostics Toolbar (if connected)
        let is_connected = tab.handle.is_some();
        let bar_h = if is_connected { 30.0 } else { 0.0 };

        if is_connected {
            let bar_rect = LogicalRect::from_xywh(term_x, term_y, term_w, bar_h);
            builder.fill_rect(bar_rect, Color::rgba(13, 21, 39, 230), 0.0);
            builder.stroke_rect(
                LogicalRect::from_xywh(term_x, term_y + bar_h - 1.0, term_w, 1.0),
                card_border,
                1.0,
                0.0,
            );

            let mut px = term_x + 8.0;
            let pills = [
                (QuickActionCmd::Htop, 78.0),
                (QuickActionCmd::Df, 74.0),
                (QuickActionCmd::Free, 80.0),
                (QuickActionCmd::Uptime, 82.0),
                (QuickActionCmd::Docker, 82.0),
                (QuickActionCmd::Ports, 92.0),
            ];

            for (cmd, pw) in pills {
                if px + pw + 6.0 > term_x + term_w - 180.0 {
                    break;
                }
                let pill_rect = LogicalRect::from_xywh(px, term_y + 4.0, pw, 22.0);
                let is_hover = app.hover == HoverTarget::QuickAction(cmd);
                let pill_bg = if is_hover {
                    Color::rgba(30, 48, 80, 240)
                } else {
                    Color::rgba(18, 29, 52, 200)
                };
                let pill_border = if is_hover {
                    accent_mint
                } else {
                    Color::rgba(45, 212, 191, 50)
                };
                builder.fill_rect(pill_rect, pill_bg, 4.0);
                builder.stroke_rect(pill_rect, pill_border, 1.0, 4.0);

                let label = self.shape_cached(cmd.label(), StyleId::Small, None);
                let text_color = if is_hover {
                    text_main
                } else {
                    Color::rgba(203, 213, 225, 240)
                };
                builder.text(LogicalPoint::new(px + 6.0, term_y + 7.5), label, text_color);

                px += pw + 6.0;
            }

            // Right-aligned actions: Search (Ctrl+F) and Split View
            if term_w >= 320.0 {
                let search_btn_w = 80.0;
                let search_rect = LogicalRect::from_xywh(
                    term_x + term_w - 170.0,
                    term_y + 4.0,
                    search_btn_w,
                    22.0,
                );
                let is_search_hover = app.hover == HoverTarget::TerminalSearchOpenBtn;
                let s_bg = if app.search_state.is_open {
                    Color::rgba(45, 212, 191, 230)
                } else if is_search_hover {
                    Color::rgba(30, 48, 80, 240)
                } else {
                    Color::rgba(18, 29, 52, 200)
                };
                let s_color = if app.search_state.is_open {
                    Color::rgba(8, 13, 25, 255)
                } else if is_search_hover {
                    accent_mint
                } else {
                    text_muted
                };
                builder.fill_rect(search_rect, s_bg, 4.0);
                builder.stroke_rect(
                    search_rect,
                    if app.search_state.is_open {
                        accent_mint
                    } else {
                        card_border
                    },
                    1.0,
                    4.0,
                );
                builder.icon(
                    ICON_SEARCH_ID,
                    LogicalRect::from_xywh(term_x + term_w - 165.0, term_y + 8.0, 14.0, 14.0),
                    s_color,
                );
                let s_txt = self.shape_cached("Поиск", StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(term_x + term_w - 146.0, term_y + 7.5),
                    s_txt,
                    s_color,
                );

                let split_btn_w = 76.0;
                let split_rect =
                    LogicalRect::from_xywh(term_x + term_w - 84.0, term_y + 4.0, split_btn_w, 22.0);
                let is_split_hover = app.hover == HoverTarget::TerminalSplitBtn;
                let is_split_active = app.split_tab.is_some();
                let sp_bg = if is_split_active {
                    Color::rgba(45, 212, 191, 230)
                } else if is_split_hover {
                    Color::rgba(30, 48, 80, 240)
                } else {
                    Color::rgba(18, 29, 52, 200)
                };
                let sp_color = if is_split_active {
                    Color::rgba(8, 13, 25, 255)
                } else if is_split_hover {
                    accent_mint
                } else {
                    text_muted
                };
                builder.fill_rect(split_rect, sp_bg, 4.0);
                builder.stroke_rect(
                    split_rect,
                    if is_split_active {
                        accent_mint
                    } else {
                        card_border
                    },
                    1.0,
                    4.0,
                );
                builder.icon(
                    ICON_SPLIT_ID,
                    LogicalRect::from_xywh(term_x + term_w - 79.0, term_y + 8.0, 14.0, 14.0),
                    sp_color,
                );
                let sp_txt = self.shape_cached("Сплит", StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(term_x + term_w - 60.0, term_y + 7.5),
                    sp_txt,
                    sp_color,
                );
            }
        }

        // 2. Terminal Text Viewport Area
        let banner_h = if !is_connected { 46.0 } else { 0.0 };
        let content_y = term_y + bar_h;
        let content_h = (term_h - bar_h - banner_h).max(0.0);
        let pad_x = term_x + 10.0;
        let pad_y = content_y + 6.0;

        let max_visible_rows = ((content_h - 12.0) / line_h).floor().max(1.0) as usize;
        let rows_to_render = tab.terminal.rows().min(max_visible_rows);
        let total_sb = tab.terminal.scrollback_len();
        let scroll_offset = tab.terminal.scroll_offset();

        for visual_y in 0..rows_to_render {
            if let Some(row_cells) = tab.terminal.get_row(visual_y) {
                let row_has_selection = if is_active_panel {
                    app.selection.as_ref().map_or(false, |sel| {
                        let (p1, p2) = sel.normalized();
                        visual_y >= p1.row && visual_y <= p2.row
                    })
                } else {
                    false
                };

                let current_abs_row = total_sb.saturating_sub(scroll_offset) + visual_y;

                let last_occupied_x = row_cells
                    .iter()
                    .rposition(|c| c.ch != ' ' || c.bg != DEFAULT_TERM_BG);
                if last_occupied_x.is_none() && !row_has_selection {
                    continue;
                }

                let end_x = if row_has_selection {
                    row_cells.len()
                } else {
                    last_occupied_x.unwrap_or(0) + 1
                };

                let mut run_str = String::new();
                let mut run_start_x = 0;
                let mut current_fg = DEFAULT_TERM_FG;
                let mut current_bg = DEFAULT_TERM_BG;

                for (x, cell) in row_cells[..end_x].iter().enumerate() {
                    let is_selected = row_has_selection
                        && app
                            .selection
                            .as_ref()
                            .map_or(false, |sel| sel.contains(x, visual_y));

                    // Search match detection
                    let mut is_search_match = false;
                    let mut is_active_search_match = false;
                    if is_active_panel
                        && app.search_state.is_open
                        && !app.search_state.matches.is_empty()
                    {
                        for (m_idx, m) in app.search_state.matches.iter().enumerate() {
                            if m.abs_row == current_abs_row && x >= m.col_start && x < m.col_end {
                                is_search_match = true;
                                if m_idx == app.search_state.selected_match_idx {
                                    is_active_search_match = true;
                                }
                                break;
                            }
                        }
                    }

                    if cell.bg != DEFAULT_TERM_BG || is_selected || is_search_match {
                        let cell_rect = LogicalRect::from_xywh(
                            pad_x + (x as f64) * char_w,
                            pad_y + (visual_y as f64) * line_h,
                            char_w,
                            line_h,
                        );

                        if is_active_search_match {
                            builder.fill_rect(cell_rect, Color::rgba(249, 115, 22, 230), 1.0);
                        } else if is_search_match {
                            builder.fill_rect(cell_rect, Color::rgba(234, 179, 8, 160), 1.0);
                        } else if cell.bg != DEFAULT_TERM_BG {
                            builder.fill_rect(cell_rect, cell.bg, 0.0);
                        }

                        if is_selected {
                            builder.fill_rect(cell_rect, Color::rgba(45, 212, 191, 75), 0.0);
                        }
                    }

                    let cell_fg = if is_active_search_match || is_search_match {
                        Color::rgba(15, 23, 42, 255)
                    } else {
                        cell.fg
                    };

                    if run_str.is_empty() {
                        run_start_x = x;
                        current_fg = cell_fg;
                        current_bg = cell.bg;
                        run_str.push(cell.ch);
                    } else if cell_fg == current_fg && cell.bg == current_bg {
                        run_str.push(cell.ch);
                    } else {
                        if !run_str.trim().is_empty() {
                            let layout = self.shape_cached(&run_str, StyleId::Mono, None);
                            builder.text(
                                LogicalPoint::new(
                                    pad_x + (run_start_x as f64) * char_w,
                                    pad_y + (visual_y as f64) * line_h,
                                ),
                                layout,
                                current_fg,
                            );
                        }
                        run_str.clear();
                        run_start_x = x;
                        current_fg = cell_fg;
                        current_bg = cell.bg;
                        run_str.push(cell.ch);
                    }
                }

                if !run_str.trim().is_empty() {
                    let layout = self.shape_cached(&run_str, StyleId::Mono, None);
                    builder.text(
                        LogicalPoint::new(
                            pad_x + (run_start_x as f64) * char_w,
                            pad_y + (visual_y as f64) * line_h,
                        ),
                        layout,
                        current_fg,
                    );
                }
            }
        }

        // Draw Cursor (blinking vertical bar)
        let (cur_x, cur_y) = tab.terminal.cursor_pos();
        if is_active_panel
            && tab.terminal.screen.cursor_visible
            && app.caret_visible
            && scroll_offset == 0
            && cur_y < rows_to_render
        {
            let cursor_x = pad_x + (cur_x as f64) * char_w;
            let cursor_y = pad_y + (cur_y as f64) * line_h;
            let cursor_rect = LogicalRect::from_xywh(cursor_x, cursor_y + 1.0, 2.0, line_h - 2.0);
            builder.fill_rect(cursor_rect, Color::rgba(45, 212, 191, 230), 1.0);
        }

        // 3. Reconnect Banner at bottom (if disconnected)
        if !is_connected {
            let banner_rect =
                LogicalRect::from_xywh(term_x + 8.0, term_y + term_h - 44.0, term_w - 16.0, 38.0);
            let is_error = tab.status.contains("🔴") || tab.status.contains("Ошибка");
            let border_color = if is_error {
                Color::rgba(239, 68, 68, 140)
            } else {
                Color::rgba(45, 212, 191, 120)
            };
            builder.fill_rect(banner_rect, Color::rgba(18, 29, 52, 240), 6.0);
            builder.stroke_rect(banner_rect, border_color, 1.2, 6.0);

            // Close button (positioned right-aligned)
            let is_close_hover = app.hover == HoverTarget::TerminalCloseTab(tab_idx);
            let close_btn_w = if term_w >= 480.0 { 92.0 } else { 32.0 };
            let close_x = term_x + term_w - 12.0 - close_btn_w;
            let close_rect =
                LogicalRect::from_xywh(close_x, term_y + term_h - 39.0, close_btn_w, 28.0);
            let close_bg = if is_close_hover {
                Color::rgba(40, 58, 92, 240)
            } else {
                Color::rgba(25, 38, 65, 220)
            };
            builder.fill_rect(close_rect, close_bg, 5.0);
            builder.stroke_rect(close_rect, card_border, 1.0, 5.0);
            if term_w >= 480.0 {
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(close_x + 8.0, term_y + term_h - 32.0, 14.0, 14.0),
                    text_muted,
                );
                let close_lbl = self.shape_cached("Закрыть", StyleId::Sans, None);
                builder.text(
                    LogicalPoint::new(close_x + 26.0, term_y + term_h - 34.0),
                    close_lbl,
                    text_main,
                );
            } else {
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(close_x + 9.0, term_y + term_h - 32.0, 14.0, 14.0),
                    text_main,
                );
            }

            // Reconnect button (positioned to the left of Close button)
            let is_recon_hover = app.hover == HoverTarget::TerminalReconnect(tab_idx);
            let (recon_btn_w, recon_label) = if term_w >= 540.0 {
                (180.0, Some("Переподключиться"))
            } else if term_w >= 360.0 {
                (105.0, Some("Повторить"))
            } else {
                (32.0, None)
            };
            let recon_x = close_x - 6.0 - recon_btn_w;
            let recon_rect =
                LogicalRect::from_xywh(recon_x, term_y + term_h - 39.0, recon_btn_w, 28.0);
            let recon_bg = if is_recon_hover {
                Color::rgba(52, 211, 153, 255)
            } else {
                accent_mint
            };
            builder.fill_rect(recon_rect, recon_bg, 5.0);
            if let Some(lbl_text) = recon_label {
                builder.icon(
                    ICON_REFRESH_ID,
                    LogicalRect::from_xywh(recon_x + 8.0, term_y + term_h - 33.0, 16.0, 16.0),
                    Color::rgba(8, 13, 25, 255),
                );
                let recon_lbl = self.shape_cached(lbl_text, StyleId::SansBold, None);
                builder.text(
                    LogicalPoint::new(recon_x + 28.0, term_y + term_h - 34.0),
                    recon_lbl,
                    Color::rgba(8, 13, 25, 255),
                );
            } else {
                builder.icon(
                    ICON_REFRESH_ID,
                    LogicalRect::from_xywh(recon_x + 8.0, term_y + term_h - 33.0, 16.0, 16.0),
                    Color::rgba(8, 13, 25, 255),
                );
            }

            // Clean up leading emoji icons if present (since alert/refresh vector icon is already displayed)
            let clean_status = tab.status.trim_start_matches(|c: char| {
                c == '🔴' || c == '🟢' || c == '⚪' || c == '🟡' || c == ' '
            });

            // Icon + Status (strictly single-line clamped to prevent wrapping and vertical spillover)
            builder.icon(
                if is_error {
                    ICON_ALERT_ID
                } else {
                    ICON_REFRESH_ID
                },
                LogicalRect::from_xywh(term_x + 18.0, term_y + term_h - 34.0, 18.0, 18.0),
                if is_error {
                    Color::rgba(248, 113, 113, 240)
                } else {
                    accent_mint
                },
            );
            let max_status_w = (recon_x - (term_x + 42.0) - 8.0).max(20.0);
            let status_txt = self.shape_clamped_cached(
                clean_status,
                StyleId::SansBold,
                LogicalPx(max_status_w),
                1,
            );
            builder.text(
                LogicalPoint::new(term_x + 42.0, term_y + term_h - 33.0),
                status_txt,
                text_main,
            );
        }

        // 4. Scrollbar
        let sb_w = 7.0;
        let sb_x = term_x + term_w - sb_w - 3.0;
        let sb_y = content_y + 4.0;
        let sb_h = (content_h - 8.0).max(10.0);

        let rows = tab.terminal.rows();
        let track_rect = LogicalRect::from_xywh(sb_x, sb_y, sb_w, sb_h);
        builder.fill_rect(track_rect, Color::rgba(14, 22, 38, 120), 3.0);

        let (thumb_y, thumb_h) = if total_sb > 0 && sb_h > 0.0 {
            let total_lines = total_sb + rows;
            let visible_ratio = if total_lines > 0 {
                (rows as f64) / (total_lines as f64)
            } else {
                1.0
            };
            let min_th = 14.0f64.min(sb_h);
            let th = (sb_h * visible_ratio).max(min_th).min(sb_h);
            let max_scroll_offset = total_sb as f64;
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

        let is_thumb_hovered = is_active_panel
            && (app.hover == HoverTarget::TerminalScrollbarThumb || app.is_dragging_scrollbar);
        let thumb_rect = LogicalRect::from_xywh(sb_x, thumb_y, sb_w, thumb_h);
        let thumb_color = if is_thumb_hovered {
            accent_mint
        } else if total_sb > 0 {
            Color::rgba(100, 116, 139, 140)
        } else {
            Color::rgba(60, 75, 95, 70)
        };
        builder.fill_rect(thumb_rect, thumb_color, 3.0);
    }

    fn render_terminal_search_widget(
        &mut self,
        builder: &mut PaintBuilder,
        app: &AppState,
        term_rect: LogicalRect,
        accent_mint: Color,
        card_border: Color,
        text_main: Color,
        text_muted: Color,
    ) {
        let sw_w = 360.0;
        let sw_h = 36.0;
        let sw_x = term_rect.origin.x.0 + term_rect.size.width.0 - sw_w - 20.0;
        let sw_y = term_rect.origin.y.0 + 8.0;

        let box_rect = LogicalRect::from_xywh(sw_x, sw_y, sw_w, sw_h);
        builder.fill_rect(box_rect, Color::rgba(15, 25, 48, 250), 6.0);
        builder.stroke_rect(box_rect, Color::rgba(45, 212, 191, 160), 1.2, 6.0);

        // Input Box for query
        let inp_w = 175.0;
        let inp_rect = LogicalRect::from_xywh(sw_x + 6.0, sw_y + 4.0, inp_w, 28.0);
        let is_input_focused = app.focused_input == Some(InputFieldId::TerminalSearch)
            || app.hover == HoverTarget::TerminalSearchInput;
        self.draw_input_box(
            builder,
            &app.search_state.query,
            inp_rect,
            is_input_focused,
            app.hover == HoverTarget::TerminalSearchInput,
            "Поиск в терминале...",
            false,
            app.caret_visible,
            accent_mint,
            card_border,
            text_main,
            text_muted,
        );

        // Match count indicator
        let count_str = if app.search_state.matches.is_empty() {
            if app.search_state.query.text.trim().is_empty() {
                String::new()
            } else {
                "0 совп.".into()
            }
        } else {
            format!(
                "{}/{}",
                app.search_state.selected_match_idx + 1,
                app.search_state.matches.len()
            )
        };

        if !count_str.is_empty() {
            let count_layout = self.shape_cached(&count_str, StyleId::Small, None);
            let color = if app.search_state.matches.is_empty() {
                Color::rgba(248, 113, 113, 230)
            } else {
                accent_mint
            };
            builder.text(
                LogicalPoint::new(sw_x + inp_w + 12.0, sw_y + 10.0),
                count_layout,
                color,
            );
        }

        // Case sensitivity toggle button [Aa]
        let is_case_hover = app.hover == HoverTarget::TerminalSearchCaseToggle;
        let case_rect = LogicalRect::from_xywh(sw_x + sw_w - 110.0, sw_y + 5.0, 24.0, 26.0);
        let case_bg = if app.search_state.case_sensitive {
            Color::rgba(45, 212, 191, 220)
        } else if is_case_hover {
            Color::rgba(30, 48, 80, 240)
        } else {
            Color::rgba(20, 32, 58, 180)
        };
        let case_text_color = if app.search_state.case_sensitive {
            Color::rgba(8, 13, 25, 255)
        } else {
            text_muted
        };
        builder.fill_rect(case_rect, case_bg, 4.0);
        builder.stroke_rect(case_rect, card_border, 1.0, 4.0);
        let case_layout = self.shape_cached("Aa", StyleId::Small, None);
        builder.text(
            LogicalPoint::new(sw_x + sw_w - 105.0, sw_y + 9.5),
            case_layout,
            case_text_color,
        );

        // Previous button [▲]
        let is_prev_hover = app.hover == HoverTarget::TerminalSearchPrevBtn;
        let prev_rect = LogicalRect::from_xywh(sw_x + sw_w - 82.0, sw_y + 5.0, 24.0, 26.0);
        let prev_bg = if is_prev_hover {
            Color::rgba(30, 48, 80, 240)
        } else {
            Color::rgba(20, 32, 58, 180)
        };
        builder.fill_rect(prev_rect, prev_bg, 4.0);
        builder.stroke_rect(prev_rect, card_border, 1.0, 4.0);
        builder.icon(
            ICON_UP_ID,
            LogicalRect::from_xywh(sw_x + sw_w - 77.0, sw_y + 10.0, 14.0, 14.0),
            if is_prev_hover {
                accent_mint
            } else {
                text_muted
            },
        );

        // Next button [▼]
        let is_next_hover = app.hover == HoverTarget::TerminalSearchNextBtn;
        let next_rect = LogicalRect::from_xywh(sw_x + sw_w - 54.0, sw_y + 5.0, 24.0, 26.0);
        let next_bg = if is_next_hover {
            Color::rgba(30, 48, 80, 240)
        } else {
            Color::rgba(20, 32, 58, 180)
        };
        builder.fill_rect(next_rect, next_bg, 4.0);
        builder.stroke_rect(next_rect, card_border, 1.0, 4.0);
        builder.icon(
            ICON_DOWN_ID,
            LogicalRect::from_xywh(sw_x + sw_w - 49.0, sw_y + 10.0, 14.0, 14.0),
            if is_next_hover {
                accent_mint
            } else {
                text_muted
            },
        );

        // Close button [✕]
        let is_close_hover = app.hover == HoverTarget::TerminalSearchCloseBtn;
        let close_rect = LogicalRect::from_xywh(sw_x + sw_w - 26.0, sw_y + 5.0, 22.0, 26.0);
        let close_bg = if is_close_hover {
            Color::rgba(239, 68, 68, 220)
        } else {
            Color::rgba(20, 32, 58, 180)
        };
        builder.fill_rect(close_rect, close_bg, 4.0);
        builder.stroke_rect(close_rect, card_border, 1.0, 4.0);
        builder.icon(
            ICON_CLOSE_ID,
            LogicalRect::from_xywh(sw_x + sw_w - 22.0, sw_y + 10.0, 14.0, 14.0),
            if is_close_hover {
                Color::rgba(255, 255, 255, 255)
            } else {
                text_muted
            },
        );
    }

    fn try_load_logo(renderer: &mut Rgba8Renderer) -> bool {
        const LOGO_BYTES: &[u8] = include_bytes!("../Logo.png");
        if let Ok(img) = image::load_from_memory(LOGO_BYTES) {
            // Pre-downscale using Lanczos3 to avoid bilinear aliasing/blurriness:
            // 1. Medium size (144x144) for Welcome Screen (72x72 @ 2x DPI)
            let large =
                image::imageops::resize(&img, 144, 144, image::imageops::FilterType::Lanczos3);
            let (lw, lh) = large.dimensions();
            let large_icon = BitmapIcon {
                width: lw,
                height: lh,
                rgba: Arc::from(large.into_raw().into_boxed_slice()),
            };
            let _ = renderer.register_icon(LOGO_ICON_ID, large_icon);

            // 2. Small size (52x52) for Header Logo (26x26 @ 2x DPI)
            let small =
                image::imageops::resize(&img, 52, 52, image::imageops::FilterType::Lanczos3);
            let (sw, sh) = small.dimensions();
            let small_icon = BitmapIcon {
                width: sw,
                height: sh,
                rgba: Arc::from(small.into_raw().into_boxed_slice()),
            };
            let _ = renderer.register_icon(LOGO_SMALL_ICON_ID, small_icon);

            return true;
        }
        false
    }

    fn register_vector_icons(renderer: &mut Rgba8Renderer) {
        // 1. Play / Quick Connect Icon (smooth modern triangle)
        let connect_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M8 5.14v13.72a1 1 0 0 0 1.5.86l11-6.86a1 1 0 0 0 0-1.72L9.5 4.28a1 1 0 0 0-1.5.86z"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_CONNECT_ID, connect_svg);

        // 2. SSH Key Icon (clean modern key)
        let key_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="7.5" cy="15.5" r="4.5"/><path d="m21 3-9.5 9.5"/><path d="m15.5 7.5 3 3"/><path d="m18 5 2 2"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_KEY_ID, key_svg);

        // 3. Edit Pencil Icon
        let edit_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_EDIT_ID, edit_svg);

        // 4. Close / Delete Cross Icon
        let close_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_CLOSE_ID, close_svg);

        // 5. Copy Icon
        let copy_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_COPY_ID, copy_svg);

        // 6. Search Icon
        let search_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_SEARCH_ID, search_svg);

        // 7. Plus Icon
        let plus_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_PLUS_ID, plus_svg);

        // 8. Settings Gear Icon
        let settings_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_SETTINGS_ID, settings_svg);

        // 9. Padlock Icon
        let lock_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_LOCK_ID, lock_svg);

        // 10. Warning Alert Icon
        let alert_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_ALERT_ID, alert_svg);

        // 11. Refresh / Reconnect Icon
        let refresh_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/><path d="M16 16h5v5"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_REFRESH_ID, refresh_svg);

        // 12. Split Screen Icon
        let split_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M12 3v18"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_SPLIT_ID, split_svg);

        // 13. Chevron Up Icon
        let up_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m18 15-6-6-6 6"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_UP_ID, up_svg);

        // 14. Chevron Down Icon
        let down_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_DOWN_ID, down_svg);

        // 15. Eye (Show Password)
        let eye_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_EYE_ID, eye_svg);

        // 16. Eye Off (Hide Password)
        let eye_off_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.88 9.88a3 3 0 1 0 4.24 4.24"/><path d="M10.73 5.08A10.43 10.43 0 0 1 12 5c7 0 10 7 10 7a13.16 13.16 0 0 1-1.67 2.68"/><path d="M6.61 6.61A13.526 13.526 0 0 0 2 12s3 7 10 7a9.74 9.74 0 0 0 5.39-1.61"/><line x1="2" y1="2" x2="22" y2="22"/></svg>"#;
        let _ = renderer.register_svg_icon(ICON_EYE_OFF_ID, eye_off_svg);
    }

    pub fn render_frame(
        &mut self,
        app: &AppState,
        scale: DpiScale,
        physical_size: PhysicalSize,
    ) -> Result<&[u8], &'static str> {
        let logical_size = physical_size.to_logical(scale);
        let mut builder = PaintBuilder::with_viewport(LogicalRect::from_xywh(
            0.0,
            0.0,
            logical_size.width.0,
            logical_size.height.0,
        ));

        // Colors matching Logo.png
        let bg_app = Color::rgba(8, 13, 25, 255);
        let bg_header = Color::rgba(13, 21, 39, 255);
        let bg_sidebar = Color::rgba(11, 18, 34, 255);
        let bg_footer = Color::rgba(11, 18, 34, 255);
        let card_bg = Color::rgba(18, 29, 52, 255);
        let card_hover_bg = Color::rgba(25, 41, 72, 255);
        let card_border = Color::rgba(28, 44, 76, 255);
        let text_main = Color::rgba(240, 246, 255, 255);
        let text_dim = Color::rgba(138, 156, 186, 255);
        let text_muted = Color::rgba(90, 108, 138, 255);
        let active_tab_bg = Color::rgba(22, 35, 62, 255);

        let accent_mint = Color::rgba(45, 212, 191, 255);
        let accent_emerald = Color::rgba(52, 211, 153, 255);
        let lang = app.hosts_doc.settings.language;

        // 1. App Background
        builder.fill_rect(
            LogicalRect::from_xywh(0.0, 0.0, logical_size.width.0, logical_size.height.0),
            bg_app,
            0.0,
        );

        let w = logical_size.width.0;
        let h = logical_size.height.0;
        let header_h = app.header_height;
        let footer_h = app.footer_height;
        let sidebar_w = app.sidebar_width;
        let body_h = (h - header_h - footer_h).max(0.0);

        // 2. Header / Tabs Bar
        builder.fill_rect(
            LogicalRect::from_xywh(0.0, 0.0, w, header_h),
            bg_header,
            0.0,
        );
        builder.stroke_rect(
            LogicalRect::from_xywh(0.0, header_h - 1.0, w, 1.0),
            card_border,
            1.0,
            0.0,
        );

        // Logo
        let mut brand_x = 12.0;
        if self.logo_loaded {
            builder.icon(
                LOGO_SMALL_ICON_ID,
                LogicalRect::from_xywh(brand_x, 9.0, 26.0, 26.0),
                Color::rgba(255, 255, 255, 255),
            );
            brand_x += 32.0;
        }

        let logo_everty = self.shape_cached("Everty", StyleId::Title, None);
        builder.text(LogicalPoint::new(brand_x, 11.0), logo_everty, text_main);
        brand_x += 52.0;

        let logo_ssh = self.shape_cached("SSH", StyleId::Title, None);
        builder.text(LogicalPoint::new(brand_x, 11.0), logo_ssh, accent_mint);
        brand_x += 42.0;

        // Settings Gear Button on far right header (with Hover)
        let is_settings_hovered = app.hover == HoverTarget::HeaderSettingsBtn;
        let settings_btn_rect = LogicalRect::from_xywh(w - 46.0, 7.0, 32.0, 30.0);
        builder.fill_rect(
            settings_btn_rect,
            if is_settings_hovered {
                Color::rgba(25, 41, 72, 255)
            } else {
                Color::rgba(18, 29, 52, 255)
            },
            5.0,
        );
        builder.stroke_rect(
            settings_btn_rect,
            if is_settings_hovered {
                accent_mint
            } else {
                card_border
            },
            1.0,
            5.0,
        );
        builder.icon(
            ICON_SETTINGS_ID,
            LogicalRect::from_xywh(w - 38.0, 14.0, 16.0, 16.0),
            if is_settings_hovered {
                accent_mint
            } else {
                text_dim
            },
        );

        // Language Switcher Button in Header (RU / EN)
        let is_lang_hovered = app.hover == HoverTarget::HeaderLangBtn;
        let lang_btn_w = 42.0;
        let lang_btn_x = w - 94.0;
        let lang_btn_rect = LogicalRect::from_xywh(lang_btn_x, 7.0, lang_btn_w, 30.0);
        builder.fill_rect(
            lang_btn_rect,
            if is_lang_hovered {
                Color::rgba(25, 41, 72, 255)
            } else {
                Color::rgba(18, 29, 52, 255)
            },
            5.0,
        );
        builder.stroke_rect(
            lang_btn_rect,
            if is_lang_hovered {
                accent_mint
            } else {
                card_border
            },
            1.0,
            5.0,
        );
        let lang_str = match lang {
            AppLanguage::Ru => "RU",
            AppLanguage::En => "EN",
        };
        let lang_l = self.shape_cached(lang_str, StyleId::SansBold, None);
        builder.text(
            LogicalPoint::new(lang_btn_x + 11.0, 13.0),
            lang_l,
            if is_lang_hovered {
                accent_mint
            } else {
                text_main
            },
        );

        // Search Button on right header
        let compact_search = w < 880.0 || (app.tabs.len() >= 3 && w < 1050.0);
        let search_btn_w = if compact_search { 32.0 } else { 150.0 };
        let search_btn_x = w - 100.0 - search_btn_w;
        let search_btn_rect = LogicalRect::from_xywh(search_btn_x, 7.0, search_btn_w, 30.0);
        let is_search_hovered = app.hover == HoverTarget::HeaderSearchBtn;

        builder.fill_rect(
            search_btn_rect,
            if is_search_hovered {
                Color::rgba(25, 41, 72, 255)
            } else {
                Color::rgba(18, 29, 52, 255)
            },
            5.0,
        );
        builder.stroke_rect(
            search_btn_rect,
            if is_search_hovered {
                accent_mint
            } else {
                card_border
            },
            1.0,
            5.0,
        );
        if compact_search {
            builder.icon(
                ICON_SEARCH_ID,
                LogicalRect::from_xywh(search_btn_x + 9.0, 15.0, 14.0, 14.0),
                if is_search_hovered {
                    accent_mint
                } else {
                    text_dim
                },
            );
        } else {
            builder.icon(
                ICON_SEARCH_ID,
                LogicalRect::from_xywh(search_btn_x + 12.0, 15.0, 14.0, 14.0),
                if is_search_hovered {
                    accent_mint
                } else {
                    text_dim
                },
            );
            let search_hint_text = tr(lang, "Ctrl+R Команды", "Ctrl+R Commands");
            let search_hint = self.shape_cached(search_hint_text, StyleId::Small, None);
            builder.text(
                LogicalPoint::new(search_btn_x + 32.0, 14.0),
                search_hint,
                if is_search_hovered {
                    text_main
                } else {
                    text_dim
                },
            );
        }

        // Tabs
        let tab_start_x = brand_x.max(150.0);
        let compact_add = w < 720.0 || (app.tabs.len() >= 3 && w < 920.0);
        let add_btn_w = if compact_add { 32.0 } else { 110.0 };
        let max_tabs_w = (search_btn_x - 10.0 - tab_start_x - add_btn_w - 6.0).max(60.0);
        let num_tabs = app.tabs.len().max(1);
        let tab_w =
            ((max_tabs_w - (num_tabs - 1) as f64 * 6.0) / num_tabs as f64).clamp(60.0, 175.0);

        let mut tab_x = tab_start_x;
        for (i, tab) in app.tabs.iter().enumerate() {
            if tab_x + tab_w > search_btn_x - 4.0 {
                break;
            }
            let is_active = i == app.active_tab;
            let is_hovered = app.hover == HoverTarget::HeaderTab(i);
            let is_close_hovered = app.hover == HoverTarget::HeaderTabClose(i);

            let tab_rect = LogicalRect::from_xywh(tab_x, 7.0, tab_w, 30.0);

            let tab_bg = if is_active {
                active_tab_bg
            } else if is_hovered {
                Color::rgba(20, 32, 58, 255)
            } else {
                Color::rgba(16, 25, 46, 255)
            };
            builder.fill_rect(tab_rect, tab_bg, 5.0);
            if is_active {
                builder.stroke_rect(tab_rect, accent_mint, 1.2, 5.0);
            } else if is_hovered {
                builder.stroke_rect(tab_rect, Color::rgba(45, 212, 191, 140), 1.0, 5.0);
            } else {
                builder.stroke_rect(tab_rect, card_border, 1.0, 5.0);
            }

            let dot_color = if tab.status.contains("Connected") {
                accent_emerald
            } else if tab.status.contains("Connecting") {
                Color::rgba(251, 191, 36, 255)
            } else {
                Color::rgba(148, 163, 184, 255)
            };
            builder.fill_rect(
                LogicalRect::from_xywh(tab_x + 10.0, 18.0, 8.0, 8.0),
                dot_color,
                4.0,
            );

            let max_title_w = (tab_w - 50.0).max(10.0);
            let title_layout =
                self.shape_clamped_cached(&tab.title, StyleId::Sans, LogicalPx(max_title_w), 1);
            builder.push_clip(LogicalRect::from_xywh(
                tab_x + 22.0,
                7.0,
                max_title_w + 2.0,
                30.0,
            ));
            builder.text(
                LogicalPoint::new(tab_x + 24.0, 13.0),
                title_layout,
                if is_active { text_main } else { text_dim },
            );
            let _ = builder.pop_clip();

            // Tab Close '×' with vector icon & hover badge
            let close_rect = LogicalRect::from_xywh(tab_x + tab_w - 24.0, 12.0, 18.0, 18.0);
            if is_close_hovered {
                builder.fill_rect(close_rect, Color::rgba(239, 68, 68, 200), 4.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(tab_x + tab_w - 21.0, 15.0, 12.0, 12.0),
                    Color::rgba(255, 255, 255, 255),
                );
            } else {
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(tab_x + tab_w - 21.0, 15.0, 12.0, 12.0),
                    text_muted,
                );
            }

            tab_x += tab_w + 6.0;
        }

        // "+ Добавить сервер" Button in Header (with Hover)
        if tab_x + add_btn_w <= search_btn_x - 4.0 {
            let is_add_hovered = app.hover == HoverTarget::HeaderAddBtn;
            let add_btn_rect = LogicalRect::from_xywh(tab_x, 7.0, add_btn_w, 30.0);

            if is_add_hovered {
                builder.fill_rect(add_btn_rect, accent_mint, 5.0);
                if compact_add {
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(tab_x + 9.0, 15.0, 14.0, 14.0),
                        Color::rgba(9, 14, 26, 255),
                    );
                } else {
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(tab_x + 10.0, 15.0, 14.0, 14.0),
                        Color::rgba(9, 14, 26, 255),
                    );
                    let add_label =
                        self.shape_cached(tr(lang, "Добавить", "Add"), StyleId::SansBold, None);
                    builder.text(
                        LogicalPoint::new(tab_x + 28.0, 13.0),
                        add_label,
                        Color::rgba(9, 14, 26, 255),
                    );
                }
            } else {
                builder.fill_rect(add_btn_rect, Color::rgba(20, 36, 60, 255), 5.0);
                builder.stroke_rect(add_btn_rect, accent_mint, 1.0, 5.0);
                if compact_add {
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(tab_x + 9.0, 15.0, 14.0, 14.0),
                        accent_mint,
                    );
                } else {
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(tab_x + 10.0, 15.0, 14.0, 14.0),
                        accent_mint,
                    );
                    let add_label =
                        self.shape_cached(tr(lang, "Добавить", "Add"), StyleId::SansBold, None);
                    builder.text(
                        LogicalPoint::new(tab_x + 28.0, 13.0),
                        add_label,
                        accent_mint,
                    );
                }
            }
        }

        // 3. Sidebar (Left Pane)
        let sidebar_rect = LogicalRect::from_xywh(0.0, header_h, sidebar_w, body_h);
        builder.fill_rect(sidebar_rect, bg_sidebar, 0.0);
        // Sidebar Splitter Border / Drag Handle
        let is_splitter_active =
            app.hover == HoverTarget::SidebarSplitter || app.is_dragging_sidebar;
        if is_splitter_active {
            builder.fill_rect(
                LogicalRect::from_xywh(sidebar_w - 1.5, header_h, 3.0, body_h),
                accent_mint,
                1.0,
            );
        } else {
            builder.stroke_rect(
                LogicalRect::from_xywh(sidebar_w - 1.0, header_h, 1.0, body_h),
                card_border,
                1.0,
                0.0,
            );
        }

        // Sidebar Navigation Tabs (with Hover)
        let nav_h = 38.0;
        let nav_btn_w = sidebar_w / 4.0;
        let nav_items = [
            (SidebarTab::Hosts, tr(lang, "Серверы", "Hosts")),
            (SidebarTab::Sessions, tr(lang, "Сессии", "Sessions")),
            (SidebarTab::Commands, tr(lang, "История", "History")),
            (SidebarTab::Snippets, tr(lang, "Сниппеты", "Snippets")),
        ];

        for (i, (tab_kind, label)) in nav_items.iter().enumerate() {
            let x = (i as f64) * nav_btn_w;
            let is_sel = app.sidebar_tab == *tab_kind;
            let is_nav_hover = app.hover == HoverTarget::SidebarNav(*tab_kind);
            let rect = LogicalRect::from_xywh(x, header_h, nav_btn_w, nav_h);

            if is_sel {
                builder.fill_rect(rect, Color::rgba(22, 35, 62, 255), 0.0);
                builder.stroke_rect(
                    LogicalRect::from_xywh(x, header_h + nav_h - 2.0, nav_btn_w, 2.0),
                    accent_mint,
                    2.0,
                    0.0,
                );
            } else if is_nav_hover {
                builder.fill_rect(rect, Color::rgba(18, 29, 52, 180), 0.0);
            }

            let text_layout = self.shape_cached(label, StyleId::Small, None);
            builder.text(
                LogicalPoint::new(x + 12.0, header_h + 11.0),
                text_layout,
                if is_sel || is_nav_hover {
                    text_main
                } else {
                    text_dim
                },
            );
        }

        // Sidebar Content
        let content_y_start = header_h + nav_h + 8.0;
        match app.sidebar_tab {
            SidebarTab::Hosts => {
                let is_search_focused = app.focused_input == Some(InputFieldId::HostSearch);
                let is_search_hover = app.hover == HoverTarget::SidebarSearchHost;
                let search_rect =
                    LogicalRect::from_xywh(10.0, content_y_start, sidebar_w - 96.0, 32.0);
                self.draw_input_box(
                    &mut builder,
                    &app.host_search_input,
                    search_rect,
                    is_search_focused,
                    is_search_hover,
                    tr(lang, "Поиск...", "Search..."),
                    false,
                    app.caret_visible && is_search_focused,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                let is_add_host_btn_hover = app.hover == HoverTarget::SidebarAddHostBtn;
                let add_btn_rect =
                    LogicalRect::from_xywh(sidebar_w - 82.0, content_y_start, 72.0, 32.0);
                let btn_text_str = tr(lang, "Новый", "+ New");

                if is_add_host_btn_hover {
                    builder.fill_rect(add_btn_rect, accent_mint, 5.0);
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(sidebar_w - 76.0, content_y_start + 9.0, 14.0, 14.0),
                        Color::rgba(9, 14, 26, 255),
                    );
                    let btn_text = self.shape_cached(btn_text_str, StyleId::Small, None);
                    builder.text(
                        LogicalPoint::new(sidebar_w - 58.0, content_y_start + 7.0),
                        btn_text,
                        Color::rgba(9, 14, 26, 255),
                    );
                } else {
                    builder.fill_rect(add_btn_rect, Color::rgba(22, 38, 70, 200), 5.0);
                    builder.stroke_rect(add_btn_rect, accent_mint, 1.0, 5.0);
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(sidebar_w - 76.0, content_y_start + 9.0, 14.0, 14.0),
                        accent_mint,
                    );
                    let btn_text = self.shape_cached(btn_text_str, StyleId::Small, None);
                    builder.text(
                        LogicalPoint::new(sidebar_w - 58.0, content_y_start + 7.0),
                        btn_text,
                        accent_mint,
                    );
                }

                let filtered = app.filtered_hosts();
                if filtered.is_empty() {
                    let empty_y = content_y_start + 50.0;
                    let msg = if app.hosts_doc.hosts.is_empty() {
                        tr(lang, "Нет серверов", "No servers")
                    } else {
                        tr(lang, "Не найдено", "Not found")
                    };
                    let empty_text = self.shape_cached(msg, StyleId::SansBold, None);
                    builder.text(LogicalPoint::new(20.0, empty_y), empty_text, text_muted);

                    let sub_msg = if app.hosts_doc.hosts.is_empty() {
                        tr(
                            lang,
                            "Нажмите '+ Новый' чтобы добавить",
                            "Click '+ New' to add a host",
                        )
                    } else {
                        tr(
                            lang,
                            "Попробуйте другой запрос",
                            "Try a different search query",
                        )
                    };
                    let sub_text = self.shape_cached(sub_msg, StyleId::Small, None);
                    builder.text(LogicalPoint::new(20.0, empty_y + 22.0), sub_text, text_dim);
                } else {
                    let mut item_y = content_y_start + 42.0;
                    for &(host_idx, host) in &filtered {
                        let is_card_hover = app.hover == HoverTarget::HostCard(host_idx)
                            || app.hover == HoverTarget::HostConnect(host_idx)
                            || app.hover == HoverTarget::HostDeployKey(host_idx)
                            || app.hover == HoverTarget::HostEdit(host_idx)
                            || app.hover == HoverTarget::HostDelete(host_idx);

                        let is_conn_hover = app.hover == HoverTarget::HostConnect(host_idx);
                        let is_deploy_hover = app.hover == HoverTarget::HostDeployKey(host_idx);
                        let is_edit_hover = app.hover == HoverTarget::HostEdit(host_idx);
                        let is_del_hover = app.hover == HoverTarget::HostDelete(host_idx);

                        let item_rect =
                            LogicalRect::from_xywh(10.0, item_y, sidebar_w - 20.0, 56.0);
                        builder.fill_rect(
                            item_rect,
                            if is_card_hover {
                                card_hover_bg
                            } else {
                                card_bg
                            },
                            6.0,
                        );
                        builder.stroke_rect(
                            item_rect,
                            if is_card_hover {
                                accent_mint
                            } else {
                                card_border
                            },
                            if is_card_hover { 1.2 } else { 1.0 },
                            6.0,
                        );

                        let badge_color = match host.badge_color.as_deref() {
                            Some("#ef4444") => Color::rgba(239, 68, 68, 255),
                            Some("#fbbf24") => Color::rgba(251, 191, 36, 255),
                            Some("#a855f7") => Color::rgba(168, 85, 247, 255),
                            Some("#3b82f6") => Color::rgba(59, 130, 246, 255),
                            _ => accent_mint,
                        };
                        builder.fill_rect(
                            LogicalRect::from_xywh(10.0, item_y, 4.0, 56.0),
                            badge_color,
                            2.0,
                        );

                        let max_title_w = (sidebar_w - 130.0).max(30.0);
                        let name_layout = self.shape_clamped_cached(
                            &host.name,
                            StyleId::SansBold,
                            LogicalPx(max_title_w),
                            1,
                        );
                        let name_w = name_layout.metrics.size.width.0;
                        builder.push_clip(LogicalRect::from_xywh(
                            20.0,
                            item_y + 4.0,
                            max_title_w + 4.0,
                            22.0,
                        ));
                        builder.text(
                            LogicalPoint::new(22.0, item_y + 8.0),
                            name_layout,
                            text_main,
                        );
                        let _ = builder.pop_clip();

                        if let Some(grp) = &host.group {
                            if !grp.is_empty() && name_w + 22.0 + 40.0 < sidebar_w - 128.0 {
                                let max_grp_w =
                                    (sidebar_w - 128.0 - (name_w + 22.0 + 6.0)).max(20.0);
                                let grp_str = format!("[{}]", grp);
                                let grp_layout = self.shape_clamped_cached(
                                    &grp_str,
                                    StyleId::Small,
                                    LogicalPx(max_grp_w),
                                    1,
                                );
                                builder.text(
                                    LogicalPoint::new(22.0 + name_w + 6.0, item_y + 9.0),
                                    grp_layout,
                                    Color::rgba(148, 163, 184, 200),
                                );
                            }
                        }

                        let max_sub_w = (sidebar_w - 130.0).max(30.0);
                        let sub_str = format!("{}@{}:{}", host.username, host.hostname, host.port);
                        let sub_layout = self.shape_clamped_cached(
                            &sub_str,
                            StyleId::Small,
                            LogicalPx(max_sub_w),
                            1,
                        );
                        builder.push_clip(LogicalRect::from_xywh(
                            20.0,
                            item_y + 26.0,
                            max_sub_w + 4.0,
                            22.0,
                        ));
                        builder.text(LogicalPoint::new(22.0, item_y + 28.0), sub_layout, text_dim);
                        let _ = builder.pop_clip();

                        // 1. Quick Connect Button (Play Vector Icon)
                        let conn_rect =
                            LogicalRect::from_xywh(sidebar_w - 118.0, item_y + 16.0, 24.0, 24.0);
                        if is_conn_hover {
                            builder.fill_rect(conn_rect, accent_mint, 5.0);
                            builder.icon(
                                ICON_CONNECT_ID,
                                LogicalRect::from_xywh(
                                    sidebar_w - 113.0,
                                    item_y + 21.0,
                                    14.0,
                                    14.0,
                                ),
                                Color::rgba(8, 13, 25, 255),
                            );
                        } else if is_card_hover {
                            builder.fill_rect(conn_rect, Color::rgba(20, 42, 68, 255), 5.0);
                            builder.stroke_rect(conn_rect, accent_mint, 1.0, 5.0);
                            builder.icon(
                                ICON_CONNECT_ID,
                                LogicalRect::from_xywh(
                                    sidebar_w - 113.0,
                                    item_y + 21.0,
                                    14.0,
                                    14.0,
                                ),
                                accent_mint,
                            );
                        } else {
                            builder.fill_rect(conn_rect, Color::rgba(14, 24, 42, 180), 5.0);
                            builder.stroke_rect(conn_rect, Color::rgba(45, 212, 191, 50), 1.0, 5.0);
                            builder.icon(
                                ICON_CONNECT_ID,
                                LogicalRect::from_xywh(
                                    sidebar_w - 113.0,
                                    item_y + 21.0,
                                    14.0,
                                    14.0,
                                ),
                                Color::rgba(148, 163, 184, 200),
                            );
                        }

                        // 2. Key Deploy Button (SSH Key Vector Icon)
                        let key_rect =
                            LogicalRect::from_xywh(sidebar_w - 90.0, item_y + 16.0, 24.0, 24.0);
                        if is_deploy_hover {
                            builder.fill_rect(key_rect, Color::rgba(245, 158, 11, 255), 5.0);
                            builder.icon(
                                ICON_KEY_ID,
                                LogicalRect::from_xywh(sidebar_w - 85.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(8, 13, 25, 255),
                            );
                        } else if is_card_hover {
                            builder.fill_rect(key_rect, Color::rgba(42, 32, 18, 240), 5.0);
                            builder.stroke_rect(key_rect, Color::rgba(251, 191, 36, 160), 1.0, 5.0);
                            builder.icon(
                                ICON_KEY_ID,
                                LogicalRect::from_xywh(sidebar_w - 85.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(251, 191, 36, 240),
                            );
                        } else {
                            builder.fill_rect(key_rect, Color::rgba(28, 22, 14, 180), 5.0);
                            builder.stroke_rect(key_rect, Color::rgba(251, 191, 36, 60), 1.0, 5.0);
                            builder.icon(
                                ICON_KEY_ID,
                                LogicalRect::from_xywh(sidebar_w - 85.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(180, 150, 100, 180),
                            );
                        }

                        // 3. Edit Button (Pencil Vector Icon)
                        let edit_rect =
                            LogicalRect::from_xywh(sidebar_w - 62.0, item_y + 16.0, 24.0, 24.0);
                        if is_edit_hover {
                            builder.fill_rect(edit_rect, Color::rgba(59, 130, 246, 255), 5.0);
                            builder.icon(
                                ICON_EDIT_ID,
                                LogicalRect::from_xywh(sidebar_w - 57.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(8, 13, 25, 255),
                            );
                        } else if is_card_hover {
                            builder.fill_rect(edit_rect, Color::rgba(20, 36, 66, 240), 5.0);
                            builder.stroke_rect(
                                edit_rect,
                                Color::rgba(96, 165, 250, 160),
                                1.0,
                                5.0,
                            );
                            builder.icon(
                                ICON_EDIT_ID,
                                LogicalRect::from_xywh(sidebar_w - 57.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(147, 197, 253, 240),
                            );
                        } else {
                            builder.fill_rect(edit_rect, Color::rgba(16, 26, 46, 180), 5.0);
                            builder.stroke_rect(edit_rect, Color::rgba(96, 165, 250, 60), 1.0, 5.0);
                            builder.icon(
                                ICON_EDIT_ID,
                                LogicalRect::from_xywh(sidebar_w - 57.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(148, 163, 184, 180),
                            );
                        }

                        // 4. Delete Button (Cross Vector Icon)
                        let del_rect =
                            LogicalRect::from_xywh(sidebar_w - 34.0, item_y + 16.0, 24.0, 24.0);
                        if is_del_hover {
                            builder.fill_rect(del_rect, Color::rgba(239, 68, 68, 255), 5.0);
                            builder.icon(
                                ICON_CLOSE_ID,
                                LogicalRect::from_xywh(sidebar_w - 29.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(255, 255, 255, 255),
                            );
                        } else if is_card_hover {
                            builder.fill_rect(del_rect, Color::rgba(46, 20, 30, 240), 5.0);
                            builder.stroke_rect(
                                del_rect,
                                Color::rgba(248, 113, 113, 160),
                                1.0,
                                5.0,
                            );
                            builder.icon(
                                ICON_CLOSE_ID,
                                LogicalRect::from_xywh(sidebar_w - 29.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(248, 113, 113, 230),
                            );
                        } else {
                            builder.fill_rect(del_rect, Color::rgba(30, 16, 22, 180), 5.0);
                            builder.stroke_rect(del_rect, Color::rgba(248, 113, 113, 50), 1.0, 5.0);
                            builder.icon(
                                ICON_CLOSE_ID,
                                LogicalRect::from_xywh(sidebar_w - 29.0, item_y + 21.0, 14.0, 14.0),
                                Color::rgba(160, 120, 130, 160),
                            );
                        }

                        item_y += 64.0;
                    }
                }
            }
            SidebarTab::Sessions => {
                let mut item_y = content_y_start;
                for (sess_idx, sess) in app.recent_sessions.iter().take(12).enumerate() {
                    let is_hover = app.hover == HoverTarget::SessionCard(sess_idx);
                    let item_rect = LogicalRect::from_xywh(10.0, item_y, sidebar_w - 20.0, 48.0);

                    builder.fill_rect(
                        item_rect,
                        if is_hover { card_hover_bg } else { card_bg },
                        5.0,
                    );
                    builder.stroke_rect(
                        item_rect,
                        if is_hover { accent_mint } else { card_border },
                        1.0,
                        5.0,
                    );

                    let title = format!("{}@{}", sess.username, sess.host_address);
                    let title_layout = self.shape_cached(&title, StyleId::Sans, None);
                    builder.text(
                        LogicalPoint::new(20.0, item_y + 6.0),
                        title_layout,
                        text_main,
                    );

                    let date_str = sess.started_at.format("%d.%m %H:%M").to_string();
                    let status_str = format!("{} • {}", date_str, sess.status);
                    let status_layout = self.shape_cached(&status_str, StyleId::Small, None);
                    builder.text(
                        LogicalPoint::new(20.0, item_y + 26.0),
                        status_layout,
                        text_dim,
                    );

                    if is_hover {
                        let reconn = self.shape_cached("▶", StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(sidebar_w - 30.0, item_y + 16.0),
                            reconn,
                            accent_mint,
                        );
                    }

                    item_y += 54.0;
                }
            }
            SidebarTab::Commands => {
                let is_cmd_focused = app.focused_input == Some(InputFieldId::CommandSearch);
                let is_cmd_hover = app.hover == HoverTarget::SidebarSearchCommand;
                let search_box_rect =
                    LogicalRect::from_xywh(10.0, content_y_start, sidebar_w - 20.0, 32.0);
                self.draw_input_box(
                    &mut builder,
                    &app.command_search_input,
                    search_box_rect,
                    is_cmd_focused,
                    is_cmd_hover,
                    tr(lang, "Поиск команд...", "Search commands..."),
                    false,
                    app.caret_visible && is_cmd_focused,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                let mut item_y = content_y_start + 40.0;
                for (cmd_idx, cmd) in app.command_search_results.iter().take(15).enumerate() {
                    let is_hover = app.hover == HoverTarget::CommandItem(cmd_idx);
                    let cmd_rect = LogicalRect::from_xywh(10.0, item_y, sidebar_w - 20.0, 38.0);

                    builder.fill_rect(
                        cmd_rect,
                        if is_hover { card_hover_bg } else { card_bg },
                        5.0,
                    );
                    builder.stroke_rect(
                        cmd_rect,
                        if is_hover { accent_mint } else { card_border },
                        1.0,
                        5.0,
                    );

                    let max_cmd_w = (sidebar_w - 60.0).max(40.0);
                    builder.push_clip(LogicalRect::from_xywh(18.0, item_y, max_cmd_w, 38.0));
                    let cmd_layout = self.shape_clamped_cached(
                        &cmd.command,
                        StyleId::Mono,
                        LogicalPx(max_cmd_w),
                        1,
                    );
                    builder.text(
                        LogicalPoint::new(20.0, item_y + 7.0),
                        cmd_layout,
                        accent_mint,
                    );
                    let _ = builder.pop_clip();

                    let date_layout = self.shape_cached(
                        &cmd.executed_at.format("%H:%M:%S").to_string(),
                        StyleId::Small,
                        None,
                    );
                    builder.text(
                        LogicalPoint::new(20.0, item_y + 23.0),
                        date_layout,
                        text_muted,
                    );

                    if is_hover {
                        let play_cmd = self.shape_cached("▶", StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(sidebar_w - 30.0, item_y + 10.0),
                            play_cmd,
                            accent_mint,
                        );
                    }

                    item_y += 44.0;
                }
            }
            SidebarTab::Snippets => {
                // Top action bar: Search box + Add Button
                let is_snip_focused = app.focused_input == Some(InputFieldId::SnippetSearch);
                let is_snip_hover = app.hover == HoverTarget::SidebarSearchSnippet;
                let search_w = sidebar_w - 96.0;
                let search_rect = LogicalRect::from_xywh(10.0, content_y_start, search_w, 32.0);
                self.draw_input_box(
                    &mut builder,
                    &app.snippet_search_input,
                    search_rect,
                    is_snip_focused,
                    is_snip_hover,
                    tr(lang, "Поиск сниппетов...", "Search snippets..."),
                    false,
                    app.caret_visible && is_snip_focused,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                let is_add_h = app.hover == HoverTarget::SidebarAddSnippetBtn;
                let add_btn_rect =
                    LogicalRect::from_xywh(sidebar_w - 82.0, content_y_start, 72.0, 32.0);
                let add_snip_label = tr(lang, "Создать", "+ Create");
                if is_add_h {
                    builder.fill_rect(add_btn_rect, Color::rgba(45, 212, 191, 230), 5.0);
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(sidebar_w - 76.0, content_y_start + 9.0, 14.0, 14.0),
                        Color::rgba(9, 14, 26, 255),
                    );
                    let add_snip_txt = self.shape_cached(add_snip_label, StyleId::Small, None);
                    builder.text(
                        LogicalPoint::new(sidebar_w - 58.0, content_y_start + 7.0),
                        add_snip_txt,
                        bg_app,
                    );
                } else {
                    builder.fill_rect(add_btn_rect, Color::rgba(22, 38, 70, 200), 5.0);
                    builder.stroke_rect(add_btn_rect, accent_mint, 1.0, 5.0);
                    builder.icon(
                        ICON_PLUS_ID,
                        LogicalRect::from_xywh(sidebar_w - 76.0, content_y_start + 9.0, 14.0, 14.0),
                        accent_mint,
                    );
                    let add_snip_txt = self.shape_cached(add_snip_label, StyleId::Small, None);
                    builder.text(
                        LogicalPoint::new(sidebar_w - 58.0, content_y_start + 7.0),
                        add_snip_txt,
                        text_main,
                    );
                }

                let mut item_y = content_y_start + 42.0;
                for (snip_idx, snip) in app.snippets.iter().enumerate() {
                    let is_card_hover = app.hover == HoverTarget::SnippetItem(snip_idx);
                    let is_run_hover = app.hover == HoverTarget::SnippetRun(snip_idx);
                    let is_copy_hover = app.hover == HoverTarget::SnippetCopy(snip_idx);
                    let is_edit_hover = app.hover == HoverTarget::SnippetEdit(snip_idx);
                    let is_del_hover = app.hover == HoverTarget::SnippetDelete(snip_idx);

                    let snip_rect = LogicalRect::from_xywh(10.0, item_y, sidebar_w - 20.0, 56.0);
                    builder.fill_rect(
                        snip_rect,
                        if is_card_hover {
                            card_hover_bg
                        } else {
                            card_bg
                        },
                        5.0,
                    );
                    builder.stroke_rect(
                        snip_rect,
                        if is_card_hover {
                            accent_mint
                        } else {
                            card_border
                        },
                        1.0,
                        5.0,
                    );

                    // Text area width leaving 130px for 4 buttons (24px each + gaps)
                    let text_w = (sidebar_w - 130.0).max(40.0);
                    builder.push_clip(LogicalRect::from_xywh(18.0, item_y, text_w, 56.0));
                    let title_layout = self.shape_clamped_cached(
                        &snip.title,
                        StyleId::SansBold,
                        LogicalPx(text_w),
                        1,
                    );
                    builder.text(
                        LogicalPoint::new(20.0, item_y + 7.0),
                        title_layout,
                        text_main,
                    );

                    let cmd_layout = self.shape_clamped_cached(
                        &snip.command,
                        StyleId::Mono,
                        LogicalPx(text_w),
                        1,
                    );
                    builder.text(
                        LogicalPoint::new(20.0, item_y + 29.0),
                        cmd_layout,
                        accent_mint,
                    );
                    let _ = builder.pop_clip();

                    // Aligned action buttons on right: Run, Copy, Edit, Delete
                    // 1. Run (sidebar_w - 118.0)
                    let run_rect =
                        LogicalRect::from_xywh(sidebar_w - 118.0, item_y + 16.0, 24.0, 24.0);
                    if is_run_hover {
                        builder.fill_rect(run_rect, accent_mint, 5.0);
                        builder.icon(
                            ICON_CONNECT_ID,
                            LogicalRect::from_xywh(sidebar_w - 113.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(8, 13, 25, 255),
                        );
                    } else if is_card_hover {
                        builder.fill_rect(run_rect, Color::rgba(20, 42, 68, 255), 5.0);
                        builder.stroke_rect(run_rect, accent_mint, 1.0, 5.0);
                        builder.icon(
                            ICON_CONNECT_ID,
                            LogicalRect::from_xywh(sidebar_w - 113.0, item_y + 21.0, 14.0, 14.0),
                            accent_mint,
                        );
                    } else {
                        builder.fill_rect(run_rect, Color::rgba(14, 24, 42, 180), 5.0);
                        builder.stroke_rect(run_rect, Color::rgba(45, 212, 191, 50), 1.0, 5.0);
                        builder.icon(
                            ICON_CONNECT_ID,
                            LogicalRect::from_xywh(sidebar_w - 113.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(148, 163, 184, 200),
                        );
                    }

                    // 2. Copy (sidebar_w - 90.0)
                    let copy_rect =
                        LogicalRect::from_xywh(sidebar_w - 90.0, item_y + 16.0, 24.0, 24.0);
                    if is_copy_hover {
                        builder.fill_rect(copy_rect, Color::rgba(45, 212, 191, 230), 5.0);
                        builder.icon(
                            ICON_COPY_ID,
                            LogicalRect::from_xywh(sidebar_w - 85.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(8, 13, 25, 255),
                        );
                    } else if is_card_hover {
                        builder.fill_rect(copy_rect, Color::rgba(22, 38, 64, 240), 5.0);
                        builder.stroke_rect(copy_rect, Color::rgba(45, 212, 191, 140), 1.0, 5.0);
                        builder.icon(
                            ICON_COPY_ID,
                            LogicalRect::from_xywh(sidebar_w - 85.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(45, 212, 191, 220),
                        );
                    } else {
                        builder.fill_rect(copy_rect, Color::rgba(16, 26, 44, 180), 5.0);
                        builder.stroke_rect(copy_rect, Color::rgba(45, 212, 191, 50), 1.0, 5.0);
                        builder.icon(
                            ICON_COPY_ID,
                            LogicalRect::from_xywh(sidebar_w - 85.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(148, 163, 184, 180),
                        );
                    }

                    // 3. Edit (sidebar_w - 62.0)
                    let edit_rect =
                        LogicalRect::from_xywh(sidebar_w - 62.0, item_y + 16.0, 24.0, 24.0);
                    if is_edit_hover {
                        builder.fill_rect(edit_rect, Color::rgba(56, 189, 248, 230), 5.0);
                        builder.icon(
                            ICON_EDIT_ID,
                            LogicalRect::from_xywh(sidebar_w - 57.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(8, 13, 25, 255),
                        );
                    } else if is_card_hover {
                        builder.fill_rect(edit_rect, Color::rgba(18, 36, 60, 240), 5.0);
                        builder.stroke_rect(edit_rect, Color::rgba(56, 189, 248, 140), 1.0, 5.0);
                        builder.icon(
                            ICON_EDIT_ID,
                            LogicalRect::from_xywh(sidebar_w - 57.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(56, 189, 248, 220),
                        );
                    } else {
                        builder.fill_rect(edit_rect, Color::rgba(16, 26, 44, 180), 5.0);
                        builder.stroke_rect(edit_rect, Color::rgba(56, 189, 248, 50), 1.0, 5.0);
                        builder.icon(
                            ICON_EDIT_ID,
                            LogicalRect::from_xywh(sidebar_w - 57.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(148, 163, 184, 180),
                        );
                    }

                    // 4. Delete (sidebar_w - 34.0)
                    let del_rect =
                        LogicalRect::from_xywh(sidebar_w - 34.0, item_y + 16.0, 24.0, 24.0);
                    if is_del_hover {
                        builder.fill_rect(del_rect, Color::rgba(239, 68, 68, 255), 5.0);
                        builder.icon(
                            ICON_CLOSE_ID,
                            LogicalRect::from_xywh(sidebar_w - 29.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(255, 255, 255, 255),
                        );
                    } else if is_card_hover {
                        builder.fill_rect(del_rect, Color::rgba(46, 20, 30, 240), 5.0);
                        builder.stroke_rect(del_rect, Color::rgba(248, 113, 113, 160), 1.0, 5.0);
                        builder.icon(
                            ICON_CLOSE_ID,
                            LogicalRect::from_xywh(sidebar_w - 29.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(248, 113, 113, 230),
                        );
                    } else {
                        builder.fill_rect(del_rect, Color::rgba(30, 16, 22, 180), 5.0);
                        builder.stroke_rect(del_rect, Color::rgba(248, 113, 113, 50), 1.0, 5.0);
                        builder.icon(
                            ICON_CLOSE_ID,
                            LogicalRect::from_xywh(sidebar_w - 29.0, item_y + 21.0, 14.0, 14.0),
                            Color::rgba(160, 120, 130, 160),
                        );
                    }

                    item_y += 62.0;
                }
            }
        }

        // 4. Main Terminal Area
        let term_x = sidebar_w;
        let term_y = header_h;
        let term_w = (w - sidebar_w).max(0.0);
        let term_h = body_h;

        let term_rect = LogicalRect::from_xywh(term_x, term_y, term_w, term_h);
        builder.fill_rect(term_rect, bg_app, 0.0);

        if let Some(tab) = app.tabs.get(app.active_tab) {
            if let Some(split_idx) = app.split_tab {
                if split_idx < app.tabs.len() && split_idx != app.active_tab {
                    let half_w = (term_w / 2.0).floor();
                    let rect_left = LogicalRect::from_xywh(term_x, term_y, half_w - 2.0, term_h);
                    let rect_right = LogicalRect::from_xywh(
                        term_x + half_w + 2.0,
                        term_y,
                        term_w - half_w - 2.0,
                        term_h,
                    );

                    // Left panel (active tab)
                    self.render_single_terminal(
                        &mut builder,
                        app,
                        app.active_tab,
                        tab,
                        true,
                        rect_left,
                        accent_mint,
                        card_border,
                        text_main,
                        text_muted,
                        bg_app,
                    );

                    // Vertical divider (clean 2px line)
                    let divider_rect =
                        LogicalRect::from_xywh(term_x + half_w - 1.0, term_y, 2.0, term_h);
                    builder.fill_rect(divider_rect, card_border, 0.0);

                    // Right panel (split tab)
                    let split_tab = &app.tabs[split_idx];
                    self.render_single_terminal(
                        &mut builder,
                        app,
                        split_idx,
                        split_tab,
                        false,
                        rect_right,
                        accent_mint,
                        card_border,
                        text_main,
                        text_muted,
                        bg_app,
                    );

                    // Dedicated Close Split button on the top-right of the split pane
                    let is_sc_hover = app.hover == HoverTarget::TerminalSplitClose;
                    let sc_btn_w = 24.0;
                    let sc_btn_h = 22.0;
                    let sc_x = rect_right.origin.x.0 + rect_right.size.width.0 - sc_btn_w - 6.0;
                    let sc_y = term_y + 4.0;
                    let sc_rect = LogicalRect::from_xywh(sc_x, sc_y, sc_btn_w, sc_btn_h);
                    let sc_bg = if is_sc_hover {
                        Color::rgba(239, 68, 68, 240)
                    } else {
                        Color::rgba(22, 35, 62, 220)
                    };
                    builder.fill_rect(sc_rect, sc_bg, 4.0);
                    builder.stroke_rect(
                        sc_rect,
                        if is_sc_hover {
                            Color::rgba(239, 68, 68, 255)
                        } else {
                            card_border
                        },
                        1.0,
                        4.0,
                    );
                    builder.icon(
                        ICON_CLOSE_ID,
                        LogicalRect::from_xywh(sc_x + 5.0, sc_y + 4.0, 14.0, 14.0),
                        text_main,
                    );
                } else {
                    self.render_single_terminal(
                        &mut builder,
                        app,
                        app.active_tab,
                        tab,
                        true,
                        term_rect,
                        accent_mint,
                        card_border,
                        text_main,
                        text_muted,
                        bg_app,
                    );
                }
            } else {
                self.render_single_terminal(
                    &mut builder,
                    app,
                    app.active_tab,
                    tab,
                    true,
                    term_rect,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                    bg_app,
                );
            }

            // Floating Search Widget
            if app.search_state.is_open {
                self.render_terminal_search_widget(
                    &mut builder,
                    app,
                    term_rect,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );
            }
        } else {
            // Welcome Screen
            let center_x = term_x + term_w / 2.0 - 220.0;
            let center_y = term_y + term_h / 2.0 - 140.0;

            if self.logo_loaded {
                builder.icon(
                    LOGO_ICON_ID,
                    LogicalRect::from_xywh(center_x + 180.0, center_y, 72.0, 72.0),
                    Color::rgba(255, 255, 255, 255),
                );
            }

            let title_y = center_y + if self.logo_loaded { 84.0 } else { 0.0 };
            let w_title = self.shape_cached(
                tr(
                    lang,
                    "EvertySSH — Универсальный SSH клиент",
                    "EvertySSH — Modern SSH Client",
                ),
                StyleId::Title,
                None,
            );
            builder.text(
                LogicalPoint::new(center_x + 70.0, title_y),
                w_title,
                text_main,
            );

            let is_w_add_hover = app.hover == HoverTarget::WelcomeAdd;
            let is_w_conn_hover = app.hover == HoverTarget::WelcomeConnect;

            let btn_add_rect = LogicalRect::from_xywh(center_x + 90.0, title_y + 36.0, 160.0, 36.0);
            builder.fill_rect(
                btn_add_rect,
                if is_w_add_hover {
                    accent_mint
                } else {
                    Color::rgba(20, 48, 72, 255)
                },
                6.0,
            );
            builder.stroke_rect(btn_add_rect, accent_mint, 1.2, 6.0);
            let btn_text = self.shape_cached(
                tr(lang, "+ Добавить сервер", "+ Add Server"),
                StyleId::SansBold,
                None,
            );
            builder.text(
                LogicalPoint::new(center_x + 110.0, title_y + 44.0),
                btn_text,
                if is_w_add_hover {
                    Color::rgba(9, 14, 26, 255)
                } else {
                    accent_mint
                },
            );

            let btn_conn_rect =
                LogicalRect::from_xywh(center_x + 260.0, title_y + 36.0, 150.0, 36.0);
            builder.fill_rect(
                btn_conn_rect,
                if is_w_conn_hover {
                    card_hover_bg
                } else {
                    card_bg
                },
                6.0,
            );
            builder.stroke_rect(
                btn_conn_rect,
                if is_w_conn_hover {
                    accent_mint
                } else {
                    card_border
                },
                1.0,
                6.0,
            );
            let btn_conn_text = self.shape_cached(
                tr(lang, "⚡ Подключиться", "⚡ Connect"),
                StyleId::Sans,
                None,
            );
            builder.text(
                LogicalPoint::new(center_x + 282.0, title_y + 44.0),
                btn_conn_text,
                text_main,
            );

            let hints = [
                tr(
                    lang,
                    "• Выберите сервер из списка слева (клик по ▶ или двойной клик)",
                    "• Select host from left list (click ▶ or double-click)",
                ),
                tr(
                    lang,
                    "• Нажмите Ctrl+N чтобы добавить новый SSH сервер",
                    "• Press Ctrl+N to add a new SSH server",
                ),
                tr(
                    lang,
                    "• Нажмите Ctrl+R для умного поиска по истории команд",
                    "• Press Ctrl+R for smart command history search",
                ),
                tr(
                    lang,
                    "• Вставка в терминал: Ctrl+V или Shift+Insert",
                    "• Paste into terminal: Ctrl+V or Shift+Insert",
                ),
                tr(
                    lang,
                    "• Профили: ~/.evertyssh/hosts.toml | История: ~/.evertyssh/history.db",
                    "• Hosts: ~/.evertyssh/hosts.toml | History: ~/.evertyssh/history.db",
                ),
            ];
            for (idx, hint) in hints.iter().enumerate() {
                let h_layout = self.shape_cached(hint, StyleId::Sans, None);
                builder.text(
                    LogicalPoint::new(center_x + 40.0, title_y + 92.0 + (idx as f64) * 24.0),
                    h_layout,
                    text_dim,
                );
            }
        }

        // 5. Add / Edit Host Modal Dialog (with Hover & Caret)
        if app.add_host_modal.is_open {
            builder.fill_rect(
                LogicalRect::from_xywh(0.0, 0.0, w, h),
                Color::rgba(0, 0, 0, 180),
                0.0,
            );

            let modal_w = 540.0f64.min(w - 40.0);
            let modal_h = 480.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;
            let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

            builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
            builder.stroke_rect(modal_rect, accent_mint, 1.5, 8.0);

            let modal_title = if app.add_host_modal.is_editing {
                tr(lang, "Редактировать сервер", "Edit Server")
            } else {
                tr(lang, "Добавить новый SSH сервер", "Add New SSH Server")
            };
            let t_layout = self.shape_cached(modal_title, StyleId::Title, None);
            builder.text(
                LogicalPoint::new(modal_x + 24.0, modal_y + 20.0),
                t_layout,
                text_main,
            );

            // Close button with hover
            let is_close_hover = app.hover == HoverTarget::ModalClose;
            let close_rect =
                LogicalRect::from_xywh(modal_x + modal_w - 38.0, modal_y + 14.0, 24.0, 24.0);
            if is_close_hover {
                builder.fill_rect(close_rect, Color::rgba(239, 68, 68, 200), 5.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 33.0, modal_y + 19.0, 14.0, 14.0),
                    Color::rgba(255, 255, 255, 255),
                );
            } else {
                builder.fill_rect(close_rect, Color::rgba(22, 34, 58, 160), 5.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 33.0, modal_y + 19.0, 14.0, 14.0),
                    text_muted,
                );
            }

            let m = &app.add_host_modal;

            let fields = [
                (
                    tr(lang, "Название сервера:", "Server Name:"),
                    &m.name,
                    tr(
                        lang,
                        "например, Production Web 01",
                        "e.g., Production Web 01",
                    ),
                ),
                (
                    tr(lang, "Хост / IP адрес:", "Host / IP Address:"),
                    &m.hostname,
                    "192.168.1.100",
                ),
                (tr(lang, "Порт:", "Port:"), &m.port, "22"),
                (tr(lang, "Пользователь:", "Username:"), &m.username, "root"),
                (
                    tr(lang, "Группа:", "Group:"),
                    &m.group,
                    "Production, Staging, Dev...",
                ),
            ];

            let mut f_y = modal_y + 60.0;
            for (idx, (label, inp, placeholder)) in fields.iter().enumerate() {
                let is_focused = m.active_field == idx;
                let is_hover = app.hover == HoverTarget::ModalField(idx);

                let l_layout = self.shape_cached(label, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, f_y + 7.0),
                    l_layout,
                    text_dim,
                );

                let inp_x = modal_x + 160.0;
                let inp_w = modal_w - 184.0;
                let inp_rect = LogicalRect::from_xywh(inp_x, f_y, inp_w, 30.0);

                self.draw_input_box(
                    &mut builder,
                    inp,
                    inp_rect,
                    is_focused,
                    is_hover,
                    placeholder,
                    false,
                    app.caret_visible,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                f_y += 38.0;
            }

            // Auth Mode Selection
            let auth_label = self.shape_cached(
                tr(lang, "Аутентификация:", "Authentication:"),
                StyleId::Small,
                None,
            );
            builder.text(
                LogicalPoint::new(modal_x + 24.0, f_y + 7.0),
                auth_label,
                text_dim,
            );

            let auth_modes = [
                (AuthModeChoice::Agent, "SSH-Agent"),
                (AuthModeChoice::Key, tr(lang, "SSH Ключ", "SSH Key")),
                (AuthModeChoice::Password, tr(lang, "Пароль", "Password")),
            ];
            let mut btn_x = modal_x + 160.0;
            for (mode, label) in auth_modes {
                let is_sel = m.auth_mode == mode;
                let is_auth_hover = app.hover == HoverTarget::ModalAuthMode(mode);
                let b_rect = LogicalRect::from_xywh(btn_x, f_y, 90.0, 28.0);

                if is_sel {
                    builder.fill_rect(b_rect, Color::rgba(24, 52, 78, 255), 4.0);
                    builder.stroke_rect(b_rect, accent_mint, 1.2, 4.0);
                } else if is_auth_hover {
                    builder.fill_rect(b_rect, Color::rgba(25, 41, 72, 255), 4.0);
                    builder.stroke_rect(b_rect, card_border, 1.0, 4.0);
                } else {
                    builder.fill_rect(b_rect, Color::rgba(20, 32, 58, 255), 4.0);
                    builder.stroke_rect(b_rect, card_border, 1.0, 4.0);
                }
                let lbl_layout = self.shape_cached(label, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(btn_x + 12.0, f_y + 6.0),
                    lbl_layout,
                    if is_sel || is_auth_hover {
                        accent_mint
                    } else {
                        text_dim
                    },
                );
                btn_x += 98.0;
            }
            f_y += 38.0;

            // Key path or Password input
            if m.auth_mode == AuthModeChoice::Key {
                let is_focused = m.active_field == 5;
                let is_hover = app.hover == HoverTarget::ModalField(5);

                let l_layout = self.shape_cached("Путь к ключу:", StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, f_y + 7.0),
                    l_layout,
                    text_dim,
                );

                let inp_x = modal_x + 160.0;
                let btn_browse_w = 70.0;
                let btn_gen_w = 84.0;
                let inp_w = modal_w - 184.0 - btn_browse_w - btn_gen_w - 16.0;
                let inp_rect = LogicalRect::from_xywh(inp_x, f_y, inp_w, 30.0);

                self.draw_input_box(
                    &mut builder,
                    &m.key_path,
                    inp_rect,
                    is_focused,
                    is_hover,
                    "~/.ssh/id_ed25519",
                    false,
                    app.caret_visible,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                // Browse Button 📁 Обзор
                let browse_x = inp_x + inp_w + 8.0;
                let browse_rect = LogicalRect::from_xywh(browse_x, f_y, btn_browse_w, 30.0);
                let is_browse_hover = app.hover == HoverTarget::ModalBrowseKey;
                builder.fill_rect(
                    browse_rect,
                    if is_browse_hover {
                        card_hover_bg
                    } else {
                        Color::rgba(22, 34, 60, 255)
                    },
                    4.0,
                );
                builder.stroke_rect(
                    browse_rect,
                    if is_browse_hover {
                        accent_mint
                    } else {
                        card_border
                    },
                    1.0,
                    4.0,
                );
                let b_txt =
                    self.shape_cached(tr(lang, "📁 Обзор", "📁 Browse"), StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(browse_x + 10.0, f_y + 7.0),
                    b_txt,
                    if is_browse_hover {
                        accent_mint
                    } else {
                        text_main
                    },
                );

                // Generate Button ⚡ Создать
                let gen_x = browse_x + btn_browse_w + 8.0;
                let gen_rect = LogicalRect::from_xywh(gen_x, f_y, btn_gen_w, 30.0);
                let is_gen_hover = app.hover == HoverTarget::ModalGenerateKey;
                builder.fill_rect(
                    gen_rect,
                    if is_gen_hover {
                        accent_mint
                    } else {
                        Color::rgba(20, 45, 68, 255)
                    },
                    4.0,
                );
                builder.stroke_rect(gen_rect, accent_mint, 1.0, 4.0);
                let g_txt =
                    self.shape_cached(tr(lang, "⚡ Создать", "⚡ Generate"), StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(gen_x + 10.0, f_y + 7.0),
                    g_txt,
                    if is_gen_hover {
                        Color::rgba(9, 14, 26, 255)
                    } else {
                        accent_mint
                    },
                );

                f_y += 38.0;
            } else if m.auth_mode == AuthModeChoice::Password {
                let is_focused = m.active_field == 5;
                let is_hover = app.hover == HoverTarget::ModalField(5);

                let l_layout =
                    self.shape_cached(tr(lang, "Пароль:", "Password:"), StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, f_y + 7.0),
                    l_layout,
                    text_dim,
                );

                let inp_x = modal_x + 160.0;
                let eye_btn_w = 34.0;
                let inp_w = modal_w - 184.0 - eye_btn_w - 8.0;
                let inp_rect = LogicalRect::from_xywh(inp_x, f_y, inp_w, 30.0);

                self.draw_input_box(
                    &mut builder,
                    &m.password,
                    inp_rect,
                    is_focused,
                    is_hover,
                    tr(lang, "Введите пароль", "Enter password"),
                    !m.show_password,
                    app.caret_visible && is_focused,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                // Eye toggle button
                let eye_x = inp_x + inp_w + 8.0;
                let eye_rect = LogicalRect::from_xywh(eye_x, f_y, eye_btn_w, 30.0);
                let is_eye_hover = app.hover == HoverTarget::ModalTogglePassword;
                builder.fill_rect(
                    eye_rect,
                    if is_eye_hover {
                        card_hover_bg
                    } else {
                        Color::rgba(20, 32, 58, 255)
                    },
                    4.0,
                );
                builder.stroke_rect(
                    eye_rect,
                    if is_eye_hover {
                        accent_mint
                    } else {
                        card_border
                    },
                    1.0,
                    4.0,
                );
                let icon_id = if m.show_password {
                    ICON_EYE_OFF_ID
                } else {
                    ICON_EYE_ID
                };
                let icon_color = if is_eye_hover || m.show_password {
                    accent_mint
                } else {
                    text_dim
                };
                builder.icon(
                    icon_id,
                    LogicalRect::from_xywh(eye_x + 9.0, f_y + 7.0, 16.0, 16.0),
                    icon_color,
                );

                f_y += 38.0;
            }

            if let Some(err) = &m.error_msg {
                let err_layout = self.shape_cached(err, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 160.0, f_y + 2.0),
                    err_layout,
                    Color::rgba(239, 68, 68, 255),
                );
            }

            // Modal Buttons with Hover
            let is_save_hover = app.hover == HoverTarget::ModalSave;
            let is_conn_hover = app.hover == HoverTarget::ModalConnect;

            let save_btn_rect = LogicalRect::from_xywh(
                modal_x + modal_w - 280.0,
                modal_y + modal_h - 48.0,
                120.0,
                34.0,
            );
            builder.fill_rect(
                save_btn_rect,
                if is_save_hover {
                    Color::rgba(28, 64, 96, 255)
                } else {
                    Color::rgba(20, 48, 72, 255)
                },
                5.0,
            );
            builder.stroke_rect(save_btn_rect, accent_mint, 1.2, 5.0);
            let s_layout =
                self.shape_cached(tr(lang, "Сохранить", "Save"), StyleId::SansBold, None);
            builder.text(
                LogicalPoint::new(modal_x + modal_w - 256.0, modal_y + modal_h - 40.0),
                s_layout,
                accent_mint,
            );

            let conn_btn_rect = LogicalRect::from_xywh(
                modal_x + modal_w - 146.0,
                modal_y + modal_h - 48.0,
                126.0,
                34.0,
            );
            builder.fill_rect(
                conn_btn_rect,
                if is_conn_hover {
                    accent_emerald
                } else {
                    Color::rgba(45, 212, 191, 220)
                },
                5.0,
            );
            let c_layout =
                self.shape_cached(tr(lang, "Подключить", "Connect"), StyleId::SansBold, None);
            builder.text(
                LogicalPoint::new(modal_x + modal_w - 128.0, modal_y + modal_h - 40.0),
                c_layout,
                Color::rgba(10, 16, 30, 255),
            );
        }

        // 5.5 Key Tool Modal (SSH Key Generator & Remote Deployer)
        if app.key_tool_modal.is_open {
            builder.fill_rect(
                LogicalRect::from_xywh(0.0, 0.0, w, h),
                Color::rgba(0, 0, 0, 180),
                0.0,
            );

            let modal_w = 580.0f64.min(w - 40.0);
            let modal_h = 520.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;
            let m_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

            builder.fill_rect(m_rect, Color::rgba(14, 22, 38, 255), 8.0);
            builder.stroke_rect(m_rect, accent_mint, 1.5, 8.0);

            let km = &app.key_tool_modal;
            let title_text = if let Some(h_idx) = km.target_host_idx {
                if let Some(h) = app.hosts_doc.hosts.get(h_idx) {
                    format!("🔑 SSH Ключ для сервера '{}'", h.name)
                } else {
                    "🔑 SSH Ключ для сервера".into()
                }
            } else {
                "🔑 Генератор SSH ключей (OpenSSH)".into()
            };
            let title_layout = self.shape_cached(&title_text, StyleId::Title, None);
            builder.text(
                LogicalPoint::new(modal_x + 24.0, modal_y + 18.0),
                title_layout,
                text_main,
            );

            let is_close_hover = app.hover == HoverTarget::KeyToolCloseBtn;
            let close_rect =
                LogicalRect::from_xywh(modal_x + modal_w - 36.0, modal_y + 14.0, 24.0, 24.0);
            if is_close_hover {
                builder.fill_rect(close_rect, Color::rgba(239, 68, 68, 200), 5.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 31.0, modal_y + 19.0, 14.0, 14.0),
                    Color::rgba(255, 255, 255, 255),
                );
            } else {
                builder.fill_rect(close_rect, Color::rgba(22, 34, 58, 160), 5.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 31.0, modal_y + 19.0, 14.0, 14.0),
                    text_muted,
                );
            }

            // Algorithm selector
            let algo_label = self.shape_cached("Алгоритм:", StyleId::Small, None);
            builder.text(
                LogicalPoint::new(modal_x + 24.0, modal_y + 58.0),
                algo_label,
                text_dim,
            );

            let algos = [
                (KeyAlgo::Ed25519, "Ed25519 (Быстрый & Безопасный)", 210.0),
                (KeyAlgo::Rsa4096, "RSA 4096-bit", 120.0),
            ];
            let mut ax = modal_x + 160.0;
            for (algo, label, btn_w) in algos {
                let is_sel = km.algo == algo;
                let is_hover = app.hover == HoverTarget::KeyToolAlgo(algo);
                let btn_rect = LogicalRect::from_xywh(ax, modal_y + 54.0, btn_w, 28.0);
                if is_sel {
                    builder.fill_rect(btn_rect, Color::rgba(24, 52, 78, 255), 4.0);
                    builder.stroke_rect(btn_rect, accent_mint, 1.2, 4.0);
                } else if is_hover {
                    builder.fill_rect(btn_rect, Color::rgba(25, 41, 72, 255), 4.0);
                    builder.stroke_rect(btn_rect, card_border, 1.0, 4.0);
                } else {
                    builder.fill_rect(btn_rect, Color::rgba(20, 32, 58, 255), 4.0);
                    builder.stroke_rect(btn_rect, card_border, 1.0, 4.0);
                }
                let lbl = self.shape_cached(label, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(ax + 10.0, modal_y + 60.0),
                    lbl,
                    if is_sel || is_hover {
                        accent_mint
                    } else {
                        text_dim
                    },
                );
                ax += btn_w + 10.0;
            }

            // Input fields: Name, Comment, Passphrase
            let fields = [
                ("Имя файла в ~/.ssh:", &km.key_name, "id_ed25519"),
                ("Комментарий (email):", &km.comment, "user@evertyssh"),
                ("Пароль (опционально):", &km.passphrase, "Без пароля"),
            ];

            let mut f_y = modal_y + 92.0;
            for (idx, (label, inp, placeholder)) in fields.iter().enumerate() {
                let is_focused = km.active_field == idx;
                let is_hover = app.hover == HoverTarget::KeyToolField(idx);

                let lbl = self.shape_cached(label, StyleId::Small, None);
                builder.text(LogicalPoint::new(modal_x + 24.0, f_y + 7.0), lbl, text_dim);

                let inp_x = modal_x + 160.0;
                let inp_w = modal_w - 184.0;
                let inp_rect = LogicalRect::from_xywh(inp_x, f_y, inp_w, 30.0);

                self.draw_input_box(
                    &mut builder,
                    inp,
                    inp_rect,
                    is_focused,
                    is_hover,
                    placeholder,
                    idx == 2, // passphrase
                    app.caret_visible,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                f_y += 38.0;
            }

            // Generate Button
            let gen_btn_rect = LogicalRect::from_xywh(modal_x + 160.0, f_y, modal_w - 184.0, 34.0);
            let is_gen_btn_hover = app.hover == HoverTarget::KeyToolGenerateBtn;
            builder.fill_rect(
                gen_btn_rect,
                if is_gen_btn_hover {
                    accent_mint
                } else {
                    Color::rgba(20, 50, 78, 255)
                },
                5.0,
            );
            builder.stroke_rect(gen_btn_rect, accent_mint, 1.2, 5.0);
            let g_lbl =
                self.shape_cached("⚡ Сгенерировать пару SSH ключей", StyleId::SansBold, None);
            builder.text(
                LogicalPoint::new(modal_x + 200.0, f_y + 8.0),
                g_lbl,
                if is_gen_btn_hover {
                    Color::rgba(9, 14, 26, 255)
                } else {
                    accent_mint
                },
            );

            f_y += 44.0;

            // Status or Error messages
            if let Some(err) = &km.error_msg {
                let err_layout = self.shape_cached(err, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 160.0, f_y),
                    err_layout,
                    Color::rgba(239, 68, 68, 255),
                );
                f_y += 22.0;
            } else if let Some(stat) = &km.status_msg {
                let stat_layout = self.shape_cached(stat, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 160.0, f_y),
                    stat_layout,
                    accent_mint,
                );
                f_y += 22.0;
            }

            // If key generated, show public key preview and actions
            if let Some(pub_key) = &km.generated_pub_key {
                let pub_box_rect =
                    LogicalRect::from_xywh(modal_x + 24.0, f_y, modal_w - 48.0, 52.0);
                builder.fill_rect(pub_box_rect, Color::rgba(10, 16, 28, 255), 4.0);
                builder.stroke_rect(pub_box_rect, card_border, 1.0, 4.0);

                let pub_label = self.shape_cached("Публичный ключ (.pub):", StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 28.0, f_y + 4.0),
                    pub_label,
                    text_dim,
                );

                let key_preview: String = pub_key.chars().take(68).collect();
                let key_preview_full = if pub_key.len() > 68 {
                    format!("{key_preview}...")
                } else {
                    key_preview
                };
                let k_layout = self.shape_cached(&key_preview_full, StyleId::Mono, None);
                builder.text(
                    LogicalPoint::new(modal_x + 28.0, f_y + 24.0),
                    k_layout,
                    accent_mint,
                );

                // Bottom Action buttons
                let btn_y = modal_y + modal_h - 48.0;
                let mut bx = modal_x + 24.0;

                // Copy pub key button
                let cp_rect = LogicalRect::from_xywh(bx, btn_y, 160.0, 34.0);
                let is_cp_hover = app.hover == HoverTarget::KeyToolCopyPubBtn;
                builder.fill_rect(
                    cp_rect,
                    if is_cp_hover {
                        card_hover_bg
                    } else {
                        Color::rgba(20, 32, 58, 255)
                    },
                    4.0,
                );
                builder.stroke_rect(
                    cp_rect,
                    if is_cp_hover {
                        accent_mint
                    } else {
                        card_border
                    },
                    1.0,
                    4.0,
                );
                let cp_lbl = self.shape_cached(
                    tr(lang, "📋 Скопировать ключ", "📋 Copy Key"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(bx + 16.0, btn_y + 9.0),
                    cp_lbl,
                    if is_cp_hover { accent_mint } else { text_main },
                );
                bx += 168.0;

                if km.target_host_idx.is_some() {
                    // Send to terminal button
                    let send_rect = LogicalRect::from_xywh(bx, btn_y, 160.0, 34.0);
                    let is_send_hover = app.hover == HoverTarget::KeyToolSendTermBtn;
                    builder.fill_rect(
                        send_rect,
                        if is_send_hover {
                            accent_mint
                        } else {
                            Color::rgba(22, 48, 72, 255)
                        },
                        4.0,
                    );
                    builder.stroke_rect(send_rect, accent_mint, 1.0, 4.0);
                    let send_lbl = self.shape_cached(
                        tr(lang, "⚡ Отправить на сервер", "⚡ Send to Server"),
                        StyleId::Small,
                        None,
                    );
                    builder.text(
                        LogicalPoint::new(bx + 12.0, btn_y + 9.0),
                        send_lbl,
                        if is_send_hover {
                            Color::rgba(9, 14, 26, 255)
                        } else {
                            accent_mint
                        },
                    );
                    bx += 168.0;

                    // Bind to host button
                    let bind_rect = LogicalRect::from_xywh(bx, btn_y, 160.0, 34.0);
                    let is_bind_hover = app.hover == HoverTarget::KeyToolBindHostBtn;
                    builder.fill_rect(
                        bind_rect,
                        if is_bind_hover {
                            card_hover_bg
                        } else {
                            Color::rgba(20, 32, 58, 255)
                        },
                        4.0,
                    );
                    builder.stroke_rect(
                        bind_rect,
                        if is_bind_hover {
                            accent_mint
                        } else {
                            card_border
                        },
                        1.0,
                        4.0,
                    );
                    let bind_lbl = self.shape_cached(
                        tr(lang, "💾 Привязать к серверу", "💾 Bind to Host"),
                        StyleId::Small,
                        None,
                    );
                    builder.text(
                        LogicalPoint::new(bx + 12.0, btn_y + 9.0),
                        bind_lbl,
                        if is_bind_hover {
                            accent_mint
                        } else {
                            text_main
                        },
                    );
                }
            }
        }

        // 6. Command Palette Modal (Ctrl+R)
        if app.palette_open {
            builder.fill_rect(
                LogicalRect::from_xywh(0.0, 0.0, w, h),
                Color::rgba(0, 0, 0, 160),
                0.0,
            );

            let pal_w = 600.0f64.min(w - 40.0);
            let pal_h = 360.0f64.min(h - 80.0);
            let pal_x = (w - pal_w) / 2.0;
            let pal_y = 70.0;
            let pal_rect = LogicalRect::from_xywh(pal_x, pal_y, pal_w, pal_h);

            builder.fill_rect(pal_rect, Color::rgba(16, 26, 48, 255), 8.0);
            builder.stroke_rect(pal_rect, accent_mint, 1.5, 8.0);

            let inp_rect = LogicalRect::from_xywh(pal_x + 16.0, pal_y + 16.0, pal_w - 32.0, 36.0);
            builder.fill_rect(inp_rect, Color::rgba(10, 16, 30, 255), 5.0);
            builder.stroke_rect(inp_rect, card_border, 1.0, 5.0);

            let is_empty = app.command_search_input.text.is_empty();
            let prompt_str = if is_empty {
                tr(
                    lang,
                    "Поиск по командам (Enter — запустить в активном терминале)...",
                    "Search commands (Enter to execute in active terminal)...",
                )
            } else {
                &app.command_search_input.text
            };
            let prompt_layout =
                Arc::new(self.text_system.shape(prompt_str, &self.sans_style, None));
            builder.text(
                LogicalPoint::new(pal_x + 28.0, pal_y + 24.0),
                prompt_layout,
                if is_empty { text_dim } else { text_main },
            );

            if app.caret_visible {
                let prefix = app.command_search_input.prefix_to_cursor();
                let prefix_w = if prefix.is_empty() {
                    0.0
                } else {
                    self.shape_cached(&prefix, StyleId::Sans, None)
                        .metrics
                        .size
                        .width
                        .0
                };
                let caret_x = pal_x + 28.0 + prefix_w;
                builder.fill_rect(
                    LogicalRect::from_xywh(caret_x, pal_y + 23.0, 2.0, 20.0),
                    accent_mint,
                    1.0,
                );
            }

            let mut res_y = pal_y + 64.0;
            for (idx, record) in app.command_search_results.iter().take(7).enumerate() {
                let is_sel = idx == app.selected_search_idx;
                let is_hover = app.hover == HoverTarget::PaletteItem(idx);
                let row_rect = LogicalRect::from_xywh(pal_x + 16.0, res_y, pal_w - 32.0, 36.0);

                if is_sel {
                    builder.fill_rect(row_rect, Color::rgba(28, 54, 86, 255), 5.0);
                    builder.stroke_rect(row_rect, accent_mint, 1.2, 5.0);
                } else if is_hover {
                    builder.fill_rect(row_rect, Color::rgba(24, 42, 70, 255), 5.0);
                    builder.stroke_rect(row_rect, Color::rgba(45, 212, 191, 140), 1.0, 5.0);
                } else {
                    builder.fill_rect(row_rect, card_bg, 5.0);
                }

                let cmd_layout = self.shape_cached(&record.command, StyleId::Mono, None);
                builder.text(
                    LogicalPoint::new(pal_x + 28.0, res_y + 9.0),
                    cmd_layout,
                    if is_sel || is_hover {
                        accent_mint
                    } else {
                        text_main
                    },
                );

                let date_str = record.executed_at.format("%d.%m %H:%M").to_string();
                let date_layout = self.shape_cached(&date_str, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(pal_x + pal_w - 110.0, res_y + 11.0),
                    date_layout,
                    text_muted,
                );

                res_y += 42.0;
            }
        }

        // 7. Context Menu Popup
        if let Some(menu) = &app.context_menu {
            if menu.is_open {
                let menu_rect = LogicalRect::from_xywh(menu.x, menu.y, 180.0, 108.0);
                // Shadow / card background
                builder.fill_rect(menu_rect, Color::rgba(11, 18, 33, 252), 6.0);
                builder.stroke_rect(menu_rect, accent_mint, 1.2, 6.0);

                let items = [
                    (
                        ContextMenuItem::Copy,
                        tr(lang, "📋  Копировать", "📋  Copy"),
                    ),
                    (
                        ContextMenuItem::SearchGoogle,
                        tr(lang, "🔍  Искать в Google", "🔍  Search Google"),
                    ),
                    (
                        ContextMenuItem::Paste,
                        tr(lang, "⚡  Вставить", "⚡  Paste"),
                    ),
                ];

                for (idx, (item, label)) in items.iter().enumerate() {
                    let item_y = menu.y + 6.0 + (idx as f64) * 32.0;
                    let item_rect = LogicalRect::from_xywh(menu.x + 4.0, item_y, 172.0, 30.0);
                    let is_hover = app.hover == HoverTarget::ContextMenu(*item);

                    if is_hover {
                        builder.fill_rect(item_rect, Color::rgba(45, 212, 191, 35), 4.0);
                    }

                    let has_selection = !menu.selected_text.is_empty();
                    let is_disabled = (*item == ContextMenuItem::Copy
                        || *item == ContextMenuItem::SearchGoogle)
                        && !has_selection;

                    let item_color = if is_disabled {
                        text_muted
                    } else if is_hover {
                        accent_mint
                    } else {
                        text_main
                    };

                    let layout = self.shape_cached(label, StyleId::Sans, None);
                    builder.text(
                        LogicalPoint::new(menu.x + 12.0, item_y + 6.0),
                        layout,
                        item_color,
                    );
                }
            }
        }

        // 8. Footer / Status Bar
        let footer_y = h - footer_h;
        builder.fill_rect(
            LogicalRect::from_xywh(0.0, footer_y, w, footer_h),
            bg_footer,
            0.0,
        );
        builder.stroke_rect(
            LogicalRect::from_xywh(0.0, footer_y, w, 1.0),
            card_border,
            1.0,
            0.0,
        );

        let status_text = match app.hover {
            HoverTarget::HostConnect(i) => {
                if let Some(h) = app.hosts_doc.hosts.get(i) {
                    format!(
                        "⚡ Быстрое подключение к {} ({}@{})",
                        h.name, h.username, h.hostname
                    )
                } else {
                    "⚡ Подключиться".into()
                }
            }
            HoverTarget::HostEdit(i) => {
                if let Some(h) = app.hosts_doc.hosts.get(i) {
                    format!("✎ Редактировать настройки сервера {}", h.name)
                } else {
                    "✎ Редактировать".into()
                }
            }
            HoverTarget::HostDelete(i) => {
                if let Some(h) = app.hosts_doc.hosts.get(i) {
                    format!("🗑 Удалить сервер {} из конфигурации", h.name)
                } else {
                    "🗑 Удалить".into()
                }
            }
            HoverTarget::HostCard(i) => {
                if let Some(h) = app.hosts_doc.hosts.get(i) {
                    format!(
                        "💻 Двойной клик — подключиться к {} | Клик по ▶ для быстрого запуска",
                        h.name
                    )
                } else {
                    "Сервер".into()
                }
            }
            HoverTarget::HeaderTabClose(_) => "× Закрыть текущую вкладку терминала".into(),
            HoverTarget::HeaderTab(i) => {
                if let Some(t) = app.tabs.get(i) {
                    format!("Вкладка #{}: {} ({})", i + 1, t.title, t.status)
                } else {
                    "Вкладка".into()
                }
            }
            HoverTarget::HeaderAddBtn | HoverTarget::SidebarAddHostBtn => {
                "+ Добавить новый профиль SSH сервера (Ctrl+N)".into()
            }
            HoverTarget::HeaderSearchBtn => {
                "🔍 Поиск по истории выполненных команд (Ctrl+R)".into()
            }
            HoverTarget::SidebarNav(SidebarTab::Hosts) => {
                "Управление SSH серверами и подключениями".into()
            }
            HoverTarget::SidebarNav(SidebarTab::Sessions) => {
                "История последних сессий подключения".into()
            }
            HoverTarget::SidebarNav(SidebarTab::Commands) => {
                "Журнал команд с быстрым поиском".into()
            }
            HoverTarget::SidebarNav(SidebarTab::Snippets) => {
                "Быстрые шаблоны команд в один клик".into()
            }
            HoverTarget::SessionCard(i) => {
                if let Some(s) = app.recent_sessions.get(i) {
                    format!(
                        "🔄 Повторить подключение к {}@{}",
                        s.username, s.host_address
                    )
                } else {
                    "Сессия".into()
                }
            }
            HoverTarget::CommandItem(i) => {
                if let Some(c) = app.command_search_results.get(i) {
                    format!("▶ Запустить команду в терминале: {}", c.command)
                } else {
                    "Команда".into()
                }
            }
            HoverTarget::SnippetItem(i) => {
                if let Some(s) = app.snippets.get(i) {
                    format!("▶ Выполнить сниппет: {}", s.title)
                } else {
                    "Сниппет".into()
                }
            }
            HoverTarget::PaletteItem(i) => {
                if let Some(c) = app.command_search_results.get(i) {
                    format!("▶ Нажмите Enter или кликните для запуска: {}", c.command)
                } else {
                    "Команда".into()
                }
            }
            HoverTarget::HostDeployKey(i) => {
                if let Some(h) = app.hosts_doc.hosts.get(i) {
                    format!(
                        "🔑 Сгенерировать и установить SSH ключ на сервер {}",
                        h.name
                    )
                } else {
                    "🔑 Установить SSH ключ".into()
                }
            }
            HoverTarget::SidebarSplitter => {
                "↔ Перетащите для изменения ширины боковой панели".into()
            }
            HoverTarget::ModalBrowseKey => {
                "📁 Выбрать приватный SSH ключ через проводник файлов".into()
            }
            HoverTarget::ModalGenerateKey => "⚡ Сгенерировать новую пару SSH ключей".into(),
            HoverTarget::KeyToolAlgo(_) => "Выбрать алгоритм шифрования ключа".into(),
            HoverTarget::KeyToolField(0) => "Имя файла приватного ключа в папке ~/.ssh/".into(),
            HoverTarget::KeyToolField(1) => "Комментарий или email идентификатор ключа".into(),
            HoverTarget::KeyToolField(2) => {
                "Парольная фраза для шифрования приватного ключа (опционально)".into()
            }
            HoverTarget::KeyToolField(_) => "Поле ввода".into(),
            HoverTarget::KeyToolGenerateBtn => "⚡ Запустить генерацию пары OpenSSH ключей".into(),
            HoverTarget::KeyToolCopyPubBtn => {
                "📋 Скопировать публичный ключ (.pub) в буфер обмена".into()
            }
            HoverTarget::KeyToolSendTermBtn => {
                "⚡ Отправить команду добавления ключа прямо в терминал сервера".into()
            }
            HoverTarget::KeyToolCopyCmdBtn => {
                "📋 Скопировать команду authorized_keys в буфер".into()
            }
            HoverTarget::KeyToolBindHostBtn => {
                "💾 Сохранить и привязать этот ключ к профилю сервера".into()
            }
            HoverTarget::KeyToolCloseBtn => "Закрыть окно генерации ключа (Esc)".into(),
            HoverTarget::ModalSave => {
                "💾 Сохранить профиль сервера в ~/.evertyssh/hosts.toml".into()
            }
            HoverTarget::ModalConnect => "⚡ Сохранить и сразу подключиться к серверу".into(),
            HoverTarget::ModalClose => "Закрыть окно без сохранения (Esc)".into(),
            HoverTarget::ContextMenu(ContextMenuItem::Copy) => {
                "Скопировать выделенный текст в буфер обмена".into()
            }
            HoverTarget::ContextMenu(ContextMenuItem::SearchGoogle) => {
                "🔍 Найти выделенный текст в Google (откроется в браузере)".into()
            }
            HoverTarget::ContextMenu(ContextMenuItem::Paste) => {
                "⚡ Вставить текст из буфера обмена в терминал".into()
            }
            HoverTarget::TerminalScrollbarThumb | HoverTarget::TerminalScrollbarTrack => {
                "Прокрутка терминала (перетащите ползунок или колесо мыши)".into()
            }
            HoverTarget::TerminalArea => {
                "Терминал (выделяйте мышкой, правый клик — вставка / поиск в Google)".into()
            }
            _ => {
                if let Some(tab) = app.tabs.get(app.active_tab) {
                    format!(
                        "{} | {} | PTY: {}x{}",
                        tab.status, tab.host_address, app.terminal_cols, app.terminal_rows
                    )
                } else {
                    format!(
                        "EvertySSH готов | Сохранено серверов: {}",
                        app.hosts_doc.hosts.len()
                    )
                }
            }
        };
        let status_layout = self.shape_cached(&status_text, StyleId::Small, None);
        builder.text(
            LogicalPoint::new(16.0, footer_y + 6.0),
            status_layout,
            text_dim,
        );

        let hotkey_hint = tr(
            lang,
            "Ctrl+N Новый хост | Ctrl+R Поиск | Ctrl+V Вставка | Ctrl+T Новая вкладка | Esc Закрыть",
            "Ctrl+N New Host | Ctrl+R Search | Ctrl+V Paste | Ctrl+T New Tab | Esc Close",
        );
        let hint_layout = self.shape_cached(hotkey_hint, StyleId::Small, None);
        builder.text(
            LogicalPoint::new(w - 480.0, footer_y + 6.0),
            hint_layout,
            text_muted,
        );

        // 9. Add Snippet Modal Dialog
        if app.add_snippet_modal.is_open {
            builder.fill_rect(
                LogicalRect::from_xywh(0.0, 0.0, w, h),
                Color::rgba(0, 0, 0, 180),
                0.0,
            );
            let modal_w = 520.0f64.min(w - 40.0);
            let modal_h = 420.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;
            let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

            builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
            builder.stroke_rect(modal_rect, accent_mint, 1.5, 8.0);

            let title_text = if app.add_snippet_modal.is_editing {
                tr(lang, "Редактировать сниппет", "Edit Snippet")
            } else {
                tr(lang, "Создать сниппет команды", "Create Command Snippet")
            };
            let t_layout = self.shape_cached(title_text, StyleId::Title, None);
            builder.text(
                LogicalPoint::new(modal_x + 24.0, modal_y + 20.0),
                t_layout,
                text_main,
            );

            // Close button
            let is_close_hover = app.hover == HoverTarget::ModalSnippetClose;
            let close_rect =
                LogicalRect::from_xywh(modal_x + modal_w - 38.0, modal_y + 14.0, 24.0, 24.0);
            if is_close_hover {
                builder.fill_rect(close_rect, Color::rgba(239, 68, 68, 200), 5.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 33.0, modal_y + 19.0, 14.0, 14.0),
                    Color::rgba(255, 255, 255, 255),
                );
            } else {
                builder.fill_rect(close_rect, Color::rgba(22, 34, 58, 160), 5.0);
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 33.0, modal_y + 19.0, 14.0, 14.0),
                    text_muted,
                );
            }

            let snip_fields = [
                (
                    tr(lang, "Название:", "Title:"),
                    &app.add_snippet_modal.title,
                    tr(lang, "например, Перезапуск Nginx", "e.g. Restart Nginx"),
                ),
                (
                    tr(lang, "Команда SSH:", "SSH Command:"),
                    &app.add_snippet_modal.command,
                    "sudo systemctl restart nginx",
                ),
                (
                    tr(lang, "Категория:", "Category:"),
                    &app.add_snippet_modal.category,
                    "General, Docker, Web, Deploy...",
                ),
                (
                    tr(lang, "Описание (опционально):", "Description (optional):"),
                    &app.add_snippet_modal.description,
                    tr(
                        lang,
                        "Быстрый рестарт веб-сервера",
                        "Quick restart of web server",
                    ),
                ),
            ];

            let mut f_y = modal_y + 55.0;
            for (idx, (label, inp, placeholder)) in snip_fields.iter().enumerate() {
                let l_layout = self.shape_cached(label, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, f_y + 2.0),
                    l_layout,
                    text_dim,
                );

                let inp_rect =
                    LogicalRect::from_xywh(modal_x + 24.0, f_y + 20.0, modal_w - 48.0, 30.0);
                let is_focused = app.add_snippet_modal.active_field == idx;
                let is_hover = app.hover == HoverTarget::ModalSnippetField(idx);

                self.draw_input_box(
                    &mut builder,
                    inp,
                    inp_rect,
                    is_focused,
                    is_hover,
                    placeholder,
                    false,
                    app.caret_visible,
                    accent_mint,
                    card_border,
                    text_main,
                    text_muted,
                );

                f_y += 58.0;
            }

            if let Some(err) = &app.add_snippet_modal.error_msg {
                let err_layout = self.shape_cached(err, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, f_y + 4.0),
                    err_layout,
                    Color::rgba(239, 68, 68, 255),
                );
            }

            // Buttons
            let btn_y = modal_y + modal_h - 48.0;
            let is_cancel_h = app.hover == HoverTarget::ModalSnippetClose;
            let cancel_rect = LogicalRect::from_xywh(modal_x + modal_w - 240.0, btn_y, 90.0, 32.0);
            if is_cancel_h {
                builder.fill_rect(cancel_rect, Color::rgba(30, 48, 85, 220), 4.0);
            }
            builder.stroke_rect(cancel_rect, card_border, 1.0, 4.0);
            let c_layout = self.shape_cached(tr(lang, "Отмена", "Cancel"), StyleId::Small, None);
            builder.text(
                LogicalPoint::new(modal_x + modal_w - 220.0, btn_y + 7.0),
                c_layout,
                text_dim,
            );

            let is_save_h = app.hover == HoverTarget::ModalSnippetSave;
            let save_rect = LogicalRect::from_xywh(modal_x + modal_w - 140.0, btn_y, 116.0, 32.0);
            builder.fill_rect(
                save_rect,
                if is_save_h {
                    Color::rgba(45, 212, 191, 230)
                } else {
                    accent_mint
                },
                4.0,
            );
            let save_btn_text = if app.add_snippet_modal.is_editing {
                tr(lang, "💾 Обновить", "💾 Update")
            } else {
                tr(lang, "💾 Сохранить", "💾 Save")
            };
            let s_layout = self.shape_cached(save_btn_text, StyleId::SansBold, None);
            builder.text(
                LogicalPoint::new(modal_x + modal_w - 130.0, btn_y + 7.0),
                s_layout,
                bg_app,
            );
        }

        // 10. Session Dashboard Modal Dialog
        if let Some(dash) = &app.session_dashboard_modal {
            if dash.is_open {
                builder.fill_rect(
                    LogicalRect::from_xywh(0.0, 0.0, w, h),
                    Color::rgba(0, 0, 0, 180),
                    0.0,
                );
                let modal_w = 640.0f64.min(w - 40.0);
                let modal_h = 520.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;
                let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

                builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
                builder.stroke_rect(modal_rect, accent_mint, 1.5, 8.0);

                // Title
                let t_layout = self.shape_cached(
                    tr(lang, "Дашборд сессии SSH", "SSH Session Dashboard"),
                    StyleId::Title,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, modal_y + 20.0),
                    t_layout,
                    text_main,
                );

                // Close button
                let is_close_hover = app.hover == HoverTarget::SessionDashboardClose;
                let close_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 38.0, modal_y + 14.0, 24.0, 24.0);
                if is_close_hover {
                    builder.fill_rect(close_rect, Color::rgba(239, 68, 68, 200), 5.0);
                    builder.icon(
                        ICON_CLOSE_ID,
                        LogicalRect::from_xywh(
                            modal_x + modal_w - 33.0,
                            modal_y + 19.0,
                            14.0,
                            14.0,
                        ),
                        Color::rgba(255, 255, 255, 255),
                    );
                } else {
                    builder.fill_rect(close_rect, Color::rgba(22, 34, 58, 160), 5.0);
                    builder.icon(
                        ICON_CLOSE_ID,
                        LogicalRect::from_xywh(
                            modal_x + modal_w - 33.0,
                            modal_y + 19.0,
                            14.0,
                            14.0,
                        ),
                        text_muted,
                    );
                }

                // Session Summary Cards / Stats
                let stat_box_y = modal_y + 55.0;
                let stat_box_h = 80.0;
                let stat_rect =
                    LogicalRect::from_xywh(modal_x + 24.0, stat_box_y, modal_w - 48.0, stat_box_h);
                builder.fill_rect(stat_rect, Color::rgba(20, 32, 58, 255), 6.0);
                builder.stroke_rect(stat_rect, card_border, 1.0, 6.0);

                let sess_info = format!(
                    "{}: {}@{}",
                    tr(lang, "Хост", "Host"),
                    dash.session.username,
                    dash.session.host_address
                );
                let s_layout = self.shape_cached(&sess_info, StyleId::SansBold, None);
                builder.text(
                    LogicalPoint::new(modal_x + 36.0, stat_box_y + 12.0),
                    s_layout,
                    accent_mint,
                );

                let start_str = dash
                    .session
                    .started_at
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
                let end_str = dash
                    .session
                    .ended_at
                    .map(|t| t.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| tr(lang, "Активна", "Active").into());
                let dur_str = dash
                    .session
                    .ended_at
                    .map(|e| {
                        let secs = (e - dash.session.started_at).num_seconds();
                        format!("{} {}", secs, tr(lang, "сек", "sec"))
                    })
                    .unwrap_or_else(|| tr(lang, "В процессе", "In progress").into());
                let sub_info = format!(
                    "{}: {} | {}: {} | {}: {}",
                    tr(lang, "Старт", "Start"),
                    start_str,
                    tr(lang, "Завершение", "End"),
                    end_str,
                    tr(lang, "Длительность", "Duration"),
                    dur_str
                );
                let sub_layout = self.shape_cached(&sub_info, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 36.0, stat_box_y + 36.0),
                    sub_layout,
                    text_dim,
                );

                let status_line = format!(
                    "{}: {} | {}: {}",
                    tr(lang, "Статус", "Status"),
                    dash.session.status,
                    tr(lang, "Зафиксировано команд", "Commands logged"),
                    dash.commands.len()
                );
                let st_layout = self.shape_cached(&status_line, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + 36.0, stat_box_y + 56.0),
                    st_layout,
                    text_muted,
                );

                // Command history header
                let cmd_h_layout = self.shape_cached(
                    tr(lang, "Журнал команд сессии:", "Session Command Log:"),
                    StyleId::SansBold,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, modal_y + 148.0),
                    cmd_h_layout,
                    text_main,
                );

                // Command list
                let cmd_start_y = modal_y + 170.0;
                if dash.commands.is_empty() {
                    let empty_l = self.shape_cached(
                        tr(
                            lang,
                            "В этой сессии еще не выполнялись команды",
                            "No commands executed in this session",
                        ),
                        StyleId::Small,
                        None,
                    );
                    builder.text(
                        LogicalPoint::new(modal_x + 36.0, cmd_start_y + 20.0),
                        empty_l,
                        text_muted,
                    );
                } else {
                    for (cmd_idx, cmd) in dash.commands.iter().enumerate().take(7) {
                        let item_y = cmd_start_y + (cmd_idx as f64) * 38.0;
                        let item_rect =
                            LogicalRect::from_xywh(modal_x + 24.0, item_y, modal_w - 48.0, 32.0);
                        builder.fill_rect(item_rect, Color::rgba(18, 29, 52, 200), 4.0);
                        builder.stroke_rect(item_rect, card_border, 1.0, 4.0);

                        let time_str = cmd.executed_at.format("%H:%M:%S").to_string();
                        let time_l = self.shape_cached(&time_str, StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 34.0, item_y + 8.0),
                            time_l,
                            text_muted,
                        );

                        let cmd_l = self.shape_cached(
                            &cmd.command,
                            StyleId::Mono,
                            Some(LogicalPx(modal_w - 180.0)),
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 100.0, item_y + 7.0),
                            cmd_l,
                            text_main,
                        );

                        // Copy button
                        let is_copy_h = app.hover == HoverTarget::SessionDashboardCopyCmd(cmd_idx);
                        let copy_rect = LogicalRect::from_xywh(
                            modal_x + modal_w - 96.0,
                            item_y + 4.0,
                            28.0,
                            24.0,
                        );
                        if is_copy_h {
                            builder.fill_rect(copy_rect, Color::rgba(45, 212, 191, 230), 4.0);
                            builder.icon(
                                ICON_COPY_ID,
                                LogicalRect::from_xywh(
                                    modal_x + modal_w - 89.0,
                                    item_y + 9.0,
                                    14.0,
                                    14.0,
                                ),
                                Color::rgba(8, 13, 25, 255),
                            );
                        } else {
                            builder.fill_rect(copy_rect, Color::rgba(22, 38, 64, 200), 4.0);
                            builder.stroke_rect(copy_rect, Color::rgba(45, 212, 191, 60), 1.0, 4.0);
                            builder.icon(
                                ICON_COPY_ID,
                                LogicalRect::from_xywh(
                                    modal_x + modal_w - 89.0,
                                    item_y + 9.0,
                                    14.0,
                                    14.0,
                                ),
                                Color::rgba(148, 163, 184, 200),
                            );
                        }

                        // Run button
                        let is_run_h = app.hover == HoverTarget::SessionDashboardRunCmd(cmd_idx);
                        let run_rect = LogicalRect::from_xywh(
                            modal_x + modal_w - 60.0,
                            item_y + 4.0,
                            28.0,
                            24.0,
                        );
                        if is_run_h {
                            builder.fill_rect(run_rect, accent_mint, 4.0);
                            builder.icon(
                                ICON_CONNECT_ID,
                                LogicalRect::from_xywh(
                                    modal_x + modal_w - 53.0,
                                    item_y + 9.0,
                                    14.0,
                                    14.0,
                                ),
                                Color::rgba(8, 13, 25, 255),
                            );
                        } else {
                            builder.fill_rect(run_rect, Color::rgba(20, 42, 68, 200), 4.0);
                            builder.stroke_rect(run_rect, Color::rgba(45, 212, 191, 60), 1.0, 4.0);
                            builder.icon(
                                ICON_CONNECT_ID,
                                LogicalRect::from_xywh(
                                    modal_x + modal_w - 53.0,
                                    item_y + 9.0,
                                    14.0,
                                    14.0,
                                ),
                                accent_mint,
                            );
                        }
                    }
                }

                // Bottom buttons: Delete session and Reconnect
                let btn_y = modal_y + modal_h - 48.0;

                let is_del_h = app.hover == HoverTarget::SessionDashboardDelete;
                let del_rect = LogicalRect::from_xywh(modal_x + 24.0, btn_y, 140.0, 34.0);
                if is_del_h {
                    builder.fill_rect(del_rect, Color::rgba(239, 68, 68, 220), 4.0);
                } else {
                    builder.fill_rect(del_rect, Color::rgba(60, 25, 35, 180), 4.0);
                }
                builder.stroke_rect(del_rect, Color::rgba(239, 68, 68, 120), 1.0, 4.0);
                let del_l = self.shape_cached(
                    tr(lang, "Удалить сессию", "Delete Session"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 36.0, btn_y + 8.0),
                    del_l,
                    text_main,
                );

                let is_rec_h = app.hover == HoverTarget::SessionDashboardReconnect;
                let rec_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 234.0, btn_y, 210.0, 34.0);
                let rec_txt = tr(lang, "Подключиться снова", "Reconnect Session");
                if is_rec_h {
                    builder.fill_rect(rec_rect, accent_mint, 5.0);
                    builder.icon(
                        ICON_CONNECT_ID,
                        LogicalRect::from_xywh(modal_x + modal_w - 222.0, btn_y + 10.0, 14.0, 14.0),
                        Color::rgba(9, 14, 26, 255),
                    );
                    let rec_l = self.shape_cached(rec_txt, StyleId::SansBold, None);
                    builder.text(
                        LogicalPoint::new(modal_x + modal_w - 200.0, btn_y + 8.0),
                        rec_l,
                        Color::rgba(9, 14, 26, 255),
                    );
                } else {
                    builder.fill_rect(rec_rect, Color::rgba(20, 35, 65, 240), 5.0);
                    builder.stroke_rect(rec_rect, accent_mint, 1.2, 5.0);
                    builder.icon(
                        ICON_CONNECT_ID,
                        LogicalRect::from_xywh(modal_x + modal_w - 222.0, btn_y + 10.0, 14.0, 14.0),
                        accent_mint,
                    );
                    let rec_l = self.shape_cached(rec_txt, StyleId::SansBold, None);
                    builder.text(
                        LogicalPoint::new(modal_x + modal_w - 200.0, btn_y + 8.0),
                        rec_l,
                        text_main,
                    );
                }
            }
        }

        // 11. Input Context Menu Popup (Cut, Copy, Paste, Select All)
        if let Some(menu) = &app.input_context_menu {
            if menu.is_open {
                let menu_w = 170.0;
                let menu_h = 132.0;
                let menu_rect = LogicalRect::from_xywh(menu.x, menu.y, menu_w, menu_h);
                builder.fill_rect(menu_rect, Color::rgba(14, 23, 42, 252), 6.0);
                builder.stroke_rect(menu_rect, accent_mint, 1.2, 6.0);

                let items = [
                    (
                        InputMenuAction::Cut,
                        tr(lang, "✂  Вырезать", "✂  Cut"),
                        "Ctrl+X",
                    ),
                    (
                        InputMenuAction::Copy,
                        tr(lang, "📋 Копировать", "📋 Copy"),
                        "Ctrl+C",
                    ),
                    (
                        InputMenuAction::Paste,
                        tr(lang, "⚡ Вставить", "⚡ Paste"),
                        "Ctrl+V",
                    ),
                    (
                        InputMenuAction::SelectAll,
                        tr(lang, "🔲 Выделить всё", "🔲 Select All"),
                        "Ctrl+A",
                    ),
                ];

                let mut item_y = menu.y + 6.0;
                for (action, label, shortcut) in items {
                    let is_hover = app.hover == HoverTarget::InputContextMenu(action);
                    let item_rect =
                        LogicalRect::from_xywh(menu.x + 4.0, item_y, menu_w - 8.0, 28.0);
                    if is_hover {
                        builder.fill_rect(item_rect, Color::rgba(30, 48, 85, 230), 4.0);
                    }
                    let l_layout = self.shape_cached(label, StyleId::Sans, None);
                    builder.text(
                        LogicalPoint::new(menu.x + 10.0, item_y + 5.0),
                        l_layout,
                        if is_hover { text_main } else { text_dim },
                    );

                    let sc_layout = self.shape_cached(shortcut, StyleId::Small, None);
                    builder.text(
                        LogicalPoint::new(menu.x + menu_w - 56.0, item_y + 6.0),
                        sc_layout,
                        text_muted,
                    );

                    item_y += 30.0;
                }
            }
        }

        // 12. Confirm Delete Host Modal
        if let Some(confirm) = &app.confirm_delete_modal {
            if confirm.is_open {
                builder.fill_rect(
                    LogicalRect::from_xywh(0.0, 0.0, w, h),
                    Color::rgba(0, 0, 0, 190),
                    0.0,
                );

                let modal_w = 480.0f64.min(w - 40.0);
                let modal_h = 210.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;
                let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

                builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
                builder.stroke_rect(modal_rect, Color::rgba(239, 68, 68, 200), 1.5, 8.0);

                // Warning Icon + Title
                builder.fill_rect(
                    LogicalRect::from_xywh(modal_x + 24.0, modal_y + 22.0, 36.0, 36.0),
                    Color::rgba(239, 68, 68, 40),
                    18.0,
                );
                builder.icon(
                    ICON_ALERT_ID,
                    LogicalRect::from_xywh(modal_x + 32.0, modal_y + 30.0, 20.0, 20.0),
                    Color::rgba(239, 68, 68, 255),
                );

                let title_l = self.shape_cached(
                    tr(lang, "Удалить сервер?", "Delete Host?"),
                    StyleId::Title,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 70.0, modal_y + 28.0),
                    title_l,
                    text_main,
                );

                // Host details
                let msg = format!(
                    "{} «{}»?",
                    tr(
                        lang,
                        "Вы действительно хотите удалить сервер",
                        "Are you sure you want to delete host"
                    ),
                    confirm.host_name
                );
                let msg_l =
                    self.shape_cached(&msg, StyleId::SansBold, Some(LogicalPx(modal_w - 50.0)));
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, modal_y + 76.0),
                    msg_l,
                    text_main,
                );

                let sub_msg = format!(
                    "{}: {}. {}",
                    tr(lang, "Адрес", "Address"),
                    confirm.host_address,
                    tr(
                        lang,
                        "Это действие нельзя отменить.",
                        "This action cannot be undone."
                    )
                );
                let sub_l =
                    self.shape_cached(&sub_msg, StyleId::Small, Some(LogicalPx(modal_w - 50.0)));
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, modal_y + 104.0),
                    sub_l,
                    text_muted,
                );

                // Buttons: Cancel (Esc) and Delete (Enter)
                let btn_y = modal_y + modal_h - 52.0;

                // Cancel Button
                let is_cancel_h = app.hover == HoverTarget::ConfirmDeleteCancel;
                let cancel_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 240.0, btn_y, 110.0, 34.0);
                if is_cancel_h {
                    builder.fill_rect(cancel_rect, Color::rgba(30, 48, 85, 230), 5.0);
                } else {
                    builder.fill_rect(cancel_rect, Color::rgba(20, 32, 58, 200), 5.0);
                }
                builder.stroke_rect(cancel_rect, card_border, 1.0, 5.0);
                let cancel_l = self.shape_cached(
                    tr(lang, "Отмена (Esc)", "Cancel (Esc)"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 224.0, btn_y + 8.0),
                    cancel_l,
                    text_dim,
                );

                // Delete Button
                let is_del_h = app.hover == HoverTarget::ConfirmDeleteConfirm;
                let del_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 120.0, btn_y, 100.0, 34.0);
                if is_del_h {
                    builder.fill_rect(del_rect, Color::rgba(220, 38, 38, 255), 5.0);
                } else {
                    builder.fill_rect(del_rect, Color::rgba(239, 68, 68, 230), 5.0);
                }
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 110.0, btn_y + 10.0, 14.0, 14.0),
                    Color::rgba(255, 255, 255, 255),
                );
                let del_btn_l =
                    self.shape_cached(tr(lang, "Удалить", "Delete"), StyleId::SansBold, None);
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 90.0, btn_y + 7.0),
                    del_btn_l,
                    Color::rgba(255, 255, 255, 255),
                );
            }
        }

        // 12.1 Confirm Delete Snippet Modal
        if let Some(confirm_snip) = &app.confirm_delete_snippet_modal {
            if confirm_snip.is_open {
                builder.fill_rect(
                    LogicalRect::from_xywh(0.0, 0.0, w, h),
                    Color::rgba(0, 0, 0, 190),
                    0.0,
                );

                let modal_w = 480.0f64.min(w - 40.0);
                let modal_h = 210.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;
                let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

                builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
                builder.stroke_rect(modal_rect, Color::rgba(239, 68, 68, 200), 1.5, 8.0);

                // Warning Icon + Title
                builder.fill_rect(
                    LogicalRect::from_xywh(modal_x + 24.0, modal_y + 22.0, 36.0, 36.0),
                    Color::rgba(239, 68, 68, 40),
                    18.0,
                );
                builder.icon(
                    ICON_ALERT_ID,
                    LogicalRect::from_xywh(modal_x + 32.0, modal_y + 30.0, 20.0, 20.0),
                    Color::rgba(239, 68, 68, 255),
                );

                let title_l = self.shape_cached(
                    tr(lang, "Удалить сниппет?", "Delete Snippet?"),
                    StyleId::Title,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 70.0, modal_y + 28.0),
                    title_l,
                    text_main,
                );

                // Snippet details
                let msg = format!(
                    "{} «{}»?",
                    tr(
                        lang,
                        "Вы действительно хотите удалить сниппет",
                        "Are you sure you want to delete snippet"
                    ),
                    confirm_snip.snippet_title
                );
                let msg_l = self.shape_clamped_cached(
                    &msg,
                    StyleId::SansBold,
                    LogicalPx(modal_w - 50.0),
                    1,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, modal_y + 76.0),
                    msg_l,
                    text_main,
                );

                let sub_msg = format!(
                    "{}: {}. {}",
                    tr(lang, "Команда", "Command"),
                    confirm_snip.snippet_command,
                    tr(
                        lang,
                        "Это действие нельзя отменить.",
                        "This action cannot be undone."
                    )
                );
                let sub_l = self.shape_clamped_cached(
                    &sub_msg,
                    StyleId::Small,
                    LogicalPx(modal_w - 50.0),
                    1,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, modal_y + 104.0),
                    sub_l,
                    text_muted,
                );

                // Buttons: Cancel (Esc) and Delete (Enter)
                let btn_y = modal_y + modal_h - 52.0;

                // Cancel Button
                let is_cancel_h = app.hover == HoverTarget::ConfirmDeleteSnippetCancel;
                let cancel_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 240.0, btn_y, 110.0, 34.0);
                if is_cancel_h {
                    builder.fill_rect(cancel_rect, Color::rgba(30, 48, 85, 230), 5.0);
                } else {
                    builder.fill_rect(cancel_rect, Color::rgba(20, 32, 58, 200), 5.0);
                }
                builder.stroke_rect(cancel_rect, card_border, 1.0, 5.0);
                let cancel_l = self.shape_cached(
                    tr(lang, "Отмена (Esc)", "Cancel (Esc)"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 224.0, btn_y + 8.0),
                    cancel_l,
                    text_dim,
                );

                // Delete Button
                let is_del_h = app.hover == HoverTarget::ConfirmDeleteSnippetConfirm;
                let del_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 120.0, btn_y, 100.0, 34.0);
                if is_del_h {
                    builder.fill_rect(del_rect, Color::rgba(220, 38, 38, 255), 5.0);
                } else {
                    builder.fill_rect(del_rect, Color::rgba(239, 68, 68, 230), 5.0);
                }
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 110.0, btn_y + 10.0, 14.0, 14.0),
                    Color::rgba(255, 255, 255, 255),
                );
                let del_btn_l =
                    self.shape_cached(tr(lang, "Удалить", "Delete"), StyleId::SansBold, None);
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 90.0, btn_y + 7.0),
                    del_btn_l,
                    Color::rgba(255, 255, 255, 255),
                );
            }
        }

        // 12.5 Confirm Run Command Modal
        if let Some(confirm_run) = &app.confirm_run_modal {
            if confirm_run.is_open {
                builder.fill_rect(
                    LogicalRect::from_xywh(0.0, 0.0, w, h),
                    Color::rgba(0, 0, 0, 190),
                    0.0,
                );

                let layout = confirm_run.layout(w, h);
                let modal_w = layout.modal_w;
                let modal_h = layout.modal_h;
                let modal_x = layout.modal_x;
                let modal_y = layout.modal_y;
                let cmd_box_h = layout.cmd_box_h;
                let btn_y = layout.btn_y;
                let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

                let border_color = if confirm_run.is_dangerous {
                    Color::rgba(245, 158, 11, 255)
                } else {
                    accent_mint
                };

                builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
                builder.stroke_rect(modal_rect, border_color, 1.5, 8.0);

                // Top Icon + Header
                let icon_id = if confirm_run.is_dangerous {
                    ICON_ALERT_ID
                } else {
                    ICON_CONNECT_ID
                };
                let icon_bg = if confirm_run.is_dangerous {
                    Color::rgba(245, 158, 11, 40)
                } else {
                    Color::rgba(45, 212, 191, 40)
                };
                let icon_fg = if confirm_run.is_dangerous {
                    Color::rgba(245, 158, 11, 255)
                } else {
                    accent_mint
                };

                builder.fill_rect(
                    LogicalRect::from_xywh(modal_x + 24.0, modal_y + 20.0, 36.0, 36.0),
                    icon_bg,
                    18.0,
                );
                builder.icon(
                    icon_id,
                    LogicalRect::from_xywh(modal_x + 32.0, modal_y + 28.0, 20.0, 20.0),
                    icon_fg,
                );

                let title_text = if confirm_run.paste {
                    tr(lang, "Подтверждение вставки", "Confirm Paste")
                } else if confirm_run.is_dangerous {
                    tr(
                        lang,
                        "⚠️ Подтверждение опасной команды",
                        "⚠️ Dangerous Command Confirmation",
                    )
                } else {
                    tr(
                        lang,
                        "Подтверждение запуска команды",
                        "Confirm Command Execution",
                    )
                };
                let title_l = self.shape_cached(title_text, StyleId::Title, None);
                builder.text(
                    LogicalPoint::new(modal_x + 70.0, modal_y + 20.0),
                    title_l,
                    text_main,
                );

                let src_l = self.shape_cached(
                    &confirm_run.source_title,
                    StyleId::Small,
                    Some(LogicalPx(modal_w - 95.0)),
                );
                builder.text(
                    LogicalPoint::new(modal_x + 70.0, modal_y + 42.0),
                    src_l,
                    text_dim,
                );

                let mut cur_y = modal_y + 68.0;

                // Dangerous reason warning banner
                if let Some(reason) = &confirm_run.danger_reason {
                    let banner_rect =
                        LogicalRect::from_xywh(modal_x + 24.0, cur_y, modal_w - 48.0, 32.0);
                    builder.fill_rect(banner_rect, Color::rgba(245, 158, 11, 30), 5.0);
                    builder.stroke_rect(banner_rect, Color::rgba(245, 158, 11, 140), 1.0, 5.0);
                    let warn_msg = format!("{}: {reason}", tr(lang, "Внимание", "Warning"));
                    let warn_l = self.shape_cached(
                        &warn_msg,
                        StyleId::Small,
                        Some(LogicalPx(modal_w - 60.0)),
                    );
                    builder.text(
                        LogicalPoint::new(modal_x + 36.0, cur_y + 8.0),
                        warn_l,
                        Color::rgba(252, 211, 77, 255),
                    );
                    cur_y += 40.0;
                }

                // Command Preview Box (Monospace)
                let cmd_rect =
                    LogicalRect::from_xywh(modal_x + 24.0, cur_y, modal_w - 48.0, cmd_box_h);
                builder.fill_rect(cmd_rect, Color::rgba(10, 16, 30, 255), 5.0);
                builder.stroke_rect(cmd_rect, card_border, 1.0, 5.0);

                builder.push_clip(LogicalRect::from_xywh(
                    modal_x + 28.0,
                    cur_y + 4.0,
                    modal_w - 56.0,
                    cmd_box_h - 8.0,
                ));
                let cmd_l = self.shape_cached(
                    &confirm_run.command,
                    StyleId::Mono,
                    Some(LogicalPx(modal_w - 68.0)),
                );
                builder.text(
                    LogicalPoint::new(modal_x + 36.0, cur_y + 12.0),
                    cmd_l,
                    Color::rgba(56, 189, 248, 255),
                );
                let _ = builder.pop_clip();

                cur_y += cmd_box_h + 10.0;

                // Target terminal status line
                let target_info = format!(
                    "{}: {}",
                    tr(lang, "Сессия", "Session"),
                    confirm_run.target_tab_title
                );
                let target_l = self.shape_cached(
                    &target_info,
                    StyleId::Small,
                    Some(LogicalPx(modal_w - 50.0)),
                );
                let target_fg = if confirm_run.can_execute {
                    accent_emerald
                } else {
                    Color::rgba(239, 68, 68, 255)
                };
                builder.text(
                    LogicalPoint::new(modal_x + 26.0, cur_y + 4.0),
                    target_l,
                    target_fg,
                );

                // Bottom Buttons: Cancel, Copy, Execute
                // 1. Cancel (Esc)
                let is_cancel_h = app.hover == HoverTarget::ConfirmRunCancel;
                let cancel_rect = LogicalRect::from_xywh(
                    layout.cancel_rect.0,
                    layout.cancel_rect.1,
                    layout.cancel_rect.2,
                    layout.cancel_rect.3,
                );
                builder.fill_rect(
                    cancel_rect,
                    if is_cancel_h {
                        Color::rgba(30, 48, 85, 230)
                    } else {
                        Color::rgba(20, 32, 58, 200)
                    },
                    5.0,
                );
                builder.stroke_rect(cancel_rect, card_border, 1.0, 5.0);
                let cancel_l = self.shape_cached(
                    tr(lang, "Отмена (Esc)", "Cancel (Esc)"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 350.0, btn_y + 8.0),
                    cancel_l,
                    text_dim,
                );

                // 2. Copy Button
                let is_copy_h = app.hover == HoverTarget::ConfirmRunCopy;
                let copy_rect = LogicalRect::from_xywh(
                    layout.copy_rect.0,
                    layout.copy_rect.1,
                    layout.copy_rect.2,
                    layout.copy_rect.3,
                );
                builder.fill_rect(
                    copy_rect,
                    if is_copy_h {
                        Color::rgba(35, 55, 95, 230)
                    } else {
                        Color::rgba(20, 35, 65, 200)
                    },
                    5.0,
                );
                builder.stroke_rect(copy_rect, Color::rgba(45, 212, 191, 100), 1.0, 5.0);
                builder.icon(
                    ICON_COPY_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 244.0, btn_y + 10.0, 14.0, 14.0),
                    accent_mint,
                );
                let copy_label = if confirm_run.copied_feedback {
                    tr(lang, "Скопировано!", "Copied!")
                } else {
                    tr(lang, "Копировать", "Copy")
                };
                let copy_l = self.shape_cached(copy_label, StyleId::Small, None);
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 224.0, btn_y + 8.0),
                    copy_l,
                    accent_mint,
                );

                // 3. Execute Button (Enter)
                let is_exec_h = app.hover == HoverTarget::ConfirmRunExecute;
                let exec_rect = LogicalRect::from_xywh(
                    layout.exec_rect.0,
                    layout.exec_rect.1,
                    layout.exec_rect.2,
                    layout.exec_rect.3,
                );
                let exec_txt = if confirm_run.paste {
                    tr(lang, "Вставить", "Paste")
                } else {
                    tr(lang, "Выполнить", "Execute")
                };

                if !confirm_run.can_execute {
                    builder.fill_rect(exec_rect, Color::rgba(30, 42, 65, 140), 5.0);
                    builder.stroke_rect(exec_rect, card_border, 1.0, 5.0);
                    let exec_l = self.shape_cached(exec_txt, StyleId::SansBold, None);
                    builder.text(
                        LogicalPoint::new(modal_x + modal_w - 110.0, btn_y + 7.0),
                        exec_l,
                        text_muted,
                    );
                } else if confirm_run.is_dangerous {
                    if is_exec_h {
                        builder.fill_rect(exec_rect, Color::rgba(220, 38, 38, 255), 5.0);
                    } else {
                        builder.fill_rect(exec_rect, Color::rgba(239, 68, 68, 230), 5.0);
                    }
                    builder.icon(
                        ICON_CONNECT_ID,
                        LogicalRect::from_xywh(modal_x + modal_w - 122.0, btn_y + 10.0, 14.0, 14.0),
                        Color::rgba(255, 255, 255, 255),
                    );
                    let exec_l = self.shape_cached(exec_txt, StyleId::SansBold, None);
                    builder.text(
                        LogicalPoint::new(modal_x + modal_w - 100.0, btn_y + 7.0),
                        exec_l,
                        Color::rgba(255, 255, 255, 255),
                    );
                } else {
                    if is_exec_h {
                        builder.fill_rect(exec_rect, accent_mint, 5.0);
                        builder.icon(
                            ICON_CONNECT_ID,
                            LogicalRect::from_xywh(
                                modal_x + modal_w - 122.0,
                                btn_y + 10.0,
                                14.0,
                                14.0,
                            ),
                            Color::rgba(9, 14, 26, 255),
                        );
                        let exec_l = self.shape_cached(exec_txt, StyleId::SansBold, None);
                        builder.text(
                            LogicalPoint::new(modal_x + modal_w - 100.0, btn_y + 7.0),
                            exec_l,
                            Color::rgba(9, 14, 26, 255),
                        );
                    } else {
                        builder.fill_rect(exec_rect, Color::rgba(20, 36, 60, 255), 5.0);
                        builder.stroke_rect(exec_rect, accent_mint, 1.2, 5.0);
                        builder.icon(
                            ICON_CONNECT_ID,
                            LogicalRect::from_xywh(
                                modal_x + modal_w - 122.0,
                                btn_y + 10.0,
                                14.0,
                                14.0,
                            ),
                            accent_mint,
                        );
                        let exec_l = self.shape_cached(exec_txt, StyleId::SansBold, None);
                        builder.text(
                            LogicalPoint::new(modal_x + modal_w - 100.0, btn_y + 7.0),
                            exec_l,
                            accent_mint,
                        );
                    }
                }
            }
        }

        // 13. Settings Modal
        if let Some(settings) = &app.settings_modal {
            if settings.is_open {
                builder.fill_rect(
                    LogicalRect::from_xywh(0.0, 0.0, w, h),
                    Color::rgba(0, 0, 0, 190),
                    0.0,
                );

                let modal_w = 620.0f64.min(w - 40.0);
                let modal_h = 490.0f64.min(h - 40.0);
                let modal_x = (w - modal_w) / 2.0;
                let modal_y = (h - modal_h) / 2.0;
                let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

                builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
                builder.stroke_rect(modal_rect, accent_mint, 1.5, 8.0);

                // Title with Icon
                builder.icon(
                    ICON_SETTINGS_ID,
                    LogicalRect::from_xywh(modal_x + 24.0, modal_y + 20.0, 20.0, 20.0),
                    accent_mint,
                );
                let t_layout = self.shape_cached(
                    tr(lang, "Настройки EvertySSH", "EvertySSH Settings"),
                    StyleId::Title,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + 52.0, modal_y + 18.0),
                    t_layout,
                    text_main,
                );

                // Close button
                let is_close_hover = app.hover == HoverTarget::SettingsClose;
                let close_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 38.0, modal_y + 14.0, 24.0, 24.0);
                if is_close_hover {
                    builder.fill_rect(close_rect, Color::rgba(239, 68, 68, 200), 5.0);
                    builder.icon(
                        ICON_CLOSE_ID,
                        LogicalRect::from_xywh(
                            modal_x + modal_w - 33.0,
                            modal_y + 19.0,
                            14.0,
                            14.0,
                        ),
                        Color::rgba(255, 255, 255, 255),
                    );
                } else {
                    builder.fill_rect(close_rect, Color::rgba(22, 34, 58, 160), 5.0);
                    builder.icon(
                        ICON_CLOSE_ID,
                        LogicalRect::from_xywh(
                            modal_x + modal_w - 33.0,
                            modal_y + 19.0,
                            14.0,
                            14.0,
                        ),
                        text_muted,
                    );
                }

                // Tabs: ℹ️ О нас | 🔒 Безопасность | 💾 Резервная копия | ⚙ Параметры
                let tabs_y = modal_y + 55.0;
                let tabs_info = [
                    (
                        SettingsTab::About,
                        tr(lang, "ℹ️ О нас", "ℹ️ About"),
                        modal_x + 24.0,
                        110.0,
                    ),
                    (
                        SettingsTab::Security,
                        tr(lang, "🔒 Безопасность", "🔒 Security"),
                        modal_x + 140.0,
                        135.0,
                    ),
                    (
                        SettingsTab::Backup,
                        tr(lang, "💾 Резервная копия", "💾 Backup"),
                        modal_x + 281.0,
                        145.0,
                    ),
                    (
                        SettingsTab::General,
                        tr(lang, "⚙ Параметры", "⚙ General"),
                        modal_x + 432.0,
                        130.0,
                    ),
                ];

                for (tab_type, tab_label, tab_x, tab_w) in tabs_info {
                    let is_active = settings.active_tab == tab_type;
                    let is_hover = app.hover == HoverTarget::SettingsTab(tab_type);
                    let tab_rect = LogicalRect::from_xywh(tab_x, tabs_y, tab_w, 32.0);

                    if is_active {
                        builder.fill_rect(tab_rect, Color::rgba(25, 41, 72, 255), 5.0);
                        builder.stroke_rect(tab_rect, accent_mint, 1.2, 5.0);
                    } else if is_hover {
                        builder.fill_rect(tab_rect, Color::rgba(20, 32, 58, 255), 5.0);
                        builder.stroke_rect(tab_rect, Color::rgba(45, 212, 191, 100), 1.0, 5.0);
                    } else {
                        builder.fill_rect(tab_rect, Color::rgba(16, 26, 48, 200), 5.0);
                        builder.stroke_rect(tab_rect, card_border, 1.0, 5.0);
                    }

                    let t_l = self.shape_cached(
                        tab_label,
                        if is_active {
                            StyleId::SansBold
                        } else {
                            StyleId::Sans
                        },
                        None,
                    );
                    builder.text(
                        LogicalPoint::new(tab_x + 14.0, tabs_y + 7.0),
                        t_l,
                        if is_active {
                            accent_mint
                        } else if is_hover {
                            text_main
                        } else {
                            text_dim
                        },
                    );
                }

                // Separator line under tabs
                builder.fill_rect(
                    LogicalRect::from_xywh(modal_x + 24.0, tabs_y + 36.0, modal_w - 48.0, 1.0),
                    card_border,
                    0.0,
                );

                // Tab Contents
                match settings.active_tab {
                    SettingsTab::About => {
                        // 1. App Header (Logo, Name, Version badge, Tagline)
                        let hdr_y = modal_y + 98.0;
                        let logo_rect = LogicalRect::from_xywh(modal_x + 24.0, hdr_y, 44.0, 44.0);
                        builder.fill_rect(logo_rect, Color::rgba(45, 212, 191, 35), 10.0);
                        builder.stroke_rect(logo_rect, accent_mint, 1.2, 10.0);
                        builder.icon(
                            ICON_CONNECT_ID,
                            LogicalRect::from_xywh(modal_x + 36.0, hdr_y + 12.0, 20.0, 20.0),
                            accent_mint,
                        );

                        // App Title
                        let title_l = self.shape_cached("EvertySSH", StyleId::Title, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 78.0, hdr_y + 2.0),
                            title_l,
                            text_main,
                        );

                        // Version badge
                        let ver_txt = concat!("v", env!("CARGO_PKG_VERSION"));
                        let ver_rect =
                            LogicalRect::from_xywh(modal_x + 195.0, hdr_y + 4.0, 56.0, 20.0);
                        builder.fill_rect(ver_rect, Color::rgba(45, 212, 191, 30), 4.0);
                        builder.stroke_rect(ver_rect, Color::rgba(45, 212, 191, 120), 1.0, 4.0);
                        let ver_l = self.shape_cached(ver_txt, StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 203.0, hdr_y + 7.0),
                            ver_l,
                            accent_mint,
                        );

                        // Tagline
                        let sub_txt = tr(
                            lang,
                            "Быстрый, современный и безопасный SSH-клиент со сниппетами и сплит-экраном",
                            "Fast, modern & secure SSH client with snippets, splits & history",
                        );
                        let sub_l = self.shape_cached(
                            sub_txt,
                            StyleId::Small,
                            Some(LogicalPx(modal_w - 110.0)),
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 78.0, hdr_y + 26.0),
                            sub_l,
                            text_dim,
                        );

                        // 2. Author card
                        let author_y = modal_y + 158.0;
                        let author_rect =
                            LogicalRect::from_xywh(modal_x + 24.0, author_y, modal_w - 48.0, 56.0);
                        builder.fill_rect(author_rect, Color::rgba(18, 28, 52, 220), 8.0);
                        builder.stroke_rect(author_rect, card_border, 1.0, 8.0);

                        // Author Avatar Icon (Stylish AV initials badge)
                        let av_rect =
                            LogicalRect::from_xywh(modal_x + 36.0, author_y + 10.0, 36.0, 36.0);
                        builder.fill_rect(av_rect, Color::rgba(45, 212, 191, 30), 18.0);
                        builder.stroke_rect(av_rect, accent_mint, 1.2, 18.0);
                        let av_sym = self.shape_cached("AV", StyleId::SansBold, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 44.0, author_y + 19.0),
                            av_sym,
                            accent_mint,
                        );

                        // Author info
                        let author_title = tr(lang, "Автор & Разработчик", "Author & Developer");
                        let at_l = self.shape_cached(author_title, StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 82.0, author_y + 10.0),
                            at_l,
                            text_dim,
                        );

                        let author_name = "Arthur Valiev";
                        let an_l = self.shape_cached(author_name, StyleId::SansBold, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 82.0, author_y + 28.0),
                            an_l,
                            text_main,
                        );

                        // Ecosystem badge on right
                        let badge_txt = "Everty Ecosystem";
                        let bd_l = self.shape_cached(badge_txt, StyleId::Small, None);
                        let bd_rect = LogicalRect::from_xywh(
                            modal_x + modal_w - 165.0,
                            author_y + 16.0,
                            125.0,
                            24.0,
                        );
                        builder.fill_rect(bd_rect, Color::rgba(16, 185, 129, 25), 5.0);
                        builder.stroke_rect(bd_rect, Color::rgba(52, 211, 153, 100), 1.0, 5.0);
                        builder.text(
                            LogicalPoint::new(modal_x + modal_w - 153.0, author_y + 21.0),
                            bd_l,
                            accent_emerald,
                        );

                        // 3. Links Section Title
                        let sec_l = self.shape_cached(
                            tr(lang, "Ссылки и контакты", "Links & Contacts"),
                            StyleId::SansBold,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 24.0, modal_y + 224.0),
                            sec_l,
                            text_dim,
                        );

                        // 4. Two columns of interactive link cards (2x2 grid)
                        let c1_x = modal_x + 24.0;
                        let c2_x = modal_x + 316.0;
                        let c_w = 276.0;
                        let r1_y = modal_y + 244.0;
                        let r2_y = modal_y + 308.0;
                        let c_h = 54.0;

                        let links = [
                            (
                                HoverTarget::SettingsAboutLinkDesk,
                                LogicalRect::from_xywh(c1_x, r1_y, c_w, c_h),
                                ICON_CONNECT_ID,
                                accent_mint,
                                "Everty Desk",
                                "desk.everty.ru",
                            ),
                            (
                                HoverTarget::SettingsAboutLinkTelegram,
                                LogicalRect::from_xywh(c2_x, r1_y, c_w, c_h),
                                ICON_REFRESH_ID,
                                Color::rgba(56, 189, 248, 255),
                                "Telegram",
                                "@Everty_web",
                            ),
                            (
                                HoverTarget::SettingsAboutLinkEmail,
                                LogicalRect::from_xywh(c1_x, r2_y, c_w, c_h),
                                ICON_EDIT_ID,
                                Color::rgba(251, 191, 36, 255),
                                "Email",
                                "info@everty.ru",
                            ),
                            (
                                HoverTarget::SettingsAboutLinkGithub,
                                LogicalRect::from_xywh(c2_x, r2_y, c_w, c_h),
                                ICON_KEY_ID,
                                Color::rgba(168, 85, 247, 255),
                                "GitHub",
                                "vaalimusic/EvertySSH",
                            ),
                        ];

                        for (target, rect, icon_id, icon_color, title, sub) in links {
                            let is_h = app.hover == target;
                            let bg = if is_h {
                                Color::rgba(26, 43, 76, 255)
                            } else {
                                Color::rgba(16, 26, 48, 200)
                            };
                            let border = if is_h { accent_mint } else { card_border };
                            builder.fill_rect(rect, bg, 8.0);
                            builder.stroke_rect(rect, border, if is_h { 1.2 } else { 1.0 }, 8.0);

                            // Vector icon circle
                            let icon_circle = LogicalRect::from_xywh(
                                rect.origin.x.0 + 10.0,
                                rect.origin.y.0 + 9.0,
                                36.0,
                                36.0,
                            );
                            builder.fill_rect(
                                icon_circle,
                                if is_h {
                                    Color::rgba(45, 212, 191, 30)
                                } else {
                                    Color::rgba(255, 255, 255, 10)
                                },
                                18.0,
                            );
                            builder.icon(
                                icon_id,
                                LogicalRect::from_xywh(
                                    rect.origin.x.0 + 20.0,
                                    rect.origin.y.0 + 19.0,
                                    16.0,
                                    16.0,
                                ),
                                if is_h { accent_mint } else { icon_color },
                            );

                            // Title & Subtitle
                            let t_l = self.shape_cached(title, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(rect.origin.x.0 + 54.0, rect.origin.y.0 + 10.0),
                                t_l,
                                if is_h { accent_mint } else { text_main },
                            );

                            let s_l = self.shape_cached(sub, StyleId::Small, None);
                            builder.text(
                                LogicalPoint::new(rect.origin.x.0 + 54.0, rect.origin.y.0 + 28.0),
                                s_l,
                                text_dim,
                            );

                            // Arrow icon on right
                            let arrow_l = self.shape_cached("↗", StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(
                                    rect.origin.x.0 + c_w - 22.0,
                                    rect.origin.y.0 + 18.0,
                                ),
                                arrow_l,
                                if is_h { accent_mint } else { text_muted },
                            );
                        }

                        // 5. Bottom Copyright & License
                        let copy_txt = "© 2026 Arthur Valiev • Everty Ecosystem • Open Source under MIT License";
                        let copy_l = self.shape_cached(copy_txt, StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 24.0, modal_y + modal_h - 28.0),
                            copy_l,
                            text_muted,
                        );
                    }
                    SettingsTab::Security => {
                        let has_existing = app.hosts_doc.settings.master_password_hash.is_some();
                        let badge_y = modal_y + 105.0;

                        // Status badge
                        let status_rect =
                            LogicalRect::from_xywh(modal_x + 24.0, badge_y, modal_w - 48.0, 34.0);
                        if has_existing {
                            builder.fill_rect(status_rect, Color::rgba(16, 185, 129, 30), 5.0);
                            builder.stroke_rect(
                                status_rect,
                                Color::rgba(52, 211, 153, 140),
                                1.0,
                                5.0,
                            );
                            builder.icon(
                                ICON_LOCK_ID,
                                LogicalRect::from_xywh(modal_x + 36.0, badge_y + 9.0, 16.0, 16.0),
                                accent_emerald,
                            );
                            let st_l = self.shape_cached(
                                tr(
                                    lang,
                                    "Защита включена: при запуске требуется мастер-пароль",
                                    "Protection active: master password required on startup",
                                ),
                                StyleId::SansBold,
                                None,
                            );
                            builder.text(
                                LogicalPoint::new(modal_x + 60.0, badge_y + 8.0),
                                st_l,
                                accent_emerald,
                            );
                        } else {
                            builder.fill_rect(status_rect, Color::rgba(30, 48, 85, 40), 5.0);
                            builder.stroke_rect(status_rect, card_border, 1.0, 5.0);
                            let st_l = self.shape_cached(
                                tr(
                                    lang,
                                    "Защита выключена: приложение запускается без пароля",
                                    "Protection disabled: application opens without password",
                                ),
                                StyleId::Sans,
                                None,
                            );
                            builder.text(
                                LogicalPoint::new(modal_x + 36.0, badge_y + 8.0),
                                st_l,
                                text_dim,
                            );
                        }

                        let mut cy = modal_y + 155.0;
                        let fields_to_show = if has_existing {
                            vec![
                                (
                                    0,
                                    tr(lang, "Текущий пароль:", "Current Password:"),
                                    &settings.current_password,
                                ),
                                (
                                    1,
                                    tr(lang, "Новый пароль:", "New Password:"),
                                    &settings.new_password,
                                ),
                                (
                                    2,
                                    tr(lang, "Повтор пароля:", "Confirm Password:"),
                                    &settings.confirm_password,
                                ),
                            ]
                        } else {
                            vec![
                                (
                                    1,
                                    tr(lang, "Новый пароль:", "New Password:"),
                                    &settings.new_password,
                                ),
                                (
                                    2,
                                    tr(lang, "Повтор пароля:", "Confirm Password:"),
                                    &settings.confirm_password,
                                ),
                            ]
                        };

                        for (f_idx, lbl, inp) in fields_to_show {
                            let l_l = self.shape_cached(lbl, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + 24.0, cy + 6.0),
                                l_l,
                                text_dim,
                            );

                            let eye_btn_w = 34.0;
                            let inp_w = modal_w - 220.0 - eye_btn_w - 8.0;
                            let input_rect =
                                LogicalRect::from_xywh(modal_x + 190.0, cy, inp_w, 32.0);
                            let is_focused =
                                app.focused_input == Some(InputFieldId::SettingsSecurity(f_idx));
                            let is_hover = app.hover == HoverTarget::SettingsSecurityField(f_idx);

                            self.draw_input_box(
                                &mut builder,
                                inp,
                                input_rect,
                                is_focused,
                                is_hover,
                                tr(lang, "Введите пароль...", "Enter password..."),
                                !settings.show_passwords,
                                app.caret_visible && is_focused,
                                accent_mint,
                                card_border,
                                text_main,
                                text_muted,
                            );

                            let eye_x = modal_x + 190.0 + inp_w + 8.0;
                            let eye_rect = LogicalRect::from_xywh(eye_x, cy, eye_btn_w, 32.0);
                            let is_eye_hover = app.hover == HoverTarget::SettingsTogglePassword;
                            builder.fill_rect(
                                eye_rect,
                                if is_eye_hover {
                                    card_hover_bg
                                } else {
                                    Color::rgba(20, 32, 58, 255)
                                },
                                4.0,
                            );
                            builder.stroke_rect(
                                eye_rect,
                                if is_eye_hover {
                                    accent_mint
                                } else {
                                    card_border
                                },
                                1.0,
                                4.0,
                            );
                            let icon_id = if settings.show_passwords {
                                ICON_EYE_OFF_ID
                            } else {
                                ICON_EYE_ID
                            };
                            let icon_color = if is_eye_hover || settings.show_passwords {
                                accent_mint
                            } else {
                                text_dim
                            };
                            builder.icon(
                                icon_id,
                                LogicalRect::from_xywh(eye_x + 9.0, cy + 8.0, 16.0, 16.0),
                                icon_color,
                            );

                            cy += 44.0;
                        }

                        // Message (error or success)
                        if let Some((msg, is_err)) = &settings.security_msg {
                            let msg_l = self.shape_cached(
                                msg,
                                StyleId::Small,
                                Some(LogicalPx(modal_w - 48.0)),
                            );
                            let col = if *is_err {
                                Color::rgba(239, 68, 68, 255)
                            } else {
                                accent_emerald
                            };
                            builder.text(
                                LogicalPoint::new(modal_x + 24.0, modal_y + modal_h - 78.0),
                                msg_l,
                                col,
                            );
                        }

                        // Bottom Action Buttons
                        let btn_y = modal_y + modal_h - 50.0;
                        if has_existing {
                            // Remove Password Button
                            let is_rem_h = app.hover == HoverTarget::SettingsRemovePassword;
                            let rem_rect = LogicalRect::from_xywh(
                                modal_x + modal_w - 320.0,
                                btn_y,
                                140.0,
                                34.0,
                            );
                            if is_rem_h {
                                builder.fill_rect(rem_rect, Color::rgba(220, 38, 38, 220), 5.0);
                            } else {
                                builder.fill_rect(rem_rect, Color::rgba(60, 20, 30, 200), 5.0);
                            }
                            builder.stroke_rect(rem_rect, Color::rgba(239, 68, 68, 160), 1.0, 5.0);
                            let rem_l = self.shape_cached(
                                tr(lang, "Снять защиту", "Disable Lock"),
                                StyleId::Small,
                                None,
                            );
                            builder.text(
                                LogicalPoint::new(modal_x + modal_w - 296.0, btn_y + 8.0),
                                rem_l,
                                Color::rgba(255, 255, 255, 255),
                            );

                            // Save / Change Password Button
                            let is_save_h = app.hover == HoverTarget::SettingsSavePassword;
                            let save_rect = LogicalRect::from_xywh(
                                modal_x + modal_w - 165.0,
                                btn_y,
                                140.0,
                                34.0,
                            );
                            let change_pwd_txt = tr(lang, "Сменить пароль", "Change Password");
                            if is_save_h {
                                builder.fill_rect(save_rect, accent_mint, 5.0);
                                let save_l =
                                    self.shape_cached(change_pwd_txt, StyleId::SansBold, None);
                                builder.text(
                                    LogicalPoint::new(modal_x + modal_w - 150.0, btn_y + 7.0),
                                    save_l,
                                    Color::rgba(9, 14, 26, 255),
                                );
                            } else {
                                builder.fill_rect(save_rect, Color::rgba(20, 36, 60, 255), 5.0);
                                builder.stroke_rect(save_rect, accent_mint, 1.2, 5.0);
                                let save_l =
                                    self.shape_cached(change_pwd_txt, StyleId::SansBold, None);
                                builder.text(
                                    LogicalPoint::new(modal_x + modal_w - 150.0, btn_y + 7.0),
                                    save_l,
                                    accent_mint,
                                );
                            }
                        } else {
                            // Set Password Button
                            let is_save_h = app.hover == HoverTarget::SettingsSavePassword;
                            let save_rect = LogicalRect::from_xywh(
                                modal_x + modal_w - 190.0,
                                btn_y,
                                166.0,
                                34.0,
                            );
                            let set_pwd_txt = tr(lang, "Установить пароль", "Set Password");
                            if is_save_h {
                                builder.fill_rect(save_rect, accent_mint, 5.0);
                                let save_l =
                                    self.shape_cached(set_pwd_txt, StyleId::SansBold, None);
                                builder.text(
                                    LogicalPoint::new(modal_x + modal_w - 176.0, btn_y + 7.0),
                                    save_l,
                                    Color::rgba(9, 14, 26, 255),
                                );
                            } else {
                                builder.fill_rect(save_rect, Color::rgba(20, 36, 60, 255), 5.0);
                                builder.stroke_rect(save_rect, accent_mint, 1.2, 5.0);
                                let save_l =
                                    self.shape_cached(set_pwd_txt, StyleId::SansBold, None);
                                builder.text(
                                    LogicalPoint::new(modal_x + modal_w - 176.0, btn_y + 7.0),
                                    save_l,
                                    accent_mint,
                                );
                            }
                        }
                    }
                    SettingsTab::Backup => {
                        // Section 1: Export
                        let exp_box_y = modal_y + 110.0;
                        let exp_box_rect =
                            LogicalRect::from_xywh(modal_x + 24.0, exp_box_y, modal_w - 48.0, 95.0);
                        builder.fill_rect(exp_box_rect, Color::rgba(20, 32, 58, 200), 6.0);
                        builder.stroke_rect(exp_box_rect, card_border, 1.0, 6.0);

                        let exp_t = self.shape_cached(
                            tr(lang, "Резервная копия конфигурации", "Configuration Backup"),
                            StyleId::SansBold,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 38.0, exp_box_y + 12.0),
                            exp_t,
                            text_main,
                        );

                        let exp_d = self.shape_cached(tr(lang, "Шифрует серверы, пароли и настройки отдельным паролем.", "Encrypts servers, credentials and settings with a separate password."), StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 38.0, exp_box_y + 34.0),
                            exp_d,
                            text_muted,
                        );

                        let is_exp_h = app.hover == HoverTarget::SettingsExportJson;
                        let exp_btn_rect =
                            LogicalRect::from_xywh(modal_x + 38.0, exp_box_y + 54.0, 200.0, 30.0);
                        let exp_btn_lbl = tr(lang, "Экспорт копии...", "Export backup...");
                        if is_exp_h {
                            builder.fill_rect(exp_btn_rect, accent_mint, 5.0);
                            let btn_l = self.shape_cached(exp_btn_lbl, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + 72.0, exp_box_y + 60.0),
                                btn_l,
                                Color::rgba(9, 14, 26, 255),
                            );
                        } else {
                            builder.fill_rect(exp_btn_rect, Color::rgba(25, 42, 70, 255), 5.0);
                            builder.stroke_rect(exp_btn_rect, accent_mint, 1.0, 5.0);
                            let btn_l = self.shape_cached(exp_btn_lbl, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + 72.0, exp_box_y + 60.0),
                                btn_l,
                                accent_mint,
                            );
                        }

                        // Section 2: Import
                        let imp_box_y = modal_y + 220.0;
                        let imp_box_rect =
                            LogicalRect::from_xywh(modal_x + 24.0, imp_box_y, modal_w - 48.0, 95.0);
                        builder.fill_rect(imp_box_rect, Color::rgba(20, 32, 58, 200), 6.0);
                        builder.stroke_rect(imp_box_rect, card_border, 1.0, 6.0);

                        let imp_t = self.shape_cached(
                            tr(lang, "Восстановление из файла", "Restore from File"),
                            StyleId::SansBold,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 38.0, imp_box_y + 12.0),
                            imp_t,
                            text_main,
                        );

                        let imp_d = self.shape_cached(
                            tr(
                                lang,
                                "Импортирует серверы из резервной копии и объединяет с текущими.",
                                "Imports servers from backup and merges with existing.",
                            ),
                            StyleId::Small,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 38.0, imp_box_y + 34.0),
                            imp_d,
                            text_muted,
                        );

                        let is_imp_h = app.hover == HoverTarget::SettingsImportJson;
                        let imp_btn_rect =
                            LogicalRect::from_xywh(modal_x + 38.0, imp_box_y + 54.0, 200.0, 30.0);
                        let imp_btn_lbl = tr(lang, "Импорт копии...", "Import backup...");
                        if is_imp_h {
                            builder.fill_rect(imp_btn_rect, accent_mint, 5.0);
                            let btn_l = self.shape_cached(imp_btn_lbl, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + 72.0, imp_box_y + 60.0),
                                btn_l,
                                Color::rgba(9, 14, 26, 255),
                            );
                        } else {
                            builder.fill_rect(imp_btn_rect, Color::rgba(25, 42, 70, 255), 5.0);
                            builder.stroke_rect(imp_btn_rect, accent_mint, 1.0, 5.0);
                            let btn_l = self.shape_cached(imp_btn_lbl, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + 72.0, imp_box_y + 60.0),
                                btn_l,
                                accent_mint,
                            );
                        }

                        for (index, label, input) in [
                            (
                                0,
                                tr(lang, "Пароль копии:", "Backup password:"),
                                &settings.backup_password,
                            ),
                            (
                                1,
                                tr(lang, "Повтор (экспорт):", "Repeat (export):"),
                                &settings.backup_confirm,
                            ),
                        ] {
                            let cy = modal_y + 330.0 + index as f64 * 40.0;
                            let label = self.shape_cached(label, StyleId::Sans, None);
                            builder.text(
                                LogicalPoint::new(modal_x + 24.0, cy + 6.0),
                                label,
                                text_dim,
                            );
                            let focused =
                                app.focused_input == Some(InputFieldId::SettingsBackup(index));
                            self.draw_input_box(
                                &mut builder,
                                input,
                                LogicalRect::from_xywh(modal_x + 200.0, cy, modal_w - 230.0, 32.0),
                                focused,
                                app.hover == HoverTarget::SettingsBackupField(index),
                                tr(lang, "Минимум 8 символов", "At least 8 characters"),
                                true,
                                app.caret_visible && focused,
                                accent_mint,
                                card_border,
                                text_main,
                                text_muted,
                            );
                        }

                        // Message (error or success)
                        if let Some((msg, is_err)) = &settings.backup_msg {
                            let msg_l = self.shape_cached(
                                msg,
                                StyleId::Small,
                                Some(LogicalPx(modal_w - 48.0)),
                            );
                            let col = if *is_err {
                                Color::rgba(239, 68, 68, 255)
                            } else {
                                accent_emerald
                            };
                            builder.text(
                                LogicalPoint::new(modal_x + 24.0, modal_y + modal_h - 40.0),
                                msg_l,
                                col,
                            );
                        }
                    }
                    SettingsTab::General => {
                        // Section: Language
                        let lang_lbl = self.shape_cached(
                            tr(lang, "Язык интерфейса:", "Interface Language:"),
                            StyleId::SansBold,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 24.0, modal_y + 110.0),
                            lang_lbl,
                            text_dim,
                        );

                        let ru_active = lang == AppLanguage::Ru;
                        let en_active = lang == AppLanguage::En;
                        let ru_hover =
                            app.hover == HoverTarget::SettingsSetLanguage(AppLanguage::Ru);
                        let en_hover =
                            app.hover == HoverTarget::SettingsSetLanguage(AppLanguage::En);

                        // RU button
                        let ru_rect =
                            LogicalRect::from_xywh(modal_x + 240.0, modal_y + 105.0, 100.0, 30.0);
                        builder.fill_rect(
                            ru_rect,
                            if ru_active {
                                accent_mint
                            } else if ru_hover {
                                card_hover_bg
                            } else {
                                Color::rgba(18, 29, 52, 255)
                            },
                            4.0,
                        );
                        builder.stroke_rect(
                            ru_rect,
                            if ru_active { accent_mint } else { card_border },
                            1.0,
                            4.0,
                        );
                        let ru_l = self.shape_cached("Русский (RU)", StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 252.0, modal_y + 112.0),
                            ru_l,
                            if ru_active {
                                Color::rgba(9, 14, 26, 255)
                            } else {
                                text_main
                            },
                        );

                        // EN button
                        let en_rect =
                            LogicalRect::from_xywh(modal_x + 350.0, modal_y + 105.0, 100.0, 30.0);
                        builder.fill_rect(
                            en_rect,
                            if en_active {
                                accent_mint
                            } else if en_hover {
                                card_hover_bg
                            } else {
                                Color::rgba(18, 29, 52, 255)
                            },
                            4.0,
                        );
                        builder.stroke_rect(
                            en_rect,
                            if en_active { accent_mint } else { card_border },
                            1.0,
                            4.0,
                        );
                        let en_l = self.shape_cached("English (EN)", StyleId::Small, None);
                        builder.text(
                            LogicalPoint::new(modal_x + 362.0, modal_y + 112.0),
                            en_l,
                            if en_active {
                                Color::rgba(9, 14, 26, 255)
                            } else {
                                text_main
                            },
                        );

                        let mut cy = modal_y + 150.0;

                        // Field 0: Default User
                        let u_lbl = self.shape_cached(
                            tr(lang, "Логин по умолчанию:", "Default Username:"),
                            StyleId::SansBold,
                            None,
                        );
                        builder.text(LogicalPoint::new(modal_x + 24.0, cy + 6.0), u_lbl, text_dim);

                        let u_rect =
                            LogicalRect::from_xywh(modal_x + 240.0, cy, modal_w - 270.0, 32.0);
                        let is_u_foc = app.focused_input == Some(InputFieldId::SettingsGeneral(0));
                        let is_u_hov = app.hover == HoverTarget::SettingsGeneralField(0);
                        self.draw_input_box(
                            &mut builder,
                            &settings.default_user,
                            u_rect,
                            is_u_foc,
                            is_u_hov,
                            "root",
                            false,
                            app.caret_visible && is_u_foc,
                            accent_mint,
                            card_border,
                            text_main,
                            text_muted,
                        );

                        cy += 45.0;

                        // Field 1: Scrollback lines
                        let sb_lbl = self.shape_cached(
                            tr(lang, "Буфер строк терминала:", "Terminal Scrollback Lines:"),
                            StyleId::SansBold,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 24.0, cy + 6.0),
                            sb_lbl,
                            text_dim,
                        );

                        let sb_rect =
                            LogicalRect::from_xywh(modal_x + 240.0, cy, modal_w - 270.0, 32.0);
                        let is_sb_foc = app.focused_input == Some(InputFieldId::SettingsGeneral(1));
                        let is_sb_hov = app.hover == HoverTarget::SettingsGeneralField(1);
                        self.draw_input_box(
                            &mut builder,
                            &settings.scrollback_lines,
                            sb_rect,
                            is_sb_foc,
                            is_sb_hov,
                            "5000",
                            false,
                            app.caret_visible && is_sb_foc,
                            accent_mint,
                            card_border,
                            text_main,
                            text_muted,
                        );

                        cy += 48.0;

                        // Checkbox: Confirm Delete
                        let is_chk_hov = app.hover == HoverTarget::SettingsToggleConfirmDelete;
                        let chk_rect = LogicalRect::from_xywh(modal_x + 24.0, cy + 2.0, 18.0, 18.0);
                        builder.fill_rect(chk_rect, Color::rgba(20, 32, 58, 255), 4.0);
                        builder.stroke_rect(
                            chk_rect,
                            if is_chk_hov { accent_mint } else { card_border },
                            1.2,
                            4.0,
                        );
                        if settings.confirm_delete {
                            builder.fill_rect(
                                LogicalRect::from_xywh(modal_x + 27.0, cy + 5.0, 12.0, 12.0),
                                accent_mint,
                                2.0,
                            );
                        }

                        let chk_l = self.shape_cached(
                            tr(
                                lang,
                                "Подтверждать удаление серверов диалоговым окном",
                                "Confirm server deletion with dialog",
                            ),
                            StyleId::Sans,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 50.0, cy + 2.0),
                            chk_l,
                            if is_chk_hov { text_main } else { text_dim },
                        );

                        let history_y = modal_y + 282.0;
                        let history_hover = app.hover == HoverTarget::SettingsToggleHistory;
                        let rect =
                            LogicalRect::from_xywh(modal_x + 24.0, history_y + 2.0, 18.0, 18.0);
                        builder.fill_rect(rect, Color::rgba(20, 32, 58, 255), 4.0);
                        builder.stroke_rect(
                            rect,
                            if history_hover {
                                accent_mint
                            } else {
                                card_border
                            },
                            1.2,
                            4.0,
                        );
                        if settings.record_command_history {
                            builder.fill_rect(
                                LogicalRect::from_xywh(modal_x + 27.0, history_y + 5.0, 12.0, 12.0),
                                accent_mint,
                                2.0,
                            );
                        }
                        let label = self.shape_cached(
                            tr(
                                lang,
                                "История команд через интеграцию с оболочкой",
                                "Command history via shell integration",
                            ),
                            StyleId::Sans,
                            None,
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 50.0, history_y + 2.0),
                            label,
                            text_dim,
                        );
                        let hint = self.shape_cached(
                            tr(
                                lang,
                                "По умолчанию выключена. Настройка: docs/SHELL_INTEGRATION.md",
                                "Off by default. Setup: docs/SHELL_INTEGRATION.md",
                            ),
                            StyleId::Small,
                            Some(LogicalPx(modal_w - 48.0)),
                        );
                        builder.text(
                            LogicalPoint::new(modal_x + 24.0, history_y + 32.0),
                            hint,
                            text_muted,
                        );

                        // Feedback message
                        if let Some((msg, is_err)) = &settings.general_msg {
                            let msg_l = self.shape_cached(
                                msg,
                                StyleId::Small,
                                Some(LogicalPx(modal_w - 48.0)),
                            );
                            let col = if *is_err {
                                Color::rgba(239, 68, 68, 255)
                            } else {
                                accent_emerald
                            };
                            builder.text(
                                LogicalPoint::new(modal_x + 24.0, modal_y + modal_h - 78.0),
                                msg_l,
                                col,
                            );
                        }

                        // Save General Button
                        let btn_y = modal_y + modal_h - 50.0;
                        let is_save_h = app.hover == HoverTarget::SettingsSaveGeneral;
                        let save_rect =
                            LogicalRect::from_xywh(modal_x + modal_w - 180.0, btn_y, 156.0, 34.0);
                        let save_txt = tr(lang, "Сохранить", "Save Settings");
                        if is_save_h {
                            builder.fill_rect(save_rect, accent_mint, 5.0);
                            let save_l = self.shape_cached(save_txt, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + modal_w - 150.0, btn_y + 7.0),
                                save_l,
                                Color::rgba(9, 14, 26, 255),
                            );
                        } else {
                            builder.fill_rect(save_rect, Color::rgba(20, 36, 60, 255), 5.0);
                            builder.stroke_rect(save_rect, accent_mint, 1.2, 5.0);
                            let save_l = self.shape_cached(save_txt, StyleId::SansBold, None);
                            builder.text(
                                LogicalPoint::new(modal_x + modal_w - 150.0, btn_y + 7.0),
                                save_l,
                                accent_mint,
                            );
                        }
                    }
                }
            }
        }

        // 12.6 Host Key Verification Modal
        if let Some(host_key) = &app.host_key_modal {
            builder.fill_rect(
                LogicalRect::from_xywh(0.0, 0.0, w, h),
                Color::rgba(0, 0, 0, 190),
                0.0,
            );

            let modal_w = 560.0f64.min(w - 40.0);
            let modal_h = (if host_key.is_changed { 320.0 } else { 280.0 } as f64).min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;
            let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

            let border_color = if host_key.is_changed {
                Color::rgba(239, 68, 68, 255)
            } else {
                accent_mint
            };

            builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 8.0);
            builder.stroke_rect(modal_rect, border_color, 1.5, 8.0);

            // Icon + Title
            let icon_id = if host_key.is_changed {
                ICON_ALERT_ID
            } else {
                ICON_CONNECT_ID
            };
            let icon_bg = if host_key.is_changed {
                Color::rgba(239, 68, 68, 40)
            } else {
                Color::rgba(45, 212, 191, 40)
            };
            let icon_fg = if host_key.is_changed {
                Color::rgba(239, 68, 68, 255)
            } else {
                accent_mint
            };

            builder.fill_rect(
                LogicalRect::from_xywh(modal_x + 24.0, modal_y + 20.0, 36.0, 36.0),
                icon_bg,
                18.0,
            );
            builder.icon(
                icon_id,
                LogicalRect::from_xywh(modal_x + 32.0, modal_y + 28.0, 20.0, 20.0),
                icon_fg,
            );

            let title_text = if host_key.is_changed {
                tr(
                    lang,
                    "⚠️ КЛЮЧ ХОСТА ИЗМЕНИЛСЯ!",
                    "⚠️ REMOTE HOST KEY CHANGED!",
                )
            } else {
                tr(
                    lang,
                    "Проверка подлинности сервера",
                    "Server Authenticity Verification",
                )
            };
            let title_l = self.shape_cached(title_text, StyleId::Title, None);
            builder.text(
                LogicalPoint::new(modal_x + 70.0, modal_y + 20.0),
                title_l,
                if host_key.is_changed {
                    Color::rgba(248, 113, 113, 255)
                } else {
                    text_main
                },
            );

            let sub_info = format!(
                "{}: {}:{} ({})",
                tr(lang, "Сервер", "Server"),
                host_key.host,
                host_key.port,
                host_key.key_type
            );
            let sub_l =
                self.shape_cached(&sub_info, StyleId::Small, Some(LogicalPx(modal_w - 95.0)));
            builder.text(
                LogicalPoint::new(modal_x + 70.0, modal_y + 42.0),
                sub_l,
                text_dim,
            );

            let mut cur_y = modal_y + 68.0;

            if host_key.is_changed {
                // Dangerous warning banner
                let banner_rect =
                    LogicalRect::from_xywh(modal_x + 24.0, cur_y, modal_w - 48.0, 36.0);
                builder.fill_rect(banner_rect, Color::rgba(239, 68, 68, 30), 5.0);
                builder.stroke_rect(banner_rect, Color::rgba(239, 68, 68, 160), 1.0, 5.0);

                let line_str = host_key
                    .old_line
                    .map(|l| format!(" (строка #{l})"))
                    .unwrap_or_default();
                let warn_text = format!(
                    "{}! {}{line_str}",
                    tr(lang, "Возможен перехват MITM", "Possible MITM attack"),
                    tr(
                        lang,
                        "Ключ в known_hosts не совпадает",
                        "Key in known_hosts differs"
                    )
                );
                let warn_l =
                    self.shape_cached(&warn_text, StyleId::Small, Some(LogicalPx(modal_w - 60.0)));
                builder.text(
                    LogicalPoint::new(modal_x + 36.0, cur_y + 9.0),
                    warn_l,
                    Color::rgba(252, 165, 165, 255),
                );
                cur_y += 44.0;
            } else {
                let note = tr(
                    lang,
                    "Не удается установить подлинность хоста. Отпечаток ключа (Fingerprint):",
                    "Authenticity of host cannot be established. Key fingerprint:",
                );
                let note_l =
                    self.shape_cached(note, StyleId::Small, Some(LogicalPx(modal_w - 48.0)));
                builder.text(
                    LogicalPoint::new(modal_x + 24.0, cur_y + 4.0),
                    note_l,
                    text_main,
                );
                cur_y += 28.0;
            }

            // Fingerprint Box + Copy Button
            let fp_box_w = modal_w - 146.0;
            let fp_box_rect = LogicalRect::from_xywh(modal_x + 24.0, cur_y, fp_box_w, 34.0);
            builder.fill_rect(fp_box_rect, Color::rgba(10, 16, 30, 255), 5.0);
            builder.stroke_rect(fp_box_rect, card_border, 1.0, 5.0);

            builder.push_clip(LogicalRect::from_xywh(
                modal_x + 28.0,
                cur_y + 2.0,
                fp_box_w - 8.0,
                30.0,
            ));
            let fp_l = self.shape_cached(
                &host_key.fingerprint,
                StyleId::Mono,
                Some(LogicalPx(fp_box_w - 16.0)),
            );
            builder.text(
                LogicalPoint::new(modal_x + 34.0, cur_y + 8.0),
                fp_l,
                Color::rgba(56, 189, 248, 255),
            );
            let _ = builder.pop_clip();

            // Copy button
            let is_copy_h = app.hover == HoverTarget::HostKeyModalCopy;
            let copy_btn_rect =
                LogicalRect::from_xywh(modal_x + modal_w - 114.0, cur_y, 90.0, 34.0);
            builder.fill_rect(
                copy_btn_rect,
                if is_copy_h {
                    Color::rgba(35, 55, 95, 230)
                } else {
                    Color::rgba(20, 35, 65, 200)
                },
                5.0,
            );
            builder.stroke_rect(copy_btn_rect, Color::rgba(45, 212, 191, 100), 1.0, 5.0);
            builder.icon(
                ICON_COPY_ID,
                LogicalRect::from_xywh(modal_x + modal_w - 106.0, cur_y + 10.0, 14.0, 14.0),
                accent_mint,
            );
            let copy_label = if host_key.copied_feedback {
                tr(lang, "Готово!", "Copied!")
            } else {
                tr(lang, "Копия", "Copy")
            };
            let copy_l = self.shape_cached(copy_label, StyleId::Small, None);
            builder.text(
                LogicalPoint::new(modal_x + modal_w - 86.0, cur_y + 8.0),
                copy_l,
                accent_mint,
            );

            cur_y += 44.0;

            // Prompt note
            let prompt_msg = if host_key.is_changed {
                tr(
                    lang,
                    "Рекомендуется отменить соединение и связаться с владельцем сервера.",
                    "Recommended to abort connection and verify with server owner.",
                )
            } else {
                tr(
                    lang,
                    "Вы уверены, что хотите продолжить подключение и сохранить этот ключ?",
                    "Are you sure you want to continue connecting and trust this key?",
                )
            };
            let prompt_l =
                self.shape_cached(prompt_msg, StyleId::Small, Some(LogicalPx(modal_w - 48.0)));
            builder.text(
                LogicalPoint::new(modal_x + 24.0, cur_y + 2.0),
                prompt_l,
                text_muted,
            );

            // Bottom action buttons
            let btn_y = modal_y + modal_h - 52.0;

            if host_key.is_changed {
                // Reject button (Esc - Prominent/Safe)
                let is_reject_h = app.hover == HoverTarget::HostKeyModalReject;
                let reject_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 380.0, btn_y, 150.0, 34.0);
                builder.fill_rect(
                    reject_rect,
                    if is_reject_h {
                        Color::rgba(220, 38, 38, 255)
                    } else {
                        Color::rgba(239, 68, 68, 230)
                    },
                    5.0,
                );
                builder.icon(
                    ICON_CLOSE_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 372.0, btn_y + 10.0, 14.0, 14.0),
                    Color::rgba(255, 255, 255, 255),
                );
                let rej_l = self.shape_cached(
                    tr(lang, "Отклонить (Esc)", "Reject (Esc)"),
                    StyleId::SansBold,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 352.0, btn_y + 7.0),
                    rej_l,
                    Color::rgba(255, 255, 255, 255),
                );

                // Accept button (Update & continue - Caution)
                let is_accept_h = app.hover == HoverTarget::HostKeyModalAccept;
                let accept_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 220.0, btn_y, 200.0, 34.0);
                builder.fill_rect(
                    accept_rect,
                    if is_accept_h {
                        Color::rgba(45, 55, 80, 240)
                    } else {
                        Color::rgba(25, 35, 55, 200)
                    },
                    5.0,
                );
                builder.stroke_rect(accept_rect, Color::rgba(239, 68, 68, 140), 1.0, 5.0);
                let acc_l = self.shape_cached(
                    tr(lang, "Обновить ключ и войти", "Update Key & Proceed"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 208.0, btn_y + 8.0),
                    acc_l,
                    Color::rgba(248, 113, 113, 255),
                );
            } else {
                // Cancel button (Esc)
                let is_cancel_h = app.hover == HoverTarget::HostKeyModalReject;
                let cancel_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 360.0, btn_y, 110.0, 34.0);
                builder.fill_rect(
                    cancel_rect,
                    if is_cancel_h {
                        Color::rgba(30, 48, 85, 230)
                    } else {
                        Color::rgba(20, 32, 58, 200)
                    },
                    5.0,
                );
                builder.stroke_rect(cancel_rect, card_border, 1.0, 5.0);
                let cancel_l = self.shape_cached(
                    tr(lang, "Отмена (Esc)", "Cancel (Esc)"),
                    StyleId::Small,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 348.0, btn_y + 8.0),
                    cancel_l,
                    text_dim,
                );

                // Trust & Connect button (Enter)
                let is_accept_h = app.hover == HoverTarget::HostKeyModalAccept;
                let accept_rect =
                    LogicalRect::from_xywh(modal_x + modal_w - 230.0, btn_y, 210.0, 34.0);
                builder.fill_rect(
                    accept_rect,
                    if is_accept_h {
                        Color::rgba(45, 212, 191, 230)
                    } else {
                        accent_mint
                    },
                    5.0,
                );
                builder.icon(
                    ICON_CONNECT_ID,
                    LogicalRect::from_xywh(modal_x + modal_w - 222.0, btn_y + 10.0, 14.0, 14.0),
                    Color::rgba(8, 13, 25, 255),
                );
                let acc_l = self.shape_cached(
                    tr(lang, "Доверять и войти (Enter)", "Trust & Connect (Enter)"),
                    StyleId::SansBold,
                    None,
                );
                builder.text(
                    LogicalPoint::new(modal_x + modal_w - 202.0, btn_y + 7.0),
                    acc_l,
                    bg_app,
                );
            }
        }

        // 14. Full-screen App Lock Overlay
        if app.is_app_locked {
            builder.fill_rect(
                LogicalRect::from_xywh(0.0, 0.0, w, h),
                Color::rgba(5, 8, 16, 245),
                0.0,
            );

            let modal_w = 400.0f64.min(w - 40.0);
            let modal_h = 280.0f64.min(h - 40.0);
            let modal_x = (w - modal_w) / 2.0;
            let modal_y = (h - modal_h) / 2.0;
            let modal_rect = LogicalRect::from_xywh(modal_x, modal_y, modal_w, modal_h);

            builder.fill_rect(modal_rect, Color::rgba(14, 23, 42, 255), 10.0);
            builder.stroke_rect(modal_rect, accent_mint, 1.5, 10.0);

            // Padlock Icon in circle badge
            let badge_cx = modal_x + modal_w / 2.0;
            let badge_y = modal_y + 24.0;
            builder.fill_rect(
                LogicalRect::from_xywh(badge_cx - 24.0, badge_y, 48.0, 48.0),
                Color::rgba(45, 212, 191, 30),
                24.0,
            );
            builder.icon(
                ICON_LOCK_ID,
                LogicalRect::from_xywh(badge_cx - 12.0, badge_y + 12.0, 24.0, 24.0),
                accent_mint,
            );

            // Title
            let t_l = self.shape_cached(
                tr(lang, "EvertySSH заблокирован", "EvertySSH Locked"),
                StyleId::Title,
                None,
            );
            let tw = t_l.metrics.size.width.0;
            builder.text(
                LogicalPoint::new(modal_x + (modal_w - tw) / 2.0, modal_y + 82.0),
                t_l,
                text_main,
            );

            let sub_l = self.shape_cached(
                tr(
                    lang,
                    "Введите мастер-пароль для доступа",
                    "Enter master password to unlock",
                ),
                StyleId::Small,
                None,
            );
            let sw = sub_l.metrics.size.width.0;
            builder.text(
                LogicalPoint::new(modal_x + (modal_w - sw) / 2.0, modal_y + 106.0),
                sub_l,
                text_muted,
            );

            // Password Input Box
            let inp_rect =
                LogicalRect::from_xywh(modal_x + 30.0, modal_y + 130.0, modal_w - 60.0, 36.0);
            let is_foc = app.focused_input == Some(InputFieldId::LockPassword)
                || app.focused_input.is_none();
            let is_hov = app.hover == HoverTarget::LockPasswordField;
            self.draw_input_box(
                &mut builder,
                &app.lock_password_input,
                inp_rect,
                is_foc,
                is_hov,
                tr(lang, "Мастер-пароль...", "Master password..."),
                true,
                app.caret_visible,
                accent_mint,
                card_border,
                text_main,
                text_muted,
            );

            // Error message if any
            if let Some(err) = &app.lock_error_msg {
                let err_l = self.shape_cached(err, StyleId::Small, Some(LogicalPx(modal_w - 60.0)));
                builder.text(
                    LogicalPoint::new(modal_x + 32.0, modal_y + 172.0),
                    err_l,
                    Color::rgba(239, 68, 68, 255),
                );
            }

            // Unlock Button
            let is_unlock_h = app.hover == HoverTarget::LockUnlockBtn;
            let unlock_rect =
                LogicalRect::from_xywh(modal_x + 30.0, modal_y + 194.0, modal_w - 60.0, 38.0);
            let un_txt = tr(lang, "Разблокировать (Enter)", "Unlock (Enter)");
            if is_unlock_h {
                builder.fill_rect(unlock_rect, accent_mint, 6.0);
                let un_l = self.shape_cached(un_txt, StyleId::SansBold, None);
                let uw = un_l.metrics.size.width.0;
                builder.text(
                    LogicalPoint::new(modal_x + (modal_w - uw) / 2.0, modal_y + 203.0),
                    un_l,
                    Color::rgba(9, 14, 26, 255),
                );
            } else {
                builder.fill_rect(unlock_rect, Color::rgba(25, 42, 70, 255), 6.0);
                builder.stroke_rect(unlock_rect, accent_mint, 1.2, 6.0);
                let un_l = self.shape_cached(un_txt, StyleId::SansBold, None);
                let uw = un_l.metrics.size.width.0;
                builder.text(
                    LogicalPoint::new(modal_x + (modal_w - uw) / 2.0, modal_y + 203.0),
                    un_l,
                    accent_mint,
                );
            }
        }

        // 15. Floating Tooltip (shown after hovering for >= 350ms)
        if let Some(tt) = &app.tooltip {
            if tt.hover_start.elapsed() >= std::time::Duration::from_millis(350) {
                let tt_layout = self.shape_cached(&tt.text, StyleId::Small, None);
                let tt_w = tt_layout.metrics.size.width.0 + 18.0;
                let tt_h = 24.0;
                let tt_x = tt.x.min(w - tt_w - 8.0).max(8.0);
                let tt_y = tt.y.min(h - tt_h - 8.0).max(8.0);
                let tt_rect = LogicalRect::from_xywh(tt_x, tt_y, tt_w, tt_h);

                builder.fill_rect(tt_rect, Color::rgba(14, 22, 38, 250), 4.0);
                builder.stroke_rect(tt_rect, Color::rgba(45, 212, 191, 160), 1.0, 4.0);
                builder.text(
                    LogicalPoint::new(tt_x + 9.0, tt_y + 4.0),
                    tt_layout,
                    text_main,
                );
            }
        }

        let paint_list = builder.finish();
        self.renderer
            .render(&paint_list, &mut self.text_system, scale, physical_size)
            .map_err(|_| "Failed to render frame")?;

        Ok(self.renderer.pixels())
    }
}
