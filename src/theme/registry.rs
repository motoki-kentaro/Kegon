//! Built-in theme registry.

use super::model::{KegonTheme, ThemeId};
use super::night_dark::NIGHT_DARK;

/// Returns the built-in theme for an ID.
pub fn resolve_theme(id: ThemeId) -> &'static KegonTheme {
    match id {
        ThemeId::NightDark => &NIGHT_DARK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_night_dark() {
        assert_eq!(ThemeId::NightDark.as_str(), "night-dark");
        assert_eq!(
            ThemeId::from_persisted("night-dark"),
            Some(ThemeId::NightDark)
        );
        assert_eq!(ThemeId::from_persisted("Night Dark"), None);
        assert_eq!(ThemeId::from_persisted("future-theme"), None);
    }

    #[test]
    fn default_theme_is_night_dark() {
        assert_eq!(ThemeId::DEFAULT, ThemeId::NightDark);
    }

    #[test]
    fn resolver_returns_the_requested_theme() {
        for id in ThemeId::ALL {
            assert_eq!(resolve_theme(id).id, id);
        }
        assert_eq!(resolve_theme(ThemeId::NightDark), &NIGHT_DARK);
    }

    #[test]
    fn display_name_is_not_the_persisted_id() {
        assert_eq!(ThemeId::NightDark.display_name(), "Night Dark");
        assert_ne!(
            ThemeId::NightDark.display_name(),
            ThemeId::NightDark.as_str()
        );
    }
}
