//! Typed application settings model.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::numeric::{BoolSetting, ExtraKeys, NumericSetting};
use crate::terminal::dopamine::DopamineSoundKind;
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

/// Dopamine Mode settings model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DopamineSettings {
    /// Whether Dopamine Mode is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<BoolSetting>,
    /// Sound profile choice ("zap", "crack", "burst"); anything else, including the removed recorded profiles, means the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    /// Keys in [dopamine] this build does not recognize; written back untouched.
    #[serde(flatten)]
    pub extra: ExtraKeys,
}

impl DopamineSettings {
    pub const DEFAULT_ENABLED: bool = false;
    pub const DEFAULT_SOUND: DopamineSoundKind = DopamineSoundKind::Zap;

    /// Whether Dopamine Mode is effectively enabled (unset defaults to false).
    pub fn enabled(&self) -> bool {
        self.enabled
            .as_ref()
            .and_then(BoolSetting::effective)
            .unwrap_or(Self::DEFAULT_ENABLED)
    }

    /// Sets the enabled state explicitly.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = Some(BoolSetting::Value(enabled));
    }

    /// Clears the explicit setting, reverting to default.
    pub fn clear_enabled(&mut self) {
        self.enabled = None;
    }

    /// Whether the settings file explicitly contains the key.
    pub fn has_enabled_key(&self) -> bool {
        self.enabled.is_some()
    }

    /// The active sound profile (defaults to Zap if unset or unrecognized).
    pub fn sound(&self) -> DopamineSoundKind {
        self.sound
            .as_deref()
            .and_then(DopamineSoundKind::from_str)
            .unwrap_or(Self::DEFAULT_SOUND)
    }

    /// Explicitly sets the sound profile.
    pub fn set_sound(&mut self, kind: DopamineSoundKind) {
        self.sound = Some(kind.as_str().to_string());
    }

    /// Clears the explicit sound setting, reverting to default.
    pub fn clear_sound(&mut self) {
        self.sound = None;
    }

    /// Whether the settings file explicitly contains the sound key.
    pub fn has_sound_key(&self) -> bool {
        self.sound.is_some()
    }

    /// The stored value when it is not a boolean and is therefore ignored.
    pub fn invalid_enabled_text(&self) -> Option<String> {
        self.enabled
            .as_ref()
            .filter(|setting| setting.effective().is_none())
            .map(BoolSetting::describe)
    }

    /// Diagnostic warning for an invalid enabled value in settings.toml.
    pub fn diagnostic(&self) -> Option<String> {
        self.invalid_enabled_text().map(|raw| {
            format!(
                "invalid enabled = {raw} in [dopamine] settings (expected true or false); \
                 treating it as false (the setting is kept as is)"
            )
        })
    }

    /// Whether this table has any keys (or extra keys) to serialize.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.sound.is_none() && self.extra.0.is_empty()
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
    /// Dopamine Mode settings.
    #[serde(default, skip_serializing_if = "DopamineSettings::is_empty")]
    pub dopamine: DopamineSettings,
    /// Top-level keys this build does not know; written back untouched.
    #[serde(flatten)]
    pub extra: ExtraKeys,
}
