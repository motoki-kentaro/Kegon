//! Low-level terminal input event model.

use iced::keyboard::{Modifiers, key};

/// The phase or action of a keyboard event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyEventKind {
    Press,
    #[allow(dead_code)]
    Repeat,
    Release,
}

/// A low-level keyboard input event passed to the terminal key encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInputEvent {
    pub logical_key: key::Key,
    pub physical_key: key::Physical,
    pub modifiers: Modifiers,
    pub text: Option<String>,
    pub kind: KeyEventKind,
}

impl TerminalInputEvent {
    pub fn new(
        logical_key: key::Key,
        physical_key: key::Physical,
        modifiers: Modifiers,
        text: Option<String>,
        kind: KeyEventKind,
    ) -> Self {
        Self {
            logical_key,
            physical_key,
            modifiers,
            text,
            kind,
        }
    }

    pub fn press(
        logical_key: key::Key,
        physical_key: key::Physical,
        modifiers: Modifiers,
        text: Option<String>,
    ) -> Self {
        Self::new(
            logical_key,
            physical_key,
            modifiers,
            text,
            KeyEventKind::Press,
        )
    }

    pub fn release(
        logical_key: key::Key,
        physical_key: key::Physical,
        modifiers: Modifiers,
    ) -> Self {
        Self::new(
            logical_key,
            physical_key,
            modifiers,
            None,
            KeyEventKind::Release,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_input_event_constructors() {
        let press = TerminalInputEvent::press(
            key::Key::Named(key::Named::Enter),
            key::Physical::Code(key::Code::Enter),
            Modifiers::NONE,
            None,
        );
        assert_eq!(press.kind, KeyEventKind::Press);
        assert_eq!(press.logical_key, key::Key::Named(key::Named::Enter));

        let release = TerminalInputEvent::release(
            key::Key::Named(key::Named::Enter),
            key::Physical::Code(key::Code::Enter),
            Modifiers::NONE,
        );
        assert_eq!(release.kind, KeyEventKind::Release);
    }
}
