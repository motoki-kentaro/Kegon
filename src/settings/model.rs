//! Typed application settings model.

use serde::{Deserialize, Serialize};

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
}
