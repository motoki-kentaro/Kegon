//! Application settings model, persistence, and precedence resolution.

mod model;
mod path;
mod precedence;
mod store;

#[allow(unused_imports)]
pub use model::{AppearanceSettings, ApplicationSettings, LocalePreference};
#[allow(unused_imports)]
pub use path::{default_settings_dir, default_settings_file_path};
pub use precedence::resolve_application_locale;
pub use store::{load_settings, save_settings};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use std::fs;

    #[test]
    fn default_locale_preference_is_system() {
        let settings = ApplicationSettings::default();
        assert_eq!(settings.locale, LocalePreference::System);
        assert_eq!(settings.appearance.ui_font_family, None);
    }

    #[test]
    fn locale_preference_serialization_round_trip() {
        let cases = [
            (LocalePreference::System, "system"),
            (LocalePreference::EnUs, "en-US"),
            (LocalePreference::JaJp, "ja-JP"),
        ];

        for (pref, expected_str) in cases {
            assert_eq!(pref.as_str(), expected_str);
            let settings = ApplicationSettings {
                locale: pref,
                appearance: AppearanceSettings::default(),
            };
            let serialized = toml::to_string_pretty(&settings).expect("should serialize");
            assert!(serialized.contains(expected_str));

            let deserialized: ApplicationSettings =
                toml::from_str(&serialized).expect("should deserialize");
            assert_eq!(deserialized.locale, pref);
            assert_eq!(deserialized.appearance.ui_font_family, None);
        }
    }

    #[test]
    fn appearance_settings_serialization_round_trip() {
        let settings = ApplicationSettings {
            locale: LocalePreference::JaJp,
            appearance: AppearanceSettings {
                ui_font_family: Some("Noto Sans JP".into()),
            },
        };

        let serialized = toml::to_string_pretty(&settings).expect("should serialize");
        assert!(serialized.contains("ui_font_family = \"Noto Sans JP\""));

        let deserialized: ApplicationSettings =
            toml::from_str(&serialized).expect("should deserialize");
        assert_eq!(deserialized.locale, LocalePreference::JaJp);
        assert_eq!(
            deserialized.appearance.ui_font_family.as_deref(),
            Some("Noto Sans JP")
        );
    }

    #[test]
    fn legacy_locale_only_toml_loads_with_default_appearance() {
        let temp_dir = tempfile_dir("legacy_toml");
        let path = temp_dir.join("settings.toml");
        let content = "locale = \"ja-JP\"\n";
        fs::write(&path, content).unwrap();

        let (settings, warning) = load_settings(Some(&path));
        assert_eq!(settings.locale, LocalePreference::JaJp);
        assert_eq!(settings.appearance.ui_font_family, None);
        assert!(warning.is_none());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn missing_file_loads_safe_default() {
        let temp_dir = tempfile_dir("missing_file");
        let path = temp_dir.join("nonexistent_settings.toml");
        let (settings, warning) = load_settings(Some(&path));
        assert_eq!(settings, ApplicationSettings::default());
        assert!(warning.is_none());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn valid_toml_loads_correctly() {
        let temp_dir = tempfile_dir("valid_toml");
        let path = temp_dir.join("settings.toml");
        let content = "locale = \"ja-JP\"\n";
        fs::write(&path, content).unwrap();

        let (settings, warning) = load_settings(Some(&path));
        assert_eq!(settings.locale, LocalePreference::JaJp);
        assert!(warning.is_none());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn malformed_toml_returns_default_and_does_not_overwrite_file() {
        let temp_dir = tempfile_dir("malformed_toml");
        let path = temp_dir.join("settings.toml");
        let broken_content = "locale = [invalid toml syntax!!!]";
        fs::write(&path, broken_content).unwrap();

        let (settings, warning) = load_settings(Some(&path));
        assert_eq!(settings, ApplicationSettings::default());
        assert!(warning.is_some());

        // File remains broken and untouched on disk
        let disk_content = fs::read_to_string(&path).unwrap();
        assert_eq!(disk_content, broken_content);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn saving_settings_creates_file_and_parent_dir() {
        let temp_dir = tempfile_dir("save_settings");
        let path = temp_dir.join("nested").join("settings.toml");
        let settings = ApplicationSettings {
            locale: LocalePreference::EnUs,
            appearance: AppearanceSettings::default(),
        };

        assert!(save_settings(&settings, Some(&path)).is_ok());

        let (loaded, warning) = load_settings(Some(&path));
        assert_eq!(loaded.locale, LocalePreference::EnUs);
        assert!(warning.is_none());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn locale_precedence_resolution() {
        let os_ja = vec!["ja-JP".to_string()];
        let os_en = vec!["en-US".to_string()];
        let os_fr = vec!["fr-FR".to_string()];

        // 1. CLI override wins regardless of settings
        assert_eq!(
            resolve_application_locale(
                Some("ja-JP"),
                LocalePreference::EnUs,
                os_en.clone().into_iter()
            ),
            Locale::JaJp
        );
        assert_eq!(
            resolve_application_locale(
                Some("en-US"),
                LocalePreference::JaJp,
                os_ja.clone().into_iter()
            ),
            Locale::EnUs
        );

        // 2. No CLI + Explicit Settings wins
        assert_eq!(
            resolve_application_locale(None, LocalePreference::JaJp, os_en.clone().into_iter()),
            Locale::JaJp
        );
        assert_eq!(
            resolve_application_locale(None, LocalePreference::EnUs, os_ja.clone().into_iter()),
            Locale::EnUs
        );

        // 3. No CLI + System Settings delegates to OS
        assert_eq!(
            resolve_application_locale(None, LocalePreference::System, os_ja.into_iter()),
            Locale::JaJp
        );
        assert_eq!(
            resolve_application_locale(None, LocalePreference::System, os_en.into_iter()),
            Locale::EnUs
        );

        // 4. Unsupported OS falls back to en-US
        assert_eq!(
            resolve_application_locale(None, LocalePreference::System, os_fr.into_iter()),
            Locale::EnUs
        );
    }

    fn tempfile_dir(prefix: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("kegon_test_{prefix}_{}", std::process::id()));
        let _ = fs::create_dir_all(&p);
        p
    }
}
