//! Display normalization for titles set by the child process (OSC 0 / OSC 2).

/// Longest title shown, in Unicode scalar values, including the trailing ellipsis.
pub const MAX_TERMINAL_TITLE_CHARS: usize = 120;

/// Product name shown alone in the window title and appended after a terminal title.
const PRODUCT_NAME: &str = "Kegon";

/// Normalizes a child-provided title for display, or returns `None` if nothing displayable
/// remains, in which case callers show their default label.
///
/// The child controls this string, so control characters (C0, DEL, C1, newlines, tabs) are
/// dropped rather than rendered, and the length is bounded so it cannot stretch the UI.
pub fn normalize_terminal_title(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.chars().count() <= MAX_TERMINAL_TITLE_CHARS {
        return Some(trimmed.to_owned());
    }
    let mut truncated: String = trimmed.chars().take(MAX_TERMINAL_TITLE_CHARS - 1).collect();
    truncated.truncate(truncated.trim_end().len());
    truncated.push('…');
    Some(truncated)
}

/// Window title for an already-normalized terminal title: `<title> - Kegon`, or `Kegon`.
pub fn window_title(terminal_title: Option<&str>) -> String {
    match terminal_title {
        Some(title) => format!("{title} - {PRODUCT_NAME}"),
        None => String::from(PRODUCT_NAME),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_title_is_kept() {
        let title = normalize_terminal_title("Kegon title test");
        assert_eq!(title.as_deref(), Some("Kegon title test"));
        assert_eq!(window_title(title.as_deref()), "Kegon title test - Kegon");
    }

    #[test]
    fn unicode_title_is_preserved() {
        for raw in [
            "日本語タイトル",
            "PowerShell 日本語",
            "build 🚀 done",
            "Café Ñandú",
        ] {
            assert_eq!(normalize_terminal_title(raw).as_deref(), Some(raw));
        }
    }

    #[test]
    fn control_characters_are_removed() {
        assert_eq!(
            normalize_terminal_title("hello\nworld\x07").as_deref(),
            Some("helloworld")
        );
        assert_eq!(
            normalize_terminal_title("\tA\r\x7fB\u{9b}C\x1b").as_deref(),
            Some("ABC")
        );
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(
            normalize_terminal_title("  pwsh  ").as_deref(),
            Some("pwsh")
        );
    }

    #[test]
    fn empty_or_undisplayable_title_falls_back_to_default() {
        for raw in ["", "   ", "\x07\x1b\n", " \t \r "] {
            assert_eq!(normalize_terminal_title(raw), None, "{raw:?}");
        }
        assert_eq!(window_title(None), "Kegon");
    }

    #[test]
    fn long_title_is_truncated_by_chars_with_ellipsis() {
        let exact: String = "あ".repeat(MAX_TERMINAL_TITLE_CHARS);
        assert_eq!(normalize_terminal_title(&exact), Some(exact.clone()));

        let long: String = "あ".repeat(MAX_TERMINAL_TITLE_CHARS + 50);
        let title = normalize_terminal_title(&long).unwrap();
        assert_eq!(title.chars().count(), MAX_TERMINAL_TITLE_CHARS);
        assert!(title.ends_with('…'));
        assert!(title.starts_with("ああ"));
    }

    #[test]
    fn truncation_does_not_leave_space_before_ellipsis() {
        let raw = format!("{} tail", "a".repeat(MAX_TERMINAL_TITLE_CHARS - 2));
        let title = normalize_terminal_title(&raw).unwrap();
        assert!(title.ends_with("a…"), "{title}");
    }
}
