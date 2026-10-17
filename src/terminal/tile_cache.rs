//! A small bounded cache for the shade tiles the renderer draws.
//!
//! A tile is described by [`ShadeTileKey`] alone (pattern, physical size and phase), never by color, so a
//! screen needs a few dozen of them at most: three patterns, eight phases, and the one to four different
//! cell sizes that rounding gives. The cache still guards against growth, because the cell size can change
//! (font size, DPI, resize) and every new size makes new keys:
//! - it never holds more than [`MAX_TILES`] entries: inserting into a full cache evicts the least recently
//!   used one;
//! - an entry not used for [`IDLE_FRAMES`] frames is dropped by a sweep that runs every [`SWEEP_EVERY`]
//!   frames, so the tiles of an old cell size disappear once nothing draws them any more.

use std::collections::HashMap;

use crate::terminal::glyph::ShadeTileKey;

/// The most tiles kept at once. Real use needs under a hundred; this is a bound, not a target.
pub const MAX_TILES: usize = 256;
/// Frames without a use after which a tile is dropped.
pub const IDLE_FRAMES: u64 = 600;
/// How often, in frames, idle tiles are looked for.
pub const SWEEP_EVERY: u64 = 64;

struct Entry<V> {
    value: V,
    last_used: u64,
}

/// Maps a tile key to a value built once for it (the renderer stores an SVG handle).
pub struct TileCache<V> {
    entries: HashMap<ShadeTileKey, Entry<V>>,
    /// Incremented by [`Self::begin_frame`].
    frame: u64,
    /// Incremented by every lookup; orders uses within a frame for the least-recently-used choice.
    clock: u64,
}

impl<V> Default for TileCache<V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            frame: 0,
            clock: 0,
        }
    }
}

impl<V: Clone> TileCache<V> {
    /// Marks the start of a frame and, now and then, drops tiles that have gone unused.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        if self.frame.is_multiple_of(SWEEP_EVERY) {
            let frame = self.frame;
            self.entries
                .retain(|_, e| frame - e.last_used / STAMP_FRAME_UNIT < IDLE_FRAMES);
        }
    }

    /// The value for `key`, built with `make` the first time (or again after it was evicted).
    pub fn get_or_insert_with(&mut self, key: ShadeTileKey, make: impl FnOnce() -> V) -> V {
        self.clock += 1;
        let (frame, clock) = (self.frame, self.clock);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = clock_stamp(frame, clock);
            return entry.value.clone();
        }
        if self.entries.len() >= MAX_TILES
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| *k)
        {
            self.entries.remove(&oldest);
        }
        let value = make();
        self.entries.insert(
            key,
            Entry {
                value: value.clone(),
                last_used: clock_stamp(frame, clock),
            },
        );
        value
    }

    /// Drops every tile (they are built again when next drawn).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// `last_used` packs the frame in the high bits and the in-frame clock in the low bits, so that it orders
/// by recency and still tells which frame the use happened in.
const STAMP_FRAME_UNIT: u64 = 1 << 32;

fn clock_stamp(frame: u64, clock: u64) -> u64 {
    (frame << 32) | (clock & (STAMP_FRAME_UNIT - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::glyph::ShadeLevel;

    fn key(width_px: i32) -> ShadeTileKey {
        ShadeTileKey {
            level: ShadeLevel::Medium,
            width_px,
            height_px: 19,
            phase_x: 0,
            phase_y: 0,
        }
    }

    #[test]
    fn a_tile_is_built_once_and_then_reused() {
        let mut cache = TileCache::default();
        let mut built = 0;
        for _ in 0..1000 {
            cache.begin_frame();
            for _ in 0..50 {
                let v = cache.get_or_insert_with(key(10), || {
                    built += 1;
                    7u32
                });
                assert_eq!(v, 7);
            }
        }
        assert_eq!(built, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_cache_never_exceeds_its_bound_however_many_sizes_come_by() {
        let mut cache = TileCache::default();
        for width in 0..10_000 {
            cache.begin_frame();
            cache.get_or_insert_with(key(width), || width);
            assert!(cache.len() <= MAX_TILES, "{}", cache.len());
        }
        assert_eq!(cache.len().min(MAX_TILES), cache.len());
    }

    #[test]
    fn a_full_cache_evicts_the_least_recently_used_tile() {
        let mut cache = TileCache::default();
        cache.begin_frame();
        for width in 0..MAX_TILES as i32 {
            cache.get_or_insert_with(key(width), || width);
        }
        // Touch everything but tile 5, then add one more: tile 5 goes.
        for width in (0..MAX_TILES as i32).filter(|&w| w != 5) {
            cache.get_or_insert_with(key(width), || -1);
        }
        cache.get_or_insert_with(key(10_000), || 10_000);
        assert_eq!(cache.len(), MAX_TILES);
        let mut rebuilt = false;
        cache.get_or_insert_with(key(5), || {
            rebuilt = true;
            5
        });
        assert!(rebuilt, "tile 5 had been evicted");
        let mut kept = true;
        cache.get_or_insert_with(key(6), || {
            kept = false;
            6
        });
        assert!(kept, "a recently used tile stays");
    }

    #[test]
    fn tiles_of_an_old_cell_size_age_out_when_nothing_draws_them() {
        let mut cache = TileCache::default();
        cache.begin_frame();
        for width in 0..20 {
            cache.get_or_insert_with(key(width), || width);
        }
        assert_eq!(cache.len(), 20);
        // The cell size changed: only the new tile is used from now on.
        for _ in 0..(IDLE_FRAMES + 2 * SWEEP_EVERY) {
            cache.begin_frame();
            cache.get_or_insert_with(key(100), || 100);
        }
        assert_eq!(cache.len(), 1, "the 20 old tiles are gone");
    }

    #[test]
    fn a_tile_in_use_survives_the_sweeps() {
        let mut cache = TileCache::default();
        for _ in 0..(10 * SWEEP_EVERY) {
            cache.begin_frame();
            cache.get_or_insert_with(key(10), || 1u8);
        }
        let mut rebuilt = false;
        cache.get_or_insert_with(key(10), || {
            rebuilt = true;
            1
        });
        assert!(!rebuilt);
    }

    #[test]
    fn clear_drops_every_tile_and_they_are_built_again_on_demand() {
        let mut cache = TileCache::default();
        for width in 0..10 {
            cache.get_or_insert_with(key(width), || width);
        }
        assert_eq!(cache.len(), 10);
        cache.clear();
        assert_eq!(cache.len(), 0);
        let mut rebuilt = false;
        cache.get_or_insert_with(key(3), || {
            rebuilt = true;
            3
        });
        assert!(rebuilt);
        assert_eq!(cache.len(), 1);
    }
}
