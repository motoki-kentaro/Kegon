//! Application execution context for keybinding resolution.

/// Execution context scoping keybindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandContext {
    /// Global application level workbench context.
    Workbench,
    /// Terminal input focused context.
    TerminalFocused,
    /// Native modal dialog context.
    Modal,
}
