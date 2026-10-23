use everty_ui::Color;
use std::collections::VecDeque;
use vte::{Params, Parser, Perform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
}

pub const DEFAULT_TERM_FG: Color = Color::rgba(226, 232, 240, 255);
pub const DEFAULT_TERM_BG: Color = Color::rgba(8, 13, 25, 255);

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_TERM_FG,
            bg: DEFAULT_TERM_BG,
            bold: false,
            underline: false,
        }
    }
}

pub struct TerminalScreen {
    pub cols: usize,
    pub rows: usize,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub cursor_visible: bool,
    saved_cursor: (usize, usize),

    current_fg: Color,
    current_bg: Color,
    default_fg: Color,
    default_bg: Color,
    bold: bool,
    underline: bool,

    screen: Vec<Vec<Cell>>,
    alt_screen: Option<Vec<Vec<Cell>>>,
    scrollback: VecDeque<Vec<Cell>>,
    max_scrollback: usize,
    pub scroll_offset: usize, // 0 = at bottom (active screen)
}

impl TerminalScreen {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let default_fg = DEFAULT_TERM_FG;
        let default_bg = DEFAULT_TERM_BG;
        let screen = vec![vec![Cell::default(); cols]; rows];

        Self {
            cols,
            rows,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            saved_cursor: (0, 0),
            current_fg: default_fg,
            current_bg: default_bg,
            default_fg,
            default_bg,
            bold: false,
            underline: false,
            screen,
            alt_screen: None,
            scrollback: VecDeque::with_capacity(max_scrollback),
            max_scrollback,
            scroll_offset: 0,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        let _old_cols = self.cols;
        let old_rows = self.rows;
        self.cols = cols;
        self.rows = rows;

        // Resize column width for all rows in primary screen
        for row in &mut self.screen {
            row.resize(
                cols,
                Cell {
                    ch: ' ',
                    fg: self.default_fg,
                    bg: self.default_bg,
                    bold: false,
                    underline: false,
                },
            );
        }

        // Also resize column width for scrollback rows
        for row in &mut self.scrollback {
            row.resize(
                cols,
                Cell {
                    ch: ' ',
                    fg: self.default_fg,
                    bg: self.default_bg,
                    bold: false,
                    underline: false,
                },
            );
        }

        if self.alt_screen.is_none() {
            if rows < old_rows {
                // Height decreased (window compressed vertically).
                // Ensure active output and prompt at the bottom stay in view by shifting
                // top lines into scrollback instead of discarding bottom lines.
                let last_used = self
                    .screen
                    .iter()
                    .rposition(|row| row.iter().any(|c| c.ch != ' ' || c.bg != self.default_bg))
                    .unwrap_or(0);
                let bottom_active = last_used.max(self.cursor_y);
                if bottom_active >= rows {
                    let overflow = (bottom_active + 1 - rows).min(self.screen.len());
                    for _ in 0..overflow {
                        let top_row = self.screen.remove(0);
                        if self.scrollback.len() >= self.max_scrollback {
                            self.scrollback.pop_front();
                        }
                        self.scrollback.push_back(top_row);
                    }
                    self.cursor_y = self.cursor_y.saturating_sub(overflow);
                }
            } else if rows > old_rows && self.scroll_offset == 0 {
                // Height increased (window expanded vertically).
                // Pull lines from scrollback to top of screen so the prompt stays anchored at bottom.
                let diff = rows - old_rows;
                let pull_count = diff.min(self.scrollback.len());
                for _ in 0..pull_count {
                    if let Some(mut row) = self.scrollback.pop_back() {
                        row.resize(
                            cols,
                            Cell {
                                ch: ' ',
                                fg: self.default_fg,
                                bg: self.default_bg,
                                bold: false,
                                underline: false,
                            },
                        );
                        self.screen.insert(0, row);
                    }
                }
                self.cursor_y = (self.cursor_y + pull_count).min(rows.saturating_sub(1));
            }
        }

        // Ensure primary screen has exactly `rows` rows
        while self.screen.len() < rows {
            self.screen.push(vec![
                Cell {
                    ch: ' ',
                    fg: self.default_fg,
                    bg: self.default_bg,
                    bold: false,
                    underline: false,
                };
                cols
            ]);
        }
        if self.screen.len() > rows {
            self.screen.truncate(rows);
        }

        // Handle alternate screen buffer if active (e.g. htop, nano, mc)
        if let Some(ref mut alt) = self.alt_screen {
            for row in alt.iter_mut() {
                row.resize(
                    cols,
                    Cell {
                        ch: ' ',
                        fg: self.default_fg,
                        bg: self.default_bg,
                        bold: false,
                        underline: false,
                    },
                );
            }
            while alt.len() < rows {
                alt.push(vec![
                    Cell {
                        ch: ' ',
                        fg: self.default_fg,
                        bg: self.default_bg,
                        bold: false,
                        underline: false,
                    };
                    cols
                ]);
            }
            if alt.len() > rows {
                alt.truncate(rows);
            }
        }

        if self.cursor_x >= cols {
            self.cursor_x = cols.saturating_sub(1);
        }
        if self.cursor_y >= rows {
            self.cursor_y = rows.saturating_sub(1);
        }
        self.scroll_offset = self.scroll_offset.min(self.scrollback.len());
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let max = self.scrollback.len();
        self.scroll_offset = (self.scroll_offset + lines).min(max);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        if self.scroll_offset >= lines {
            self.scroll_offset -= lines;
        } else {
            self.scroll_offset = 0;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn get_row(&self, visual_y: usize) -> Option<&[Cell]> {
        if visual_y >= self.rows {
            return None;
        }

        if self.scroll_offset == 0 {
            self.screen.get(visual_y).map(|r| r.as_slice())
        } else {
            let total_scrollback = self.scrollback.len();
            if self.scroll_offset > total_scrollback {
                return None;
            }
            let scrollback_start = total_scrollback - self.scroll_offset;
            let offset = scrollback_start + visual_y;
            if offset < total_scrollback {
                self.scrollback.get(offset).map(|r| r.as_slice())
            } else {
                let screen_idx = offset - total_scrollback;
                self.screen.get(screen_idx).map(|r| r.as_slice())
            }
        }
    }

    fn linefeed(&mut self) {
        if self.cursor_y + 1 < self.rows {
            self.cursor_y += 1;
        } else if !self.screen.is_empty() {
            let top_row = self.screen.remove(0);
            if self.scrollback.len() >= self.max_scrollback {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(top_row);
            self.screen.push(vec![
                Cell {
                    ch: ' ',
                    fg: self.default_fg,
                    bg: self.default_bg,
                    bold: false,
                    underline: false,
                };
                self.cols
            ]);
        }
    }

    fn reset_sgr(&mut self) {
        self.current_fg = self.default_fg;
        self.current_bg = self.default_bg;
        self.bold = false;
        self.underline = false;
    }

    pub fn ansi_color(code: u16, bright: bool) -> Color {
        match (code, bright) {
            (0, false) => Color::rgba(0, 0, 0, 255),
            (0, true) => Color::rgba(100, 100, 100, 255),
            (1, false) => Color::rgba(220, 50, 47, 255),
            (1, true) => Color::rgba(255, 100, 95, 255),
            (2, false) => Color::rgba(40, 180, 99, 255),
            (2, true) => Color::rgba(72, 219, 137, 255),
            (3, false) => Color::rgba(212, 172, 13, 255),
            (3, true) => Color::rgba(245, 215, 66, 255),
            (4, false) => Color::rgba(41, 128, 185, 255),
            (4, true) => Color::rgba(93, 173, 226, 255),
            (5, false) => Color::rgba(142, 68, 173, 255),
            (5, true) => Color::rgba(187, 143, 206, 255),
            (6, false) => Color::rgba(22, 160, 133, 255),
            (6, true) => Color::rgba(72, 201, 176, 255),
            (7, false) => Color::rgba(200, 200, 200, 255),
            (7, true) => Color::rgba(255, 255, 255, 255),
            _ => Color::rgba(220, 225, 230, 255),
        }
    }

    pub fn color_from_256(index: u16) -> Color {
        if index < 16 {
            Self::ansi_color(index % 8, index >= 8)
        } else if index < 232 {
            let idx = index - 16;
            let r = (idx / 36) * 51;
            let g = ((idx % 36) / 6) * 51;
            let b = (idx % 6) * 51;
            Color::rgba(r as u8, g as u8, b as u8, 255)
        } else {
            let gray = (index - 232) * 10 + 8;
            Color::rgba(gray as u8, gray as u8, gray as u8, 255)
        }
    }
}

impl Perform for TerminalScreen {
    fn print(&mut self, c: char) {
        if self.cursor_x >= self.cols {
            self.cursor_x = 0;
            self.linefeed();
        }

        let x = self.cursor_x;
        let y = self.cursor_y;

        if y < self.screen.len() && x < self.cols {
            if let Some(row) = self.screen.get_mut(y) {
                if x < row.len() {
                    row[x] = Cell {
                        ch: c,
                        fg: self.current_fg,
                        bg: self.current_bg,
                        bold: self.bold,
                        underline: self.underline,
                    };
                    self.cursor_x += 1;
                }
            }
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0C => {
                self.linefeed();
            }
            b'\r' => {
                self.cursor_x = 0;
            }
            b'\t' => {
                let next_tab = (self.cursor_x / 8 + 1) * 8;
                self.cursor_x = next_tab.min(self.cols.saturating_sub(1));
            }
            0x08 => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Collect primary sub-parameter safely without indexing panic
        let mut p: Vec<u16> = Vec::new();
        for sub in params.iter() {
            for &val in sub {
                p.push(val);
            }
        }

        let is_private = intermediates.contains(&b'?');

        if is_private {
            match action {
                'h' => {
                    // DEC Private Mode Set
                    for &val in &p {
                        match val {
                            25 => self.cursor_visible = true,
                            1049 | 47 => {
                                // Enable alternate screen buffer
                                if self.alt_screen.is_none() {
                                    let alt = vec![vec![Cell::default(); self.cols]; self.rows];
                                    self.alt_screen = Some(std::mem::replace(&mut self.screen, alt));
                                }
                            }
                            _ => {}
                        }
                    }
                    return;
                }
                'l' => {
                    // DEC Private Mode Reset
                    for &val in &p {
                        match val {
                            25 => self.cursor_visible = false,
                            1049 | 47 => {
                                // Disable alternate screen buffer
                                if let Some(primary) = self.alt_screen.take() {
                                    self.screen = primary;
                                }
                            }
                            _ => {}
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        match action {
            'm' => {
                if p.is_empty() {
                    self.reset_sgr();
                    return;
                }
                let mut i = 0;
                while i < p.len() {
                    match p[i] {
                        0 => self.reset_sgr(),
                        1 => self.bold = true,
                        4 => self.underline = true,
                        22 => self.bold = false,
                        24 => self.underline = false,
                        30..=37 => self.current_fg = Self::ansi_color(p[i] - 30, false),
                        39 => self.current_fg = self.default_fg,
                        40..=47 => self.current_bg = Self::ansi_color(p[i] - 40, false),
                        49 => self.current_bg = self.default_bg,
                        90..=97 => self.current_fg = Self::ansi_color(p[i] - 90, true),
                        100..=107 => self.current_bg = Self::ansi_color(p[i] - 100, true),
                        38 => {
                            if i + 2 < p.len() && p[i + 1] == 5 {
                                self.current_fg = Self::color_from_256(p[i + 2]);
                                i += 2;
                            } else if i + 4 < p.len() && p[i + 1] == 2 {
                                self.current_fg =
                                    Color::rgba(p[i + 2] as u8, p[i + 3] as u8, p[i + 4] as u8, 255);
                                i += 4;
                            }
                        }
                        48 => {
                            if i + 2 < p.len() && p[i + 1] == 5 {
                                self.current_bg = Self::color_from_256(p[i + 2]);
                                i += 2;
                            } else if i + 4 < p.len() && p[i + 1] == 2 {
                                self.current_bg =
                                    Color::rgba(p[i + 2] as u8, p[i + 3] as u8, p[i + 4] as u8, 255);
                                i += 4;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            'A' => {
                let n = p.first().copied().unwrap_or(1).max(1) as usize;
                self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            'B' => {
                let n = p.first().copied().unwrap_or(1).max(1) as usize;
                self.cursor_y = (self.cursor_y + n).min(self.rows.saturating_sub(1));
            }
            'C' => {
                let n = p.first().copied().unwrap_or(1).max(1) as usize;
                self.cursor_x = (self.cursor_x + n).min(self.cols.saturating_sub(1));
            }
            'D' => {
                let n = p.first().copied().unwrap_or(1).max(1) as usize;
                self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            'H' | 'f' => {
                let r = p.first().copied().unwrap_or(1).max(1) as usize - 1;
                let c = p.get(1).copied().unwrap_or(1).max(1) as usize - 1;
                self.cursor_y = r.min(self.rows.saturating_sub(1));
                self.cursor_x = c.min(self.cols.saturating_sub(1));
            }
            'J' => {
                let mode = p.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        let cur_y = self.cursor_y;
                        let cur_x = self.cursor_x;
                        if cur_y < self.screen.len() {
                            for x in cur_x..self.cols {
                                if x < self.screen[cur_y].len() {
                                    self.screen[cur_y][x] = Cell::default();
                                }
                            }
                        }
                        for y in (cur_y + 1)..self.rows {
                            if y < self.screen.len() {
                                for x in 0..self.cols {
                                    if x < self.screen[y].len() {
                                        self.screen[y][x] = Cell::default();
                                    }
                                }
                            }
                        }
                    }
                    1 => {
                        let cur_y = self.cursor_y;
                        let cur_x = self.cursor_x;
                        for y in 0..cur_y {
                            if y < self.screen.len() {
                                for x in 0..self.cols {
                                    if x < self.screen[y].len() {
                                        self.screen[y][x] = Cell::default();
                                    }
                                }
                            }
                        }
                        if cur_y < self.screen.len() {
                            for x in 0..=cur_x.min(self.cols.saturating_sub(1)) {
                                if x < self.screen[cur_y].len() {
                                    self.screen[cur_y][x] = Cell::default();
                                }
                            }
                        }
                    }
                    2 | 3 => {
                        for row in &mut self.screen {
                            for cell in row.iter_mut() {
                                *cell = Cell::default();
                            }
                        }
                        self.cursor_x = 0;
                        self.cursor_y = 0;
                    }
                    _ => {}
                }
            }
            'K' => {
                let mode = p.first().copied().unwrap_or(0);
                let cur_y = self.cursor_y;
                let cur_x = self.cursor_x;
                if cur_y < self.screen.len() {
                    match mode {
                        0 => {
                            for x in cur_x..self.cols {
                                if x < self.screen[cur_y].len() {
                                    self.screen[cur_y][x] = Cell::default();
                                }
                            }
                        }
                        1 => {
                            for x in 0..=cur_x.min(self.cols.saturating_sub(1)) {
                                if x < self.screen[cur_y].len() {
                                    self.screen[cur_y][x] = Cell::default();
                                }
                            }
                        }
                        2 => {
                            for cell in self.screen[cur_y].iter_mut() {
                                *cell = Cell::default();
                            }
                        }
                        _ => {}
                    }
                }
            }
            's' => {
                self.saved_cursor = (self.cursor_x, self.cursor_y);
            }
            'u' => {
                self.cursor_x = self.saved_cursor.0.min(self.cols.saturating_sub(1));
                self.cursor_y = self.saved_cursor.1.min(self.rows.saturating_sub(1));
            }
            _ => {}
        }
    }
}

pub struct Terminal {
    pub screen: TerminalScreen,
    parser: Parser,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        Self {
            screen: TerminalScreen::new(cols, rows, max_scrollback),
            parser: Parser::new(),
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.screen.resize(cols, rows);
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.screen.scroll_up(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.screen.scroll_down(lines);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.screen.scroll_to_bottom();
    }

    pub fn get_row(&self, visual_y: usize) -> Option<&[Cell]> {
        self.screen.get_row(visual_y)
    }

    pub fn cols(&self) -> usize {
        self.screen.cols
    }

    pub fn rows(&self) -> usize {
        self.screen.rows
    }

    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.screen.cursor_x, self.screen.cursor_y)
    }

    pub fn scrollback_len(&self) -> usize {
        self.screen.scrollback.len()
    }

    pub fn scroll_offset(&self) -> usize {
        self.screen.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        let max = self.screen.scrollback.len();
        self.screen.scroll_offset = offset.min(max);
    }

    pub fn get_text_range(&self, start: (usize, usize), end: (usize, usize)) -> String {
        let ((c1, r1), (c2, r2)) = if start.1 < end.1 || (start.1 == end.1 && start.0 <= end.0) {
            (start, end)
        } else {
            (end, start)
        };

        let mut lines = Vec::new();
        for r in r1..=r2 {
            if let Some(row) = self.get_row(r) {
                let start_col = if r == r1 { c1 } else { 0 };
                let end_col = if r == r2 { (c2 + 1).min(row.len()) } else { row.len() };
                if start_col < end_col && start_col < row.len() {
                    let text: String = row[start_col..end_col].iter().map(|c| c.ch).collect();
                    lines.push(text.trim_end().to_string());
                } else {
                    lines.push(String::new());
                }
            }
        }
        lines.join("\n")
    }

    pub fn search(&self, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let total_sb = self.screen.scrollback.len();
        let total_lines = total_sb + self.screen.rows;
        let mut matches = Vec::new();

        let query_cmp = if case_sensitive {
            q.to_string()
        } else {
            q.to_lowercase()
        };

        for abs_row in 0..total_lines {
            let row_cells: Option<&[Cell]> = if abs_row < total_sb {
                self.screen.scrollback.get(abs_row).map(|r| r.as_slice())
            } else {
                let s_idx = abs_row - total_sb;
                if s_idx < self.screen.screen.len() {
                    Some(&self.screen.screen[s_idx])
                } else {
                    None
                }
            };

            if let Some(cells) = row_cells {
                let row_str: String = cells.iter().map(|c| c.ch).collect();
                let compare_str = if case_sensitive {
                    row_str.clone()
                } else {
                    row_str.to_lowercase()
                };

                if !query_cmp.is_empty() {
                    let mut start_idx = 0;
                    while let Some(found_idx) = compare_str[start_idx..].find(&query_cmp) {
                        let byte_start = start_idx + found_idx;
                        let col_start = compare_str[..byte_start].chars().count();
                        let col_end = col_start + query_cmp.chars().count();
                        matches.push(SearchMatch {
                            abs_row,
                            col_start,
                            col_end,
                        });
                        start_idx = byte_start + query_cmp.len();
                        if start_idx >= compare_str.len() {
                            break;
                        }
                    }
                }
            }
        }

        matches
    }

    pub fn scroll_to_abs_row(&mut self, abs_row: usize) {
        let total_sb = self.screen.scrollback.len();
        if abs_row >= total_sb {
            self.screen.scroll_offset = 0;
        } else {
            let center_offset = self.screen.rows / 2;
            let target_offset = total_sb.saturating_sub(abs_row);
            let adjusted = target_offset.saturating_add(center_offset).min(total_sb);
            self.screen.scroll_offset = adjusted;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub abs_row: usize,
    pub col_start: usize,
    pub col_end: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_basic_print_and_ansi() {
        let mut term = Terminal::new(80, 24, 100);
        term.write_bytes(b"Hello, EvertySSH!\r\n");
        assert_eq!(term.screen.cursor_y, 1);
        assert_eq!(term.screen.cursor_x, 0);

        let row0 = term.get_row(0).unwrap();
        let text: String = row0[..17].iter().map(|c| c.ch).collect();
        assert_eq!(text, "Hello, EvertySSH!");

        term.write_bytes(b"\x1b[32mActive\x1b[0m");
        let row1 = term.get_row(1).unwrap();
        assert_eq!(row1[0].ch, 'A');
        assert_eq!(row1[0].fg, TerminalScreen::ansi_color(2, false));
    }

    #[test]
    fn test_terminal_search() {
        let mut term = Terminal::new(80, 24, 100);
        term.write_bytes(b"nginx error in /var/log/nginx.log\r\nconnection timeout\r\n");
        let matches = term.search("nginx", false);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].abs_row, 0);
        assert_eq!(matches[0].col_start, 0);
        assert_eq!(matches[0].col_end, 5);
        assert_eq!(matches[1].col_start, 24);
        assert_eq!(matches[1].col_end, 29);

        let matches_case = term.search("NGINX", true);
        assert_eq!(matches_case.len(), 0);
    }

    #[test]
    fn test_terminal_prompt_sequence() {
        let mut term = Terminal::new(80, 24, 100);
        term.write_bytes(b"\x1b[?2004h");
        term.write_bytes(b"\x1b]0;root@rigid-purple: ~\x07root@rigid-purple:~# ");
        let row = term.get_row(0).unwrap();
        let s: String = row.iter().map(|c| c.ch).collect();
        println!("Terminal row 0: {:?}", s.trim_end());
        assert!(s.contains("root@rigid-purple"));
    }

    #[test]
    fn test_terminal_resize_shrink_keeps_prompt_visible() {
        let mut term = Terminal::new(80, 10, 100);
        // Fill 10 rows with commands and output, ending with a prompt
        for i in 0..9 {
            term.write_bytes(format!("line {}\r\n", i).as_bytes());
        }
        term.write_bytes(b"vaali@viewracebeta:~$ ");

        // Prompt is at row 9 (bottom row of 10 rows)
        let (cx, cy) = term.cursor_pos();
        assert_eq!(cy, 9);
        assert_eq!(cx, 22);

        let row9: String = term.get_row(9).unwrap().iter().map(|c| c.ch).collect();
        assert!(row9.starts_with("vaali@viewracebeta:~$ "));

        // Shrink height from 10 to 5 rows
        term.resize(80, 5);

        // Prompt MUST still be visible at the bottom (row 4)
        assert_eq!(term.rows(), 5);
        let (_, new_cy) = term.cursor_pos();
        assert_eq!(new_cy, 4);

        let new_row4: String = term.get_row(4).unwrap().iter().map(|c| c.ch).collect();
        assert!(new_row4.starts_with("vaali@viewracebeta:~$ "));

        // Previous rows must have moved to scrollback
        assert!(term.scrollback_len() >= 5);
    }

    #[test]
    fn test_terminal_resize_expand_pulls_from_scrollback() {
        let mut term = Terminal::new(80, 5, 100);
        for i in 0..10 {
            term.write_bytes(format!("output {}\r\n", i).as_bytes());
        }
        term.write_bytes(b"user@host:~$ ");

        // Prompt is at bottom row 4
        let (_, cy) = term.cursor_pos();
        assert_eq!(cy, 4);

        // Expand height from 5 to 8 rows
        term.resize(80, 8);
        assert_eq!(term.rows(), 8);

        // Prompt stays at bottom row (row 7) because 3 rows were pulled from scrollback
        let (_, new_cy) = term.cursor_pos();
        assert_eq!(new_cy, 7);
        let bottom_row: String = term.get_row(7).unwrap().iter().map(|c| c.ch).collect();
        assert!(bottom_row.starts_with("user@host:~$ "));
    }
}
