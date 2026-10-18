//! Encapsulates a terminal session, PTY lifecycle, and alacritty_terminal emulation state.

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use iced::futures::channel::mpsc::UnboundedSender;

use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite, Options, Shell};

use crate::terminal::cwd::local_host_name;
use crate::terminal::integration::Osc133Marker;
use crate::terminal::osc::{OscEvent, OscScanner};
use crate::terminal::shell::Launch;

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
    /// Child set a title (OSC 0 / OSC 2); raw, normalized by the UI before display.
    Title(String),
    /// Child restored an unset title (`CSI 23 t` popping a title pushed before any was set);
    /// show the default.
    ResetTitle,
    /// Child process exited.
    ChildExit(Option<i32>),
    /// The shell reported its current local working directory (Shell Integration).
    WorkingDirectory(PathBuf),
    /// Raw OSC 133 shell integration marker.
    ShellIntegration(Osc133Marker),
}

/// The PTY with a tap on its output: every read is scanned for working-directory reports
/// and OSC 133 shell integration markers.
///
/// `alacritty_terminal` ignores OSC 7, 9;9, and 133, and the event loop owns the only read path,
/// so the tap sits between the loop and the real PTY. Bytes are passed through unchanged.
struct TappedPty<P> {
    inner: P,
    scanner: OscScanner,
    sink: EventSink,
}

impl<P: EventedReadWrite> io::Read for TappedPty<P> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = io::Read::read(self.inner.reader(), buf)?;
        let events = self.scanner.scan(&buf[..read]);
        let mut last_cwd = None;
        for event in events {
            match event {
                OscEvent::WorkingDirectory(path) => {
                    last_cwd = Some(path);
                }
                OscEvent::ShellIntegration(marker) => {
                    let _ = self
                        .sink
                        .unbounded_send(TerminalEvent::ShellIntegration(marker));
                }
            }
        }
        // Only the last working directory report of a read matters: it supersedes the others,
        // and forwarding just one keeps a flood of reports from flooding the event channel.
        if let Some(path) = last_cwd {
            let _ = self
                .sink
                .unbounded_send(TerminalEvent::WorkingDirectory(path));
        }
        Ok(read)
    }
}

impl<P: EventedReadWrite> EventedReadWrite for TappedPty<P> {
    type Reader = Self;
    type Writer = P::Writer;

    unsafe fn register(
        &mut self,
        poller: &Arc<polling::Poller>,
        interest: polling::Event,
        mode: polling::PollMode,
    ) -> io::Result<()> {
        unsafe { self.inner.register(poller, interest, mode) }
    }

    fn reregister(
        &mut self,
        poller: &Arc<polling::Poller>,
        interest: polling::Event,
        mode: polling::PollMode,
    ) -> io::Result<()> {
        self.inner.reregister(poller, interest, mode)
    }

    fn deregister(&mut self, poller: &Arc<polling::Poller>) -> io::Result<()> {
        self.inner.deregister(poller)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        self
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl<P: EventedPty> EventedPty for TappedPty<P> {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl<P: OnResize> OnResize for TappedPty<P> {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size);
    }
}

/// Destination for terminal-generated responses (DA, DSR, DECRQM, XTWINOPS replies).
enum PtySink {
    EventLoop(EventLoopSender),
    #[cfg(test)]
    Channel(std::sync::mpsc::Sender<Vec<u8>>),
}

impl PtySink {
    fn write(&self, bytes: Vec<u8>) {
        // Errors only occur once the event loop has shut down, when the child no longer
        // reads replies anyway.
        match self {
            PtySink::EventLoop(sender) => {
                let _ = sender.send(Msg::Input(bytes.into()));
            }
            #[cfg(test)]
            PtySink::Channel(tx) => {
                let _ = tx.send(bytes);
            }
        }
    }
}

/// Where a session delivers its [`TerminalEvent`]s.
///
/// Type-erased so an owner can tag each event with the identity of the session that produced it
/// (see [`EventSink::from_fn`]) without the session knowing about that identity.
#[derive(Clone)]
pub struct EventSink(Arc<dyn Fn(TerminalEvent) + Send + Sync>);

impl EventSink {
    /// Delivers each event to `deliver`, which runs on the PTY event loop thread.
    pub fn from_fn(deliver: impl Fn(TerminalEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(deliver))
    }

    fn unbounded_send(&self, event: TerminalEvent) -> Result<(), ()> {
        (self.0)(event);
        Ok(())
    }
}

impl From<UnboundedSender<TerminalEvent>> for EventSink {
    fn from(tx: UnboundedSender<TerminalEvent>) -> Self {
        Self::from_fn(move |event| {
            // Fails only once the receiver is gone, when nobody is left to notify.
            let _ = tx.unbounded_send(event);
        })
    }
}

#[derive(Clone)]
pub(crate) struct EventProxy {
    event_tx: EventSink,
    state: Arc<Mutex<SessionState>>,
    // The proxy must exist before `EventLoop::new`, which is what creates the sender, so the
    // sink is injected once the loop is built and before it is spawned.
    pty_sink: Arc<OnceLock<PtySink>>,
    window_size: Arc<Mutex<WindowSize>>,
}

impl EventProxy {
    /// Writes a terminal-generated reply straight to the PTY event loop.
    ///
    /// Replies bypass the iced subscription: a child that blocks on a query must not wait for
    /// a UI frame, and the event loop queues the bytes instead of re-entering the parser.
    fn write_to_pty(&self, text: String) {
        // Matches upstream `Notifier`: writing zero bytes can hang the PTY.
        if text.is_empty() {
            return;
        }
        if let Some(sink) = self.pty_sink.get() {
            sink.write(text.into_bytes());
        }
    }
}

impl EventListener for EventProxy {
    // Called from the PTY event loop thread while it holds the `Term` lock (the parser runs
    // under it), so nothing here may lock `Term`; `FairMutex` is not reentrant.
    fn send_event(&self, event: AlacrittyEvent) {
        match event {
            AlacrittyEvent::PtyWrite(text) => self.write_to_pty(text),
            AlacrittyEvent::TextAreaSizeRequest(format) => {
                let window_size = self.window_size.lock().ok().map(|size| *size);
                if let Some(window_size) = window_size {
                    self.write_to_pty(format(window_size));
                }
            }
            AlacrittyEvent::Wakeup => {
                let _ = self.event_tx.unbounded_send(TerminalEvent::Wakeup);
            }
            AlacrittyEvent::Title(t) => {
                let _ = self.event_tx.unbounded_send(TerminalEvent::Title(t));
            }
            AlacrittyEvent::ResetTitle => {
                let _ = self.event_tx.unbounded_send(TerminalEvent::ResetTitle);
            }
            AlacrittyEvent::ChildExit(status) => {
                let code = status.code();
                if let Ok(mut state_guard) = self.state.lock() {
                    *state_guard = SessionState::Exited(code);
                }
                let _ = self.event_tx.unbounded_send(TerminalEvent::ChildExit(code));
                let _ = self.event_tx.unbounded_send(TerminalEvent::Wakeup);
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

/// `Term` configuration for real sessions.
///
/// `kitty_keyboard` lets programs negotiate the Kitty keyboard protocol (`CSI > Nu`, `CSI ? u`,
/// `CSI < Nu`). Without it alacritty_terminal ignores those sequences, so `keyboard_mode()` never
/// leaves legacy and full-screen programs that request it (Issue #35) get bare control bytes.
fn term_config() -> Config {
    Config {
        kitty_keyboard: true,
        ..Config::default()
    }
}

/// A terminal session managing the child process, PTY, and emulator state.
pub struct TerminalSession {
    term: Arc<FairMutex<Term<EventProxy>>>,
    event_loop_sender: EventLoopSender,
    state: Arc<Mutex<SessionState>>,
    window_size: Arc<Mutex<WindowSize>>,
    selection_anchor: Mutex<Option<alacritty_terminal::index::Point>>,
    shell_program: String,
    integration_launch: crate::terminal::shell::ShellIntegrationLaunch,
}

impl TerminalSession {
    /// Spawns a new terminal session with the default shell, in the default directory.
    #[allow(dead_code)]
    pub fn spawn(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        event_tx: impl Into<EventSink>,
    ) -> Result<Self, String> {
        Self::spawn_in(cols, rows, cell_width, cell_height, event_tx, None)
    }

    /// Spawns a new terminal session with the default shell, started in `working_directory` when
    /// given. The shell is set up to report its directory (see [`Launch`]).
    #[allow(dead_code)]
    pub fn spawn_in(
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        event_tx: impl Into<EventSink>,
        working_directory: Option<PathBuf>,
    ) -> Result<Self, String> {
        Self::spawn_launch(
            Launch::resolve_default(),
            cols,
            rows,
            cell_width,
            cell_height,
            event_tx,
            working_directory,
        )
    }

    /// Spawns a new terminal session configured according to [`crate::settings::TerminalSettings`].
    pub fn spawn_with_settings(
        settings: &crate::settings::TerminalSettings,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        event_tx: impl Into<EventSink>,
        working_directory: Option<PathBuf>,
    ) -> Result<Self, String> {
        let launch = Launch::from_settings(settings, std::env::var("PROMPT").ok().as_deref())?;
        Self::spawn_launch(
            launch,
            cols,
            rows,
            cell_width,
            cell_height,
            event_tx,
            working_directory,
        )
    }

    /// Spawns a session for an explicit [`Launch`] (shell, arguments, environment).
    pub(crate) fn spawn_launch(
        launch: Launch,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        event_tx: impl Into<EventSink>,
        working_directory: Option<PathBuf>,
    ) -> Result<Self, String> {
        #[cfg(all(test, windows))]
        wait_for_shells_at_test_exit();

        let shell_program = launch.config.program.clone();
        let integration_launch = launch.integration_launch;
        let event_tx = event_tx.into();
        let options = Options {
            drain_on_exit: true,
            shell: Some(Shell::new(launch.config.program, launch.config.args)),
            working_directory,
            env: launch.env,
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
        let pty = TappedPty {
            inner: pty,
            scanner: OscScanner::new(local_host_name()),
            sink: event_tx.clone(),
        };

        let state = Arc::new(Mutex::new(SessionState::Running));
        let pty_sink = Arc::new(OnceLock::new());
        let shared_window_size = Arc::new(Mutex::new(window_size));

        let proxy = EventProxy {
            event_tx,
            state: state.clone(),
            pty_sink: pty_sink.clone(),
            window_size: shared_window_size.clone(),
        };

        let term_size = TermSize {
            cols: cols.max(1) as usize,
            lines: rows.max(1) as usize,
        };

        let term = Term::new(term_config(), &term_size, proxy.clone());
        let term = Arc::new(FairMutex::new(term));

        let event_loop = EventLoop::new(term.clone(), proxy, pty, false, false)
            .map_err(|e| format!("Failed to create event loop: {e}"))?;

        let event_loop_sender = event_loop.channel();
        // Set before `spawn`: no PTY output is parsed until the loop runs, so no reply can be
        // generated without a sink.
        let _ = pty_sink.set(PtySink::EventLoop(event_loop_sender.clone()));
        let _join_handle = event_loop.spawn();

        Ok(Self {
            term,
            event_loop_sender,
            state,
            window_size: shared_window_size,
            selection_anchor: Mutex::new(None),
            shell_program,
            integration_launch,
        })
    }

    /// Returns the executable path or name of the shell running in this session.
    pub fn shell_program(&self) -> &str {
        &self.shell_program
    }

    /// Returns the Shell Integration disposition determined at session launch.
    pub fn integration_launch(&self) -> crate::terminal::shell::ShellIntegrationLaunch {
        self.integration_launch
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

    /// The size last sent to the PTY: (columns, rows, cell width px, cell height px).
    #[cfg(test)]
    pub(crate) fn pty_size(&self) -> (u16, u16, u16, u16) {
        let size = *self.window_size.lock().unwrap();
        (
            size.num_cols,
            size.num_lines,
            size.cell_width,
            size.cell_height,
        )
    }

    /// Resizes the terminal grid and PTY.
    pub fn resize(&self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        if self.is_exited() {
            return;
        }
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

        // `Msg::Resize` only resizes the PTY; the event loop never touches the `Term` grid, so
        // it is resized here too. Both happen under the `Term` lock (as upstream Alacritty does)
        // so the PTY thread cannot parse the child's redraw for the new size into a stale grid.
        // Lock order `Term` -> `window_size` matches `EventProxy::send_event`.
        let mut term = self.term.lock();
        if let Ok(mut size_guard) = self.window_size.lock() {
            *size_guard = window_size;
        }
        let _ = self.event_loop_sender.send(Msg::Resize(window_size));
        term.resize(TermSize {
            cols: cols as usize,
            lines: rows as usize,
        });
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

    /// Returns the active terminal cursor column and row in display grid coordinates.
    pub fn cursor_position(&self) -> (usize, usize) {
        let term = self.term.lock();
        let content = term.renderable_content();
        let col = content.cursor.point.column.0;
        let line = content.cursor.point.line.0.max(0) as usize;
        (col, line)
    }

    /// The foreground colour of the printed character just left of the cursor, if there is one.
    ///
    /// Used only to tint visual effects; it is read from the grid at the moment of the call, so it can
    /// be a neighbouring character when the line is changing quickly.
    pub fn foreground_left_of_cursor(&self) -> Option<alacritty_terminal::vte::ansi::Color> {
        use alacritty_terminal::index::{Column, Line, Point};
        let term = self.term.lock();
        let cursor = term.renderable_content().cursor.point;
        let column = cursor.column.0.checked_sub(1)?;
        let line = cursor.line;
        if line.0 < 0 || line.0 as usize >= term.screen_lines() || column >= term.columns() {
            return None;
        }
        let cell = &term.grid()[Point::new(Line(line.0), Column(column))];
        (!cell.c.is_whitespace() && cell.c != ' ').then_some(cell.fg)
    }

    /// Whether there is an active text selection in the terminal.
    pub fn has_selection(&self) -> bool {
        let term = self.term.lock();
        term.selection
            .as_ref()
            .and_then(|s| s.to_range(&term))
            .is_some()
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
        if let Ok(mut anchor) = self.selection_anchor.lock() {
            *anchor = None;
        }
    }

    /// Starts a simple mouse drag selection at the specified grid cell.
    pub fn start_selection(&self, col: usize, line: usize) {
        use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = self.term.lock();
        let cols = term.columns();
        let lines = term.screen_lines();
        let col = col.min(cols.saturating_sub(1));
        let line = line.min(lines.saturating_sub(1));
        let display_offset = term.grid().display_offset() as i32;
        let point = TermPoint::new(Line(line as i32 - display_offset), Column(col));
        if let Ok(mut anchor) = self.selection_anchor.lock() {
            *anchor = Some(point);
        }
        term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
    }

    /// Extends the active mouse drag selection to the specified grid cell.
    pub fn update_selection(&self, col: usize, line: usize) {
        use alacritty_terminal::index::{Column, Line, Point as TermPoint, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        let mut term = self.term.lock();
        let cols = term.columns();
        let lines = term.screen_lines();
        let col = col.min(cols.saturating_sub(1));
        let line = line.min(lines.saturating_sub(1));
        let display_offset = term.grid().display_offset() as i32;
        let point = TermPoint::new(Line(line as i32 - display_offset), Column(col));
        let anchor_point = self.selection_anchor.lock().ok().and_then(|guard| *guard);
        if let Some(anchor) = anchor_point {
            let mut selection = if point < anchor {
                Selection::new(SelectionType::Simple, anchor, Side::Right)
            } else {
                Selection::new(SelectionType::Simple, anchor, Side::Left)
            };
            let side = if point < anchor {
                Side::Left
            } else {
                Side::Right
            };
            selection.update(point, side);
            term.selection = Some(selection);
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

/// The test binary has no `main` of ours, so the shells that its tests spawned and dropped would
/// be orphaned by the process exit just like in the app. Register the same wait with the C
/// runtime once, when the first session is spawned.
#[cfg(all(test, windows))]
fn wait_for_shells_at_test_exit() {
    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> i32;
    }
    extern "C" fn wait() {
        let cleanup = wait_for_shell_cleanup(SHUTDOWN_WAIT);
        if let Some(warning) = cleanup.exit_warning(SHUTDOWN_WAIT) {
            eprintln!("kegon tests: {warning}");
        }
    }
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| {
        // SAFETY: `wait` is a plain `extern "C"` function with no captured state.
        unsafe { atexit(wait) };
    });
}

/// Longest time [`wait_for_shell_cleanup`] is given at exit. A reaped shell takes about 0.5 s at
/// most (grace period) plus the termination check, and a healthy one is gone in milliseconds.
pub const SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// How waiting for the shells of closed sessions ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellCleanup {
    /// Every shell is gone.
    Complete,
    /// The wait ran out while `pending` shells were still being closed or reaped (`unresolved`
    /// others could not be confirmed gone before).
    TimedOut { pending: usize, unresolved: usize },
    /// Nothing is pending, but `unresolved` shells could not be confirmed gone.
    Unconfirmed { unresolved: usize },
}

impl ShellCleanup {
    #[cfg(test)]
    pub fn is_complete(self) -> bool {
        self == ShellCleanup::Complete
    }

    /// The warning to print at exit, if shells may have been left behind. A timeout (shells still
    /// being closed when the process exits) and a shell that could not be confirmed gone are
    /// worded differently.
    pub fn exit_warning(self, timeout: std::time::Duration) -> Option<String> {
        match self {
            ShellCleanup::Complete => None,
            ShellCleanup::TimedOut {
                pending,
                unresolved,
            } => Some(format!(
                "{pending} shell(s) were still being closed after {timeout:?} and may be left                  running ({unresolved} more could not be confirmed gone)"
            )),
            ShellCleanup::Unconfirmed { unresolved } => Some(format!(
                "{unresolved} shell(s) could not be confirmed gone and may be left running"
            )),
        }
    }
}

#[cfg(windows)]
impl From<alacritty_terminal::tty::ShellCleanup> for ShellCleanup {
    fn from(cleanup: alacritty_terminal::tty::ShellCleanup) -> Self {
        use alacritty_terminal::tty::ShellCleanup as Fork;
        match cleanup {
            Fork::Complete => ShellCleanup::Complete,
            Fork::TimedOut {
                pending,
                unresolved,
            } => ShellCleanup::TimedOut {
                pending,
                unresolved,
            },
            Fork::Unconfirmed { unresolved } => ShellCleanup::Unconfirmed { unresolved },
        }
    }
}

/// Waits until the shells of all dropped sessions are gone, terminating the ones that do not
/// leave by themselves (Windows). Dropping a [`TerminalSession`] only asks its PTY thread to stop;
/// that thread closes the pseudoconsole and a helper thread reaps the shell, and a process that
/// exits first ends both. Call this once at the end of `main`, after the UI (and with it every
/// session) is gone. It returns at once when nothing is pending and does not sleep for a fixed
/// time. A timeout and an unconfirmed shell are reported as different results (and are logged by
/// the fork).
pub fn wait_for_shell_cleanup(timeout: std::time::Duration) -> ShellCleanup {
    #[cfg(windows)]
    {
        alacritty_terminal::tty::wait_for_pending_shells(timeout).into()
    }
    #[cfg(not(windows))]
    {
        let _ = timeout;
        ShellCleanup::Complete
    }
}

impl Drop for TerminalSession {
    /// Stops the PTY event loop, which closes the PTY and with it the child process. Safe after a
    /// natural exit: the send just fails once the loop has already ended.
    fn drop(&mut self) {
        let _ = self.event_loop_sender.send(Msg::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::channel::mpsc::unbounded;
    use iced::futures::{FutureExt, StreamExt};

    #[test]
    fn session_spawns_and_reports_initial_state() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx);
        assert!(session.is_ok());
        let session = session.unwrap();
        assert!(!session.is_exited());
        assert_eq!(session.exit_code(), None);
    }

    #[test]
    fn selection_lifecycle_starts_updates_and_clears() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();

        assert!(!session.has_selection());
        session.start_selection(0, 0);
        session.update_selection(5, 0);
        assert!(session.has_selection());

        session.clear_selection();
        assert!(!session.has_selection());
    }

    #[test]
    fn test_selection_details() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        {
            let mut term = session.term.lock();
            use alacritty_terminal::vte::ansi::Processor;
            let mut parser: Processor = Processor::new();
            parser.advance(
                &mut *term,
                b"Hello World\r\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\r\nLine Three\r\n",
            );
        }

        // Single click (start_selection only) -> degenerate selection, not copyable
        session.start_selection(0, 0);
        assert!(!session.has_selection());
        assert_eq!(session.copy_selection(), None);

        // Drag forward (0,0) to (4,0) -> "Hello"
        session.start_selection(0, 0);
        session.update_selection(4, 0);
        assert!(session.has_selection());
        assert_eq!(session.copy_selection().as_deref(), Some("Hello"));

        // Drag backward (4,0) to (0,0) -> "Hello"
        session.start_selection(4, 0);
        session.update_selection(0, 0);
        assert!(session.has_selection());
        assert_eq!(session.copy_selection().as_deref(), Some("Hello"));

        // Multiline drag forward (0,0) to (2,1)
        session.start_selection(0, 0);
        session.update_selection(2, 1);
        assert!(session.has_selection());
        assert_eq!(
            session.copy_selection().as_deref(),
            Some("Hello World\n日本")
        );

        // Japanese text drag (0,1) to (5,1) -> "日本語"
        session.start_selection(0, 1);
        session.update_selection(5, 1);
        assert!(session.has_selection());
        assert_eq!(session.copy_selection().as_deref(), Some("日本語"));
    }

    const TEST_WINDOW_SIZE: WindowSize = WindowSize {
        num_lines: 24,
        num_cols: 80,
        cell_width: 10,
        cell_height: 20,
    };

    /// Builds a proxy whose PTY replies land in a channel instead of a live event loop.
    fn proxy_with_pty_channel() -> (
        EventProxy,
        std::sync::mpsc::Receiver<Vec<u8>>,
        Arc<Mutex<WindowSize>>,
    ) {
        let (event_tx, _event_rx) = unbounded();
        let (pty_tx, pty_rx) = std::sync::mpsc::channel();
        let pty_sink = Arc::new(OnceLock::new());
        let _ = pty_sink.set(PtySink::Channel(pty_tx));
        let window_size = Arc::new(Mutex::new(TEST_WINDOW_SIZE));
        let proxy = EventProxy {
            event_tx: event_tx.into(),
            state: Arc::new(Mutex::new(SessionState::Running)),
            pty_sink,
            window_size: window_size.clone(),
        };
        (proxy, pty_rx, window_size)
    }

    fn drain(rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        rx.try_iter().collect()
    }

    #[test]
    fn pty_write_forwards_exact_bytes() {
        let (proxy, pty_rx, _) = proxy_with_pty_channel();
        proxy.send_event(AlacrittyEvent::PtyWrite(String::from("\x1b[?6c")));
        assert_eq!(drain(&pty_rx), vec![b"\x1b[?6c".to_vec()]);
    }

    #[test]
    fn empty_pty_write_is_not_forwarded() {
        let (proxy, pty_rx, _) = proxy_with_pty_channel();
        proxy.send_event(AlacrittyEvent::PtyWrite(String::new()));
        assert!(drain(&pty_rx).is_empty());
    }

    #[test]
    fn text_area_size_request_formats_current_window_size() {
        let (proxy, pty_rx, window_size) = proxy_with_pty_channel();
        let request = || {
            AlacrittyEvent::TextAreaSizeRequest(Arc::new(|size: WindowSize| {
                format!("{}x{}", size.num_cols * size.cell_width, size.num_lines)
            }))
        };

        proxy.send_event(request());
        *window_size.lock().unwrap() = WindowSize {
            num_lines: 30,
            num_cols: 100,
            cell_width: 9,
            cell_height: 18,
        };
        proxy.send_event(request());

        assert_eq!(drain(&pty_rx), vec![b"800x24".to_vec(), b"900x30".to_vec()]);
    }

    #[test]
    fn replies_without_sink_or_after_shutdown_are_dropped_silently() {
        let (event_tx, _event_rx) = unbounded();
        let proxy = EventProxy {
            event_tx: event_tx.into(),
            state: Arc::new(Mutex::new(SessionState::Running)),
            pty_sink: Arc::new(OnceLock::new()),
            window_size: Arc::new(Mutex::new(TEST_WINDOW_SIZE)),
        };
        proxy.send_event(AlacrittyEvent::PtyWrite(String::from("\x1b[0n")));

        let (proxy, pty_rx, _) = proxy_with_pty_channel();
        drop(pty_rx);
        proxy.send_event(AlacrittyEvent::PtyWrite(String::from("\x1b[0n")));
    }

    /// Drives real query sequences through `Term` while its lock is held, as the PTY event
    /// loop does; a re-lock inside the proxy would deadlock this test.
    #[test]
    fn terminal_queries_reply_through_proxy_while_term_is_locked() {
        use alacritty_terminal::vte::ansi::Processor;

        let (proxy, pty_rx, _) = proxy_with_pty_channel();
        let size = TermSize {
            cols: 80,
            lines: 24,
        };
        let term = FairMutex::new(Term::new(Config::default(), &size, proxy));
        let mut parser: Processor = Processor::new();

        let mut guard = term.lock();
        let mut query = |bytes: &[u8]| {
            parser.advance(&mut *guard, bytes);
            drain(&pty_rx)
        };

        assert_eq!(query(b"\x1b[c"), vec![b"\x1b[?6c".to_vec()]);
        assert_eq!(query(b"\x1b[>c"), vec![b"\x1b[>0;2600;1c".to_vec()]);
        assert_eq!(query(b"\x1b[5n"), vec![b"\x1b[0n".to_vec()]);
        assert_eq!(query(b"abc\x1b[6n"), vec![b"\x1b[1;4R".to_vec()]);
        assert_eq!(query(b"\x1b[?2004$p"), vec![b"\x1b[?2004;2$y".to_vec()]);
        assert_eq!(query(b"\x1b[18t"), vec![b"\x1b[8;24;80t".to_vec()]);
        assert_eq!(query(b"\x1b[14t"), vec![b"\x1b[4;480;800t".to_vec()]);
    }

    #[test]
    fn resize_updates_size_used_for_text_area_replies() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        session.resize(100, 30, 9, 18);
        let size = *session.window_size.lock().unwrap();
        assert_eq!(
            (
                size.num_cols,
                size.num_lines,
                size.cell_width,
                size.cell_height
            ),
            (100, 30, 9, 18)
        );
    }

    fn grid_size(session: &TerminalSession) -> (usize, usize) {
        let term = session.term().lock();
        (term.columns(), term.screen_lines())
    }

    #[test]
    fn resize_updates_term_grid_with_pty_dimensions() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        assert_eq!(grid_size(&session), (80, 24));

        session.resize(120, 40, 10, 20);
        assert_eq!(grid_size(&session), (120, 40));
        let size = *session.window_size.lock().unwrap();
        assert_eq!((size.num_cols, size.num_lines), (120, 40));
    }

    #[test]
    fn repeated_resizes_leave_grid_at_last_size() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        for (cols, rows) in [(120, 40), (40, 10), (100, 30)] {
            session.resize(cols, rows, 10, 20);
            assert_eq!(grid_size(&session), (cols as usize, rows as usize));
        }
    }

    #[test]
    fn zero_resize_is_clamped_identically_for_grid_and_pty() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        session.resize(0, 0, 0, 0);
        assert_eq!(grid_size(&session), (1, 1));
        let size = *session.window_size.lock().unwrap();
        assert_eq!((size.num_cols, size.num_lines), (1, 1));
    }

    /// `CSI 18 t` is answered from the `Term` grid and `CSI 14 t` from the shared pixel size,
    /// so both must follow a resize.
    #[test]
    fn size_replies_follow_resized_term() {
        use alacritty_terminal::vte::ansi::Processor;

        let (proxy, pty_rx, window_size) = proxy_with_pty_channel();
        let term = FairMutex::new(Term::new(
            Config::default(),
            &TermSize {
                cols: 80,
                lines: 24,
            },
            proxy,
        ));
        let mut parser: Processor = Processor::new();

        let mut guard = term.lock();
        *window_size.lock().unwrap() = WindowSize {
            num_lines: 40,
            num_cols: 120,
            cell_width: 10,
            cell_height: 20,
        };
        guard.resize(TermSize {
            cols: 120,
            lines: 40,
        });
        parser.advance(&mut *guard, b"\x1b[18t\x1b[14t");
        assert_eq!(
            drain(&pty_rx),
            vec![b"\x1b[8;40;120t".to_vec(), b"\x1b[4;800;1200t".to_vec()]
        );
    }

    #[test]
    fn event_proxy_delivers_events_in_exact_order() {
        let (tx, mut rx) = unbounded();
        let state = Arc::new(Mutex::new(SessionState::Running));

        let proxy = EventProxy {
            event_tx: tx.into(),
            state: state.clone(),
            pty_sink: Arc::new(OnceLock::new()),
            window_size: Arc::new(Mutex::new(TEST_WINDOW_SIZE)),
        };

        proxy.send_event(AlacrittyEvent::Wakeup);
        proxy.send_event(AlacrittyEvent::Title("Custom Title".to_string()));
        proxy.send_event(AlacrittyEvent::ResetTitle);

        assert_eq!(
            rx.next().now_or_never().flatten(),
            Some(TerminalEvent::Wakeup)
        );
        assert_eq!(
            rx.next().now_or_never().flatten(),
            Some(TerminalEvent::Title("Custom Title".to_string()))
        );
        assert_eq!(
            rx.next().now_or_never().flatten(),
            Some(TerminalEvent::ResetTitle)
        );
        assert_eq!(*state.lock().unwrap(), SessionState::Running);
    }

    #[test]
    fn event_channel_drop_safety() {
        let (tx, mut rx) = unbounded::<TerminalEvent>();
        drop(tx);
        assert_eq!(rx.next().now_or_never().flatten(), None);

        let (tx, rx) = unbounded::<TerminalEvent>();
        drop(rx);
        let res = tx.unbounded_send(TerminalEvent::Wakeup);
        assert!(res.is_err());
    }

    #[test]
    fn selection_lifecycle_with_scrollback() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();

        session.scroll_display(5);
        session.start_selection(0, 0);
        session.update_selection(10, 0);
        assert!(session.has_selection());

        session.clear_selection();
        assert!(!session.has_selection());
    }

    #[test]
    fn cursor_position_returns_initial_coordinates() {
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        let (col, line) = session.cursor_position();
        assert_eq!((col, line), (0, 0));
    }

    /// Number of live processes whose command line contains `marker`, excluding the query itself.
    #[cfg(windows)]
    fn processes_with_marker(marker: &str) -> usize {
        let script = format!(
            "(Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{marker}*' -and $_.ProcessId -ne $PID }} | Measure-Object).Count"
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// Dropping a session must end its child processes, not merely stop the event loop. Spawns a
    /// real long-running child, so it is opt-in: `cargo test -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore = "spawns real processes and inspects the OS process table"]
    fn dropping_a_session_terminates_its_child_processes() {
        let marker = format!("KegonDropMarker{}", std::process::id());
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        session.write_input(
            format!("powershell -NoProfile -Command 'Start-Sleep 300; Write-Output {marker}'\r\n")
                .into_bytes(),
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while processes_with_marker(&marker) == 0 {
            assert!(std::time::Instant::now() < deadline, "child never started");
            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        drop(session);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while processes_with_marker(&marker) != 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "child survived the session"
            );
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    #[test]
    fn exit_warnings_tell_a_timeout_from_an_unconfirmed_shell() {
        use std::time::Duration;

        let wait = Duration::from_secs(3);
        assert_eq!(ShellCleanup::Complete.exit_warning(wait), None);
        assert!(ShellCleanup::Complete.is_complete());

        let timed_out = ShellCleanup::TimedOut {
            pending: 2,
            unresolved: 1,
        };
        let unconfirmed = ShellCleanup::Unconfirmed { unresolved: 1 };
        assert!(!timed_out.is_complete() && !unconfirmed.is_complete());

        let timed_out = timed_out.exit_warning(wait).unwrap();
        assert!(timed_out.contains("2 shell(s) were still being closed after 3s"));
        assert!(timed_out.contains("1 more could not be confirmed gone"));
        let unconfirmed = unconfirmed.exit_warning(wait).unwrap();
        assert!(unconfirmed.contains("1 shell(s) could not be confirmed gone"));
        assert!(!unconfirmed.contains("still being closed"));
    }

    /// Shells and console hosts that are direct children of this test process.
    #[cfg(windows)]
    fn own_shell_children() -> usize {
        let script = format!(
            "(Get-CimInstance Win32_Process | Where-Object {{ $_.ParentProcessId -eq {} -and $_.Name -in 'cmd.exe','pwsh.exe','conhost.exe' }} | Measure-Object).Count",
            std::process::id()
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// Adds the flags that suppress error dialogs to this process's error mode. Shells closed
    /// right after spawning may fail with `0xC0000142`, which would otherwise show a dialog.
    #[cfg(windows)]
    fn suppress_error_dialogs() {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetErrorMode(mode: u32) -> u32;
        }
        // SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX, added to the current mode.
        let previous = unsafe { SetErrorMode(3) };
        unsafe { SetErrorMode(previous | 3) };
    }

    /// A session running `program` (with Shell Integration, like a real tab).
    #[cfg(windows)]
    fn spawn_tab(
        program: &str,
    ) -> (
        TerminalSession,
        iced::futures::channel::mpsc::UnboundedReceiver<TerminalEvent>,
    ) {
        let launch = Launch::new(
            crate::terminal::shell::ShellConfig {
                program: program.to_string(),
                args: vec![],
            },
            true,
            &local_host_name(),
            None,
        );
        let (tx, rx) = unbounded();
        (
            TerminalSession::spawn_launch(launch, 80, 24, 10, 20, tx, None).unwrap(),
            rx,
        )
    }

    /// Closes tabs right after spawning them, one at a time and six at once, and checks that no
    /// shell or console host is left and that closing does not block the caller. Shells closed
    /// this early may fail with `0xC0000142`, so error dialogs are suppressed for the test
    /// process.
    #[cfg(windows)]
    fn assert_no_shell_survives_immediate_close(program: String) {
        use std::time::{Duration, Instant};

        suppress_error_dialogs();

        let baseline = own_shell_children();
        let mut slowest = Duration::ZERO;
        for _ in 0..10 {
            let (session, _rx) = spawn_tab(&program);
            let start = Instant::now();
            drop(session);
            slowest = slowest.max(start.elapsed());
        }
        let tabs: Vec<_> = (0..6).map(|_| spawn_tab(&program)).collect();
        let start = Instant::now();
        drop(tabs);
        slowest = slowest.max(start.elapsed());
        assert!(
            slowest < Duration::from_millis(100),
            "closing blocked the caller for {slowest:?}"
        );

        // The reaper gives a shell 500 ms and then confirms the termination; allow far more.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let now = own_shell_children();
            if now <= baseline {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{} shell or console host processes left after the tabs closed (baseline {baseline})",
                now - baseline
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Run with `cargo test -- --ignored --test-threads=1`: other tests would add child processes.
    #[cfg(windows)]
    #[test]
    #[ignore = "spawns and closes many real shells and inspects the OS process table"]
    fn cmd_tabs_closed_right_after_spawn_leave_no_process() {
        assert_no_shell_survives_immediate_close(
            crate::terminal::shell::installed::cmd().expect("cmd is not installed"),
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "spawns and closes many real shells and inspects the OS process table"]
    fn pwsh_tabs_closed_right_after_spawn_leave_no_process() {
        assert_no_shell_survives_immediate_close(
            crate::terminal::shell::installed::pwsh().expect("pwsh is not installed"),
        );
    }

    /// What the app exit relies on: after the tabs were closed, waiting for the cleanup returns
    /// `true` without a fixed delay, and no shell or console host is left right afterwards. (A
    /// process that exits without waiting can leave shells behind; that race is measured with
    /// the real process exit outside of the test harness.)
    #[cfg(windows)]
    #[test]
    #[ignore = "spawns and closes real shells and inspects the OS process table"]
    fn waiting_for_cleanup_after_closing_tabs_leaves_no_process() {
        use std::time::{Duration, Instant};

        suppress_error_dialogs();
        let program = crate::terminal::shell::installed::pwsh().expect("pwsh is not installed");
        assert!(
            wait_for_shell_cleanup(SHUTDOWN_WAIT).is_complete(),
            "shells left over from an earlier test"
        );
        let baseline = own_shell_children();

        let tabs: Vec<_> = (0..6).map(|_| spawn_tab(&program)).collect();
        drop(tabs);
        let start = Instant::now();
        assert!(
            wait_for_shell_cleanup(SHUTDOWN_WAIT).is_complete(),
            "shells were still pending after {SHUTDOWN_WAIT:?}"
        );
        assert!(start.elapsed() < SHUTDOWN_WAIT);

        // Console hosts leave a few milliseconds after their shell.
        let deadline = Instant::now() + Duration::from_secs(2);
        while own_shell_children() > baseline {
            assert!(
                Instant::now() < deadline,
                "shell or console host processes left after waiting for the cleanup"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// A shell that ends by itself still reports its own exit code; the reaper only acts after
    /// the session is dropped.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs cmd.exe and ConPTY"]
    fn natural_exit_reports_the_shell_exit_code() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (tx, rx) = mpsc::channel();
        let sink = EventSink::from_fn(move |event| {
            if let TerminalEvent::ChildExit(code) = event {
                let _ = tx.send(code);
            }
        });
        let launch = Launch::new(
            crate::terminal::shell::ShellConfig {
                program: crate::terminal::shell::installed::cmd().expect("cmd is not installed"),
                args: vec![],
            },
            true,
            &local_host_name(),
            None,
        );
        let session = TerminalSession::spawn_launch(launch, 80, 24, 10, 20, sink, None).unwrap();
        session.write_input(b"exit 7\r\n".to_vec());

        assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap(), Some(7));
        assert_eq!(session.exit_code(), Some(7));
    }

    /// A scratch directory whose name has spaces and non-ASCII text.
    #[cfg(windows)]
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kegon cwd 日本語 {tag} {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(windows)]
    fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
        let norm = |p: &std::path::Path| {
            p.to_string_lossy()
                .trim_start_matches("\\\\?\\")
                .trim_end_matches('\\')
                .to_lowercase()
        };
        norm(a) == norm(b)
    }

    /// Starts `program` with Shell Integration in `start`, moves it to `target` with `command`, and
    /// checks that both the initial prompt and the `cd` are reported.
    #[cfg(windows)]
    fn assert_shell_reports_directories(
        program: String,
        command: impl Fn(&std::path::Path) -> String,
    ) {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let start = scratch_dir("start");
        let target = scratch_dir("target");
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::from_fn(move |event| {
            if let TerminalEvent::WorkingDirectory(path) = event {
                let _ = tx.send(path);
            }
        });
        let launch = Launch::new(
            crate::terminal::shell::ShellConfig {
                program,
                args: vec![],
            },
            true,
            &local_host_name(),
            None,
        );
        assert!(launch.integrated);
        let session =
            TerminalSession::spawn_launch(launch, 100, 30, 10, 20, sink, Some(start.clone()))
                .unwrap();

        let wait_for = |expected: &std::path::Path| {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                if let Ok(path) = rx.recv_timeout(Duration::from_millis(200))
                    && same_dir(&path, expected)
                {
                    return;
                }
            }
            panic!("no report for {expected:?}");
        };
        wait_for(&start);
        session.write_input(format!("{}\r", command(&target)).into_bytes());
        wait_for(&target);
        // The visible screen must not contain the control sequence.
        let text = {
            let term = session.term().lock();
            let grid = term.grid();
            (0..grid.screen_lines() as i32)
                .map(|l| {
                    (0..grid.columns())
                        .map(|c| {
                            grid[alacritty_terminal::index::Point::new(
                                alacritty_terminal::index::Line(l),
                                alacritty_terminal::index::Column(c),
                            )]
                            .c
                        })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(
                    "
",
                )
        };
        assert!(!text.contains("]7;") && !text.contains("file://"), "{text}");
        let _ = std::fs::remove_dir_all(&start);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[cfg(windows)]
    fn powershell_cd(target: &std::path::Path) -> String {
        format!("Set-Location -LiteralPath '{}'", target.display())
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs a real PowerShell 7 and ConPTY"]
    fn pwsh_reports_its_directory() {
        let program = crate::terminal::shell::installed::pwsh().expect("pwsh is not installed");
        assert_shell_reports_directories(program, powershell_cd);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs Windows PowerShell 5.1 and ConPTY"]
    fn windows_powershell_reports_its_directory() {
        let program = crate::terminal::shell::installed::windows_powershell()
            .expect("Windows PowerShell is not installed");
        assert_shell_reports_directories(program, powershell_cd);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs a real PowerShell 7 and ConPTY; spawns real shells"]
    fn pwsh_shell_integration_emits_osc133_markers() {
        let program = crate::terminal::shell::installed::pwsh().expect("pwsh is not installed");
        assert_powershell_emits_osc133_markers(program, true);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs PowerShell 7 and ConPTY; spawns a real shell"]
    fn pwsh_hook_preserves_error_history_and_prompt_status() {
        assert_hook_preserves_state(
            crate::terminal::shell::installed::pwsh().expect("pwsh is not installed"),
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs Windows PowerShell 5.1 and ConPTY; spawns a real shell"]
    fn windows_powershell_hook_preserves_error_history_and_prompt_status() {
        assert_hook_preserves_state(
            crate::terminal::shell::installed::windows_powershell()
                .expect("Windows PowerShell is not installed"),
        );
    }

    #[cfg(windows)]
    fn assert_hook_preserves_state(program: String) {
        use crate::terminal::shell::{ShellConfig, encode_powershell_command};
        use std::sync::mpsc;
        use std::time::Duration;

        // Define an input provider even if PSReadLine is unavailable in this host.
        // The fixture replaces only that provider after installing the production hooks.
        let script = format!(
            "try {{ function global:PSConsoleHostReadLine {{ '' }}\n{}\n{}\nexit 0\n}} catch {{ [Console]::Error.WriteLine($_); exit 1 }}",
            include_str!("../../assets/shell-integration/powershell.ps1"),
            include_str!("../../assets/shell-integration/tests/hook-state.ps1"),
        );
        let launch = Launch::new(
            ShellConfig {
                program,
                args: vec![
                    "-NoProfile".into(),
                    "-EncodedCommand".into(),
                    encode_powershell_command(&script),
                ],
            },
            false,
            &local_host_name(),
            None,
        );
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::from_fn(move |event| {
            if let TerminalEvent::ChildExit(code) = event {
                let _ = tx.send(code);
            }
        });
        let session = TerminalSession::spawn_launch(launch, 120, 40, 10, 20, sink, None).unwrap();
        let result = rx.recv_timeout(Duration::from_secs(30));
        // Drop on success, failure, or timeout; the PTY shutdown reaps the process.
        drop(session);
        assert_eq!(result, Ok(Some(0)), "hook state regression: {result:?}");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs Windows PowerShell 5.1 and ConPTY; spawns real shells"]
    fn windows_powershell_shell_integration_emits_osc133_markers() {
        let program = crate::terminal::shell::installed::windows_powershell()
            .expect("Windows PowerShell is not installed");
        // Windows PowerShell 5.1 built-in PSReadLine does not load in ConPTY, so command tracking (C/D)
        // is safely degraded while prompt markers (A/B) and CWD (OSC 7) are emitted.
        assert_powershell_emits_osc133_markers(program, false);
    }

    #[cfg(windows)]
    fn assert_powershell_emits_osc133_markers(program: String, expect_command_markers: bool) {
        use std::sync::mpsc;
        use std::time::Duration;

        let start = scratch_dir("pwsh-osc133-start");
        let missing_output = start.join("missing-parent").join("out.txt");
        assert!(!missing_output.parent().unwrap().exists());
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::from_fn(move |event| {
            if let TerminalEvent::ShellIntegration(marker) = event {
                let _ = tx.send(marker);
            }
        });
        let launch = Launch::new(
            crate::terminal::shell::ShellConfig {
                program,
                args: vec![],
            },
            true,
            &local_host_name(),
            None,
        );
        assert!(launch.integrated);
        let session =
            TerminalSession::spawn_launch(launch, 100, 30, 10, 20, sink, Some(start.clone()))
                .unwrap();

        // Assert every marker, not just the eventual presence of a matching D:
        // a numeric D, duplicate C/D, or reordered boundary must fail immediately.
        let expect = |marker: Osc133Marker, context: &str| {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(30)),
                Ok(marker),
                "{context}: expected {marker:?}"
            );
        };
        expect(Osc133Marker::PromptStarted, "initial prompt");
        expect(Osc133Marker::InputStarted, "initial input");

        if expect_command_markers {
            let redirect_failure = format!(
                "cmd.exe /c exit 0 > '{}'",
                missing_output.display().to_string().replace('\'', "''")
            );
            let cases = [
                ("native success", "cmd.exe /c exit 0"),
                ("native failure", "cmd.exe /c exit 42"),
                (
                    "pre-execution redirection failure with stale 42",
                    redirect_failure.as_str(),
                ),
                (
                    "unexecuted branch",
                    "if ($false) { cmd.exe /c exit 0 }; Write-Error bad -ErrorAction Ignore",
                ),
                (
                    "argument evaluation failure",
                    "cmd.exe /c exit $(throw 'kegon argument failure')",
                ),
                (
                    "command resolution failure",
                    "nonexistent_kegon_command_784392",
                ),
                ("compound success", "cmd.exe /c exit 0; Write-Output done"),
                (
                    "compound failure",
                    "cmd.exe /c exit 0; Write-Error bad -ErrorAction Ignore",
                ),
                (
                    "failure followed by success",
                    "cmd.exe /c exit 42; Write-Output done",
                ),
                ("quoted native invocation", "& \"cmd.exe\" /c exit 17"),
                ("dynamic invocation", "$exe = 'cmd.exe'; & $exe /c exit 42"),
                ("pipeline", "cmd.exe /c exit 42 | Out-Null"),
                ("cmdlet failure", "Get-Item NonExistentPath_Test123"),
            ];
            for (context, command) in cases {
                session.write_input(format!("{command}\r").into_bytes());
                expect(Osc133Marker::CommandStarted, context);
                expect(Osc133Marker::CommandFinished(None), context);
                expect(Osc133Marker::PromptStarted, context);
                expect(Osc133Marker::InputStarted, context);
            }

            // Empty input must not create a command. Accept either no redraw or A/B.
            session.write_input(b"\r".to_vec());
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Osc133Marker::PromptStarted) => {
                    expect(Osc133Marker::InputStarted, "empty input");
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                other => panic!("empty input emitted a command marker: {other:?}"),
            }
        }

        // OSC 133 must not leak into the visible terminal grid.
        let text = {
            let term = session.term().lock();
            let grid = term.grid();
            (0..grid.screen_lines() as i32)
                .map(|l| {
                    (0..grid.columns())
                        .map(|c| {
                            grid[alacritty_terminal::index::Point::new(
                                alacritty_terminal::index::Line(l),
                                alacritty_terminal::index::Column(c),
                            )]
                            .c
                        })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(
            !text.contains("]133;")
                && !text.contains("133;A")
                && !text.contains("133;B")
                && !text.contains("133;C")
                && !text.contains("133;D"),
            "{text}"
        );
        // Drop also runs on any failed assertion/timeout; existing PTY shutdown and
        // the test binary's bounded cleanup wait reap the child.
        drop(session);
        let _ = std::fs::remove_dir_all(&start);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs cmd.exe and ConPTY"]
    fn cmd_reports_its_directory() {
        let program = crate::terminal::shell::installed::cmd().expect("cmd is not installed");
        assert_shell_reports_directories(program, |target| {
            format!("cd /d \"{}\"", target.display())
        });
    }

    /// Starts an integrated shell, runs `setup`, then `cd`s into a fresh directory and reports
    /// whether that directory was ever reported. Used to pin down known staleness limits.
    #[cfg(windows)]
    fn reports_after_setup(
        program: String,
        setup: &str,
        cd: impl Fn(&std::path::Path) -> String,
    ) -> bool {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let start = scratch_dir("stale-start");
        let target = scratch_dir("stale-target");
        let (tx, rx) = mpsc::channel();
        let sink = EventSink::from_fn(move |event| {
            if let TerminalEvent::WorkingDirectory(path) = event {
                let _ = tx.send(path);
            }
        });
        let launch = Launch::new(
            crate::terminal::shell::ShellConfig {
                program,
                args: vec![],
            },
            true,
            &local_host_name(),
            None,
        );
        let session =
            TerminalSession::spawn_launch(launch, 100, 30, 10, 20, sink, Some(start.clone()))
                .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            assert!(Instant::now() < deadline, "no initial report");
            if rx
                .recv_timeout(Duration::from_millis(200))
                .is_ok_and(|p| same_dir(&p, &start))
            {
                break;
            }
        }
        session.write_input(format!("{setup}\r").into_bytes());
        std::thread::sleep(Duration::from_millis(500));
        session.write_input(format!("{}\r", cd(&target)).into_bytes());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut reported = false;
        while Instant::now() < deadline && !reported {
            reported = rx
                .recv_timeout(Duration::from_millis(200))
                .is_ok_and(|p| same_dir(&p, &target));
        }
        let _ = std::fs::remove_dir_all(&start);
        let _ = std::fs::remove_dir_all(&target);
        reported
    }

    /// Characterizes a known limit (see docs/architecture/terminal.md): once the user replaces
    /// the prompt, the shell stops reporting, so the last report goes stale.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs PowerShell 7 and ConPTY; takes several seconds"]
    fn redefining_the_powershell_prompt_stops_reports() {
        let program = crate::terminal::shell::installed::pwsh().expect("pwsh is not installed");
        assert!(!reports_after_setup(
            program,
            "function prompt { 'plain> ' }",
            powershell_cd
        ));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs cmd.exe and ConPTY; takes several seconds"]
    fn changing_the_cmd_prompt_stops_reports() {
        let program = crate::terminal::shell::installed::cmd().expect("cmd is not installed");
        assert!(!reports_after_setup(program, "prompt $G", |target| {
            format!("cd /d \"{}\"", target.display())
        }));
    }

    /// Prints how long sessions take to absorb a large output through the whole stack (ConPTY,
    /// `TappedPty`, event loop, emulator) with one terminal, then four at once.
    ///
    /// A measurement, not a speed assertion: it only fails if the output never finishes.
    /// Run with `cargo test --release end_to_end_output -- --ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore = "benchmark; spawns real shells and prints timings"]
    fn end_to_end_output_throughput() {
        use std::time::{Duration, Instant};

        let dir = scratch_dir("e2e");
        let file = dir.join("big.txt");
        let line = "line 0123456789 \x1b[32mgreen\x1b[0m the quick brown fox 日本語の全角文字\r\n";
        let body = line.repeat(8 * 1024 * 1024 / line.len());
        std::fs::write(&file, &body).unwrap();
        let megabytes = body.len() as f64 / 1_048_576.0;

        let run = |sessions: usize| -> Duration {
            let (tx, _rx) = iced::futures::channel::mpsc::unbounded();
            let started = Instant::now();
            let all: Vec<TerminalSession> = (0..sessions)
                .map(|_| {
                    let launch = Launch::new(
                        crate::terminal::shell::ShellConfig {
                            program: crate::terminal::shell::installed::cmd().unwrap(),
                            args: vec![],
                        },
                        true,
                        &local_host_name(),
                        None,
                    );
                    TerminalSession::spawn_launch(launch, 120, 40, 10, 20, tx.clone(), None)
                        .unwrap()
                })
                .collect();
            for session in &all {
                session.write_input(
                    format!("type \"{}\" & echo KEGONDONE\r", file.display()).into_bytes(),
                );
            }
            let deadline = Instant::now() + Duration::from_secs(180);
            for session in &all {
                while !marker_printed(session) {
                    assert!(Instant::now() < deadline, "output never finished");
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            started.elapsed()
        };

        // The first run also warms the file cache and the shell start-up.
        let _ = run(1);
        for sessions in [1, 1, 4] {
            let elapsed = run(sessions);
            println!(
                "{sessions} session(s): {:.2?} for {megabytes:.1} MiB each ({:.1} MiB/s per session)",
                elapsed,
                megabytes / elapsed.as_secs_f64()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Kitty keyboard negotiation as AGY performs it: push flag 1, query, pop. The flags are
    /// independent, so flag 1 alone must not turn on event types or all-keys-as-escape.
    #[test]
    fn kitty_keyboard_flags_are_negotiated_and_restored() {
        use alacritty_terminal::vte::ansi::Processor;

        let (proxy, pty_rx, _) = proxy_with_pty_channel();
        let size = TermSize {
            cols: 80,
            lines: 24,
        };
        let term = FairMutex::new(Term::new(term_config(), &size, proxy));
        let mut parser: Processor = Processor::new();
        let mut guard = term.lock();

        assert!(!guard.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));

        parser.advance(&mut *guard, b"\x1b[>1u\x1b[?u");
        let mode = *guard.mode();
        assert!(mode.contains(TermMode::DISAMBIGUATE_ESC_CODES));
        assert!(!mode.contains(TermMode::REPORT_EVENT_TYPES));
        assert!(!mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC));
        assert!(!mode.contains(TermMode::REPORT_ALTERNATE_KEYS));
        assert!(!mode.contains(TermMode::REPORT_ASSOCIATED_TEXT));
        assert_eq!(drain(&pty_rx), vec![b"\x1b[?1u".to_vec()]);

        parser.advance(&mut *guard, b"\x1b[<1u");
        assert!(!guard.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        parser.advance(&mut *guard, b"\x1b[?u");
        assert_eq!(drain(&pty_rx), vec![b"\x1b[?0u".to_vec()]);
    }

    /// True once `echo KEGONDONE` has produced its output line, i.e. the marker appears on a line
    /// that does not also contain the command text.
    #[cfg(windows)]
    fn marker_printed(session: &TerminalSession) -> bool {
        use alacritty_terminal::index::{Column, Line, Point};
        let term = session.term().lock();
        let grid = term.grid();
        (0..grid.screen_lines() as i32).any(|line| {
            let text: String = (0..grid.columns())
                .map(|col| grid[Point::new(Line(line), Column(col))].c)
                .collect();
            text.trim() == "KEGONDONE"
        })
    }
}
