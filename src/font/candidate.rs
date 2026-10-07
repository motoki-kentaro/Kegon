//! Font candidate and source metadata model.

use std::path::PathBuf;

/// Source location of a font candidate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FontSource {
    /// Binary font file on disk with face index (for TTC/OTC font collections).
    Binary { path: PathBuf, index: u32 },
    /// Synthetic font in memory (used for tests without filesystem access).
    Synthetic { id: String, bytes: Vec<u8> },
}

/// Representative style metadata for a font family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentativeFace {
    pub weight: u16,
    pub is_italic: bool,
}

impl Default for RepresentativeFace {
    fn default() -> Self {
        Self {
            weight: 400,
            is_italic: false,
        }
    }
}

/// A normalized font family candidate available for user selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontCandidate {
    /// Family name (e.g. "Cascadia Mono", "Segoe UI", "Yu Gothic").
    pub family_name: String,
    /// Whether this font family is classified as monospaced/fixed-pitch.
    pub is_monospace: bool,
    /// Source metadata for loading font bytes.
    pub source: FontSource,
    /// Representative face style information.
    pub representative_face: RepresentativeFace,
}

impl FontCandidate {
    #[allow(dead_code)]
    pub fn new(family_name: impl Into<String>, is_monospace: bool, source: FontSource) -> Self {
        Self {
            family_name: family_name.into(),
            is_monospace,
            source,
            representative_face: RepresentativeFace::default(),
        }
    }
}
