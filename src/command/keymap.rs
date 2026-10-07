//! Keymap data structures and default platform configurations.

use super::command_id::CommandId;
use super::context::CommandContext;
use super::key_chord::KeyChord;
use super::platform::Platform;

/// A single binding rule mapping a context and key chord to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingRule {
    pub context: CommandContext,
    pub chord: KeyChord,
    pub command_id: CommandId,
}

/// A collection of keybinding rules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keymap {
    rules: Vec<KeybindingRule>,
}

impl Keymap {
    #[allow(dead_code)]
    pub fn new(rules: Vec<KeybindingRule>) -> Self {
        Self { rules }
    }

    pub fn rules(&self) -> &[KeybindingRule] {
        &self.rules
    }

    pub fn add_rule(&mut self, context: CommandContext, chord: KeyChord, command_id: CommandId) {
        self.rules.push(KeybindingRule {
            context,
            chord,
            command_id,
        });
    }

    pub fn find(&self, context: CommandContext, chord: &KeyChord) -> Option<CommandId> {
        for rule in &self.rules {
            if rule.context == context && &rule.chord == chord {
                return Some(rule.command_id);
            }
        }
        None
    }

    /// Construct default keymap for a given platform.
    pub fn default_for_platform(platform: Platform) -> Self {
        let mut keymap = Self::default();

        match platform {
            Platform::Windows | Platform::Linux => {
                // Workbench activity focus shortcuts
                keymap.add_rule(
                    CommandContext::Workbench,
                    KeyChord::ctrl_shift_char('e'),
                    CommandId::WorkbenchExplorerFocus,
                );
                keymap.add_rule(
                    CommandContext::Workbench,
                    KeyChord::ctrl_shift_char('f'),
                    CommandId::WorkbenchSearchFocus,
                );
                keymap.add_rule(
                    CommandContext::Workbench,
                    KeyChord::ctrl_shift_char('g'),
                    CommandId::WorkbenchSourceControlFocus,
                );

                // Terminal shortcuts
                keymap.add_rule(
                    CommandContext::TerminalFocused,
                    KeyChord::ctrl_char('c'),
                    CommandId::TerminalCopyOrInterrupt,
                );
                keymap.add_rule(
                    CommandContext::TerminalFocused,
                    KeyChord::ctrl_char('v'),
                    CommandId::TerminalPaste,
                );
            }
            Platform::MacOS => {
                // Workbench activity focus shortcuts
                keymap.add_rule(
                    CommandContext::Workbench,
                    KeyChord::cmd_shift_char('e'),
                    CommandId::WorkbenchExplorerFocus,
                );
                keymap.add_rule(
                    CommandContext::Workbench,
                    KeyChord::cmd_shift_char('f'),
                    CommandId::WorkbenchSearchFocus,
                );
                keymap.add_rule(
                    CommandContext::Workbench,
                    KeyChord::cmd_shift_char('g'),
                    CommandId::WorkbenchSourceControlFocus,
                );

                // Terminal shortcuts
                keymap.add_rule(
                    CommandContext::TerminalFocused,
                    KeyChord::cmd_char('c'),
                    CommandId::TerminalCopy,
                );
                keymap.add_rule(
                    CommandContext::TerminalFocused,
                    KeyChord::cmd_char('v'),
                    CommandId::TerminalPaste,
                );
                keymap.add_rule(
                    CommandContext::TerminalFocused,
                    KeyChord::ctrl_char('c'),
                    CommandId::TerminalInterrupt,
                );
            }
        }

        keymap
    }
}
