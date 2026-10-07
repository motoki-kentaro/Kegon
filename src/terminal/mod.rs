//! Kegon terminal subsystem.
//!
//! Encapsulates terminal emulation, PTY process management, keyboard input translation,
//! clipboard operations, and UI canvas rendering behind a Kegon-owned abstraction boundary.

pub mod clipboard;
pub mod input;
pub mod renderer;
pub mod session;
pub mod shell;

pub use clipboard::SystemClipboard;
pub use input::{InputAction, format_paste, process_key_event};
pub use renderer::{DEFAULT_CELL_HEIGHT, DEFAULT_CELL_WIDTH, TerminalProgram, calculate_grid_size};
pub use session::{TerminalEvent, TerminalSession};
#[allow(unused_imports)]
pub use shell::ShellConfig;
