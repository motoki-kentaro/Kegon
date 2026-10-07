//! Locale precedence resolution logic.

use super::model::LocalePreference;
use crate::i18n::{Locale, resolve as resolve_locale};

/// Resolves the effective application [`Locale`] according to strict precedence rules:
///
/// 1. `cli_override` (`--locale`), if given.
/// 2. Persisted `settings_preference` (`EnUs` / `JaJp`), if explicitly set.
/// 3. OS preferred UI languages (if `settings_preference` is `System` or unconfigured).
/// 4. Fallback to `en-US`.
pub fn resolve_application_locale(
    cli_override: Option<&str>,
    settings_preference: LocalePreference,
    os_preferences: impl Iterator<Item = String>,
) -> Locale {
    if let Some(cli_tag) = cli_override {
        return resolve_locale(Some(cli_tag), os_preferences);
    }

    match settings_preference {
        LocalePreference::EnUs => Locale::EnUs,
        LocalePreference::JaJp => Locale::JaJp,
        LocalePreference::System => resolve_locale(None, os_preferences),
    }
}
