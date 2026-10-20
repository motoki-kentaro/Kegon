//! Combo Transfers that wait for their Gain Chime to start before the visuals begin.
//!
//! Rendering a big chime takes up to ~120 ms on a worker thread, so a Transfer whose sound is
//! on the way is held here and released at the instant the sound is handed to the audio engine.
//! Gauge fill and particles then start from that shared instant and end on the chime's landing
//! bell ([`super::chime::ChimeTimeline::landing`]). A Transfer never waits for ever: if the
//! sound does not start within [`MAX_AUDIO_WAIT`] (worker failure, very slow machine) the
//! visuals start on their own. Without a sound there is nothing to wait for and the caller
//! starts the visuals at once.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use iced::Point;

use crate::workbench::TabId;

/// Longest a Transfer waits for its sound before the visuals go ahead without it.
pub const MAX_AUDIO_WAIT: Duration = Duration::from_millis(300);

/// Estimated delay between handing PCM to the audio engine and hearing it (device buffer). This
/// is an assumption, not a measurement: the backend does not report its output latency.
pub const OUTPUT_LATENCY_ESTIMATE: Duration = Duration::from_millis(30);

#[derive(Debug, Clone)]
pub struct PendingTransfer {
    pub tab: TabId,
    pub ticket: u64,
    /// Original combo: the particle count and the chime's `delta`.
    pub points: u32,
    pub gauge_total: u32,
    pub origin: Point,
    pub target: Point,
    pub event_at: Instant,
}

#[derive(Debug, Default)]
pub struct PendingTransfers {
    items: VecDeque<PendingTransfer>,
}

impl PendingTransfers {
    pub fn push(&mut self, item: PendingTransfer) {
        self.items.push_back(item);
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn has_tab(&self, tab: TabId) -> bool {
        self.items.iter().any(|p| p.tab == tab)
    }

    pub fn remove_tab(&mut self, tab: TabId) {
        self.items.retain(|p| p.tab != tab);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Removes and returns the Transfers that may start now, each with its start instant, in
    /// their original order.
    ///
    /// * A Transfer whose ticket is in `started` starts at `now + OUTPUT_LATENCY_ESTIMATE`; older
    ///   Transfers of the same tab start with it, so a fast small chime cannot overtake a slow
    ///   big one and the Gauge never fills out of order.
    /// * A Transfer older than [`MAX_AUDIO_WAIT`] starts at `now` (its sound did not come).
    pub fn take_ready(&mut self, started: &[u64], now: Instant) -> Vec<(PendingTransfer, Instant)> {
        if self.items.is_empty() {
            return Vec::new();
        }
        let audible = now + OUTPUT_LATENCY_ESTIMATE;
        let mut ready = Vec::new();
        let mut waiting = VecDeque::with_capacity(self.items.len());
        // Per tab, the queue index of its last Transfer whose sound has started.
        let last_started = |tab: TabId| {
            self.items
                .iter()
                .enumerate()
                .filter(|(_, p)| p.tab == tab && started.contains(&p.ticket))
                .map(|(i, _)| i)
                .max()
        };
        let sweep_until: Vec<Option<usize>> =
            self.items.iter().map(|p| last_started(p.tab)).collect();
        for (i, item) in self.items.drain(..).enumerate() {
            if sweep_until[i].is_some_and(|last| i <= last) {
                ready.push((item, audible));
            } else if now.saturating_duration_since(item.event_at) >= MAX_AUDIO_WAIT {
                ready.push((item, now));
            } else {
                waiting.push_back(item);
            }
        }
        self.items = waiting;
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::{TabKind, WorkbenchTabs};

    fn item(tab: TabId, ticket: u64, at: Instant) -> PendingTransfer {
        PendingTransfer {
            tab,
            ticket,
            points: 10,
            gauge_total: 10,
            origin: Point::new(1.0, 1.0),
            target: Point::new(2.0, 2.0),
            event_at: at,
        }
    }

    fn tabs() -> (TabId, TabId) {
        let mut t = WorkbenchTabs::default();
        (t.add(TabKind::Terminal), t.add(TabKind::Terminal))
    }

    #[test]
    fn a_transfer_waits_until_its_sound_starts_then_follows_the_audible_instant() {
        let (a, _) = tabs();
        let t0 = Instant::now();
        let mut p = PendingTransfers::default();
        p.push(item(a, 1, t0));
        assert!(p.take_ready(&[], t0 + Duration::from_millis(50)).is_empty());
        assert!(p.has_tab(a));
        let now = t0 + Duration::from_millis(110);
        let ready = p.take_ready(&[1], now);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].1, now + OUTPUT_LATENCY_ESTIMATE);
        assert!(p.is_empty());
    }

    #[test]
    fn a_missing_sound_never_blocks_the_visuals_for_more_than_the_wait_limit() {
        let (a, _) = tabs();
        let t0 = Instant::now();
        let mut p = PendingTransfers::default();
        p.push(item(a, 1, t0));
        assert!(
            p.take_ready(&[], t0 + MAX_AUDIO_WAIT - Duration::from_millis(1))
                .is_empty()
        );
        let now = t0 + MAX_AUDIO_WAIT;
        let ready = p.take_ready(&[], now);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].1, now, "no latency offset without a sound");
        assert!(p.is_empty());
    }

    #[test]
    fn a_fast_small_chime_releases_the_older_big_one_with_it_so_the_gauge_fills_in_order() {
        let (a, b) = tabs();
        let t0 = Instant::now();
        let mut p = PendingTransfers::default();
        p.push(item(a, 1, t0)); // big, slow to render
        p.push(item(b, 2, t0)); // other tab
        p.push(item(a, 3, t0 + Duration::from_millis(5))); // small, ready first
        let now = t0 + Duration::from_millis(40);
        let ready = p.take_ready(&[3], now);
        let tickets: Vec<u64> = ready.iter().map(|r| r.0.ticket).collect();
        assert_eq!(tickets, vec![1, 3], "original order, same tab only");
        assert_eq!(p.len(), 1);
        assert!(p.has_tab(b) && !p.has_tab(a));
    }

    #[test]
    fn tabs_are_removed_and_cleared_independently() {
        let (a, b) = tabs();
        let t0 = Instant::now();
        let mut p = PendingTransfers::default();
        p.push(item(a, 1, t0));
        p.push(item(b, 2, t0));
        p.remove_tab(a);
        assert!(!p.has_tab(a) && p.has_tab(b));
        p.clear();
        assert!(p.is_empty());
    }
}
