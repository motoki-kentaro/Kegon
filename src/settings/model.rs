//! Typed application settings model.

use serde::{Deserialize, Serialize};

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

/// Typed model holding Kegon application-level configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ApplicationSettings {
    /// Preferred UI language.
    #[serde(default)]
    pub locale: LocalePreference,
}
