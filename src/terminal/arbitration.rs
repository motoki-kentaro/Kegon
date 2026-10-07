//! Input arbitration defining ownership of keyboard and IME events.

use crate::command::{CommandContext, CommandId, KeyChord, KeybindingResolver};

/// Target ownership route for an input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputRoute {
    /// Input is owned by IME composition (preedit or commit).
    Ime,
    /// Input is owned by a Kegon application command.
    Command(CommandId),
    /// Input is owned by the Terminal / PTY child process.
    Terminal,
}

/// Arbitrates input events between IME, Kegon Commands, and Terminal input fallthrough.
#[derive(Debug, Clone)]
pub struct InputArbiter;

impl InputArbiter {
    /// Arbitrate a keyboard event given the active context, key event data, and keybinding resolver.
    pub fn arbitrate_key_event(
        resolver: &KeybindingResolver,
        context: CommandContext,
        logical_key: &iced::keyboard::key::Key,
        modifiers: iced::keyboard::Modifiers,
        is_ime_composing: bool,
    ) -> InputRoute {
        // 1. IME precedence: if active composition is ongoing, IME owns keyboard input
        if is_ime_composing {
            return InputRoute::Ime;
        }

        // 2. Kegon Command precedence: check if key maps to an opt-in Kegon command
        if let Some(chord) = KeyChord::from_iced(logical_key, modifiers)
            && let Some(command_id) = resolver.resolve(context, &chord)
        {
            return InputRoute::Command(command_id);
        }

        // 3. Terminal precedence: default owner for all other keyboard events
        InputRoute::Terminal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Platform;
    use iced::keyboard::{Modifiers, key};

    #[test]
    fn bound_kegon_command_routes_to_command() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);
        let route = InputArbiter::arbitrate_key_event(
            &resolver,
            CommandContext::Workbench,
            &key::Key::Character("e".into()),
            Modifiers::COMMAND | Modifiers::SHIFT,
            false,
        );
        assert_eq!(
            route,
            InputRoute::Command(CommandId::WorkbenchExplorerFocus)
        );
    }

    #[test]
    fn unbound_keys_route_to_terminal() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);

        // Ctrl+D
        let route_d = InputArbiter::arbitrate_key_event(
            &resolver,
            CommandContext::TerminalFocused,
            &key::Key::Character("d".into()),
            Modifiers::COMMAND,
            false,
        );
        assert_eq!(route_d, InputRoute::Terminal);

        // Shift+Enter
        let route_shift_enter = InputArbiter::arbitrate_key_event(
            &resolver,
            CommandContext::TerminalFocused,
            &key::Key::Named(key::Named::Enter),
            Modifiers::SHIFT,
            false,
        );
        assert_eq!(route_shift_enter, InputRoute::Terminal);
    }

    #[test]
    fn ime_composing_takes_precedence() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);
        let route = InputArbiter::arbitrate_key_event(
            &resolver,
            CommandContext::TerminalFocused,
            &key::Key::Character("e".into()),
            Modifiers::COMMAND | Modifiers::SHIFT,
            true, // IME active
        );
        assert_eq!(route, InputRoute::Ime);
    }
}
