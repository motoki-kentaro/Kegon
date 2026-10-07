//! Font discovery, normalization, loading, and preview caching system.

pub mod cache;
pub mod candidate;
pub mod catalog;
pub mod resolution;

#[allow(unused_imports)]
pub use cache::FontCache;
#[allow(unused_imports)]
pub use candidate::{FontCandidate, FontSource, RepresentativeFace};
pub use catalog::SystemFontCatalog;
pub use resolution::{UiFontResolution, UiFontStatus, resolve_ui_font};
