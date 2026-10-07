//! Platform-neutral key chord model and normalization.

use std::fmt;

/// Represents a single physical or logical key in a platform-neutral format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// Character key represented as a lower-case string (e.g., "e", "c", "v").
    Character(String),
    /// Standard named non-character keys.
    Named(NamedKey),
}

/// Named keys recognized by Kegon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Backspace,
    Tab,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
    Other,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Character(c) => write!(f, "{}", c.to_uppercase()),
            Key::Named(named) => write!(f, "{named:?}"),
        }
    }
}

/// Modifier keys state for a key chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_key: bool,
}

/// A platform-neutral key chord combining a key and modifier keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl KeyChord {
    #[allow(dead_code)]
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    pub fn ctrl_shift_char(c: char) -> Self {
        Self {
            key: Key::Character(c.to_lowercase().to_string()),
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
                super_key: false,
            },
        }
    }

    pub fn cmd_shift_char(c: char) -> Self {
        Self {
            key: Key::Character(c.to_lowercase().to_string()),
            modifiers: Modifiers {
                ctrl: false,
                shift: true,
                alt: false,
                super_key: true,
            },
        }
    }

    pub fn ctrl_char(c: char) -> Self {
        Self {
            key: Key::Character(c.to_lowercase().to_string()),
            modifiers: Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                super_key: false,
            },
        }
    }

    pub fn cmd_char(c: char) -> Self {
        Self {
            key: Key::Character(c.to_lowercase().to_string()),
            modifiers: Modifiers {
                ctrl: false,
                shift: false,
                alt: false,
                super_key: true,
            },
        }
    }

    /// Convert low-level iced key event to a domain [`KeyChord`].
    pub fn from_iced(
        logical_key: &iced::keyboard::key::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Option<Self> {
        let key = match logical_key {
            iced::keyboard::key::Key::Character(c) => {
                let s = c.to_lowercase();
                if s.is_empty() {
                    return None;
                }
                Key::Character(s)
            }
            iced::keyboard::key::Key::Named(named) => {
                let named_key = match named {
                    iced::keyboard::key::Named::Enter => NamedKey::Enter,
                    iced::keyboard::key::Named::Backspace => NamedKey::Backspace,
                    iced::keyboard::key::Named::Tab => NamedKey::Tab,
                    iced::keyboard::key::Named::Escape => NamedKey::Escape,
                    iced::keyboard::key::Named::ArrowUp => NamedKey::ArrowUp,
                    iced::keyboard::key::Named::ArrowDown => NamedKey::ArrowDown,
                    iced::keyboard::key::Named::ArrowLeft => NamedKey::ArrowLeft,
                    iced::keyboard::key::Named::ArrowRight => NamedKey::ArrowRight,
                    iced::keyboard::key::Named::Home => NamedKey::Home,
                    iced::keyboard::key::Named::End => NamedKey::End,
                    iced::keyboard::key::Named::PageUp => NamedKey::PageUp,
                    iced::keyboard::key::Named::PageDown => NamedKey::PageDown,
                    iced::keyboard::key::Named::Insert => NamedKey::Insert,
                    iced::keyboard::key::Named::Delete => NamedKey::Delete,
                    iced::keyboard::key::Named::F1 => NamedKey::F(1),
                    iced::keyboard::key::Named::F2 => NamedKey::F(2),
                    iced::keyboard::key::Named::F3 => NamedKey::F(3),
                    iced::keyboard::key::Named::F4 => NamedKey::F(4),
                    iced::keyboard::key::Named::F5 => NamedKey::F(5),
                    iced::keyboard::key::Named::F6 => NamedKey::F(6),
                    iced::keyboard::key::Named::F7 => NamedKey::F(7),
                    iced::keyboard::key::Named::F8 => NamedKey::F(8),
                    iced::keyboard::key::Named::F9 => NamedKey::F(9),
                    iced::keyboard::key::Named::F10 => NamedKey::F(10),
                    iced::keyboard::key::Named::F11 => NamedKey::F(11),
                    iced::keyboard::key::Named::F12 => NamedKey::F(12),
                    _ => NamedKey::Other,
                };
                Key::Named(named_key)
            }
            _ => return None,
        };

        let domain_modifiers = Modifiers {
            ctrl: modifiers.control(),
            shift: modifiers.shift(),
            alt: modifiers.alt(),
            super_key: modifiers.logo(),
        };

        Some(KeyChord {
            key,
            modifiers: domain_modifiers,
        })
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl");
        }
        if self.modifiers.super_key {
            parts.push("Cmd");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        let key_str = self.key.to_string();
        parts.push(&key_str);
        write!(f, "{}", parts.join("+"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_chord_display_formatting() {
        let ctrl_shift_e = KeyChord::ctrl_shift_char('e');
        assert_eq!(ctrl_shift_e.to_string(), "Ctrl+Shift+E");

        let cmd_c = KeyChord::cmd_char('c');
        assert_eq!(cmd_c.to_string(), "Cmd+C");
    }
}
