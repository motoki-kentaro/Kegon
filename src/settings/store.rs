//! Loading, parsing, and atomic saving of `settings.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use super::model::ApplicationSettings;
use super::path::default_settings_file_path;

/// Loads settings from the specified path or standard OS config directory.
///
/// Behavior for missing or invalid files:
/// - Missing file: returns default `ApplicationSettings` (System) and `None` diagnostic.
/// - Valid file: returns parsed `ApplicationSettings` and `None` diagnostic.
/// - Malformed file: returns default `ApplicationSettings` and `Some(warning_message)`.
///   Crucially, the broken source file is **never** overwritten or deleted on startup.
pub fn load_settings(path: Option<&Path>) -> (ApplicationSettings, Option<String>) {
    let target_path = match path {
        Some(p) => p.to_path_buf(),
        None => match default_settings_file_path() {
            Some(p) => p,
            None => return (ApplicationSettings::default(), None),
        },
    };

    if !target_path.exists() {
        return (ApplicationSettings::default(), None);
    }

    match fs::read_to_string(&target_path) {
        Ok(content) => match toml::from_str::<ApplicationSettings>(&content) {
            Ok(settings) => (settings, None),
            Err(err) => {
                let warning = format!(
                    "failed to parse settings file at {}: {}; using safe defaults",
                    target_path.display(),
                    err
                );
                (ApplicationSettings::default(), Some(warning))
            }
        },
        Err(err) => {
            let warning = format!(
                "failed to read settings file at {}: {}; using safe defaults",
                target_path.display(),
                err
            );
            (ApplicationSettings::default(), Some(warning))
        }
    }
}

/// Atomically saves `ApplicationSettings` to the specified path or standard OS config directory.
///
/// Creates parent directories if missing. Uses atomic file replacement (writing to `.tmp`
/// first and renaming) to prevent corrupted files on crash.
pub fn save_settings(settings: &ApplicationSettings, path: Option<&Path>) -> Result<(), String> {
    let target_path: PathBuf = match path {
        Some(p) => p.to_path_buf(),
        None => default_settings_file_path()
            .ok_or_else(|| String::from("unable to determine user configuration directory"))?,
    };

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create settings directory {}: {e}",
                parent.display()
            )
        })?;
    }

    let toml_string = toml::to_string_pretty(settings)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;

    let tmp_path = target_path.with_extension("toml.tmp");

    fs::write(&tmp_path, &toml_string).map_err(|e| {
        format!(
            "failed to write temp settings file {}: {e}",
            tmp_path.display()
        )
    })?;

    if let Err(e) = fs::rename(&tmp_path, &target_path) {
        fs::write(&target_path, &toml_string).map_err(|e2| {
            format!(
                "failed to replace settings file {}: {e} (fallback error: {e2})",
                target_path.display()
            )
        })?;
        let _ = fs::remove_file(&tmp_path);
    }

    Ok(())
}
