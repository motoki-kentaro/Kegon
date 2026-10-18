//! Typed application settings model.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::numeric::{BoolSetting, ExtraKeys, NumericSetting};
use crate::theme::ThemeId;

/// Supported user preferences for application UI language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LocalePreference {
    /// Follow the OS preferred UI language, falling back to en-US.
    #[default]
    #[serde(rename = "system")]
    System,
    /// Explicitly request English (United States).
    #[serde(rename = "en-US")]
    EnUs,
    /// Explicitly request Japanese.
    #[serde(rename = "ja-JP")]
    JaJp,
}

impl LocalePreference {
    /// All available preference options in display order.
    #[allow(dead_code)]
    pub const ALL: [Self; 3] = [Self::System, Self::EnUs, Self::JaJp];

    /// Returns the stable string identifier stored in settings.toml.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::EnUs => "en-US",
            Self::JaJp => "ja-JP",
        }
    }
}

impl std::fmt::Display for LocalePreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The persisted theme choice.
///
/// Stored as the theme's stable ID string. An ID this build does not know
/// (from a newer version or a hand edit) is kept verbatim, so saving other
/// settings never rewrites it; at runtime it falls back to the default theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemePreference {
    Builtin(ThemeId),
    Unknown(String),
}

impl ThemePreference {
    pub fn from_persisted(id: &str) -> Self {
        ThemeId::from_persisted(id).map_or_else(|| Self::Unknown(id.to_owned()), Self::Builtin)
    }

    pub fn as_persisted(&self) -> &str {
        match self {
            Self::Builtin(id) => id.as_str(),
            Self::Unknown(id) => id,
        }
    }

    /// The theme to use: the configured one, or the default if it is unknown.
    pub fn effective_id(&self) -> ThemeId {
        match self {
            Self::Builtin(id) => *id,
            Self::Unknown(_) => ThemeId::DEFAULT,
        }
    }

    /// A warning to show when the configured theme is unknown.
    pub fn diagnostic(&self) -> Option<String> {
        match self {
            Self::Builtin(_) => None,
            Self::Unknown(id) => Some(format!(
                "unknown theme {id:?} in settings; using {} (the setting is kept as is)",
                ThemeId::DEFAULT.as_str()
            )),
        }
    }
}

impl Default for ThemePreference {
    fn default() -> Self {
        Self::Builtin(ThemeId::DEFAULT)
    }
}

impl Serialize for ThemePreference {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_persisted())
    }
}

impl<'de> Deserialize<'de> for ThemePreference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self::from_persisted(&id))
    }
}

/// User preferences for visual appearance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppearanceSettings {
    /// Color theme. Missing in files written before themes existed.
    #[serde(default)]
    pub theme: ThemePreference,
    /// Preferred UI font family name (e.g. "Noto Sans JP", "Segoe UI").
    /// `None` indicates system default font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font_family: Option<String>,
    /// Preferred Terminal font family name (e.g. "Cascadia Mono", "Consolas").
    /// `None` indicates system default monospaced font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_font_family: Option<String>,
    /// Terminal font size in points. `None` keeps the built-in 13 logical px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_font_size: Option<NumericSetting>,
    /// Terminal cell height as a multiple of the font size in px. `None` follows the font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cell_height: Option<NumericSetting>,
    /// Terminal cell width as a multiple of the font size in px. `None` follows the font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cell_width: Option<NumericSetting>,
    /// Draw the Unicode box-drawing and block characters (U+2500..U+259F) with built-in shapes instead of
    /// the font. `None`, and any value that is not a boolean, means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_builtin_glyphs: Option<BoolSetting>,
    /// Keys of `[appearance]` this build does not know; written back untouched.
    #[serde(flatten)]
    pub extra: ExtraKeys,
}

/// Allowed range of `terminal_font_size` (points).
pub const TERMINAL_FONT_SIZE_RANGE: RangeInclusive<f64> = RangeInclusive::new(1.0, 100.0);
/// Allowed range of `terminal_cell_height` (multiple of the font size).
pub const TERMINAL_CELL_HEIGHT_RANGE: RangeInclusive<f64> = RangeInclusive::new(0.5, 3.0);
/// Allowed range of `terminal_cell_width` (multiple of the font size).
pub const TERMINAL_CELL_WIDTH_RANGE: RangeInclusive<f64> = RangeInclusive::new(0.2, 2.0);

/// One of the three terminal display metrics a user can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalMetric {
    FontSize,
    CellHeight,
    CellWidth,
}

impl TerminalMetric {
    pub const ALL: [Self; 3] = [Self::FontSize, Self::CellHeight, Self::CellWidth];

    pub fn range(self) -> RangeInclusive<f64> {
        match self {
            Self::FontSize => TERMINAL_FONT_SIZE_RANGE,
            Self::CellHeight => TERMINAL_CELL_HEIGHT_RANGE,
            Self::CellWidth => TERMINAL_CELL_WIDTH_RANGE,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::FontSize => "terminal_font_size",
            Self::CellHeight => "terminal_cell_height",
            Self::CellWidth => "terminal_cell_width",
        }
    }
}

impl AppearanceSettings {
    /// Whether the built-in glyphs are used: on unless the file explicitly says `false`. A value of the
    /// wrong type counts as on.
    pub fn builtin_glyphs(&self) -> bool {
        self.terminal_builtin_glyphs
            .as_ref()
            .and_then(BoolSetting::effective)
            .unwrap_or(true)
    }

    /// Records the user's choice, replacing whatever was stored, including a value of the wrong type.
    pub fn set_builtin_glyphs(&mut self, enabled: bool) {
        self.terminal_builtin_glyphs = Some(BoolSetting::Value(enabled));
    }

    /// Removes the key: the built-in glyphs go back to their default (on).
    pub fn clear_builtin_glyphs(&mut self) {
        self.terminal_builtin_glyphs = None;
    }

    /// Whether the file has a key for the built-in glyphs at all (valid or not).
    pub fn has_builtin_glyphs_key(&self) -> bool {
        self.terminal_builtin_glyphs.is_some()
    }

    /// The stored value when it is not a boolean and is therefore ignored.
    pub fn invalid_builtin_glyphs_text(&self) -> Option<String> {
        self.terminal_builtin_glyphs
            .as_ref()
            .filter(|setting| setting.effective().is_none())
            .map(BoolSetting::describe)
    }

    fn metric_slot(&self, metric: TerminalMetric) -> &Option<NumericSetting> {
        match metric {
            TerminalMetric::FontSize => &self.terminal_font_size,
            TerminalMetric::CellHeight => &self.terminal_cell_height,
            TerminalMetric::CellWidth => &self.terminal_cell_width,
        }
    }

    /// Sets (or, with `None`, clears) one metric. `Some(v)` replaces whatever was stored, including
    /// an invalid value.
    pub fn set_metric(&mut self, metric: TerminalMetric, value: Option<f64>) {
        let slot = match metric {
            TerminalMetric::FontSize => &mut self.terminal_font_size,
            TerminalMetric::CellHeight => &mut self.terminal_cell_height,
            TerminalMetric::CellWidth => &mut self.terminal_cell_width,
        };
        *slot = value.map(NumericSetting::from_number);
    }

    /// The usable value of a metric: set, finite, and inside its range.
    pub fn metric(&self, metric: TerminalMetric) -> Option<f64> {
        self.metric_slot(metric)
            .as_ref()
            .and_then(|setting| setting.effective(&metric.range()))
    }

    /// Whether the file has a key for this metric at all (valid or not).
    pub fn has_metric_key(&self, metric: TerminalMetric) -> bool {
        self.metric_slot(metric).is_some()
    }

    /// Whether the file holds a value for this metric that is being ignored.
    pub fn metric_is_invalid(&self, metric: TerminalMetric) -> bool {
        self.metric_slot(metric).is_some() && self.metric(metric).is_none()
    }

    /// The raw stored value of an ignored metric, for the notice.
    pub fn invalid_metric_text(&self, metric: TerminalMetric) -> Option<String> {
        self.metric_is_invalid(metric)
            .then(|| {
                self.metric_slot(metric)
                    .as_ref()
                    .map(NumericSetting::describe)
            })
            .flatten()
    }

    /// Warnings for every ignored metric value (the file is left as it is).
    pub fn metric_diagnostics(&self) -> Vec<String> {
        let builtin = self.invalid_builtin_glyphs_text().map(|raw| {
            format!(
                "invalid terminal_builtin_glyphs = {raw} in settings (expected true or false); \
                 treating it as true (the setting is kept as is)"
            )
        });
        builtin
            .into_iter()
            .chain(TerminalMetric::ALL.into_iter().filter_map(|metric| {
                let raw = self.invalid_metric_text(metric)?;
                let range = metric.range();
                Some(format!(
                    "invalid {} = {raw} in settings (expected a number from {} to {}); \
                     ignoring it (the setting is kept as is)",
                    metric.key(),
                    range.start(),
                    range.end()
                ))
            }))
            .collect()
    }
}

/// Validation errors when verifying an explicit custom shell path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPathError {
    Empty,
    NotAbsolute,
    NotFound,
    IsDirectory,
    NotExecutable,
    HasArguments,
}

/// Validates an explicit custom shell executable path.
///
/// Acceptance criteria:
/// - Must not be empty.
/// - Must not contain command-line arguments or quotation marks.
/// - Must be an absolute path with drive letter/prefix on Windows.
/// - Must end with `.exe` (case-insensitive).
/// - Must exist on disk.
/// - Must be a regular file, not a directory.
pub fn validate_shell_path(raw: &str) -> Result<PathBuf, ShellPathError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ShellPathError::Empty);
    }

    if trimmed.contains('"') || trimmed.contains('\'') {
        return Err(ShellPathError::HasArguments);
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(pos) = lower.find(".exe") {
        let remainder = &trimmed[pos + 4..];
        if remainder.starts_with(|c: char| c.is_whitespace()) && !remainder.trim().is_empty() {
            return Err(ShellPathError::HasArguments);
        }
    }

    let path = Path::new(trimmed);

    #[cfg(target_os = "windows")]
    {
        use std::path::Component;
        let mut components = path.components();
        let is_valid_abs = matches!(components.next(), Some(Component::Prefix(_)))
            && matches!(components.next(), Some(Component::RootDir));
        if !is_valid_abs {
            return Err(ShellPathError::NotAbsolute);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if !path.is_absolute() {
            return Err(ShellPathError::NotAbsolute);
        }
    }

    if path.is_dir() {
        return Err(ShellPathError::IsDirectory);
    }

    let ext_is_exe = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("exe"))
        == Some(true);

    if !ext_is_exe {
        return Err(ShellPathError::NotExecutable);
    }

    if !path.exists() {
        return Err(ShellPathError::NotFound);
    }

    if !path.is_file() {
        return Err(ShellPathError::NotExecutable);
    }

    Ok(path.to_path_buf())
}

/// Selection mode for which shell executable to run in terminal sessions.
#[derive(Debug, Clone, Default)]
pub enum ShellMode {
    /// Auto-detect the best available shell (pwsh -> powershell -> cmd).
    #[default]
    Auto,
    /// Explicitly run a user-specified executable path.
    Custom,
    /// Unknown value in settings.toml; kept as-is and reported as diagnostic.
    Unknown(toml::Value),
}

impl PartialEq for ShellMode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Auto, Self::Auto) => true,
            (Self::Custom, Self::Custom) => true,
            (Self::Unknown(a), Self::Unknown(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ShellMode {}

impl ShellMode {
    #[allow(dead_code)]
    pub const ALL: [Self; 2] = [Self::Auto, Self::Custom];

    pub fn diagnostic(&self) -> Option<String> {
        match self {
            Self::Auto | Self::Custom => None,
            Self::Unknown(raw) => Some(format!(
                "unknown terminal.shell_mode = {raw} in settings (expected \"auto\" or \"custom\"); \
                 the setting is kept as is"
            )),
        }
    }
}

impl Serialize for ShellMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Custom => serializer.serialize_str("custom"),
            Self::Unknown(val) => val.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ShellMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let val = toml::Value::deserialize(deserializer)?;
        Ok(match val {
            toml::Value::String(ref s) if s == "auto" => Self::Auto,
            toml::Value::String(ref s) if s == "custom" => Self::Custom,
            other => Self::Unknown(other),
        })
    }
}

/// Mode for Shell Integration (OSC 133 semantic prompt marking).
#[derive(Debug, Clone, Default)]
pub enum ShellIntegrationMode {
    /// Automatically inject Shell Integration in supported shells (pwsh, powershell).
    #[default]
    Auto,
    /// Disable Shell Integration for all shells.
    Disabled,
    /// Unknown value in settings.toml; kept as-is and reported as diagnostic.
    Unknown(toml::Value),
}

impl PartialEq for ShellIntegrationMode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Auto, Self::Auto) => true,
            (Self::Disabled, Self::Disabled) => true,
            (Self::Unknown(a), Self::Unknown(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ShellIntegrationMode {}

impl ShellIntegrationMode {
    #[allow(dead_code)]
    pub const ALL: [Self; 2] = [Self::Auto, Self::Disabled];

    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Auto)
    }

    pub fn diagnostic(&self) -> Option<String> {
        match self {
            Self::Auto | Self::Disabled => None,
            Self::Unknown(raw) => Some(format!(
                "unknown terminal.shell_integration = {raw} in settings (expected \"auto\" or \"disabled\"); \
                 the setting is kept as is"
            )),
        }
    }
}

impl Serialize for ShellIntegrationMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Disabled => serializer.serialize_str("disabled"),
            Self::Unknown(val) => val.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ShellIntegrationMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let val = toml::Value::deserialize(deserializer)?;
        Ok(match val {
            toml::Value::String(ref s) if s == "auto" => Self::Auto,
            toml::Value::String(ref s) if s == "disabled" => Self::Disabled,
            other => Self::Unknown(other),
        })
    }
}

/// User preferences for terminal shell launching and integration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalSettings {
    /// Shell selection mode ("auto" or "custom").
    #[serde(default)]
    pub shell_mode: ShellMode,
    /// Explicit executable path when `shell_mode` is `custom`.
    #[serde(default)]
    pub shell_path: String,
    /// Shell Integration mode ("auto" or "disabled").
    #[serde(default)]
    pub shell_integration: ShellIntegrationMode,
    /// Keys of `[terminal]` this build does not know; written back untouched.
    #[serde(flatten)]
    pub extra: ExtraKeys,
}

impl TerminalSettings {
    pub fn diagnostics(&self) -> Vec<String> {
        let mut list = Vec::new();
        if let Some(d) = self.shell_mode.diagnostic() {
            list.push(d);
        }
        if let Some(d) = self.shell_integration.diagnostic() {
            list.push(d);
        }
        if self.shell_mode == ShellMode::Custom
            && let Err(err) = validate_shell_path(&self.shell_path)
        {
            list.push(format!(
                "invalid terminal.shell_path {:?} in settings: {:?}",
                self.shell_path, err
            ));
        }
        list
    }
}

/// Typed model holding Kegon application-level configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ApplicationSettings {
    /// Preferred UI language.
    #[serde(default)]
    pub locale: LocalePreference,
    /// Preferred visual appearance.
    #[serde(default)]
    pub appearance: AppearanceSettings,
    /// Terminal shell and integration settings.
    #[serde(default)]
    pub terminal: TerminalSettings,
    /// Top-level keys this build does not know; written back untouched.
    #[serde(flatten)]
    pub extra: ExtraKeys,
}
