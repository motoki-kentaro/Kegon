//! Keybinding resolution and conflict validation.

use std::fmt;

use super::command_id::CommandId;
use super::context::CommandContext;
use super::key_chord::KeyChord;
use super::keymap::Keymap;
use super::platform::Platform;

/// Error returned when keymap validation detects conflicting keybindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeybindingConflictError {
    DuplicateBinding {
        context: CommandContext,
        chord: KeyChord,
        first: CommandId,
        second: CommandId,
    },
}

impl fmt::Display for KeybindingConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateBinding {
                context,
                chord,
                first,
                second,
            } => write!(
                f,
                "Conflict detected in context {context:?} for chord '{chord}': '{first}' conflicts with '{second}'"
            ),
        }
    }
}

impl std::error::Error for KeybindingConflictError {}

/// Resolves user key input into executable command identifiers based on platform and context.
#[derive(Debug, Clone)]
pub struct KeybindingResolver {
    #[allow(dead_code)]
    platform: Platform,
    keymap: Keymap,
}

impl KeybindingResolver {
    /// Construct a resolver for a specified platform and keymap after validating for conflicts.
    pub fn new(platform: Platform, keymap: Keymap) -> Result<Self, KeybindingConflictError> {
        let resolver = Self { platform, keymap };
        resolver.validate()?;
        Ok(resolver)
    }

    /// Construct default resolver for the specified platform.
    pub fn default_for_platform(platform: Platform) -> Self {
        let keymap = Keymap::default_for_platform(platform);
        Self::new(platform, keymap)
            .expect("default platform keymap must be valid without conflicts")
    }

    #[allow(dead_code)]
    pub fn platform(&self) -> Platform {
        self.platform
    }

    #[allow(dead_code)]
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Validates that no two rules in the same context bind to the same key chord.
    pub fn validate(&self) -> Result<(), KeybindingConflictError> {
        let rules = self.keymap.rules();
        for i in 0..rules.len() {
            for j in (i + 1)..rules.len() {
                let r1 = &rules[i];
                let r2 = &rules[j];
                if r1.context == r2.context && r1.chord == r2.chord {
                    return Err(KeybindingConflictError::DuplicateBinding {
                        context: r1.context,
                        chord: r1.chord.clone(),
                        first: r1.command_id,
                        second: r2.command_id,
                    });
                }
            }
        }
        Ok(())
    }

    /// Resolve a key chord in the active context, falling back to global Workbench context if needed.
    pub fn resolve(&self, active_context: CommandContext, chord: &KeyChord) -> Option<CommandId> {
        // 1. Check active context (e.g. TerminalFocused)
        if let Some(cmd) = self.keymap.find(active_context, chord) {
            return Some(cmd);
        }

        // 2. Fall back to global Workbench context if active_context is not Workbench
        if active_context != CommandContext::Workbench
            && let Some(cmd) = self.keymap.find(CommandContext::Workbench, chord)
        {
            return Some(cmd);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_windows_default_keymap() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);

        // Workbench context
        let ctrl_shift_e = KeyChord::ctrl_shift_char('e');
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &ctrl_shift_e),
            Some(CommandId::WorkbenchExplorerFocus)
        );

        let ctrl_shift_f = KeyChord::ctrl_shift_char('f');
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &ctrl_shift_f),
            Some(CommandId::WorkbenchSearchFocus)
        );

        let ctrl_shift_g = KeyChord::ctrl_shift_char('g');
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &ctrl_shift_g),
            Some(CommandId::WorkbenchSourceControlFocus)
        );

        // Terminal context
        let ctrl_c = KeyChord::ctrl_char('c');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &ctrl_c),
            Some(CommandId::TerminalCopyOrInterrupt)
        );

        let ctrl_v = KeyChord::ctrl_char('v');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &ctrl_v),
            Some(CommandId::TerminalPaste)
        );
    }

    #[test]
    fn resolve_macos_default_keymap() {
        let resolver = KeybindingResolver::default_for_platform(Platform::MacOS);

        let cmd_shift_e = KeyChord::cmd_shift_char('e');
        assert_eq!(
            resolver.resolve(CommandContext::Workbench, &cmd_shift_e),
            Some(CommandId::WorkbenchExplorerFocus)
        );

        let cmd_c = KeyChord::cmd_char('c');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &cmd_c),
            Some(CommandId::TerminalCopy)
        );

        let ctrl_c = KeyChord::ctrl_char('c');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &ctrl_c),
            Some(CommandId::TerminalInterrupt)
        );

        let cmd_v = KeyChord::cmd_char('v');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &cmd_v),
            Some(CommandId::TerminalPaste)
        );
    }

    #[test]
    fn resolve_terminal_focused_fallback_to_workbench() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);

        // Ctrl+Shift+E pressed when Terminal is focused falls back to Workbench context
        let ctrl_shift_e = KeyChord::ctrl_shift_char('e');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &ctrl_shift_e),
            Some(CommandId::WorkbenchExplorerFocus)
        );
    }

    #[test]
    fn resolve_unregistered_key_returns_none() {
        let resolver = KeybindingResolver::default_for_platform(Platform::Windows);
        let key_a = KeyChord::ctrl_char('a');
        assert_eq!(
            resolver.resolve(CommandContext::TerminalFocused, &key_a),
            None
        );
    }

    #[test]
    fn keymap_conflict_detection() {
        let mut keymap = Keymap::default();
        let ctrl_c = KeyChord::ctrl_char('c');

        keymap.add_rule(
            CommandContext::TerminalFocused,
            ctrl_c.clone(),
            CommandId::TerminalCopyOrInterrupt,
        );
        keymap.add_rule(
            CommandContext::TerminalFocused,
            ctrl_c.clone(),
            CommandId::TerminalInterrupt,
        );

        let result = KeybindingResolver::new(Platform::Windows, keymap);
        assert!(matches!(
            result,
            Err(KeybindingConflictError::DuplicateBinding {
                context: CommandContext::TerminalFocused,
                first: CommandId::TerminalCopyOrInterrupt,
                second: CommandId::TerminalInterrupt,
                ..
            })
        ));
    }
}
