//! UI and Terminal font resolution, metric derivation, and fallback management.

use iced::Font;

use crate::font::cache::FontCache;
use crate::font::candidate::{FontCandidate, FontSource};
use crate::font::catalog::SystemFontCatalog;

pub const DEFAULT_TERMINAL_FONT_SIZE: f32 = 13.0;

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

/// Cell geometry metrics derived from an active or fallback Terminal font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalCellMetrics {
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
}

impl Default for TerminalCellMetrics {
    fn default() -> Self {
        Self {
            cell_width: 8.5,
            cell_height: 18.0,
            font_size: DEFAULT_TERMINAL_FONT_SIZE,
        }
    }
}

impl TerminalCellMetrics {
    pub fn new(cell_width: f32, cell_height: f32, font_size: f32) -> Self {
        Self {
            cell_width: cell_width.max(1.0),
            cell_height: cell_height.max(1.0),
            font_size: font_size.max(1.0),
        }
    }

    /// Calculates grid columns and rows for a given viewport size.
    pub fn grid_size(&self, width: f32, height: f32) -> (u16, u16) {
        let cols = (width / self.cell_width).floor().max(1.0) as u16;
        let rows = (height / self.cell_height).floor().max(1.0) as u16;
        (cols, rows)
    }
}

/// Operational status of the application Terminal font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalFontStatus {
    /// Using system default Terminal font.
    Default,
    /// Explicitly configured font family is active and loaded.
    Active(String),
    /// Configured family was saved in settings, but is not installed on the system.
    NotInstalled(String),
    /// Configured family exists in system catalog, but loading font bytes or metrics failed.
    LoadFailed(String),
}

/// Result of resolving a Terminal font configuration against installed system fonts and deriving cell metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalFontConfig {
    pub status: TerminalFontStatus,
    pub font: Font,
    pub metrics: TerminalCellMetrics,
}

impl Default for TerminalFontConfig {
    fn default() -> Self {
        Self {
            status: TerminalFontStatus::Default,
            font: Font::MONOSPACE,
            metrics: TerminalCellMetrics::default(),
        }
    }
}

/// Derive `TerminalCellMetrics` from font face bytes using OpenType font metrics and glyph advance width.
pub fn derive_font_metrics(
    candidate: &FontCandidate,
    bytes: &[u8],
    font_size: f32,
) -> Option<TerminalCellMetrics> {
    let index = match &candidate.source {
        FontSource::Binary { index, .. } => *index,
        _ => 0,
    };
    let face = ttf_parser::Face::parse(bytes, index).ok()?;
    let upem = face.units_per_em() as f32;
    if upem <= 0.0 {
        return None;
    }

    let scale = font_size / upem;
    let ascender = face.ascender() as f32;
    let descender = face.descender() as f32;
    let line_gap = face.line_gap() as f32;

    let font_height = (ascender - descender + line_gap) * scale;
    if font_height <= 0.0 {
        return None;
    }

    let m_gid = face
        .glyph_index('M')
        .or_else(|| face.glyph_index('0'))
        .or_else(|| face.glyph_index('x'))
        .or_else(|| face.glyph_index(' '));

    let advance_units = m_gid.and_then(|gid| face.glyph_hor_advance(gid))? as f32;
    let cell_width = advance_units * scale;
    if cell_width <= 0.0 {
        return None;
    }

    Some(TerminalCellMetrics::new(cell_width, font_height, font_size))
}

#[derive(Debug, Clone)]
struct ConcreteTerminalFont {
    font: Font,
    metrics: TerminalCellMetrics,
}

fn resolve_concrete_terminal_font(
    candidate: &FontCandidate,
    cache: &mut FontCache,
) -> Option<ConcreteTerminalFont> {
    let bytes = cache.get_or_load(candidate)?;
    let metrics = derive_font_metrics(candidate, &bytes, DEFAULT_TERMINAL_FONT_SIZE)?;
    let static_name: &'static str = Box::leak(candidate.family_name.clone().into_boxed_str());
    Some(ConcreteTerminalFont {
        font: Font::with_name(static_name),
        metrics,
    })
}

fn resolve_default_terminal_font(
    catalog: &SystemFontCatalog,
    cache: &mut FontCache,
) -> ConcreteTerminalFont {
    if let Some(candidate) = catalog.candidates().iter().find(|c| c.is_monospace)
        && let Some(concrete) = resolve_concrete_terminal_font(candidate, cache)
    {
        return concrete;
    }
    ConcreteTerminalFont {
        font: Font::MONOSPACE,
        metrics: TerminalCellMetrics::default(),
    }
}

/// Resolves the requested Terminal font family against the system catalog and byte cache.
pub fn resolve_terminal_font(
    configured_family: Option<&str>,
    catalog: &SystemFontCatalog,
    cache: &mut FontCache,
) -> TerminalFontConfig {
    let default_font = resolve_default_terminal_font(catalog, cache);

    let Some(family) = configured_family else {
        return TerminalFontConfig {
            status: TerminalFontStatus::Default,
            font: default_font.font,
            metrics: default_font.metrics,
        };
    };

    let family = family.trim();
    if family.is_empty() {
        return TerminalFontConfig {
            status: TerminalFontStatus::Default,
            font: default_font.font,
            metrics: default_font.metrics,
        };
    }

    let candidate = catalog
        .candidates()
        .iter()
        .find(|c| c.family_name.eq_ignore_ascii_case(family));

    let Some(candidate) = candidate else {
        return TerminalFontConfig {
            status: TerminalFontStatus::NotInstalled(family.to_string()),
            font: default_font.font,
            metrics: default_font.metrics,
        };
    };

    if let Some(concrete) = resolve_concrete_terminal_font(candidate, cache) {
        TerminalFontConfig {
            status: TerminalFontStatus::Active(candidate.family_name.clone()),
            font: concrete.font,
            metrics: concrete.metrics,
        }
    } else {
        TerminalFontConfig {
            status: TerminalFontStatus::LoadFailed(family.to_string()),
            font: default_font.font,
            metrics: default_font.metrics,
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

    #[test]
    fn terminal_cell_metrics_grid_size_calculation() {
        let metrics = TerminalCellMetrics::new(10.0, 20.0, 13.0);
        assert_eq!(metrics.grid_size(1000.0, 400.0), (100, 20));
        assert_eq!(metrics.grid_size(1005.0, 409.0), (100, 20));
        assert_eq!(metrics.grid_size(5.0, 5.0), (1, 1));
    }

    #[test]
    fn terminal_font_resolution_none_returns_default() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();

        let config = resolve_terminal_font(None, &catalog, &mut cache);
        assert_eq!(config.status, TerminalFontStatus::Default);
        assert_eq!(config.font, Font::MONOSPACE);
    }

    #[test]
    fn terminal_font_resolution_missing_returns_not_installed() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();

        let config = resolve_terminal_font(Some("Missing Terminal Font"), &catalog, &mut cache);
        assert_eq!(
            config.status,
            TerminalFontStatus::NotInstalled("Missing Terminal Font".into())
        );
        assert_eq!(config.font, Font::MONOSPACE);
    }

    #[test]
    fn terminal_font_resolution_system_catalog_default_uses_same_concrete_font() {
        let catalog = SystemFontCatalog::load_system();
        let mut cache = FontCache::new();

        let config = resolve_terminal_font(None, &catalog, &mut cache);
        assert_eq!(config.status, TerminalFontStatus::Default);

        if let Some(mono_cand) = catalog.candidates().iter().find(|c| c.is_monospace)
            && let Some(bytes) = cache.get_or_load(mono_cand)
            && let Some(metrics) =
                derive_font_metrics(mono_cand, &bytes, DEFAULT_TERMINAL_FONT_SIZE)
        {
            assert_eq!(
                config.font,
                Font::with_name(Box::leak(mono_cand.family_name.clone().into_boxed_str()))
            );
            assert_eq!(config.metrics, metrics);
        }
    }

    #[test]
    fn terminal_font_resolution_system_catalog_configured_uses_same_concrete_font() {
        let catalog = SystemFontCatalog::load_system();
        let mut cache = FontCache::new();

        let target_family = catalog
            .candidates()
            .iter()
            .find(|c| c.is_monospace)
            .map(|c| c.family_name.clone());

        if let Some(family) = target_family {
            let config = resolve_terminal_font(Some(&family), &catalog, &mut cache);
            assert_eq!(config.status, TerminalFontStatus::Active(family.clone()));
            assert_eq!(
                config.font,
                Font::with_name(Box::leak(family.into_boxed_str()))
            );
        }
    }

    #[test]
    fn terminal_font_resolution_missing_uses_default_concrete_font_and_metrics() {
        let catalog = SystemFontCatalog::load_system();
        let mut cache = FontCache::new();

        let default_config = resolve_terminal_font(None, &catalog, &mut cache);
        let missing_config =
            resolve_terminal_font(Some("Nonexistent Font 12345"), &catalog, &mut cache);

        assert_eq!(
            missing_config.status,
            TerminalFontStatus::NotInstalled("Nonexistent Font 12345".into())
        );
        assert_eq!(missing_config.font, default_config.font);
        assert_eq!(missing_config.metrics, default_config.metrics);
    }
}
