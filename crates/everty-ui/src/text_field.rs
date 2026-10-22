//! Single-line text editing: the model ([`TextEdit`]) and its painting
//! ([`TextField`]). The model is plain data (text, caret, selection) with the
//! usual editing commands on grapheme boundaries; the painter draws the field
//! with caret, selection, placeholder and horizontal scrolling in the shared
//! theme. Platform input (key events, IME, clipboard) belongs to the host: it
//! feeds [`TextEdit::insert`] and the movement commands.

use std::sync::Arc;
use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Key, LogicalPoint, LogicalPx, LogicalRect, Modifiers, PaintBuilder, TextStyle, TextSystem,
    Theme,
};

/// Caret movement commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Movement {
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

/// Text, caret and selection of one single-line field. Offsets are byte
/// indexes that always sit on grapheme boundaries.
#[derive(Clone, Debug)]
pub struct TextEdit {
    text: String,
    caret: usize,
    /// The fixed end of the selection; the caret is the moving end.
    anchor: Option<usize>,
    max_chars: usize,
    revision: u64,
}

impl TextEdit {
    pub fn new(max_chars: usize) -> Self {
        Self {
            text: String::new(),
            caret: 0,
            anchor: None,
            max_chars,
            revision: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Changes whenever text, caret or selection change; hosts use it to
    /// restart the caret blink.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clear(&mut self) {
        if !self.text.is_empty() || self.anchor.is_some() {
            self.text.clear();
            self.caret = 0;
            self.anchor = None;
            self.revision += 1;
        }
    }

    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.caret = 0;
        self.anchor = None;
        self.insert(text);
    }

    /// Selected byte range, if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.caret).then(|| (anchor.min(self.caret), anchor.max(self.caret)))
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|(start, end)| &self.text[start..end])
    }

    pub fn select_all(&mut self) {
        if !self.text.is_empty() {
            self.anchor = Some(0);
            self.caret = self.text.len();
            self.revision += 1;
        }
    }

    /// Replaces the selection (if any) with `value` at the caret. Line breaks
    /// become spaces, other control characters are dropped, and the field's
    /// character limit is respected.
    pub fn insert(&mut self, value: &str) {
        self.delete_selection();
        let room = self.max_chars.saturating_sub(self.text.chars().count());
        let clean: String = value
            .replace("\r\n", "\n")
            .chars()
            .filter_map(|ch| match ch {
                '\r' | '\n' | '\t' => Some(' '),
                ch if ch.is_control() => None,
                ch => Some(ch),
            })
            .take(room)
            .collect();
        if clean.is_empty() {
            return;
        }
        self.text.insert_str(self.caret, &clean);
        self.caret += clean.len();
        self.revision += 1;
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = None;
        self.revision += 1;
        true
    }

    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() || self.caret == 0 {
            return;
        }
        let start = if word {
            self.word_boundary(self.caret, false)
        } else {
            self.grapheme_boundary(self.caret, false)
        };
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.revision += 1;
    }

    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() || self.caret >= self.text.len() {
            return;
        }
        let end = if word {
            self.word_boundary(self.caret, true)
        } else {
            self.grapheme_boundary(self.caret, true)
        };
        self.text.replace_range(self.caret..end, "");
        self.revision += 1;
    }

    pub fn move_caret(&mut self, movement: Movement, extend: bool) {
        let target = match movement {
            Movement::Left => match self.selection() {
                Some((start, _)) if !extend => start,
                _ => self.grapheme_boundary(self.caret, false),
            },
            Movement::Right => match self.selection() {
                Some((_, end)) if !extend => end,
                _ => self.grapheme_boundary(self.caret, true),
            },
            Movement::WordLeft => self.word_boundary(self.caret, false),
            Movement::WordRight => self.word_boundary(self.caret, true),
            Movement::Home => 0,
            Movement::End => self.text.len(),
        };
        self.set_caret(target, extend);
    }

    /// Places the caret (a click or drag); `extend` keeps the anchor.
    pub fn set_caret(&mut self, byte: usize, extend: bool) {
        let byte = self.snap(byte);
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        if byte != self.caret || !extend {
            self.revision += 1;
        }
        self.caret = byte;
    }

    /// Starts a selection at the caret (drag from a click).
    pub fn begin_selection(&mut self) {
        self.anchor = Some(self.caret);
    }

    /// Applies an editing key. Returns `true` if the key was an editing
    /// command (the host then must not forward it anywhere else). Clipboard
    /// commands and plain text are left to the host.
    pub fn handle_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let word = modifiers.control;
        match key {
            Key::Backspace => self.backspace(word),
            Key::Delete => self.delete(word),
            Key::ArrowLeft => self.move_caret(
                if word {
                    Movement::WordLeft
                } else {
                    Movement::Left
                },
                modifiers.shift,
            ),
            Key::ArrowRight => self.move_caret(
                if word {
                    Movement::WordRight
                } else {
                    Movement::Right
                },
                modifiers.shift,
            ),
            Key::Home => self.move_caret(Movement::Home, modifiers.shift),
            Key::End => self.move_caret(Movement::End, modifiers.shift),
            Key::Character(ch) if modifiers.control && ch.eq_ignore_ascii_case(&'a') => {
                self.select_all();
            }
            _ => return false,
        }
        true
    }

    fn snap(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        if self.text.is_char_boundary(byte)
            && self
                .text
                .grapheme_indices(true)
                .any(|(index, _)| index == byte)
            || byte == self.text.len()
        {
            return byte;
        }
        self.grapheme_boundary(byte, false)
    }

    fn grapheme_boundary(&self, from: usize, forward: bool) -> usize {
        if forward {
            self.text[from..]
                .grapheme_indices(true)
                .nth(1)
                .map_or(self.text.len(), |(index, _)| from + index)
        } else {
            self.text[..from]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(index, _)| index)
        }
    }

    fn word_boundary(&self, from: usize, forward: bool) -> usize {
        if forward {
            let rest = &self.text[from..];
            let mut offset = from;
            let mut seen_word = false;
            for (index, word) in rest.split_word_bound_indices() {
                let is_word = word.chars().any(char::is_alphanumeric);
                if seen_word && !is_word {
                    return from + index;
                }
                if is_word {
                    seen_word = true;
                }
                offset = from + index + word.len();
            }
            offset
        } else {
            let head = &self.text[..from];
            let mut result = 0;
            let mut seen_word = false;
            for (index, word) in head.split_word_bound_indices().rev() {
                let is_word = word.chars().any(char::is_alphanumeric);
                if seen_word && !is_word {
                    return index + word.len();
                }
                if is_word {
                    seen_word = true;
                }
                result = index;
            }
            result
        }
    }
}

/// Blink phase of a caret: visible for the first half of every period after the
/// last edit. The host repaints when this flips.
pub fn caret_visible(now: Duration, last_edit: Duration) -> bool {
    const HALF_PERIOD_MS: u128 = 530;
    (now.saturating_sub(last_edit).as_millis() / HALF_PERIOD_MS).is_multiple_of(2)
}

/// Painting and hit-testing of a single-line field.
pub struct TextField<'a> {
    pub edit: &'a TextEdit,
    pub placeholder: &'a str,
    pub focused: bool,
    pub caret_on: bool,
    /// Horizontal scroll (logical px) kept between frames so the caret stays
    /// in view without jumping.
    pub scroll: &'a mut f64,
}

const FIELD_PAD: f64 = 12.0;

fn field_style(theme: Theme) -> TextStyle {
    TextStyle {
        font_size: theme.metrics.font_md,
        line_height: theme.metrics.line_height_md,
        ..TextStyle::default()
    }
}

impl TextField<'_> {
    pub fn paint(
        self,
        rect: LogicalRect,
        text: &mut TextSystem,
        theme: Theme,
        builder: &mut PaintBuilder,
    ) {
        let colors = theme.colors;
        let radius = theme.metrics.radius_md.0;
        builder.fill_rect(rect, colors.window, radius);
        builder.stroke_rect(
            rect,
            if self.focused {
                colors.focus
            } else {
                colors.border
            },
            if self.focused {
                theme.metrics.focus_width.0
            } else {
                theme.metrics.border_width.0
            },
            radius,
        );
        let style = field_style(theme);
        let inner_left = rect.left() + FIELD_PAD;
        let inner_width = (rect.size.width.0 - 2.0 * FIELD_PAD).max(1.0);
        let line_height = style.line_height.0;
        let top = rect.top() + (rect.size.height.0 - line_height) / 2.0;
        let layout = Arc::new(text.shape(self.edit.text(), &style, None));
        let caret_x = layout.caret_x(self.edit.caret());
        let text_width = layout.metrics.size.width.0;
        // Keep the caret inside the visible window, with a little room.
        let mut scroll = (*self.scroll).clamp(0.0, (text_width - inner_width + 4.0).max(0.0));
        if caret_x - scroll > inner_width - 2.0 {
            scroll = caret_x - inner_width + 2.0;
        } else if caret_x - scroll < 0.0 {
            scroll = caret_x;
        }
        scroll = scroll.clamp(0.0, (text_width - inner_width + 4.0).max(0.0));
        *self.scroll = scroll;
        builder.push_clip(rect.inset(crate::Insets::all(1.0)));
        if self.edit.is_empty() {
            let hint = Arc::new(text.shape(self.placeholder, &style, Some(LogicalPx(inner_width))));
            builder.text(
                LogicalPoint::new(inner_left, top),
                hint,
                colors.text_disabled,
            );
        } else {
            if let Some((start, end)) = self.edit.selection() {
                let (x0, x1) = (layout.caret_x(start), layout.caret_x(end));
                builder.fill_rect(
                    LogicalRect::from_xywh(
                        inner_left + x0 - scroll,
                        top,
                        (x1 - x0).max(1.0),
                        line_height,
                    ),
                    colors.selection,
                    2.0,
                );
            }
            builder.text(
                LogicalPoint::new(inner_left - scroll, top),
                layout,
                colors.text_primary,
            );
        }
        if self.focused && self.caret_on {
            builder.fill_rect(
                LogicalRect::from_xywh(
                    inner_left + caret_x - scroll,
                    top + 1.0,
                    1.5,
                    line_height - 2.0,
                ),
                colors.accent,
                0.0,
            );
        }
        let _ = builder.pop_clip();
    }

    /// Byte index under logical x position `x` (a click in the field).
    pub fn index_at(
        text_value: &str,
        scroll: f64,
        rect: LogicalRect,
        text: &mut TextSystem,
        theme: Theme,
        x: f64,
    ) -> usize {
        let layout = text.shape(text_value, &field_style(theme), None);
        layout.index_at_x(x - (rect.left() + FIELD_PAD) + scroll, text_value.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> TextEdit {
        let mut edit = TextEdit::new(50);
        edit.insert(text);
        edit
    }

    #[test]
    fn insert_filters_controls_and_respects_the_limit() {
        let mut edit = TextEdit::new(10);
        edit.insert("Привет\r\nмир\u{0}!!!!");
        assert_eq!(edit.text(), "Привет мир");
        edit.insert("x");
        assert_eq!(edit.text(), "Привет мир", "limit reached");
        assert_eq!(edit.caret(), edit.text().len());
    }

    #[test]
    fn caret_moves_by_grapheme_and_inserts_in_the_middle() {
        let mut edit = typed("a👨‍👩‍👧b");
        edit.move_caret(Movement::Left, false);
        edit.move_caret(Movement::Left, false);
        assert_eq!(edit.caret(), 1, "the ZWJ family is one step");
        edit.insert("-");
        assert_eq!(edit.text(), "a-👨‍👩‍👧b");
        edit.move_caret(Movement::Home, false);
        assert_eq!(edit.caret(), 0);
        edit.move_caret(Movement::End, false);
        assert_eq!(edit.caret(), edit.text().len());
        edit.backspace(false);
        edit.backspace(false);
        assert_eq!(edit.text(), "a-");
    }

    #[test]
    fn selection_replace_delete_and_select_all() {
        let mut edit = typed("hello world");
        edit.select_all();
        assert_eq!(edit.selected_text(), Some("hello world"));
        edit.insert("Привет");
        assert_eq!(edit.text(), "Привет");
        assert_eq!(edit.selection(), None);
        edit.move_caret(Movement::Left, true);
        edit.move_caret(Movement::Left, true);
        assert_eq!(edit.selected_text(), Some("ет"));
        edit.delete(false);
        assert_eq!(edit.text(), "Прив");
        // Left collapses a selection to its start without moving further.
        edit.select_all();
        edit.move_caret(Movement::Left, false);
        assert_eq!((edit.caret(), edit.selection()), (0, None));
    }

    #[test]
    fn word_commands_work_across_scripts_and_punctuation() {
        let mut edit = typed("отчёт за квартал, финал");
        edit.backspace(true);
        assert_eq!(edit.text(), "отчёт за квартал, ");
        edit.backspace(true);
        assert_eq!(edit.text(), "отчёт за ");
        edit.move_caret(Movement::WordLeft, false);
        assert_eq!(edit.caret(), "отчёт ".len());
        edit.move_caret(Movement::WordRight, false);
        assert_eq!(edit.caret(), "отчёт за".len());
        edit.set_caret(0, false);
        edit.delete(true);
        assert_eq!(edit.text(), " за ");
    }

    #[test]
    fn handle_key_reports_only_editing_keys() {
        let mut edit = typed("abc");
        let none = Modifiers::default();
        assert!(edit.handle_key(Key::Backspace, none));
        assert_eq!(edit.text(), "ab");
        assert!(edit.handle_key(
            Key::ArrowLeft,
            Modifiers {
                shift: true,
                ..none
            }
        ));
        assert_eq!(edit.selected_text(), Some("b"));
        assert!(edit.handle_key(
            Key::Character('a'),
            Modifiers {
                control: true,
                ..none
            }
        ));
        assert_eq!(edit.selected_text(), Some("ab"));
        assert!(!edit.handle_key(Key::Enter, none));
        assert!(!edit.handle_key(Key::Escape, none));
        assert!(!edit.handle_key(Key::Character('x'), none));
    }

    #[test]
    fn revision_changes_on_every_visible_change_only() {
        let mut edit = typed("ab");
        let before = edit.revision();
        edit.move_caret(Movement::End, false);
        assert_eq!(
            edit.revision(),
            before + 1,
            "a plain caret move restarts the blink"
        );
        let mid = edit.revision();
        edit.insert("");
        assert_eq!(edit.revision(), mid, "inserting nothing changes nothing");
        edit.clear();
        assert_ne!(edit.revision(), mid);
    }

    #[test]
    fn caret_blinks_with_a_visible_first_half() {
        let t0 = Duration::from_secs(5);
        assert!(caret_visible(t0, t0));
        assert!(caret_visible(t0 + Duration::from_millis(500), t0));
        assert!(!caret_visible(t0 + Duration::from_millis(600), t0));
        assert!(caret_visible(t0 + Duration::from_millis(1100), t0));
    }

    #[test]
    fn caret_positions_round_trip_through_the_shaped_text() {
        let mut system = TextSystem::with_cache_limits(64, 1 << 20);
        let style = field_style(Theme::EVERTY_LIGHT);
        for text in ["hello world", "Привет, мир", "a👨‍👩‍👧b", ""] {
            let layout = system.shape(text, &style, None);
            let boundaries: Vec<usize> = text
                .grapheme_indices(true)
                .map(|(index, _)| index)
                .chain([text.len()])
                .collect();
            let mut previous = -1.0;
            for byte in &boundaries {
                let x = layout.caret_x(*byte);
                assert!(x >= previous, "{text:?}: caret x is monotonic");
                previous = x;
                // Clicking exactly on a caret position selects that position.
                assert_eq!(
                    layout.index_at_x(x, text.len()),
                    *byte,
                    "{text:?} at {byte}"
                );
            }
            assert!((layout.caret_x(text.len()) - layout.metrics.size.width.0).abs() < 1.5);
        }
    }

    #[test]
    fn painting_scrolls_to_keep_the_caret_visible_and_draws_a_caret() {
        let theme = Theme::EVERTY_LIGHT;
        let mut system = TextSystem::with_cache_limits(64, 1 << 20);
        let mut edit = TextEdit::new(500);
        edit.insert(&"очень длинная строка ".repeat(8));
        let rect = LogicalRect::from_xywh(10.0, 10.0, 200.0, 40.0);
        let mut scroll = 0.0;
        let mut builder = PaintBuilder::new();
        TextField {
            edit: &edit,
            placeholder: "…",
            focused: true,
            caret_on: true,
            scroll: &mut scroll,
        }
        .paint(rect, &mut system, theme, &mut builder);
        assert!(scroll > 100.0, "scrolled to the end: {scroll}");
        let caret_visible_x = {
            let layout = system.shape(edit.text(), &field_style(theme), None);
            rect.left() + FIELD_PAD + layout.caret_x(edit.caret()) - scroll
        };
        assert!(
            caret_visible_x <= rect.right() - FIELD_PAD + 2.0 && caret_visible_x >= rect.left()
        );
        // Click in the middle maps to a position before the end.
        let index = TextField::index_at(
            edit.text(),
            scroll,
            rect,
            &mut system,
            theme,
            rect.left() + 100.0,
        );
        assert!(index < edit.text().len());
    }
}
