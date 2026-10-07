//! Translation of iced keyboard events into terminal escape sequences and actions.

use crate::workbench::ActivityItem;
use iced::keyboard::{Modifiers, key};

/// Result of processing a keyboard event in the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
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
pub fn process_key_event(
    physical_key: key::Physical,
    logical_key: &key::Key,
    modifiers: Modifiers,
    text: Option<&str>,
    has_selection: bool,
    app_cursor_keys: bool,
) -> InputAction {
    // 1. Check Workbench Activity shortcuts (Ctrl+Shift+E / F / G)
    if modifiers.command()
        && modifiers.shift()
        && !modifiers.alt()
        && let key::Physical::Code(code) = physical_key
    {
        match code {
            key::Code::KeyE => return InputAction::WorkbenchShortcut(ActivityItem::Explorer),
            key::Code::KeyF => return InputAction::WorkbenchShortcut(ActivityItem::Search),
            key::Code::KeyG => return InputAction::WorkbenchShortcut(ActivityItem::Git),
            _ => {}
        }
    }

    // 2. Check Ctrl+C / Ctrl+V policy
    if modifiers.command()
        && !modifiers.shift()
        && !modifiers.alt()
        && let key::Key::Character(c) = logical_key
    {
        let lower = c.to_lowercase();
        if lower == "c" {
            if has_selection {
                return InputAction::CopySelection;
            } else {
                return InputAction::SendToPty(vec![0x03]); // SIGINT / Ctrl+C
            }
        } else if lower == "v" {
            return InputAction::PasteFromClipboard;
        }
    }

    // 3. Handle Special Keys and Escape Sequences
    match logical_key {
        key::Key::Named(named) => {
            if let Some(bytes) = map_named_key(*named, modifiers, app_cursor_keys) {
                return InputAction::SendToPty(bytes);
            }
        }
        key::Key::Character(c) => {
            if modifiers.command() && !modifiers.shift() && !modifiers.alt() {
                // Ctrl + letter (A-Z) => Control codes \x01 - \x1a
                if let Some(first_char) = c.chars().next() {
                    let ascii = first_char.to_ascii_uppercase();
                    if ascii.is_ascii_uppercase() {
                        let ctrl_byte = (ascii as u8) - b'A' + 1;
                        return InputAction::SendToPty(vec![ctrl_byte]);
                    }
                }
            } else if modifiers.alt() && !modifiers.command() {
                // Alt + key => ESC prefix
                let mut bytes = vec![0x1b];
                bytes.extend_from_slice(c.as_bytes());
                return InputAction::SendToPty(bytes);
            }
        }
        _ => {}
    }

    // 4. Normal character input (from IME commit or standard typing)
    if let Some(txt) = text
        && !txt.is_empty()
        && !modifiers.command()
        && !modifiers.alt()
    {
        return InputAction::SendToPty(txt.as_bytes().to_vec());
    }

    InputAction::Ignore
}

fn map_named_key(
    named: key::Named,
    modifiers: Modifiers,
    app_cursor_keys: bool,
) -> Option<Vec<u8>> {
    let bytes = match named {
        key::Named::Enter => vec![b'\r'],
        key::Named::Backspace => vec![0x7f],
        key::Named::Tab => {
            if modifiers.shift() {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }
        }
        key::Named::Escape => vec![0x1b],
        key::Named::ArrowUp => {
            if app_cursor_keys {
                b"\x1bOA".to_vec()
            } else {
                b"\x1b[A".to_vec()
            }
        }
        key::Named::ArrowDown => {
            if app_cursor_keys {
                b"\x1bOB".to_vec()
            } else {
                b"\x1b[B".to_vec()
            }
        }
        key::Named::ArrowRight => {
            if app_cursor_keys {
                b"\x1bOC".to_vec()
            } else {
                b"\x1b[C".to_vec()
            }
        }
        key::Named::ArrowLeft => {
            if app_cursor_keys {
                b"\x1bOD".to_vec()
            } else {
                b"\x1b[D".to_vec()
            }
        }
        key::Named::Home => b"\x1b[H".to_vec(),
        key::Named::End => b"\x1b[F".to_vec(),
        key::Named::PageUp => b"\x1b[5~".to_vec(),
        key::Named::PageDown => b"\x1b[6~".to_vec(),
        key::Named::Insert => b"\x1b[2~".to_vec(),
        key::Named::Delete => b"\x1b[3~".to_vec(),
        key::Named::F1 => b"\x1bOP".to_vec(),
        key::Named::F2 => b"\x1bOQ".to_vec(),
        key::Named::F3 => b"\x1bOR".to_vec(),
        key::Named::F4 => b"\x1bOS".to_vec(),
        key::Named::F5 => b"\x1b[15~".to_vec(),
        key::Named::F6 => b"\x1b[17~".to_vec(),
        key::Named::F7 => b"\x1b[18~".to_vec(),
        key::Named::F8 => b"\x1b[19~".to_vec(),
        key::Named::F9 => b"\x1b[20~".to_vec(),
        key::Named::F10 => b"\x1b[21~".to_vec(),
        key::Named::F11 => b"\x1b[23~".to_vec(),
        key::Named::F12 => b"\x1b[24~".to_vec(),
        _ => return None,
    };

    Some(bytes)
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
