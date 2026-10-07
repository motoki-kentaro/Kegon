//! Font bytes cache and runtime registration manager.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;

use crate::font::candidate::{FontCandidate, FontSource};

/// In-memory cache for font file bytes to avoid redundant disk reads per frame.
#[derive(Debug, Default)]
pub struct FontCache {
    /// Cached font bytes keyed by family name.
    bytes_cache: HashMap<String, Arc<Vec<u8>>>,
    /// Set of family names whose font byte loading failed (prevents retrying bad files).
    failed_families: HashSet<String>,
}

impl FontCache {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieve or load font bytes for the given candidate.
    /// Returns `Some(Arc<Vec<u8>>)` on success, or `None` if font file could not be loaded.
    pub fn get_or_load(&mut self, candidate: &FontCandidate) -> Option<Arc<Vec<u8>>> {
        let family_name = &candidate.family_name;

        if let Some(bytes) = self.bytes_cache.get(family_name) {
            return Some(Arc::clone(bytes));
        }

        if self.failed_families.contains(family_name) {
            return None;
        }

        let bytes = match &candidate.source {
            FontSource::Binary { path, .. } => match fs::read(path) {
                Ok(data) => data,
                Err(err) => {
                    eprintln!(
                        "Warning: Failed to load font file '{path:?}' for family '{family_name}': {err}"
                    );
                    self.failed_families.insert(family_name.clone());
                    return None;
                }
            },
            FontSource::Synthetic { bytes, .. } => bytes.clone(),
        };

        let arc_bytes = Arc::new(bytes);
        self.bytes_cache
            .insert(family_name.clone(), Arc::clone(&arc_bytes));
        Some(arc_bytes)
    }

    /// Check whether font bytes are already cached in memory.
    #[allow(dead_code)]
    pub fn is_cached(&self, family_name: &str) -> bool {
        self.bytes_cache.contains_key(family_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn cache_prevents_duplicate_loads() {
        let mut cache = FontCache::new();
        let candidate = FontCandidate::new(
            "Test Family",
            false,
            FontSource::Synthetic {
                id: "test".into(),
                bytes: vec![1, 2, 3, 4],
            },
        );

        let load1 = cache.get_or_load(&candidate);
        assert!(load1.is_some());
        assert!(cache.is_cached("Test Family"));

        let load2 = cache.get_or_load(&candidate);
        assert!(load2.is_some());

        // Same Arc pointer returned
        assert!(Arc::ptr_eq(&load1.unwrap(), &load2.unwrap()));
    }

    #[test]
    fn failed_file_load_is_recorded_and_does_not_panic() {
        let mut cache = FontCache::new();
        let candidate = FontCandidate::new(
            "Nonexistent Font",
            false,
            FontSource::Binary {
                path: PathBuf::from("nonexistent_font_file.ttf"),
                index: 0,
            },
        );

        let result = cache.get_or_load(&candidate);
        assert!(result.is_none());
        assert!(cache.failed_families.contains("Nonexistent Font"));

        // Subsequent load attempt returns None immediately without disk retry
        let retry_result = cache.get_or_load(&candidate);
        assert!(retry_result.is_none());
    }
}
