use crate::command::{CommandContext, CommandId, KeybindingResolver, Platform};
use crate::terminal::arbitration::{InputArbiter, InputRoute};
use crate::terminal::input_event::TerminalInputEvent;
use crate::terminal::key_encoder::TerminalKeyEncoder;
use crate::terminal::mode::TerminalKeyboardMode;
use crate::workbench::ActivityItem;
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
    /// Intercepted application shortcut (e.g. Ctrl+Shift+E).
    WorkbenchShortcut(ActivityItem),
    /// Ignore event (not handled by terminal).
    Ignore,
}

/// Formats text for pasting into the terminal based on bracketed paste mode.
pub fn format_paste(text: &str, bracketed_paste: bool) -> Vec<u8> {
    if bracketed_paste {
        let mut buf = Vec::with_capacity(text.len() + 12);
        buf.extend_from_slice(b"\x1b[200~");
        buf.extend_from_slice(text.as_bytes());
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
            CommandId::WorkbenchExplorerFocus => {
                InputAction::WorkbenchShortcut(ActivityItem::Explorer)
            }
            CommandId::WorkbenchSearchFocus => InputAction::WorkbenchShortcut(ActivityItem::Search),
            CommandId::WorkbenchSourceControlFocus => {
                InputAction::WorkbenchShortcut(ActivityItem::Git)
            }
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

    #[test]
    fn activity_shortcuts_are_intercepted() {
        let action = process_key_event(
            code(key::Code::KeyE),
            &key::Key::Character("e".into()),
            Modifiers::COMMAND | Modifiers::SHIFT,
            None,
            false,
            false,
        );
        assert_eq!(
            action,
            InputAction::WorkbenchShortcut(ActivityItem::Explorer)
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
