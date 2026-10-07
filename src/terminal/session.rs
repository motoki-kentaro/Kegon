//! Encapsulates a terminal session, PTY lifecycle, and alacritty_terminal emulation state.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty::{self, Options, Shell};

use crate::terminal::shell::ShellConfig;

/// State of a terminal session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Exited(Option<i32>),
}

/// Events emitted by the terminal session to notify the UI loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Terminal buffer state updated; re-render needed.
    Wakeup,
    /// Terminal window title changed by escape sequence.
    Title(String),
    /// Child process exited.
    ChildExit(Option<i32>),
}

#[derive(Clone)]
pub(crate) struct EventProxy {
    event_tx: Sender<TerminalEvent>,
    title: Arc<Mutex<String>>,
    state: Arc<Mutex<SessionState>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacrittyEvent) {
        match event {
            AlacrittyEvent::Wakeup => {
                let _ = self.event_tx.send(TerminalEvent::Wakeup);
            }
            AlacrittyEvent::Title(t) => {
                if let Ok(mut title_guard) = self.title.lock() {
                    *title_guard = t.clone();
                }
                let _ = self.event_tx.send(TerminalEvent::Title(t));
            }
            AlacrittyEvent::ChildExit(status) => {
                let code = status.code();
                if let Ok(mut state_guard) = self.state.lock() {
                    *state_guard = SessionState::Exited(code);
                }
                let _ = self.event_tx.send(TerminalEvent::ChildExit(code));
                let _ = self.event_tx.send(TerminalEvent::Wakeup);
            }
            _ => {}
        }
    }
}

/// Grid size dimensions for alacritty_terminal initialization.
#[derive(Clone, Copy, Debug)]
pub struct TermSize {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// A terminal session managing the child process, PTY, and emulator state.
pub struct TerminalSession {
    term: Arc<FairMutex<Term<EventProxy>>>,
    event_loop_sender: EventLoopSender,
    #[allow(dead_code)]
    title: Arc<Mutex<String>>,
    state: Arc<Mutex<SessionState>>,
}

impl TerminalSession {
    /// Spawns a new terminal session with the default shell.
    pub fn spawn(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        event_tx: Sender<TerminalEvent>,
    ) -> Result<Self, String> {
        let shell_cfg = ShellConfig::resolve_default();
        let options = Options {
            drain_on_exit: true,
            shell: Some(Shell::new(shell_cfg.program, shell_cfg.args)),
            working_directory: None,
            env: HashMap::new(),
            escape_args: false,
        };

        let window_size = WindowSize {
            num_lines: rows.max(1),
            num_cols: cols.max(1),
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
        };

        let pty =
            tty::new(&options, window_size, 0).map_err(|e| format!("Failed to spawn PTY: {e}"))?;

        let title = Arc::new(Mutex::new(String::from("Terminal")));
        let state = Arc::new(Mutex::new(SessionState::Running));

        let proxy = EventProxy {
            event_tx,
            title: title.clone(),
            state: state.clone(),
        };

        let term_size = TermSize {
            cols: cols.max(1) as usize,
            lines: rows.max(1) as usize,
        };

        let config = Config::default();
        let term = Term::new(config, &term_size, proxy.clone());
        let term = Arc::new(FairMutex::new(term));

        let event_loop = EventLoop::new(term.clone(), proxy, pty, false, false)
            .map_err(|e| format!("Failed to create event loop: {e}"))?;

        let event_loop_sender = event_loop.channel();
        let _join_handle = event_loop.spawn();

        Ok(Self {
            term,
            event_loop_sender,
            title,
            state,
        })
    }

    /// Access the underlying locked terminal for rendering and state queries.
    pub fn term(&self) -> &Arc<FairMutex<Term<EventProxy>>> {
        &self.term
    }

    /// Sends input bytes to the PTY.
    pub fn write_input(&self, input: impl Into<Cow<'static, [u8]>>) {
        if self.is_exited() {
            return;
        }
        let _ = self.event_loop_sender.send(Msg::Input(input.into()));
    }

    /// Resizes the terminal grid and PTY.
    pub fn resize(&self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let cell_width = cell_width.max(1);
        let cell_height = cell_height.max(1);

        let window_size = WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width,
            cell_height,
        };

        let _ = self.event_loop_sender.send(Msg::Resize(window_size));
    }

    /// Whether the child process has exited.
    pub fn is_exited(&self) -> bool {
        matches!(*self.state.lock().unwrap(), SessionState::Exited(_))
    }

    /// The exit code if the child process has exited.
    pub fn exit_code(&self) -> Option<i32> {
        if let SessionState::Exited(code) = *self.state.lock().unwrap() {
            code
        } else {
            None
        }
    }

    /// Current window title set by the terminal.
    #[allow(dead_code)]
    pub fn title(&self) -> String {
        self.title.lock().unwrap().clone()
    }

    /// Whether there is an active text selection in the terminal.
    pub fn has_selection(&self) -> bool {
        let term = self.term.lock();
        term.selection.is_some()
    }

    /// Copies selected text from the terminal grid, if any selection exists.
    pub fn copy_selection(&self) -> Option<String> {
        let term = self.term.lock();
        term.selection_to_string()
    }

    /// Clears any active selection in the terminal.
    pub fn clear_selection(&self) {
        let mut term = self.term.lock();
        term.selection = None;
    }

    /// Starts a simple mouse drag selection at the specified grid cell.
    pub fn start_selection(&self, col: usize, line: usize) {
        use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = self.term.lock();
        let point = TermPoint::new(Line(line as i32), Column(col));
        term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
    }

    /// Extends the active mouse drag selection to the specified grid cell.
    pub fn update_selection(&self, col: usize, line: usize) {
        use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
        let mut term = self.term.lock();
        if let Some(selection) = term.selection.as_mut() {
            let point = TermPoint::new(Line(line as i32), Column(col));
            selection.update(point, Side::Right);
        }
    }

    /// Whether bracketed paste mode is active in the child application.
    pub fn is_bracketed_paste(&self) -> bool {
        let term = self.term.lock();
        term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Whether application cursor keys mode (`DECCKM`) is active.
    #[allow(dead_code)]
    pub fn is_app_cursor_keys(&self) -> bool {
        let term = self.term.lock();
        term.mode().contains(TermMode::APP_CURSOR)
    }

    /// Terminal keyboard mode flags derived from alacritty_terminal state.
    pub fn keyboard_mode(&self) -> crate::terminal::mode::TerminalKeyboardMode {
        let term = self.term.lock();
        let mode = term.mode();
        crate::terminal::mode::TerminalKeyboardMode {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            disambiguate_esc_codes: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
            report_event_types: mode.contains(TermMode::REPORT_EVENT_TYPES),
            report_alternate_keys: mode.contains(TermMode::REPORT_ALTERNATE_KEYS),
            report_all_keys_as_esc: mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC),
            report_associated_text: mode.contains(TermMode::REPORT_ASSOCIATED_TEXT),
        }
    }

    /// Scroll display offset up or down by `delta` lines.
    pub fn scroll_display(&self, delta: i32) {
        use alacritty_terminal::grid::Scroll;
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Delta(delta));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn session_spawns_and_reports_initial_state() {
        let (tx, _rx) = channel();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx);
        assert!(session.is_ok());
        let session = session.unwrap();
        assert!(!session.is_exited());
        assert_eq!(session.exit_code(), None);
        assert!(!session.title().is_empty());
    }

    #[test]
    fn selection_lifecycle_starts_updates_and_clears() {
        let (tx, _rx) = channel();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();

        assert!(!session.has_selection());
        session.start_selection(0, 0);
        session.update_selection(5, 0);
        assert!(session.has_selection());

        session.clear_selection();
        assert!(!session.has_selection());
    }
}
