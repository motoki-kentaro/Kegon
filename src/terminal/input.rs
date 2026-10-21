use crate::command::{CommandContext, CommandId, KeybindingResolver, Platform};
use crate::terminal::arbitration::{InputArbiter, InputRoute};
use crate::terminal::input_event::TerminalInputEvent;
use crate::terminal::key_encoder::TerminalKeyEncoder;
use crate::terminal::mode::TerminalKeyboardMode;
use iced::keyboard::{Modifiers, key};

/// Result of processing a keyboard event in the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum InputAction {
    /// Send escape sequence / bytes to the child process via PTY.
    SendToPty(Vec<u8>),
    /// Perform a copy operation (Ctrl+C with active selection).
    CopySelection,
    /// Perform a paste operation (Ctrl+V).
    PasteFromClipboard,
    /// Open (or re-select) the Settings tab.
    OpenSettingsTab,
    /// Ignore event (not handled by terminal).
    Ignore,
}

/// Formats text for pasting into the terminal based on bracketed paste mode.
///
/// In bracketed paste mode the receiver treats everything up to `ESC [ 2 0 1 ~` as data, so a body
/// that contains that sequence would end the paste early and have the rest run as typed input.
/// Every ESC is therefore removed from the body: no escape sequence, the end marker included, can
/// be formed from it. Only the two markers Kegon adds are sent as escape sequences; the rest of the
/// body (newlines, Unicode) is unchanged.
pub fn format_paste(text: &str, bracketed_paste: bool) -> Vec<u8> {
    if bracketed_paste {
        let mut buf = Vec::with_capacity(text.len() + 12);
        buf.extend_from_slice(b"\x1b[200~");
        buf.extend(text.bytes().filter(|&byte| byte != 0x1b));
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        // Normalize line endings to CR for terminal input
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Processes a key press event and classifies it into an [`InputAction`].
#[allow(dead_code)]
pub fn process_key_event(
    physical_key: key::Physical,
    logical_key: &key::Key,
    modifiers: Modifiers,
    text: Option<&str>,
    has_selection: bool,
    app_cursor_keys: bool,
) -> InputAction {
    process_key_event_with_resolver(
        &KeybindingResolver::default_for_platform(Platform::current()),
        physical_key,
        logical_key,
        modifiers,
        text,
        has_selection,
        app_cursor_keys,
    )
}

/// Processes a key press event using a specific [`KeybindingResolver`].
#[allow(dead_code)]
pub fn process_key_event_with_resolver(
    resolver: &KeybindingResolver,
    physical_key: key::Physical,
    logical_key: &key::Key,
    modifiers: Modifiers,
    text: Option<&str>,
    has_selection: bool,
    app_cursor_keys: bool,
) -> InputAction {
    let route = InputArbiter::arbitrate_key_event(
        resolver,
        CommandContext::TerminalFocused,
        logical_key,
        modifiers,
        false,
        false,
    );

    match route {
        InputRoute::Modal | InputRoute::Ime => InputAction::Ignore,
        InputRoute::Command(command) => match command {
            CommandId::TerminalCopy => InputAction::CopySelection,
            CommandId::TerminalInterrupt => InputAction::SendToPty(vec![0x03]),
            CommandId::TerminalCopyOrInterrupt => {
                if has_selection {
                    InputAction::CopySelection
                } else {
                    InputAction::SendToPty(vec![0x03])
                }
            }
            CommandId::TerminalPaste => InputAction::PasteFromClipboard,
            CommandId::WorkbenchSettingsOpen => InputAction::OpenSettingsTab,
            CommandId::DialogConfirm | CommandId::DialogCancel => InputAction::Ignore,
        },
        InputRoute::Terminal => {
            let input_evt = TerminalInputEvent::press(
                logical_key.clone(),
                physical_key,
                modifiers,
                text.map(String::from),
            );
            let mode = TerminalKeyboardMode {
                app_cursor: app_cursor_keys,
                ..Default::default()
            };
            if let Some(bytes) = TerminalKeyEncoder::encode(&input_evt, mode) {
                InputAction::SendToPty(bytes)
            } else {
                InputAction::Ignore
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(c: key::Code) -> key::Physical {
        key::Physical::Code(c)
    }

    #[test]
    fn ctrl_c_with_selection_copies_text() {
        let action = process_key_event(
            code(key::Code::KeyC),
            &key::Key::Character("c".into()),
            Modifiers::COMMAND,
            Some("c"),
            true, // has_selection = true
            false,
        );
        assert_eq!(action, InputAction::CopySelection);
    }

    #[test]
    fn ctrl_c_without_selection_sends_interrupt() {
        let action = process_key_event(
            code(key::Code::KeyC),
            &key::Key::Character("c".into()),
            Modifiers::COMMAND,
            Some("c"),
            false, // has_selection = false
            false,
        );
        assert_eq!(action, InputAction::SendToPty(vec![0x03]));
    }

    #[test]
    fn ctrl_v_triggers_paste() {
        let action = process_key_event(
            code(key::Code::KeyV),
            &key::Key::Character("v".into()),
            Modifiers::COMMAND,
            Some("v"),
            false,
            false,
        );
        assert_eq!(action, InputAction::PasteFromClipboard);
    }

    #[test]
    fn bracketed_paste_formatting() {
        let plain = format_paste("hello\nworld", false);
        assert_eq!(plain, b"hello\rworld");

        let bracketed = format_paste("hello\nworld", true);
        assert_eq!(bracketed, b"\x1b[200~hello\nworld\x1b[201~");
    }

    const START: &[u8] = b"\x1b[200~";
    const END: &[u8] = b"\x1b[201~";

    fn bracketed(body: &[u8]) -> Vec<u8> {
        [START, body, END].concat()
    }

    #[test]
    fn bracketed_paste_keeps_ascii_japanese_emoji_and_newlines() {
        assert_eq!(format_paste("ls -la", true), bracketed(b"ls -la"));
        let japanese = "日本語　全角ＡＢＣ";
        assert_eq!(format_paste(japanese, true), bracketed(japanese.as_bytes()));
        let emoji = "🦀🌈 e\u{301}";
        assert_eq!(format_paste(emoji, true), bracketed(emoji.as_bytes()));
        assert_eq!(
            format_paste("a\nb\r\nc\td", true),
            bracketed(b"a\nb\r\nc\td")
        );
    }

    #[test]
    fn bracketed_paste_cannot_end_the_paste_early() {
        // The attack: end the paste, then type a command.
        assert_eq!(
            format_paste("safe\x1b[201~rm -rf ~\n", true),
            bracketed(b"safe[201~rm -rf ~\n")
        );
        // The stripped body holds no escape sequence at all.
        let out = format_paste("a\x1b[201~b\x1b[200~c", true);
        let inner = &out[START.len()..out.len() - END.len()];
        assert_eq!(inner, b"a[201~b[200~c");
        assert!(!inner.contains(&0x1b));
    }

    #[test]
    fn bracketed_paste_removes_every_escape_character() {
        assert_eq!(format_paste("\x1b[200~x", true), bracketed(b"[200~x"));
        // An end marker spread over several ESCs cannot be assembled either.
        assert_eq!(
            format_paste("\x1b\x1b[\x1b2\x1b0\x1b1\x1b~", true),
            bracketed(b"[201~")
        );
        assert_eq!(format_paste("\x1b", true), bracketed(b""));
        assert_eq!(format_paste("a\x1bb", true), bracketed(b"ab"));
        // ESC next to multibyte characters leaves them whole.
        assert_eq!(format_paste("あ\x1bい", true), bracketed("あい".as_bytes()));
    }

    #[test]
    fn bracketed_paste_of_nothing_is_just_the_markers() {
        assert_eq!(format_paste("", true), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn bracketed_paste_has_one_start_and_one_end_marker() {
        for text in [
            "",
            "x",
            "\x1b[200~a\x1b[201~",
            "\x1b[201~\x1b[201~",
            "あ\nい",
        ] {
            let out = format_paste(text, true);
            assert!(out.starts_with(START) && out.ends_with(END), "{text:?}");
            let count = |marker: &[u8]| out.windows(marker.len()).filter(|w| *w == marker).count();
            assert_eq!(count(START), 1, "{text:?}");
            assert_eq!(count(END), 1, "{text:?}");
        }
    }

    #[test]
    fn plain_paste_is_unchanged() {
        assert_eq!(format_paste("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(format_paste("日本語\n", false), "日本語\r".as_bytes());
        assert_eq!(format_paste("", false), b"");
        // Plain mode has no markers to protect; its escape characters are not part of this change.
        assert_eq!(format_paste("a\x1b[201~b", false), b"a\x1b[201~b");
    }

    #[test]
    fn former_side_bar_shortcuts_are_sent_to_the_terminal() {
        let action = process_key_event(
            code(key::Code::KeyE),
            &key::Key::Character("e".into()),
            Modifiers::COMMAND | Modifiers::SHIFT,
            None,
            false,
            false,
        );
        assert!(
            matches!(action, InputAction::SendToPty(_) | InputAction::Ignore),
            "{action:?}"
        );
    }

    #[test]
    fn arrow_keys_mapped_to_ansi() {
        let up = process_key_event(
            code(key::Code::ArrowUp),
            &key::Key::Named(key::Named::ArrowUp),
            Modifiers::NONE,
            None,
            false,
            false,
        );
        assert_eq!(up, InputAction::SendToPty(b"\x1b[A".to_vec()));
    }
}
