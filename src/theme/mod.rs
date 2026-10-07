//! Kegon's color themes.
//!
//! A [`KegonTheme`] owns every color Kegon itself chooses. The application
//! resolves the configured [`ThemeId`] once into an active theme and passes it
//! to the workbench, dialogs, and terminal renderer. See
//! `docs/architecture/themes.md`.

mod model;
mod night_dark;
mod registry;
pub mod style;

pub use model::{KegonTheme, TerminalColors, ThemeId};
#[cfg(test)]
pub use night_dark::NIGHT_DARK;
pub use registry::resolve_theme;
