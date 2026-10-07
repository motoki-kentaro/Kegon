//! Terminal keyboard mode flags reflecting terminal emulator state.

/// Keyboard mode flags for terminal key encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalKeyboardMode {
    /// DECCKM application cursor keys mode.
    pub app_cursor: bool,
    /// Bracketed paste mode.
    pub bracketed_paste: bool,
    /// Kitty keyboard protocol: disambiguate escape codes (flag 1).
    pub disambiguate_esc_codes: bool,
    /// Kitty keyboard protocol: report event types press/repeat/release (flag 2).
    pub report_event_types: bool,
    /// Kitty keyboard protocol: report alternate keys (flag 4).
    pub report_alternate_keys: bool,
    /// Kitty keyboard protocol: report all keys as escape sequences (flag 8).
    pub report_all_keys_as_esc: bool,
    /// Kitty keyboard protocol: report associated text (flag 16).
    pub report_associated_text: bool,
}

impl TerminalKeyboardMode {
    /// Returns true if any Kitty extended keyboard protocol flag is active.
    pub fn is_extended(&self) -> bool {
        self.disambiguate_esc_codes
            || self.report_event_types
            || self.report_alternate_keys
            || self.report_all_keys_as_esc
            || self.report_associated_text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_not_extended() {
        let mode = TerminalKeyboardMode::default();
        assert!(!mode.is_extended());
        assert!(!mode.app_cursor);
        assert!(!mode.bracketed_paste);
    }

    #[test]
    fn extended_detected_when_any_kitty_flag_set() {
        let mode = TerminalKeyboardMode {
            disambiguate_esc_codes: true,
            ..Default::default()
        };
        assert!(mode.is_extended());
    }
}
