//! Command execution dispatcher.

use super::command_id::CommandId;
use crate::terminal::{SystemClipboard, TerminalSession, format_paste};
use crate::workbench::{ActivityItem, Workbench};

/// Outcome of attempting to dispatch a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    Executed,
    Ignored,
}

/// Dispatches executable commands to appropriate subsystem targets.
pub struct CommandDispatcher;

impl CommandDispatcher {
    /// Dispatch a command ID using application targets.
    pub fn dispatch(
        command_id: CommandId,
        workbench: &mut Workbench,
        terminal_session: Option<&TerminalSession>,
        system_clipboard: &mut SystemClipboard,
    ) -> CommandOutcome {
        match command_id {
            CommandId::WorkbenchExplorerFocus => {
                workbench.select_activity(ActivityItem::Explorer);
                CommandOutcome::Executed
            }
            CommandId::WorkbenchSearchFocus => {
                workbench.select_activity(ActivityItem::Search);
                CommandOutcome::Executed
            }
            CommandId::WorkbenchSourceControlFocus => {
                workbench.select_activity(ActivityItem::Git);
                CommandOutcome::Executed
            }
            CommandId::TerminalCopy => {
                if let Some(session) = terminal_session {
                    if let Some(selected_text) = session.copy_selection() {
                        system_clipboard.set_text(selected_text);
                        session.clear_selection();
                    }
                    CommandOutcome::Executed
                } else {
                    CommandOutcome::Ignored
                }
            }
            CommandId::TerminalInterrupt => {
                if let Some(session) = terminal_session {
                    session.write_input(vec![0x03]);
                    CommandOutcome::Executed
                } else {
                    CommandOutcome::Ignored
                }
            }
            CommandId::TerminalCopyOrInterrupt => {
                if let Some(session) = terminal_session {
                    if session.has_selection() {
                        if let Some(selected_text) = session.copy_selection() {
                            system_clipboard.set_text(selected_text);
                            session.clear_selection();
                        }
                    } else {
                        session.write_input(vec![0x03]);
                    }
                    CommandOutcome::Executed
                } else {
                    CommandOutcome::Ignored
                }
            }
            CommandId::TerminalPaste => {
                if let Some(session) = terminal_session {
                    if let Some(pasted_text) = system_clipboard.get_text() {
                        let formatted = format_paste(&pasted_text, session.is_bracketed_paste());
                        session.write_input(formatted);
                    }
                    CommandOutcome::Executed
                } else {
                    CommandOutcome::Ignored
                }
            }
            CommandId::WorkbenchSettingsOpen => {
                workbench.select_activity(ActivityItem::Settings);
                CommandOutcome::Executed
            }
            CommandId::DialogConfirm | CommandId::DialogCancel => CommandOutcome::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::channel::mpsc::unbounded;

    #[test]
    fn copy_or_interrupt_with_selection_copies_text() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        {
            let mut term = session.term().lock();
            use alacritty_terminal::vte::ansi::Processor;
            let mut parser: Processor = Processor::new();
            parser.advance(&mut *term, b"Copy Target Text\r\n");
        }

        session.start_selection(0, 0);
        session.update_selection(10, 0);
        assert!(session.has_selection());

        let mut workbench = Workbench::default();
        let mut clipboard = SystemClipboard::new();
        let outcome = CommandDispatcher::dispatch(
            CommandId::TerminalCopyOrInterrupt,
            &mut workbench,
            Some(&session),
            &mut clipboard,
        );

        assert_eq!(outcome, CommandOutcome::Executed);
        assert!(
            !session.has_selection(),
            "Selection should be cleared after copy"
        );
    }

    #[test]
    fn copy_or_interrupt_without_selection_does_not_panic() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        assert!(!session.has_selection());

        let mut workbench = Workbench::default();
        let mut clipboard = SystemClipboard::new();
        let outcome = CommandDispatcher::dispatch(
            CommandId::TerminalCopyOrInterrupt,
            &mut workbench,
            Some(&session),
            &mut clipboard,
        );

        assert_eq!(outcome, CommandOutcome::Executed);
    }
}
