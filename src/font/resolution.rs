//! UI font resolution and fallback management.

use iced::Font;

use crate::font::cache::FontCache;
use crate::font::catalog::SystemFontCatalog;

/// Operational status of the application UI font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiFontStatus {
    /// Using system default UI font.
    SystemDefault,
    /// Explicitly configured font family is active and loaded.
    Active(String),
    /// Configured family was saved in settings, but is not installed on the system.
    NotInstalled(String),
    /// Configured family exists in system catalog, but loading font bytes failed.
    LoadFailed(String),
}

/// Result of resolving a UI font configuration against installed system fonts.
#[derive(Debug, Clone)]
pub struct UiFontResolution {
    pub status: UiFontStatus,
    pub font: Font,
}

/// Resolves the requested UI font family against the system catalog and byte cache.
pub fn resolve_ui_font(
    configured_family: Option<&str>,
    catalog: &SystemFontCatalog,
    cache: &mut FontCache,
) -> UiFontResolution {
    let Some(family) = configured_family else {
        return UiFontResolution {
            status: UiFontStatus::SystemDefault,
            font: Font::DEFAULT,
        };
    };

    let family = family.trim();
    if family.is_empty() {
        return UiFontResolution {
            status: UiFontStatus::SystemDefault,
            font: Font::DEFAULT,
        };
    }

    let candidate = catalog
        .candidates()
        .iter()
        .find(|c| c.family_name.eq_ignore_ascii_case(family));

    let Some(candidate) = candidate else {
        return UiFontResolution {
            status: UiFontStatus::NotInstalled(family.to_string()),
            font: Font::DEFAULT,
        };
    };

    if cache.get_or_load(candidate).is_some() {
        let static_name: &'static str = Box::leak(candidate.family_name.clone().into_boxed_str());
        UiFontResolution {
            status: UiFontStatus::Active(candidate.family_name.clone()),
            font: Font::with_name(static_name),
        }
    } else {
        UiFontResolution {
            status: UiFontStatus::LoadFailed(family.to_string()),
            font: Font::DEFAULT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::candidate::{FontCandidate, FontSource};

    fn test_catalog() -> SystemFontCatalog {
        let candidates = vec![
            FontCandidate::new(
                "Noto Sans JP",
                false,
                FontSource::Synthetic {
                    id: "1".into(),
                    bytes: vec![1, 2, 3],
                },
            ),
            FontCandidate::new(
                "Cascadia Mono",
                true,
                FontSource::Synthetic {
                    id: "2".into(),
                    bytes: vec![4, 5, 6],
                },
            ),
        ];
        SystemFontCatalog::from_candidates(candidates)
    }

    #[test]
    fn none_configured_resolves_to_system_default() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();

        let res = resolve_ui_font(None, &catalog, &mut cache);
        assert_eq!(res.status, UiFontStatus::SystemDefault);
        assert_eq!(res.font, Font::DEFAULT);
    }

    #[test]
    fn valid_installed_family_resolves_to_active() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();

        let res = resolve_ui_font(Some("Noto Sans JP"), &catalog, &mut cache);
        assert_eq!(res.status, UiFontStatus::Active("Noto Sans JP".into()));
        assert_eq!(res.font, Font::with_name("Noto Sans JP"));
    }

    #[test]
    fn missing_family_resolves_to_not_installed_with_default_fallback() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();

        let res = resolve_ui_font(Some("Nonexistent Font Family"), &catalog, &mut cache);
        assert_eq!(
            res.status,
            UiFontStatus::NotInstalled("Nonexistent Font Family".into())
        );
        assert_eq!(res.font, Font::DEFAULT);
    }
}
