//! Dopamine Mode subsystem: the Backspace "Neon Disintegration" particles and their sound.
//!
//! [`params`] holds every tunable number, [`particles`] the simulation, [`overlay`] the drawing,
//! [`synth`] the procedural sounds and [`audio`] their playback.

pub mod audio;
pub mod border;
pub mod overlay;
pub mod params;
pub mod particles;
pub mod synth;

use std::collections::HashMap;
use std::time::Instant;

use iced::{Color, Point};

use crate::workbench::TabId;

pub use audio::{DopamineAudio, DopamineSoundKind};
pub use border::{ActiveBorder, BORDER_DURATION, BorderEffect, BorderKind};
pub use overlay::ParticleOverlayProgram;
#[cfg(test)]
pub use particles::BACKSPACE_BURST_COUNT;
pub use particles::{MAX_PARTICLES_PER_TAB, Particle, ParticleKind, TabParticles};

/// High-level controller managing particle simulations, border effects, and audio feedback across terminal tabs.
pub struct DopamineController {
    enabled: bool,
    tab_particles: HashMap<TabId, TabParticles>,
    tab_borders: HashMap<TabId, BorderEffect>,
    audio: DopamineAudio,
    last_tick: Option<Instant>,
}

impl DopamineController {
    /// Creates a new controller with the specified enabled state.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            tab_particles: HashMap::new(),
            tab_borders: HashMap::new(),
            audio: DopamineAudio::new(),
            last_tick: None,
        }
    }

    /// Whether Dopamine Mode is active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Sets the enabled state. If disabled, all active particles and sounds are terminated immediately.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.clear_all();
            self.audio.stop_all();
        }
    }

    /// Sets the active sound profile.
    pub fn set_sound_kind(&mut self, kind: DopamineSoundKind) {
        self.audio.set_sound_kind(kind);
    }

    /// Triggers the Backspace effect at the given cursor cell pixel coordinates, in the fallback
    /// palette.
    #[allow(dead_code)]
    pub fn on_backspace(&mut self, tab_id: TabId, origin: Point) {
        self.on_backspace_colored(tab_id, origin, None, Instant::now());
    }

    /// Triggers the Backspace effect. `base` is the foreground colour of the character next to the
    /// cursor when it is known. Sight and sound are fired together; each limits its own rate.
    pub fn on_backspace_colored(
        &mut self,
        tab_id: TabId,
        origin: Point,
        base: Option<Color>,
        now: Instant,
    ) {
        if !self.enabled {
            return;
        }
        self.tab_particles
            .entry(tab_id)
            .or_default()
            .spawn_erase_burst(origin, base, now);
        self.audio.play_erase_at(now);
    }

    /// Updates physics and lifetimes of all particles across all tabs.
    pub fn update(&mut self, dt: f32, viewport_height: f32) {
        if !self.enabled || self.tab_particles.is_empty() {
            self.last_tick = None;
            return;
        }

        let mut remaining = dt.clamp(0.001, 5.0);
        while remaining > 0.0001 {
            let step = remaining.min(0.033);
            for tab_particles in self.tab_particles.values_mut() {
                tab_particles.update(step, viewport_height);
            }
            remaining -= step;
        }

        // Remove entries whose particle pools are exhausted to keep memory bounded
        self.tab_particles.retain(|_, tp| !tp.is_empty());

        if self.tab_particles.is_empty() {
            self.last_tick = None;
        }
    }

    /// Advances simulation using wall clock time and prunes particles and expired borders.
    pub fn tick(&mut self, now: Instant, viewport_height: f32) {
        let dt = if let Some(last) = self.last_tick {
            (now - last).as_secs_f32()
        } else {
            0.016
        };
        self.last_tick = Some(now);
        self.update(dt, viewport_height);

        // Prune expired border effects
        self.tab_borders.retain(|_, b| !b.is_expired(now));
    }

    /// Triggers a command completion border effect for the specified tab.
    ///
    /// If an effect is already active for this tab:
    /// - If the `command_id` matches the existing effect, it is deduplicated and ignored.
    /// - Otherwise, the existing effect is replaced with the new one.
    pub fn trigger_command_border(
        &mut self,
        tab_id: TabId,
        command_id: u64,
        kind: BorderKind,
        now: Instant,
    ) {
        if !self.enabled {
            return;
        }

        if let Some(existing) = self.tab_borders.get(&tab_id)
            && existing.command_id == command_id
            && !existing.is_expired(now)
        {
            return;
        }

        self.tab_borders
            .insert(tab_id, BorderEffect::new(kind, command_id, now));
    }

    /// Returns the active border rendering state for `tab_id`, if active and not expired.
    pub fn border_for_tab(&self, tab_id: TabId, now: Instant) -> Option<ActiveBorder> {
        if !self.enabled {
            return None;
        }
        self.tab_borders
            .get(&tab_id)
            .and_then(|b| b.current_active(now))
    }

    /// Whether any terminal tab currently has an active border effect requiring frame redraws.
    pub fn has_active_borders(&self, now: Instant) -> bool {
        self.enabled && self.tab_borders.values().any(|b| !b.is_expired(now))
    }

    /// Whether any tab has active particles or active border effects requiring animation frame redraws.
    pub fn has_active_animations(&self) -> bool {
        if !self.enabled {
            return false;
        }
        self.has_active_particles() || self.has_active_borders(Instant::now())
    }

    /// Whether any terminal tab currently has alive particles that require animation frames.
    pub fn has_active_particles(&self) -> bool {
        self.enabled && self.tab_particles.values().any(|tp| !tp.is_empty())
    }

    /// Returns a slice of active particles for the given tab, or an empty slice if none or disabled.
    pub fn particles_for_tab(&self, tab_id: TabId) -> &[Particle] {
        if !self.enabled {
            return &[];
        }
        self.tab_particles
            .get(&tab_id)
            .map(|tp| tp.particles())
            .unwrap_or(&[])
    }

    /// Cleans up state when a terminal tab is closed.
    pub fn clear_tab(&mut self, tab_id: TabId) {
        self.tab_particles.remove(&tab_id);
        self.tab_borders.remove(&tab_id);
    }

    /// Clears all particles and border effects across all tabs.
    pub fn clear_all(&mut self) {
        self.tab_particles.clear();
        self.tab_borders.clear();
        self.last_tick = None;
    }

    /// Reference to the audio subsystem.
    #[allow(dead_code)]
    pub fn audio(&self) -> &DopamineAudio {
        &self.audio
    }

    /// Mutable reference to the audio subsystem.
    #[allow(dead_code)]
    pub fn audio_mut(&mut self) -> &mut DopamineAudio {
        &mut self.audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::{TabKind, WorkbenchTabs};

    #[test]
    fn controller_respects_enabled_state() {
        let mut ctrl = DopamineController::new(false);
        let mut tabs = WorkbenchTabs::default();
        let tab = tabs.add(TabKind::Terminal);

        ctrl.on_backspace(tab, Point::new(10.0, 10.0));
        assert!(!ctrl.has_active_particles());
        assert_eq!(ctrl.particles_for_tab(tab).len(), 0);

        ctrl.set_enabled(true);
        ctrl.on_backspace(tab, Point::new(10.0, 10.0));
        assert!(ctrl.has_active_particles());
        assert_eq!(ctrl.particles_for_tab(tab).len(), BACKSPACE_BURST_COUNT);

        ctrl.set_enabled(false);
        assert!(!ctrl.has_active_particles());
        assert_eq!(ctrl.particles_for_tab(tab).len(), 0);
    }

    #[test]
    fn tab_isolation() {
        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let tab2 = tabs.add(TabKind::Terminal);

        ctrl.on_backspace(tab1, Point::new(10.0, 10.0));

        assert_eq!(ctrl.particles_for_tab(tab1).len(), BACKSPACE_BURST_COUNT);
        assert_eq!(ctrl.particles_for_tab(tab2).len(), 0);

        ctrl.clear_tab(tab1);
        assert_eq!(ctrl.particles_for_tab(tab1).len(), 0);
    }

    #[test]
    fn animation_idle_transition_after_particles_fall_off_screen() {
        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab = tabs.add(TabKind::Terminal);

        // Spawn near the bottom of an 800px screen
        ctrl.on_backspace(tab, Point::new(100.0, 700.0));
        assert!(ctrl.has_active_particles());

        // Fast-forward 1.5 seconds: particles accelerate down, pass 800px, and are pruned
        ctrl.update(4.0, 800.0);

        assert!(!ctrl.has_active_particles(), "Must return to idle 0% CPU");
        assert!(ctrl.tab_particles.is_empty());
    }

    #[test]
    fn audio_accessors_and_sound_kind_sync() {
        let mut ctrl = DopamineController::new(true);
        ctrl.set_sound_kind(DopamineSoundKind::Crack);
        assert_eq!(ctrl.audio().sound_kind(), DopamineSoundKind::Crack);
        ctrl.audio_mut().stop_all();
    }

    #[test]
    fn controller_command_border_lifecycle_and_deduplication() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(false);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let tab2 = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        // 1. Disabled controller ignores triggers
        ctrl.trigger_command_border(tab1, 100, BorderKind::Rainbow, t0);
        assert!(!ctrl.has_active_borders(t0));
        assert!(ctrl.border_for_tab(tab1, t0).is_none());

        // 2. Enable controller and trigger Rainbow on tab1
        ctrl.set_enabled(true);
        ctrl.trigger_command_border(tab1, 100, BorderKind::Rainbow, t0);
        assert!(ctrl.has_active_borders(t0));
        assert!(ctrl.has_active_animations());
        let border1 = ctrl.border_for_tab(tab1, t0).unwrap();
        assert_eq!(border1.kind, BorderKind::Rainbow);
        assert_eq!(border1.alpha, 1.0);

        // Tab isolation: tab2 has no border
        assert!(ctrl.border_for_tab(tab2, t0).is_none());

        // 3. Deduplication: triggering same command_id (100) before expiry is ignored
        let t_half = t0 + Duration::from_millis(200);
        ctrl.trigger_command_border(tab1, 100, BorderKind::Red, t_half);
        // Still Rainbow with faded alpha, not replaced by Red
        let border_dedup = ctrl.border_for_tab(tab1, t_half).unwrap();
        assert_eq!(border_dedup.kind, BorderKind::Rainbow);
        assert!((border_dedup.alpha - 0.5).abs() < 0.05);

        // 4. Replacement: new command_id (101) immediately replaces with fresh Red border
        ctrl.trigger_command_border(tab1, 101, BorderKind::Red, t_half);
        let border_rep = ctrl.border_for_tab(tab1, t_half).unwrap();
        assert_eq!(border_rep.kind, BorderKind::Red);
        assert_eq!(border_rep.alpha, 1.0);

        // 5. Expiry after tick
        let t_exp = t_half + Duration::from_millis(450);
        ctrl.tick(t_exp, 800.0);
        assert!(!ctrl.has_active_borders(t_exp));
        assert!(ctrl.border_for_tab(tab1, t_exp).is_none());

        // 6. Tab clear
        ctrl.trigger_command_border(tab2, 200, BorderKind::Rainbow, t0);
        assert!(ctrl.border_for_tab(tab2, t0).is_some());
        ctrl.clear_tab(tab2);
        assert!(ctrl.border_for_tab(tab2, t0).is_none());

        // 7. Clear all when disabled
        ctrl.trigger_command_border(tab1, 300, BorderKind::Rainbow, t0);
        assert!(ctrl.border_for_tab(tab1, t0).is_some());
        ctrl.set_enabled(false);
        assert!(!ctrl.has_active_borders(t0));
        assert!(ctrl.border_for_tab(tab1, t0).is_none());
    }
}
