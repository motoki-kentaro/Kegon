//! Encodes terminal input events into ANSI / CSI escape sequences and bytes.

use crate::terminal::input_event::{KeyEventKind, TerminalInputEvent};
use crate::terminal::mode::TerminalKeyboardMode;
use iced::keyboard::{Modifiers, key};

/// Key encoder for converting terminal input events into PTY bytes.
pub struct TerminalKeyEncoder;

impl TerminalKeyEncoder {
    /// Encode a terminal input event given the current terminal keyboard mode.
    pub fn encode(event: &TerminalInputEvent, mode: TerminalKeyboardMode) -> Option<Vec<u8>> {
        // If event is a release and report_event_types is off, do not send release bytes
        if event.kind == KeyEventKind::Release && !mode.report_event_types {
            return None;
        }

        if mode.is_extended() {
            Self::encode_extended(event, mode)
        } else {
            Self::encode_legacy(event, mode.app_cursor)
        }
    }

    fn encode_legacy(event: &TerminalInputEvent, app_cursor: bool) -> Option<Vec<u8>> {
        if event.kind == KeyEventKind::Release {
            return None;
        }

        match &event.logical_key {
            key::Key::Named(named) => map_named_key_legacy(*named, event.modifiers, app_cursor),
            key::Key::Character(c) => {
                if event.modifiers.control() && !event.modifiers.shift() && !event.modifiers.alt() {
                    if let Some(first_char) = c.chars().next() {
                        let ascii = first_char.to_ascii_uppercase();
                        if ascii.is_ascii_uppercase() {
                            let ctrl_byte = (ascii as u8) - b'A' + 1;
                            return Some(vec![ctrl_byte]);
                        }
                    }
                } else if event.modifiers.alt() && !event.modifiers.control() {
                    let mut bytes = vec![0x1b];
                    bytes.extend_from_slice(c.as_bytes());
                    return Some(bytes);
                }
                if let Some(txt) = &event.text
                    && !txt.is_empty()
                    && !event.modifiers.control()
                    && !event.modifiers.alt()
                {
                    return Some(txt.as_bytes().to_vec());
                }
                None
            }
            _ => {
                if let Some(txt) = &event.text
                    && !txt.is_empty()
                    && !event.modifiers.control()
                    && !event.modifiers.alt()
                {
                    return Some(txt.as_bytes().to_vec());
                }
                None
            }
        }
    }

    fn encode_extended(event: &TerminalInputEvent, mode: TerminalKeyboardMode) -> Option<Vec<u8>> {
        let mod_param = modifier_param(event.modifiers);
        let event_suffix = event_type_suffix(event.kind, mode.report_event_types);

        match &event.logical_key {
            key::Key::Named(named) => match named {
                key::Named::Enter => {
                    if has_modifiers(event.modifiers) || mode.report_all_keys_as_esc {
                        Some(format!("\x1b[13;{mod_param}{event_suffix}u").into_bytes())
                    } else {
                        Some(vec![b'\r'])
                    }
                }
                key::Named::Tab => {
                    if has_modifiers(event.modifiers) || mode.report_all_keys_as_esc {
                        Some(format!("\x1b[9;{mod_param}{event_suffix}u").into_bytes())
                    } else {
                        Some(vec![b'\t'])
                    }
                }
                key::Named::Backspace => {
                    if has_modifiers(event.modifiers) || mode.report_all_keys_as_esc {
                        Some(format!("\x1b[127;{mod_param}{event_suffix}u").into_bytes())
                    } else {
                        Some(vec![0x7f])
                    }
                }
                key::Named::Escape => {
                    if has_modifiers(event.modifiers) || mode.report_all_keys_as_esc {
                        Some(format!("\x1b[27;{mod_param}{event_suffix}u").into_bytes())
                    } else {
                        Some(vec![0x1b])
                    }
                }
                key::Named::ArrowUp => {
                    encode_arrow_extended("A", mod_param, event_suffix, mode.app_cursor)
                }
                key::Named::ArrowDown => {
                    encode_arrow_extended("B", mod_param, event_suffix, mode.app_cursor)
                }
                key::Named::ArrowRight => {
                    encode_arrow_extended("C", mod_param, event_suffix, mode.app_cursor)
                }
                key::Named::ArrowLeft => {
                    encode_arrow_extended("D", mod_param, event_suffix, mode.app_cursor)
                }
                key::Named::Home => Some(format!("\x1b[1;{mod_param}{event_suffix}H").into_bytes()),
                key::Named::End => Some(format!("\x1b[1;{mod_param}{event_suffix}F").into_bytes()),
                key::Named::PageUp => {
                    Some(format!("\x1b[5;{mod_param}{event_suffix}~").into_bytes())
                }
                key::Named::PageDown => {
                    Some(format!("\x1b[6;{mod_param}{event_suffix}~").into_bytes())
                }
                key::Named::Insert => {
                    Some(format!("\x1b[2;{mod_param}{event_suffix}~").into_bytes())
                }
                key::Named::Delete => {
                    Some(format!("\x1b[3;{mod_param}{event_suffix}~").into_bytes())
                }
                key::Named::F1 => Some(format!("\x1b[1;{mod_param}{event_suffix}P").into_bytes()),
                key::Named::F2 => Some(format!("\x1b[1;{mod_param}{event_suffix}Q").into_bytes()),
                key::Named::F3 => Some(format!("\x1b[1;{mod_param}{event_suffix}R").into_bytes()),
                key::Named::F4 => Some(format!("\x1b[1;{mod_param}{event_suffix}S").into_bytes()),
                key::Named::F5 => Some(format!("\x1b[15;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F6 => Some(format!("\x1b[17;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F7 => Some(format!("\x1b[18;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F8 => Some(format!("\x1b[19;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F9 => Some(format!("\x1b[20;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F10 => Some(format!("\x1b[21;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F11 => Some(format!("\x1b[23;{mod_param}{event_suffix}~").into_bytes()),
                key::Named::F12 => Some(format!("\x1b[24;{mod_param}{event_suffix}~").into_bytes()),
                _ => None,
            },
            key::Key::Character(c) => {
                if (mode.report_all_keys_as_esc
                    || event.modifiers.control()
                    || event.modifiers.alt())
                    && let Some(ch) = c.chars().next()
                {
                    let codepoint = ch as u32;
                    return Some(
                        format!("\x1b[{codepoint};{mod_param}{event_suffix}u").into_bytes(),
                    );
                }
                if let Some(txt) = &event.text
                    && !txt.is_empty()
                {
                    return Some(txt.as_bytes().to_vec());
                }
                None
            }
            _ => {
                if let Some(txt) = &event.text
                    && !txt.is_empty()
                {
                    return Some(txt.as_bytes().to_vec());
                }
                None
            }
        }
    }
}

fn modifier_param(modifiers: Modifiers) -> u8 {
    let mut param = 1u8;
    if modifiers.shift() {
        param += 1;
    }
    if modifiers.alt() {
        param += 2;
    }
    if modifiers.control() {
        param += 4;
    }
    if modifiers.logo() {
        param += 8;
    }
    param
}

fn has_modifiers(modifiers: Modifiers) -> bool {
    modifiers.shift() || modifiers.alt() || modifiers.control() || modifiers.logo()
}

fn event_type_suffix(kind: KeyEventKind, report_event_types: bool) -> String {
    if report_event_types {
        let code = match kind {
            KeyEventKind::Press => 1,
            KeyEventKind::Repeat => 2,
            KeyEventKind::Release => 3,
        };
        format!(":{code}")
    } else {
        String::new()
    }
}

fn encode_arrow_extended(
    dir: &str,
    mod_param: u8,
    event_suffix: String,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    if mod_param > 1 || !event_suffix.is_empty() {
        Some(format!("\x1b[1;{mod_param}{event_suffix}{dir}").into_bytes())
    } else if app_cursor {
        Some(format!("\x1bO{dir}").into_bytes())
    } else {
        Some(format!("\x1b[{dir}").into_bytes())
    }
}

fn map_named_key_legacy(
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

    fn press_event(
        logical: key::Key,
        physical: key::Code,
        modifiers: Modifiers,
        text: Option<&str>,
    ) -> TerminalInputEvent {
        TerminalInputEvent::press(
            logical,
            key::Physical::Code(physical),
            modifiers,
            text.map(String::from),
        )
    }

    #[test]
    fn legacy_encoding_enter_backspace_tab() {
        let mode = TerminalKeyboardMode::default();

        let enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::NONE,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&enter, mode),
            Some(b"\r".to_vec())
        );

        let shift_enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::SHIFT,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&shift_enter, mode),
            Some(b"\r".to_vec())
        );

        let ctrl_enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::COMMAND,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&ctrl_enter, mode),
            Some(b"\r".to_vec())
        );

        let backspace = press_event(
            key::Key::Named(key::Named::Backspace),
            key::Code::Backspace,
            Modifiers::NONE,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&backspace, mode),
            Some(vec![0x7f])
        );

        let tab = press_event(
            key::Key::Named(key::Named::Tab),
            key::Code::Tab,
            Modifiers::NONE,
            None,
        );
        assert_eq!(TerminalKeyEncoder::encode(&tab, mode), Some(vec![b'\t']));

        let shift_tab = press_event(
            key::Key::Named(key::Named::Tab),
            key::Code::Tab,
            Modifiers::SHIFT,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&shift_tab, mode),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn legacy_encoding_ctrl_and_alt() {
        let mode = TerminalKeyboardMode::default();

        let ctrl_d = press_event(
            key::Key::Character("d".into()),
            key::Code::KeyD,
            Modifiers::COMMAND,
            Some("d"),
        );
        assert_eq!(TerminalKeyEncoder::encode(&ctrl_d, mode), Some(vec![0x04]));

        let alt_x = press_event(
            key::Key::Character("x".into()),
            key::Code::KeyX,
            Modifiers::ALT,
            Some("x"),
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&alt_x, mode),
            Some(b"\x1bx".to_vec())
        );
    }

    #[test]
    fn extended_encoding_enter_variants() {
        let mode = TerminalKeyboardMode {
            disambiguate_esc_codes: true,
            ..Default::default()
        };

        let enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::NONE,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&enter, mode),
            Some(b"\r".to_vec())
        );

        let shift_enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::SHIFT,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&shift_enter, mode),
            Some(b"\x1b[13;2u".to_vec())
        );

        let alt_enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::ALT,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&alt_enter, mode),
            Some(b"\x1b[13;3u".to_vec())
        );

        let ctrl_enter = press_event(
            key::Key::Named(key::Named::Enter),
            key::Code::Enter,
            Modifiers::COMMAND,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&ctrl_enter, mode),
            Some(b"\x1b[13;5u".to_vec())
        );
    }

    #[test]
    fn extended_encoding_event_types() {
        let mode = TerminalKeyboardMode {
            disambiguate_esc_codes: true,
            report_event_types: true,
            ..Default::default()
        };

        let press = TerminalInputEvent::press(
            key::Key::Named(key::Named::Enter),
            key::Physical::Code(key::Code::Enter),
            Modifiers::SHIFT,
            None,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&press, mode),
            Some(b"\x1b[13;2:1u".to_vec())
        );

        let repeat = TerminalInputEvent::new(
            key::Key::Named(key::Named::Enter),
            key::Physical::Code(key::Code::Enter),
            Modifiers::SHIFT,
            None,
            KeyEventKind::Repeat,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&repeat, mode),
            Some(b"\x1b[13;2:2u".to_vec())
        );

        let release = TerminalInputEvent::release(
            key::Key::Named(key::Named::Enter),
            key::Physical::Code(key::Code::Enter),
            Modifiers::SHIFT,
        );
        assert_eq!(
            TerminalKeyEncoder::encode(&release, mode),
            Some(b"\x1b[13;2:3u".to_vec())
        );
    }
}
