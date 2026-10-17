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

/// User overrides of the terminal display metrics (all optional).
///
/// - `font_size_pt`: font size in points; `font_px = pt × 96 / 72`. Unset keeps the built-in
///   [`DEFAULT_TERMINAL_FONT_SIZE`] (13 logical px).
/// - `cell_width` / `cell_height`: multiples of `font_px`. Unset follows the font's own metrics
///   (scaled to `font_px`).
///
/// Values are expected to be validated (finite, inside the settings ranges); anything else is
/// ignored here as a last line of defence so the grid can never see NaN or zero.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TerminalMetricOverrides {
    pub font_size_pt: Option<f32>,
    pub cell_width: Option<f32>,
    pub cell_height: Option<f32>,
}

impl TerminalMetricOverrides {
    /// The usable (valid, in-range) metric settings; ignored values count as unset.
    pub fn from_settings(appearance: &crate::settings::AppearanceSettings) -> Self {
        use crate::settings::TerminalMetric;
        let get = |metric| appearance.metric(metric).map(|v| v as f32);
        Self {
            font_size_pt: get(TerminalMetric::FontSize),
            cell_width: get(TerminalMetric::CellWidth),
            cell_height: get(TerminalMetric::CellHeight),
        }
    }

    fn usable(value: Option<f32>) -> Option<f32> {
        value.filter(|v| v.is_finite() && *v > 0.0)
    }

    /// The glyph size in logical px.
    pub fn font_px(&self) -> f32 {
        Self::usable(self.font_size_pt)
            .map(|pt| pt * 96.0 / 72.0)
            .unwrap_or(DEFAULT_TERMINAL_FONT_SIZE)
    }

    /// Applies the cell overrides to metrics derived (or scaled) for [`Self::font_px`].
    pub fn apply(&self, base: TerminalCellMetrics) -> TerminalCellMetrics {
        let px = self.font_px();
        let width = Self::usable(self.cell_width).map_or(base.cell_width, |m| px * m);
        let height = Self::usable(self.cell_height).map_or(base.cell_height, |m| px * m);
        TerminalCellMetrics::new(width, height, px)
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
    overrides: &TerminalMetricOverrides,
) -> Option<ConcreteTerminalFont> {
    let bytes = cache.get_or_load(candidate)?;
    let metrics = overrides.apply(derive_font_metrics(candidate, &bytes, overrides.font_px())?);
    let static_name: &'static str = Box::leak(candidate.family_name.clone().into_boxed_str());
    Some(ConcreteTerminalFont {
        font: Font::with_name(static_name),
        metrics,
    })
}

fn resolve_default_terminal_font(
    catalog: &SystemFontCatalog,
    cache: &mut FontCache,
    overrides: &TerminalMetricOverrides,
) -> ConcreteTerminalFont {
    if let Some(candidate) = catalog.candidates().iter().find(|c| c.is_monospace)
        && let Some(concrete) = resolve_concrete_terminal_font(candidate, cache, overrides)
    {
        return concrete;
    }
    // No usable font file: scale the built-in cell to the requested size, then apply the overrides.
    let default = TerminalCellMetrics::default();
    let scale = overrides.font_px() / DEFAULT_TERMINAL_FONT_SIZE;
    ConcreteTerminalFont {
        font: Font::MONOSPACE,
        metrics: overrides.apply(TerminalCellMetrics::new(
            default.cell_width * scale,
            default.cell_height * scale,
            overrides.font_px(),
        )),
    }
}

/// Resolves the requested Terminal font family against the system catalog and byte cache.
pub fn resolve_terminal_font(
    configured_family: Option<&str>,
    overrides: &TerminalMetricOverrides,
    catalog: &SystemFontCatalog,
    cache: &mut FontCache,
) -> TerminalFontConfig {
    let default_font = resolve_default_terminal_font(catalog, cache, overrides);

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

    if let Some(concrete) = resolve_concrete_terminal_font(candidate, cache, overrides) {
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

        let config = resolve_terminal_font(None, &Default::default(), &catalog, &mut cache);
        assert_eq!(config.status, TerminalFontStatus::Default);
        assert_eq!(config.font, Font::MONOSPACE);
    }

    fn overrides(
        size: Option<f32>,
        width: Option<f32>,
        height: Option<f32>,
    ) -> TerminalMetricOverrides {
        TerminalMetricOverrides {
            font_size_pt: size,
            cell_width: width,
            cell_height: height,
        }
    }

    #[test]
    fn no_overrides_keep_the_legacy_metrics_bit_for_bit() {
        let base = TerminalCellMetrics::new(7.617, 15.107, DEFAULT_TERMINAL_FONT_SIZE);
        let none = TerminalMetricOverrides::default();
        assert_eq!(none.font_px(), DEFAULT_TERMINAL_FONT_SIZE);
        assert_eq!(none.apply(base), base);

        // The fallback cell (no usable font file) is the built-in one, unscaled.
        let catalog = test_catalog();
        let mut cache = FontCache::new();
        let config = resolve_terminal_font(None, &none, &catalog, &mut cache);
        assert_eq!(config.metrics, TerminalCellMetrics::default());
    }

    #[test]
    fn size_only_scales_the_font_derived_cell_proportionally() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();
        let config = resolve_terminal_font(
            None,
            &overrides(Some(12.0), None, None),
            &catalog,
            &mut cache,
        );
        let scale = 16.0 / DEFAULT_TERMINAL_FONT_SIZE;
        let default = TerminalCellMetrics::default();
        assert!((config.metrics.font_size - 16.0).abs() < 1e-4);
        assert!((config.metrics.cell_width - default.cell_width * scale).abs() < 1e-3);
        assert!((config.metrics.cell_height - default.cell_height * scale).abs() < 1e-3);
    }

    #[test]
    fn width_only_and_height_only_use_the_default_13px_as_reference() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();
        let default = TerminalCellMetrics::default();

        let w = resolve_terminal_font(
            None,
            &overrides(None, Some(0.6), None),
            &catalog,
            &mut cache,
        )
        .metrics;
        assert!((w.cell_width - 13.0 * 0.6).abs() < 1e-4);
        assert_eq!(w.cell_height, default.cell_height);
        assert_eq!(w.font_size, DEFAULT_TERMINAL_FONT_SIZE);

        let h = resolve_terminal_font(
            None,
            &overrides(None, None, Some(1.2)),
            &catalog,
            &mut cache,
        )
        .metrics;
        assert!((h.cell_height - 13.0 * 1.2).abs() < 1e-4);
        assert_eq!(h.cell_width, default.cell_width);
        assert_eq!(h.font_size, DEFAULT_TERMINAL_FONT_SIZE);
    }

    #[test]
    fn all_three_overrides_follow_pt_to_px_conversion() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();
        let m = resolve_terminal_font(
            None,
            &overrides(Some(12.0), Some(0.6), Some(1.2)),
            &catalog,
            &mut cache,
        )
        .metrics;
        assert!((m.font_size - 16.0).abs() < 1e-4);
        assert!((m.cell_width - 9.6).abs() < 1e-4);
        assert!((m.cell_height - 19.2).abs() < 1e-4);
    }

    #[test]
    fn boundary_values_give_positive_finite_metrics_and_sane_grids() {
        for (size, width, height) in [
            (1.0, 0.2, 0.5),
            (100.0, 2.0, 3.0),
            (1.0, 2.0, 3.0),
            (100.0, 0.2, 0.5),
        ] {
            let ov = overrides(Some(size), Some(width), Some(height));
            let m = ov.apply(TerminalCellMetrics::new(6.5, 13.0, ov.font_px()));
            assert!(m.cell_width.is_finite() && m.cell_width >= 1.0, "{m:?}");
            assert!(m.cell_height.is_finite() && m.cell_height >= 1.0, "{m:?}");
            assert!(m.font_size.is_finite() && m.font_size >= 1.0, "{m:?}");
            let (cols, rows) = m.grid_size(888.0, 725.0);
            assert!(cols >= 1 && rows >= 1);
            assert_eq!(m.grid_size(0.0, 0.0), (1, 1));
        }
    }

    #[test]
    fn unusable_override_values_are_ignored_as_a_last_line_of_defence() {
        let default = TerminalCellMetrics::default();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
            let ov = overrides(Some(bad), Some(bad), Some(bad));
            assert_eq!(ov.font_px(), DEFAULT_TERMINAL_FONT_SIZE);
            assert_eq!(ov.apply(default), default, "{bad}");
        }
    }

    #[test]
    fn real_system_font_size_scales_proportionally() {
        let catalog = SystemFontCatalog::load_system();
        let mut cache = FontCache::new();
        let base = resolve_terminal_font(None, &Default::default(), &catalog, &mut cache).metrics;
        let big = resolve_terminal_font(
            None,
            &overrides(Some(24.0), None, None),
            &catalog,
            &mut cache,
        )
        .metrics;
        let k = 32.0 / DEFAULT_TERMINAL_FONT_SIZE;
        assert!(
            (big.cell_width - base.cell_width * k).abs() < 0.01,
            "{base:?} {big:?}"
        );
        assert!(
            (big.cell_height - base.cell_height * k).abs() < 0.01,
            "{base:?} {big:?}"
        );
    }

    #[test]
    fn terminal_font_resolution_missing_returns_not_installed() {
        let catalog = test_catalog();
        let mut cache = FontCache::new();

        let config = resolve_terminal_font(
            Some("Missing Terminal Font"),
            &Default::default(),
            &catalog,
            &mut cache,
        );
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

        let config = resolve_terminal_font(None, &Default::default(), &catalog, &mut cache);
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
            let config =
                resolve_terminal_font(Some(&family), &Default::default(), &catalog, &mut cache);
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

        let default_config = resolve_terminal_font(None, &Default::default(), &catalog, &mut cache);
        let missing_config = resolve_terminal_font(
            Some("Nonexistent Font 12345"),
            &Default::default(),
            &catalog,
            &mut cache,
        );

        assert_eq!(
            missing_config.status,
            TerminalFontStatus::NotInstalled("Nonexistent Font 12345".into())
        );
        assert_eq!(missing_config.font, default_config.font);
        assert_eq!(missing_config.metrics, default_config.metrics);
    }
}
