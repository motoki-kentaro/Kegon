//! Whether national flags can be drawn as colour flags (#82).
//!
//! A flag is a pair of Regional Indicators. Segoe UI Emoji has no flag glyphs, so the run is drawn with
//! `Noto Color Emoji` named explicitly, and only when that font can really draw it. The family name
//! alone proves nothing: the COLRv1 build of Noto shapes a flag into one glyph that swash then rasterizes
//! to nothing, while the CBDT build shapes and rasterizes it (#82 probe). So the check goes the whole way
//! a drawn flag goes:
//!
//!   1. the family is registered in the `FontSystem`;
//!   2. a pair laid out the way a terminal `Text` is laid out selects that family for every glyph;
//!   3. it becomes exactly one real glyph (not the two letters of the Regional Indicators);
//!   4. swash rasterizes that glyph to a non-empty colour image.
//!
//! The `FontSystem` asked is iced's global one, which the wgpu and tiny-skia text paths and the canvas
//! `fill_text` all share, so the answer is about the very font the renderer will get. The system fonts
//! are read when that `FontSystem` is created and Kegon loads none afterwards, and the flag font does not
//! depend on the terminal font setting, so an answer holds for the process: the first flag costs one
//! probe, every later frame a map lookup. Anything unclear (a poisoned lock, an unreadable font) counts
//! as unsupported, which leaves the flag to the per-cell path.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::Font;
use iced::advanced::graphics::text as gtext;
use iced::advanced::text::Alignment;
use iced::advanced::text::Shaping;

use gtext::cosmic_text as ct;

/// The only family flag runs are drawn with.
pub const FLAG_FONT_FAMILY: &str = "Noto Color Emoji";

/// The flag that stands for the font as a whole when the renderer first asks (🇯🇵).
const SAMPLE_FLAG: &str = "\u{1F1EF}\u{1F1F5}";

/// Size and line height only steer the layout; whether a glyph exists does not depend on them.
const PROBE_SIZE: f32 = 16.0;
const PROBE_LINE_HEIGHT: f32 = 19.2;

/// The font a flag run is drawn with.
pub fn font() -> Font {
    Font::with_name(FLAG_FONT_FAMILY)
}

/// How far a flag gets on the way to the screen. Only `Available` lets a flag be drawn as one `Text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagFontStatus {
    Available,
    /// The family is not registered in the `FontSystem`.
    NotInstalled,
    /// Registered, but the layout did not pick it for the glyphs of the flag.
    NotSelected,
    /// Selected, but the pair did not become one flag glyph.
    NoFlagGlyph,
    /// A flag glyph exists, but swash gives no non-empty colour image for it.
    RasterFailed,
}

#[derive(Debug, Clone, PartialEq)]
struct RasterObservation {
    colour: bool,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct GlyphObservation {
    from_flag_font: bool,
    glyph_id: u16,
    raster: Option<RasterObservation>,
}

/// What laying a pair out in a `FontSystem` showed; `classify` turns it into a status.
#[derive(Debug, Clone, PartialEq)]
struct Observation {
    registered: bool,
    glyphs: Vec<GlyphObservation>,
}

fn classify(o: &Observation) -> FlagFontStatus {
    if !o.registered {
        return FlagFontStatus::NotInstalled;
    }
    if o.glyphs.is_empty() || o.glyphs.iter().any(|g| !g.from_flag_font) {
        return FlagFontStatus::NotSelected;
    }
    let [glyph] = o.glyphs.as_slice() else {
        return FlagFontStatus::NoFlagGlyph;
    };
    if glyph.glyph_id == 0 {
        return FlagFontStatus::NoFlagGlyph;
    }
    match &glyph.raster {
        Some(r) if r.colour && r.width > 0 && r.height > 0 => FlagFontStatus::Available,
        _ => FlagFontStatus::RasterFailed,
    }
}

/// Lays `pair` out with the flag font as iced lays out a terminal `Text` and records what happened.
fn observe(fs: &mut ct::FontSystem, pair: &str) -> Observation {
    let registered = fs
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == FLAG_FONT_FAMILY));
    if !registered {
        return Observation {
            registered,
            glyphs: Vec::new(),
        };
    }
    let mut buffer = ct::Buffer::new(fs, ct::Metrics::new(PROBE_SIZE, PROBE_LINE_HEIGHT));
    buffer.set_size(fs, Some(f32::INFINITY), Some(f32::INFINITY));
    buffer.set_text(
        fs,
        pair,
        &gtext::to_attributes(font()),
        gtext::to_shaping(Shaping::Advanced, pair),
        None,
    );
    gtext::align(&mut buffer, fs, Alignment::Default);

    let mut swash = ct::SwashCache::new();
    let mut glyphs = Vec::new();
    for run in buffer.layout_runs() {
        for g in run.glyphs {
            let from_flag_font = fs.db().face(g.font_id).is_some_and(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name == FLAG_FONT_FAMILY)
            });
            let raster = swash
                .get_image_uncached(fs, g.physical((0.0, 0.0), 1.0).cache_key)
                .map(|image| RasterObservation {
                    colour: image.content == ct::SwashContent::Color,
                    width: image.placement.width,
                    height: image.placement.height,
                });
            glyphs.push(GlyphObservation {
                from_flag_font,
                glyph_id: g.glyph_id,
                raster,
            });
        }
    }
    Observation { registered, glyphs }
}

/// Probes `pair` in the `FontSystem` iced draws with. A poisoned lock reads as "not available".
fn probe_global(pair: &str) -> FlagFontStatus {
    match gtext::font_system().write() {
        Ok(mut system) => classify(&observe(system.raw(), pair)),
        Err(_) => FlagFontStatus::NotSelected,
    }
}

/// The status of the flag font for the process, from the sample flag. Probed once.
pub fn status() -> FlagFontStatus {
    static STATUS: OnceLock<FlagFontStatus> = OnceLock::new();
    *STATUS.get_or_init(|| probe_global(SAMPLE_FLAG))
}

/// Whether flag runs may be drawn at all. A cheap read once `status` is known.
pub fn supports_flags() -> bool {
    status() == FlagFontStatus::Available
}

/// Whether this very pair draws as a colour flag. One flag working does not promise the others do (a
/// font may lack a region), so each distinct pair is probed once and remembered.
pub fn supports_flag(pair: &str) -> bool {
    static PAIRS: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    if !supports_flags() {
        return false;
    }
    let pairs = PAIRS.get_or_init(Default::default);
    if let Some(&known) = pairs
        .lock()
        .ok()
        .and_then(|p| p.get(pair).copied())
        .as_ref()
    {
        return known;
    }
    let supported = probe_global(pair) == FlagFontStatus::Available;
    if let Ok(mut known) = pairs.lock() {
        known.insert(pair.to_string(), supported);
    }
    supported
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(
        from_flag_font: bool,
        glyph_id: u16,
        raster: Option<(bool, u32, u32)>,
    ) -> GlyphObservation {
        GlyphObservation {
            from_flag_font,
            glyph_id,
            raster: raster.map(|(colour, width, height)| RasterObservation {
                colour,
                width,
                height,
            }),
        }
    }

    fn seen(registered: bool, glyphs: Vec<GlyphObservation>) -> Observation {
        Observation { registered, glyphs }
    }

    #[test]
    fn a_registered_font_that_draws_one_colour_glyph_is_available() {
        let o = seen(true, vec![glyph(true, 1663, Some((true, 19, 18)))]);
        assert_eq!(classify(&o), FlagFontStatus::Available);
    }

    #[test]
    fn a_missing_family_is_not_installed() {
        let o = seen(false, vec![]);
        assert_eq!(classify(&o), FlagFontStatus::NotInstalled);
    }

    #[test]
    fn glyphs_from_another_font_mean_the_flag_font_was_not_selected() {
        let segoe = seen(true, vec![glyph(false, 282, Some((false, 4, 9)))]);
        assert_eq!(classify(&segoe), FlagFontStatus::NotSelected);
        assert_eq!(
            classify(&seen(true, vec![])),
            FlagFontStatus::NotSelected,
            "no glyph at all"
        );
        // One glyph from the flag font is not enough when another comes from elsewhere.
        let mixed = seen(
            true,
            vec![
                glyph(true, 1663, Some((true, 19, 18))),
                glyph(false, 288, None),
            ],
        );
        assert_eq!(classify(&mixed), FlagFontStatus::NotSelected);
    }

    #[test]
    fn two_letters_or_notdef_are_not_a_flag_glyph() {
        let letters = seen(
            true,
            vec![
                glyph(true, 3363, Some((true, 19, 18))),
                glyph(true, 3364, Some((true, 19, 18))),
            ],
        );
        assert_eq!(classify(&letters), FlagFontStatus::NoFlagGlyph);
        let notdef = seen(true, vec![glyph(true, 0, Some((true, 19, 18)))]);
        assert_eq!(classify(&notdef), FlagFontStatus::NoFlagGlyph);
    }

    #[test]
    fn an_empty_or_monochrome_raster_is_a_failed_raster() {
        // The COLRv1 build of Noto: one flag glyph, but swash yields an empty mask.
        let empty = seen(true, vec![glyph(true, 2552, Some((false, 1, 0)))]);
        assert_eq!(classify(&empty), FlagFontStatus::RasterFailed);
        let none = seen(true, vec![glyph(true, 2552, None)]);
        assert_eq!(classify(&none), FlagFontStatus::RasterFailed);
        let mask = seen(true, vec![glyph(true, 2552, Some((false, 19, 18)))]);
        assert_eq!(classify(&mask), FlagFontStatus::RasterFailed);
        let blank_colour = seen(true, vec![glyph(true, 2552, Some((true, 0, 0)))]);
        assert_eq!(classify(&blank_colour), FlagFontStatus::RasterFailed);
    }

    #[test]
    fn a_font_system_without_fonts_reports_not_installed() {
        let mut fs =
            ct::FontSystem::new_with_locale_and_db("en-US".into(), ct::fontdb::Database::new());
        let observed = observe(&mut fs, SAMPLE_FLAG);
        assert!(!observed.registered && observed.glyphs.is_empty());
        assert_eq!(classify(&observed), FlagFontStatus::NotInstalled);
    }

    /// Depends on the fonts installed here, so it only reports:
    /// `cargo test --locked flag_font -- --ignored --nocapture`
    #[test]
    #[ignore = "reports this machine's fonts"]
    fn reports_the_flag_font_status_of_this_machine() {
        println!("status = {:?}", status());
        for pair in ["🇯🇵", "🇺🇸", "🇬🇧", "🇩🇪", "🇯🇺"] {
            println!("{pair} supported = {}", supports_flag(pair));
        }
    }

    #[test]
    fn the_flag_font_is_named_explicitly() {
        assert_eq!(font(), Font::with_name(FLAG_FONT_FAMILY));
    }
}
