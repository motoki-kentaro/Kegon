//! Application settings model, persistence, and precedence resolution.

mod model;
mod numeric;
mod path;
mod precedence;
mod store;

pub use model::{
    AppearanceSettings, ApplicationSettings, LocalePreference, ShellIntegrationMode, ShellMode,
    ShellPathError, TerminalMetric, TerminalSettings, ThemePreference, validate_shell_path,
};
#[allow(unused_imports)]
pub use numeric::{BoolSetting, NumericSetting};
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
                ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
        settings.appearance.terminal_font_family = Some("Cascadia Mono".into());
        save_settings(&settings, Some(&path)).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("theme = \"future-theme\""), "{saved}");
        assert!(
            saved.contains("terminal_font_family = \"Cascadia Mono\""),
            "{saved}"
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn missing_terminal_font_family_is_preserved_when_saving_other_settings() {
        let temp_dir = tempfile_dir("missing_terminal_font");
        let path = temp_dir.join("settings.toml");
        let content = "locale = \"ja-JP\"\n\n[appearance]\nterminal_font_family = \"Kegon Definitely Missing Font\"\n";
        fs::write(&path, content).unwrap();

        let (mut settings, warning) = load_settings(Some(&path));
        assert!(warning.is_none());
        assert_eq!(
            settings.appearance.terminal_font_family.as_deref(),
            Some("Kegon Definitely Missing Font")
        );

        settings.appearance.ui_font_family = Some("Segoe UI".into());
        save_settings(&settings, Some(&path)).unwrap();

        let (reloaded, warning) = load_settings(Some(&path));
        assert!(warning.is_none());
        assert_eq!(
            reloaded.appearance.terminal_font_family.as_deref(),
            Some("Kegon Definitely Missing Font")
        );
        assert_eq!(
            reloaded.appearance.ui_font_family.as_deref(),
            Some("Segoe UI")
        );

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

    // --- terminal metric settings (font size / cell height / cell width) ---

    fn save_and_read(settings: &ApplicationSettings, prefix: &str) -> String {
        let temp_dir = tempfile_dir(prefix);
        let path = temp_dir.join("settings.toml");
        save_settings(settings, Some(&path)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_dir_all(temp_dir);
        text
    }

    #[test]
    fn legacy_file_without_metric_keys_loads_with_all_metrics_unset() {
        let (settings, warning) = load_str(
            "metrics_legacy",
            "locale = \"en-US\"\n\n[appearance]\ntheme = \"night-dark\"\nterminal_font_family = \"Consolas\"\n",
        );
        assert!(warning.is_none());
        for metric in TerminalMetric::ALL {
            assert_eq!(settings.appearance.metric(metric), None);
            assert!(!settings.appearance.metric_is_invalid(metric));
        }
        // Saving does not invent the new keys.
        let text = save_and_read(&settings, "metrics_legacy_save");
        assert!(!text.contains("terminal_font_size"), "{text}");
        assert!(!text.contains("terminal_cell"), "{text}");
    }

    #[test]
    fn valid_metrics_round_trip_including_boundaries() {
        let mut settings = ApplicationSettings::default();
        for (metric, value) in [
            (TerminalMetric::FontSize, 1.0),
            (TerminalMetric::CellHeight, 3.0),
            (TerminalMetric::CellWidth, 0.2),
        ] {
            settings.appearance.set_metric(metric, Some(value));
        }
        let text = save_and_read(&settings, "metrics_round_trip");
        let (reloaded, warning) = load_str("metrics_round_trip_load", &text);
        assert!(warning.is_none(), "{text}");
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::FontSize),
            Some(1.0)
        );
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::CellHeight),
            Some(3.0)
        );
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::CellWidth),
            Some(0.2)
        );

        settings
            .appearance
            .set_metric(TerminalMetric::FontSize, Some(100.0));
        settings
            .appearance
            .set_metric(TerminalMetric::CellHeight, Some(0.5));
        settings
            .appearance
            .set_metric(TerminalMetric::CellWidth, Some(2.0));
        let text = save_and_read(&settings, "metrics_round_trip2");
        let (reloaded, _) = load_str("metrics_round_trip2_load", &text);
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::FontSize),
            Some(100.0)
        );
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::CellHeight),
            Some(0.5)
        );
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::CellWidth),
            Some(2.0)
        );
    }

    #[test]
    fn integers_are_accepted_as_numbers() {
        let (settings, warning) = load_str(
            "metrics_integer",
            "[appearance]\nterminal_font_size = 12\nterminal_cell_height = 1\n",
        );
        assert!(warning.is_none());
        assert_eq!(
            settings.appearance.metric(TerminalMetric::FontSize),
            Some(12.0)
        );
        assert_eq!(
            settings.appearance.metric(TerminalMetric::CellHeight),
            Some(1.0)
        );
    }

    #[test]
    fn invalid_metric_values_count_as_unset_without_resetting_other_settings() {
        let content = "locale = \"ja-JP\"\n\n[appearance]\nui_font_family = \"Segoe UI\"\n\
            terminal_font_size = \"big\"\nterminal_cell_height = 0.0\nterminal_cell_width = nan\n";
        let (settings, warning) = load_str("metrics_invalid", content);
        assert!(
            warning.is_none(),
            "a bad metric must not trigger the malformed-file fallback"
        );
        assert_eq!(settings.locale, LocalePreference::JaJp);
        assert_eq!(
            settings.appearance.ui_font_family.as_deref(),
            Some("Segoe UI")
        );
        for metric in TerminalMetric::ALL {
            assert_eq!(settings.appearance.metric(metric), None, "{metric:?}");
            assert!(settings.appearance.metric_is_invalid(metric), "{metric:?}");
        }
        assert_eq!(settings.appearance.metric_diagnostics().len(), 3);
    }

    #[test]
    fn out_of_range_negative_and_infinite_values_are_ignored() {
        for (key, bad) in [
            ("terminal_font_size", "0.9"),
            ("terminal_font_size", "100.1"),
            ("terminal_font_size", "-12"),
            ("terminal_cell_height", "0.49"),
            ("terminal_cell_height", "3.01"),
            ("terminal_cell_width", "0.19"),
            ("terminal_cell_width", "2.01"),
            ("terminal_cell_width", "inf"),
            ("terminal_cell_width", "-inf"),
            ("terminal_font_size", "true"),
            ("terminal_font_size", "[1, 2]"),
        ] {
            let (settings, warning) =
                load_str("metrics_bad", &format!("[appearance]\n{key} = {bad}\n"));
            assert!(warning.is_none(), "{key} = {bad}");
            let metric = TerminalMetric::ALL
                .into_iter()
                .find(|m| {
                    key.ends_with(match m {
                        TerminalMetric::FontSize => "font_size",
                        TerminalMetric::CellHeight => "cell_height",
                        TerminalMetric::CellWidth => "cell_width",
                    })
                })
                .unwrap();
            assert_eq!(settings.appearance.metric(metric), None, "{key} = {bad}");
            assert!(
                settings.appearance.metric_is_invalid(metric),
                "{key} = {bad}"
            );
        }
    }

    #[test]
    fn invalid_values_survive_saving_other_settings() {
        let content = "[appearance]\nterminal_font_size = \"big\"\nterminal_cell_width = 9.5\n";
        let (mut settings, _) = load_str("metrics_keep", content);
        settings.locale = LocalePreference::EnUs;
        settings.appearance.ui_font_family = Some("Segoe UI".into());

        let text = save_and_read(&settings, "metrics_keep_save");
        let (reloaded, warning) = load_str("metrics_keep_reload", &text);
        assert!(warning.is_none(), "{text}");
        assert!(text.contains("terminal_font_size = \"big\""), "{text}");
        assert!(text.contains("terminal_cell_width = 9.5"), "{text}");
        assert!(
            reloaded
                .appearance
                .metric_is_invalid(TerminalMetric::FontSize)
        );
        assert!(
            reloaded
                .appearance
                .metric_is_invalid(TerminalMetric::CellWidth)
        );
    }

    #[test]
    fn committing_a_valid_value_replaces_an_invalid_one_and_reset_clears_the_key() {
        let (mut settings, _) = load_str(
            "metrics_replace",
            "[appearance]\nterminal_font_size = \"big\"\n",
        );
        settings
            .appearance
            .set_metric(TerminalMetric::FontSize, Some(12.0));
        assert!(
            !settings
                .appearance
                .metric_is_invalid(TerminalMetric::FontSize)
        );
        let text = save_and_read(&settings, "metrics_replace_save");
        assert!(text.contains("terminal_font_size = 12.0"), "{text}");
        assert!(!text.contains("big"), "{text}");

        settings
            .appearance
            .set_metric(TerminalMetric::FontSize, None);
        let text = save_and_read(&settings, "metrics_reset_save");
        assert!(!text.contains("terminal_font_size"), "{text}");
    }

    #[test]
    fn unknown_keys_and_tables_are_not_dropped_when_saving() {
        let content = "locale = \"en-US\"\nfuture_top_level = 7\n\n[appearance]\nterminal_font_size = 12.0\n\
            future_appearance_key = \"x\"\n\n[future_table]\nanswer = 42\n";
        let (mut settings, warning) = load_str("metrics_unknown", content);
        assert!(warning.is_none());
        settings
            .appearance
            .set_metric(TerminalMetric::CellWidth, Some(0.6));

        let text = save_and_read(&settings, "metrics_unknown_save");
        let (reloaded, warning) = load_str("metrics_unknown_reload", &text);
        assert!(warning.is_none(), "{text}");
        for needle in [
            "future_top_level = 7",
            "future_appearance_key = \"x\"",
            "answer = 42",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::CellWidth),
            Some(0.6)
        );
        assert_eq!(
            reloaded.appearance.metric(TerminalMetric::FontSize),
            Some(12.0)
        );
    }

    #[test]
    fn built_in_glyphs_default_to_on_and_old_files_do_not_gain_the_key() {
        let (settings, warning) = load_str(
            "glyphs_legacy",
            "locale = \"en-US\"

[appearance]
theme = \"night-dark\"
",
        );
        assert!(warning.is_none());
        assert!(settings.appearance.builtin_glyphs());
        assert!(!settings.appearance.has_builtin_glyphs_key());
        assert!(settings.appearance.invalid_builtin_glyphs_text().is_none());
        let text = save_and_read(&settings, "glyphs_legacy_save");
        assert!(!text.contains("terminal_builtin_glyphs"), "{text}");
        assert!(ApplicationSettings::default().appearance.builtin_glyphs());
    }

    #[test]
    fn built_in_glyphs_true_and_false_round_trip() {
        for value in [true, false] {
            let mut settings = ApplicationSettings::default();
            settings.appearance.set_builtin_glyphs(value);
            let text = save_and_read(&settings, "glyphs_round_trip");
            assert!(
                text.contains(&format!("terminal_builtin_glyphs = {value}")),
                "{text}"
            );
            let (reloaded, warning) = load_str("glyphs_round_trip_load", &text);
            assert!(warning.is_none(), "{text}");
            assert_eq!(reloaded.appearance.builtin_glyphs(), value);
            assert!(reloaded.appearance.invalid_builtin_glyphs_text().is_none());
            assert!(reloaded.appearance.metric_diagnostics().is_empty());
        }
    }

    #[test]
    fn a_built_in_glyphs_value_of_the_wrong_type_counts_as_on_and_survives_saving() {
        for bad in ["\"false\"", "0", "1", "\"\"", "[false]", "{ a = 1 }", "2.5"] {
            let content = format!(
                "[appearance]
terminal_builtin_glyphs = {bad}
future = 1
"
            );
            let (mut settings, warning) = load_str("glyphs_bad", &content);
            assert!(warning.is_none(), "{bad}: the file still loads");
            assert!(settings.appearance.builtin_glyphs(), "{bad}: treated as on");
            assert!(settings.appearance.has_builtin_glyphs_key(), "{bad}");
            assert!(
                settings.appearance.invalid_builtin_glyphs_text().is_some(),
                "{bad}"
            );
            assert_eq!(settings.appearance.metric_diagnostics().len(), 1, "{bad}");

            settings.locale = LocalePreference::EnUs;
            let text = save_and_read(&settings, "glyphs_bad_save");
            let (reloaded, warning) = load_str("glyphs_bad_reload", &text);
            assert!(warning.is_none(), "{bad}: {text}");
            assert!(text.contains("future = 1"), "{text}");
            assert_eq!(
                reloaded.appearance.invalid_builtin_glyphs_text(),
                settings.appearance.invalid_builtin_glyphs_text(),
                "{bad}: kept as written"
            );

            settings.appearance.set_builtin_glyphs(false);
            assert!(settings.appearance.invalid_builtin_glyphs_text().is_none());
            let text = save_and_read(&settings, "glyphs_bad_replace");
            assert!(text.contains("terminal_builtin_glyphs = false"), "{text}");
            settings.appearance.clear_builtin_glyphs();
            let text = save_and_read(&settings, "glyphs_bad_clear");
            assert!(!text.contains("terminal_builtin_glyphs"), "{text}");
        }
    }

    #[test]
    fn terminal_settings_default_values() {
        let defaults = TerminalSettings::default();
        assert_eq!(defaults.shell_mode, ShellMode::Auto);
        assert_eq!(defaults.shell_path, "");
        assert_eq!(defaults.shell_integration, ShellIntegrationMode::Auto);
        assert!(defaults.extra.0.is_empty());
        assert!(defaults.diagnostics().is_empty());
    }

    #[test]
    fn terminal_settings_toml_persistence_round_trip() {
        let mut settings = ApplicationSettings::default();
        settings.terminal.shell_mode = ShellMode::Custom;
        settings.terminal.shell_path = "C:\\テスト ユーザー\\bin\\custom shell.exe".to_string();
        settings.terminal.shell_integration = ShellIntegrationMode::Disabled;

        let text = save_and_read(&settings, "terminal_settings_round_trip");
        assert!(text.contains("shell_mode = \"custom\""), "{text}");
        assert!(
            text.contains("C:\\テスト ユーザー\\bin\\custom shell.exe"),
            "{text}"
        );
        assert!(text.contains("shell_integration = \"disabled\""), "{text}");

        let (reloaded, warning) = load_str("terminal_settings_reload", &text);
        assert!(warning.is_none());
        assert_eq!(reloaded.terminal.shell_mode, ShellMode::Custom);
        assert_eq!(
            reloaded.terminal.shell_path,
            "C:\\テスト ユーザー\\bin\\custom shell.exe"
        );
        assert_eq!(
            reloaded.terminal.shell_integration,
            ShellIntegrationMode::Disabled
        );
    }

    #[test]
    fn legacy_settings_toml_without_terminal_loads_clean_defaults() {
        let content = "locale = \"ja-JP\"\n";
        let (settings, warning) = load_str("legacy_no_terminal", content);
        assert!(warning.is_none());
        assert_eq!(settings.terminal.shell_mode, ShellMode::Auto);
        assert_eq!(settings.terminal.shell_path, "");
        assert_eq!(
            settings.terminal.shell_integration,
            ShellIntegrationMode::Auto
        );
        assert!(settings.terminal.diagnostics().is_empty());
    }

    #[test]
    fn unknown_shell_mode_and_integration_are_preserved_and_emit_diagnostics() {
        let content = r#"
[terminal]
shell_mode = "zsh"
shell_path = "C:\\Windows\\System32\\cmd.exe"
shell_integration = "hypothetical"
"#;
        let (settings, warning) = load_str("unknown_enums", content);
        assert!(warning.is_none());
        assert_eq!(
            settings.terminal.shell_mode,
            ShellMode::Unknown("zsh".into())
        );
        assert_eq!(
            settings.terminal.shell_integration,
            ShellIntegrationMode::Unknown("hypothetical".into())
        );

        let diagnostics = settings.terminal.diagnostics();
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics[0].contains("unknown terminal.shell_mode = \"zsh\""));
        assert!(diagnostics[1].contains("unknown terminal.shell_integration = \"hypothetical\""));

        // When saved, the raw strings are preserved verbatim
        let text = save_and_read(&settings, "unknown_enums_save");
        assert!(text.contains("shell_mode = \"zsh\""), "{text}");
        assert!(
            text.contains("shell_integration = \"hypothetical\""),
            "{text}"
        );
    }

    #[test]
    fn non_string_shell_mode_and_integration_survive_as_unknown() {
        let content = r#"
[terminal]
shell_mode = 42
shell_integration = true
"#;
        let (settings, warning) = load_str("non_string_enums", content);
        assert!(warning.is_none());
        assert_eq!(
            settings.terminal.shell_mode,
            ShellMode::Unknown("42".into())
        );
        assert_eq!(
            settings.terminal.shell_integration,
            ShellIntegrationMode::Unknown("true".into())
        );

        let diagnostics = settings.terminal.diagnostics();
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn extra_unknown_keys_in_terminal_section_are_preserved() {
        let content = r#"
[terminal]
shell_mode = "auto"
custom_experimental_feature = true
timeout_seconds = 120
"#;
        let (settings, warning) = load_str("extra_terminal_keys", content);
        assert!(warning.is_none());
        assert_eq!(settings.terminal.extra.0.len(), 2);

        let text = save_and_read(&settings, "extra_terminal_keys_save");
        assert!(
            text.contains("custom_experimental_feature = true"),
            "{text}"
        );
        assert!(text.contains("timeout_seconds = 120"), "{text}");
    }

    #[test]
    fn validate_shell_path_rules() {
        // Empty / whitespace
        assert_eq!(validate_shell_path(""), Err(ShellPathError::Empty));
        assert_eq!(validate_shell_path("   "), Err(ShellPathError::Empty));

        // Relative paths
        assert_eq!(
            validate_shell_path("cmd.exe"),
            Err(ShellPathError::NotAbsolute)
        );
        assert_eq!(
            validate_shell_path(r".\cmd.exe"),
            Err(ShellPathError::NotAbsolute)
        );
        assert_eq!(
            validate_shell_path(r"bin\pwsh.exe"),
            Err(ShellPathError::NotAbsolute)
        );
        assert_eq!(
            validate_shell_path("/pwsh.exe"),
            Err(ShellPathError::NotAbsolute)
        );
        assert_eq!(
            validate_shell_path(r"\Windows\System32\cmd.exe"),
            Err(ShellPathError::NotAbsolute)
        );

        // Arguments prohibited
        assert_eq!(
            validate_shell_path(r"C:\Windows\System32\cmd.exe /c dir"),
            Err(ShellPathError::HasArguments)
        );
        assert_eq!(
            validate_shell_path(r#""C:\Windows\System32\cmd.exe" -NoLogo"#),
            Err(ShellPathError::HasArguments)
        );

        // Non-existent file
        assert_eq!(
            validate_shell_path(r"C:\Windows\System32\nonexistent_shell_98765.exe"),
            Err(ShellPathError::NotFound)
        );

        // Directory
        assert_eq!(
            validate_shell_path(r"C:\Windows"),
            Err(ShellPathError::IsDirectory)
        );

        // Non-executable extension
        assert_eq!(
            validate_shell_path(r"C:\Windows\System32\drivers\etc\hosts"),
            Err(ShellPathError::NotExecutable)
        );

        // Valid system cmd.exe (always exists on Windows)
        let valid = validate_shell_path(r"C:\Windows\System32\cmd.exe");
        assert!(valid.is_ok(), "{valid:?}");
    }

    fn tempfile_dir(prefix: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("kegon_test_{prefix}_{}", std::process::id()));
        let _ = fs::create_dir_all(&p);
        p
    }
}
