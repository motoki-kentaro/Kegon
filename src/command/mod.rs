//! Command and keybinding infrastructure for Kegon.

#![allow(unused_imports)]

pub mod command_id;
pub mod context;
pub mod dispatcher;
pub mod key_chord;
pub mod keymap;
pub mod platform;
pub mod resolver;

pub use command_id::{CommandId, ParseCommandIdError};
pub use context::CommandContext;
pub use dispatcher::{CommandDispatcher, CommandOutcome};
pub use key_chord::{Key, KeyChord, Modifiers, NamedKey};
pub use keymap::{KeybindingRule, Keymap};
pub use platform::Platform;
pub use resolver::{KeybindingConflictError, KeybindingResolver};
