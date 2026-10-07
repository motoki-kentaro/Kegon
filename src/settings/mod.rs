//! Application settings model, persistence, and precedence resolution.

mod model;
mod path;
mod precedence;
mod store;

#[allow(unused_imports)]
pub use model::{AppearanceSettings, ApplicationSettings, LocalePreference, ThemePreference};
#[allow(unused_imports)]
pub use path::{default_settings_dir, default_settings_file_path};
pub use precedence::resolve_application_locale;
pub use store::{load_settings, save_settings};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use crate::theme::ThemeId;
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
                ..AppearanceSettings::default()
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

    fn load_str(prefix: &str, content: &str) -> (ApplicationSettings, Option<String>) {
        let temp_dir = tempfile_dir(prefix);
        let path = temp_dir.join("settings.toml");
        fs::write(&path, content).unwrap();
        let result = load_settings(Some(&path));
        let _ = fs::remove_dir_all(temp_dir);
        result
    }

    #[test]
    fn default_theme_is_night_dark() {
        let settings = ApplicationSettings::default();
        assert_eq!(
            settings.appearance.theme,
            ThemePreference::Builtin(ThemeId::NightDark)
        );
    }

    #[test]
    fn locale_only_settings_default_to_night_dark() {
        let (settings, warning) = load_str("theme_locale_only", "locale = \"ja-JP\"\n");
        assert!(warning.is_none());
        assert_eq!(settings.locale, LocalePreference::JaJp);
        assert_eq!(settings.appearance.theme, ThemePreference::default());
    }

    #[test]
    fn locale_and_ui_font_settings_default_to_night_dark_and_keep_the_font() {
        let content = "locale = \"ja-JP\"\n\n[appearance]\nui_font_family = \"Yu Gothic UI\"\n";
        let (settings, warning) = load_str("theme_locale_font", content);
        assert!(warning.is_none());
        assert_eq!(settings.locale, LocalePreference::JaJp);
        assert_eq!(settings.appearance.theme.effective_id(), ThemeId::NightDark);
        assert_eq!(
            settings.appearance.ui_font_family.as_deref(),
            Some("Yu Gothic UI")
        );
    }

    #[test]
    fn explicit_night_dark_round_trips() {
        let content = "locale = \"en-US\"\n\n[appearance]\ntheme = \"night-dark\"\n";
        let (settings, warning) = load_str("theme_explicit", content);
        assert!(warning.is_none());
        assert_eq!(
            settings.appearance.theme,
            ThemePreference::Builtin(ThemeId::NightDark)
        );

        let serialized = toml::to_string_pretty(&settings).unwrap();
        assert!(
            serialized.contains("theme = \"night-dark\""),
            "{serialized}"
        );
        let reparsed: ApplicationSettings = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed, settings);
    }

    #[test]
    fn theme_ui_font_and_locale_are_independent() {
        let content = "locale = \"ja-JP\"\n\n[appearance]\ntheme = \"night-dark\"\nui_font_family = \"Yu Gothic UI\"\n";
        let (mut settings, _) = load_str("theme_independent", content);

        settings.appearance.ui_font_family = None;
        assert_eq!(settings.appearance.theme.effective_id(), ThemeId::NightDark);
        assert_eq!(settings.locale, LocalePreference::JaJp);

        settings.locale = LocalePreference::EnUs;
        assert_eq!(settings.appearance.theme.effective_id(), ThemeId::NightDark);

        settings.appearance.theme = ThemePreference::Builtin(ThemeId::NightDark);
        settings.appearance.ui_font_family = Some("Segoe UI".into());
        assert_eq!(settings.locale, LocalePreference::EnUs);

        let serialized = toml::to_string_pretty(&settings).unwrap();
        assert!(serialized.contains("theme = \"night-dark\""));
        assert!(serialized.contains("ui_font_family = \"Segoe UI\""));
        assert!(serialized.contains("locale = \"en-US\""));
    }

    #[test]
    fn unknown_theme_falls_back_without_losing_other_settings() {
        let content = "locale = \"ja-JP\"\n\n[appearance]\ntheme = \"future-theme\"\nui_font_family = \"Yu Gothic UI\"\n";
        let (settings, warning) = load_str("theme_unknown", content);

        // Not a parse error: the rest of the file still applies.
        assert!(warning.is_none());
        assert_eq!(settings.locale, LocalePreference::JaJp);
        assert_eq!(
            settings.appearance.ui_font_family.as_deref(),
            Some("Yu Gothic UI")
        );
        assert_eq!(
            settings.appearance.theme,
            ThemePreference::Unknown("future-theme".into())
        );
        assert_eq!(settings.appearance.theme.effective_id(), ThemeId::NightDark);
        assert!(settings.appearance.theme.diagnostic().is_some());
        assert!(ThemePreference::default().diagnostic().is_none());
    }

    #[test]
    fn unknown_theme_is_preserved_on_disk() {
        let temp_dir = tempfile_dir("theme_unknown_preserved");
        let path = temp_dir.join("settings.toml");
        let content = "locale = \"ja-JP\"\n\n[appearance]\ntheme = \"future-theme\"\n";
        fs::write(&path, content).unwrap();

        // Loading never rewrites the file.
        let (mut settings, _) = load_settings(Some(&path));
        assert_eq!(fs::read_to_string(&path).unwrap(), content);

        // Saving an unrelated change keeps the unknown theme ID verbatim.
        settings.appearance.ui_font_family = Some("Segoe UI".into());
        save_settings(&settings, Some(&path)).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("theme = \"future-theme\""), "{saved}");

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn non_string_theme_follows_the_malformed_file_policy() {
        let temp_dir = tempfile_dir("theme_malformed");
        let path = temp_dir.join("settings.toml");
        let content = "[appearance]\ntheme = 5\n";
        fs::write(&path, content).unwrap();

        let (settings, warning) = load_settings(Some(&path));
        assert_eq!(settings, ApplicationSettings::default());
        assert!(warning.is_some());
        assert_eq!(fs::read_to_string(&path).unwrap(), content);

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
