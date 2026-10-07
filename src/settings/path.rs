//! Cross-platform configuration directory path resolution.

use std::path::PathBuf;

/// Returns the configuration directory for Kegon (`<config_dir>/Kegon`).
///
/// Platform resolutions:
/// - Windows: `%APPDATA%\Kegon` (e.g. `C:\Users\<user>\AppData\Roaming\Kegon`)
/// - macOS: `$HOME/Library/Application Support/Kegon`
/// - Linux: `$XDG_CONFIG_HOME/Kegon` or `$HOME/.config/Kegon`
pub fn default_settings_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Kegon"))
}

/// Returns the default settings file path (`<config_dir>/Kegon/settings.toml`).
pub fn default_settings_file_path() -> Option<PathBuf> {
    default_settings_dir().map(|dir| dir.join("settings.toml"))
}
