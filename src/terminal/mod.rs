//! Kegon terminal subsystem.
//!
//! Encapsulates terminal emulation, PTY process management, keyboard input translation,
//! clipboard operations, and UI canvas rendering behind a Kegon-owned abstraction boundary.

#![allow(unused_imports)]

pub mod arbitration;
pub mod clipboard;
pub mod input;
pub mod input_event;
pub mod key_encoder;
pub mod mode;
pub mod renderer;
pub mod session;
pub mod shell;
pub mod surface;

pub use arbitration::{InputArbiter, InputRoute};
pub use clipboard::SystemClipboard;
pub use input::{InputAction, format_paste, process_key_event};
pub use input_event::{KeyEventKind, TerminalInputEvent};
pub use key_encoder::TerminalKeyEncoder;
pub use mode::TerminalKeyboardMode;
pub use renderer::{DEFAULT_CELL_HEIGHT, DEFAULT_CELL_WIDTH, TerminalProgram, calculate_grid_size};
pub use session::{TerminalEvent, TerminalSession};
pub use shell::ShellConfig;
pub use surface::{TerminalSurface, calculate_cursor_rectangle};
