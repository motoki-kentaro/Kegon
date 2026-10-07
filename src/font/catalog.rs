//! System font catalog enumeration, family normalization, and deterministic sorting.

use std::collections::HashMap;
use std::fs;

use fontdb::{Database, Source};

use crate::font::candidate::{FontCandidate, FontSource, RepresentativeFace};

/// Enumerates and normalizes font families available on the system or in memory.
#[derive(Debug, Clone, Default)]
pub struct SystemFontCatalog {
    candidates: Vec<FontCandidate>,
}

impl SystemFontCatalog {
    /// Load system fonts using `fontdb::Database` and build normalized family candidates.
    pub fn load_system() -> Self {
        let mut db = Database::new();
        db.load_system_fonts();
        Self::from_database(&db)
    }

    /// Build family candidates from an existing `fontdb::Database`.
    pub fn from_database(db: &Database) -> Self {
        let mut family_map: HashMap<String, FamilyCollector> = HashMap::new();

        for face in db.faces() {
            let primary_family = face
                .families
                .first()
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| face.post_script_name.clone());

            if primary_family.is_empty() {
                continue;
            }

            let source = match &face.source {
                Source::File(path) => Some(FontSource::Binary {
                    path: path.clone(),
                    index: face.index,
                }),
                Source::Binary(data) => Some(FontSource::Synthetic {
                    id: primary_family.clone(),
                    bytes: data.as_ref().as_ref().to_vec(),
                }),
                _ => None,
            };

            let Some(source) = source else {
                continue;
            };

            // Detect monospace classification via ttf-parser post table isFixedPitch metadata
            let is_mono = detect_monospace(face, &source);

            let collector =
                family_map
                    .entry(primary_family.clone())
                    .or_insert_with(|| FamilyCollector {
                        family_name: primary_family,
                        is_monospace: false,
                        best_source: None,
                        best_face: None,
                    });

            if is_mono {
                collector.is_monospace = true;
            }

            let weight_diff = (face.weight.0 as i32 - 400).abs();
            let is_normal_style = face.style == fontdb::Style::Normal;

            let score = (if is_normal_style { 0 } else { 1000 }) + weight_diff;

            let current_best_score = collector.best_face.as_ref().map(|(s, _)| *s);
            if current_best_score.is_none() || Some(score) < current_best_score {
                collector.best_face = Some((
                    score,
                    RepresentativeFace {
                        weight: face.weight.0,
                        is_italic: face.style != fontdb::Style::Normal,
                    },
                ));
                collector.best_source = Some(source);
            }
        }

        let mut candidates: Vec<FontCandidate> = family_map
            .into_values()
            .filter_map(|col| {
                let source = col.best_source?;
                let rep_face = col.best_face.map(|(_, f)| f).unwrap_or_default();
                Some(FontCandidate {
                    family_name: col.family_name,
                    is_monospace: col.is_monospace,
                    source,
                    representative_face: rep_face,
                })
            })
            .collect();

        // Sort candidates deterministically by family_name (case-insensitive with stable tie-break)
        candidates.sort_by(|a, b| {
            a.family_name
                .to_lowercase()
                .cmp(&b.family_name.to_lowercase())
                .then_with(|| a.family_name.cmp(&b.family_name))
        });

        Self { candidates }
    }

    /// Construct a catalog from explicit candidates (useful for unit tests).
    #[allow(dead_code)]
    pub fn from_candidates(mut candidates: Vec<FontCandidate>) -> Self {
        candidates.sort_by(|a, b| {
            a.family_name
                .to_lowercase()
                .cmp(&b.family_name.to_lowercase())
                .then_with(|| a.family_name.cmp(&b.family_name))
        });

        // Deduplicate by family_name
        candidates.dedup_by(|a, b| a.family_name == b.family_name);

        Self { candidates }
    }

    pub fn candidates(&self) -> &[FontCandidate] {
        &self.candidates
    }
}

struct FamilyCollector {
    family_name: String,
    is_monospace: bool,
    best_source: Option<FontSource>,
    best_face: Option<(i32, RepresentativeFace)>,
}

fn detect_monospace(face: &fontdb::FaceInfo, source: &FontSource) -> bool {
    // 1. Primary check: ttf-parser is_monospaced() on the underlying font file data
    match source {
        FontSource::Binary { path, index } => {
            if let Ok(bytes) = fs::read(path)
                && let Ok(parsed_face) = ttf_parser::Face::parse(&bytes, *index)
            {
                return parsed_face.is_monospaced();
            }
        }
        FontSource::Synthetic { bytes, .. } => {
            if let Ok(parsed_face) = ttf_parser::Face::parse(bytes, face.index) {
                return parsed_face.is_monospaced();
            }
        }
    }

    // 2. Fallback check: fontdb FaceInfo monospaced metadata if file read fails
    face.monospaced
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_sorting_and_deduplication() {
        let candidates = vec![
            FontCandidate::new(
                "Segoe UI",
                false,
                FontSource::Synthetic {
                    id: "1".into(),
                    bytes: vec![],
                },
            ),
            FontCandidate::new(
                "Cascadia Mono",
                true,
                FontSource::Synthetic {
                    id: "2".into(),
                    bytes: vec![],
                },
            ),
            FontCandidate::new(
                "Segoe UI",
                false,
                FontSource::Synthetic {
                    id: "3".into(),
                    bytes: vec![],
                },
            ),
            FontCandidate::new(
                "Arial",
                false,
                FontSource::Synthetic {
                    id: "4".into(),
                    bytes: vec![],
                },
            ),
        ];

        let catalog = SystemFontCatalog::from_candidates(candidates);
        let names: Vec<&str> = catalog
            .candidates()
            .iter()
            .map(|c| c.family_name.as_str())
            .collect();

        assert_eq!(names, vec!["Arial", "Cascadia Mono", "Segoe UI"]);
    }
}
