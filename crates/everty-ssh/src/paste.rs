pub fn needs_preview(text: &str) -> bool {
    text.chars().any(|c| c.is_control() && c != '\t')
}

pub fn encode(text: &str, bracketed: bool) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    // Embedded ESC must never be able to terminate the bracketed paste early.
    let cleaned: String = normalized
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    if bracketed {
        format!("\x1b[200~{cleaned}\x1b[201~")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_paste_is_reviewed_and_cannot_inject_an_end_marker() {
        assert!(!needs_preview("ls -la"));
        assert!(needs_preview("ls\r\nreboot\n"));
        assert_eq!(encode("ls\r\npwd\r", true), "\x1b[200~ls\npwd\n\x1b[201~");
        assert_eq!(encode("a\x1b[201~\x03b", true), "\x1b[200~a[201~b\x1b[201~");
        assert_eq!(encode("plain", false), "plain");
    }
}
