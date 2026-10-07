//! Stable internal identifiers for executable commands.

use std::fmt;
use std::str::FromStr;

/// Stable identifier for a command in Kegon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
    /// Focus the Explorer view in the side bar.
    WorkbenchExplorerFocus,
    /// Focus the Search view in the side bar.
    WorkbenchSearchFocus,
    /// Focus the Source Control (Git) view in the side bar.
    WorkbenchSourceControlFocus,
    /// Copy selection in the terminal to clipboard.
    TerminalCopy,
    /// Copy selection if present, otherwise send interrupt (SIGINT) to PTY.
    TerminalCopyOrInterrupt,
    /// Send interrupt (SIGINT) to PTY.
    TerminalInterrupt,
    /// Paste clipboard content into terminal.
    TerminalPaste,
    /// Confirm the active modal dialog primary action.
    DialogConfirm,
    /// Cancel or close the active modal dialog.
    DialogCancel,
    /// Open the Settings surface in the workbench.
    WorkbenchSettingsOpen,
}

impl CommandId {
    /// Returns the stable string identifier for this command.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WorkbenchExplorerFocus => "workbench.explorer.focus",
            Self::WorkbenchSearchFocus => "workbench.search.focus",
            Self::WorkbenchSourceControlFocus => "workbench.sourceControl.focus",
            Self::TerminalCopy => "terminal.copy",
            Self::TerminalCopyOrInterrupt => "terminal.copyOrInterrupt",
            Self::TerminalInterrupt => "terminal.interrupt",
            Self::TerminalPaste => "terminal.paste",
            Self::DialogConfirm => "dialog.confirm",
            Self::DialogCancel => "dialog.cancel",
            Self::WorkbenchSettingsOpen => "workbench.settings.open",
        }
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCommandIdError(pub String);

impl fmt::Display for ParseCommandIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown command identifier: {}", self.0)
    }
}

impl std::error::Error for ParseCommandIdError {}

impl FromStr for CommandId {
    type Err = ParseCommandIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "workbench.explorer.focus" => Ok(Self::WorkbenchExplorerFocus),
            "workbench.search.focus" => Ok(Self::WorkbenchSearchFocus),
            "workbench.sourceControl.focus" => Ok(Self::WorkbenchSourceControlFocus),
            "terminal.copy" => Ok(Self::TerminalCopy),
            "terminal.copyOrInterrupt" => Ok(Self::TerminalCopyOrInterrupt),
            "terminal.interrupt" => Ok(Self::TerminalInterrupt),
            "terminal.paste" => Ok(Self::TerminalPaste),
            "dialog.confirm" => Ok(Self::DialogConfirm),
            "dialog.cancel" => Ok(Self::DialogCancel),
            "workbench.settings.open" => Ok(Self::WorkbenchSettingsOpen),
            _ => Err(ParseCommandIdError(s.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_id_string_roundtrip() {
        let commands = [
            CommandId::WorkbenchExplorerFocus,
            CommandId::WorkbenchSearchFocus,
            CommandId::WorkbenchSourceControlFocus,
            CommandId::TerminalCopy,
            CommandId::TerminalCopyOrInterrupt,
            CommandId::TerminalInterrupt,
            CommandId::TerminalPaste,
            CommandId::DialogConfirm,
            CommandId::DialogCancel,
            CommandId::WorkbenchSettingsOpen,
        ];

        for cmd in commands {
            let s = cmd.as_str();
            let parsed: CommandId = s.parse().expect("should parse successfully");
            assert_eq!(cmd, parsed);
            assert_eq!(cmd.to_string(), s);
        }
    }

    #[test]
    fn invalid_command_id_returns_error() {
        assert!("invalid.command".parse::<CommandId>().is_err());
    }
}
