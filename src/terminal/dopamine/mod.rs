//! Dopamine Mode subsystem: the Backspace "Neon Disintegration" particles and their sound.
//!
//! [`params`] holds every tunable number, [`particles`] the simulation, [`overlay`] the drawing,
//! [`synth`] the procedural sounds and [`audio`] their playback.

pub mod audio;
pub mod overlay;
pub mod params;
pub mod particles;
pub mod synth;

use std::collections::HashMap;
use std::time::Instant;

use iced::{Color, Point};

use crate::workbench::TabId;

pub use audio::{DopamineAudio, DopamineSoundKind};
pub use overlay::ParticleOverlayProgram;
#[cfg(test)]
pub use particles::BACKSPACE_BURST_COUNT;
pub use particles::{MAX_PARTICLES_PER_TAB, Particle, ParticleKind, TabParticles};

/// High-level controller managing particle simulations and audio feedback across terminal tabs.
pub struct DopamineController {
    enabled: bool,
    tab_particles: HashMap<TabId, TabParticles>,
    audio: DopamineAudio,
    last_tick: Option<Instant>,
}

impl DopamineController {
    /// Creates a new controller with the specified enabled state.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            tab_particles: HashMap::new(),
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

    /// Advances simulation using wall clock time and prunes particles exiting `viewport_height`.
    pub fn tick(&mut self, now: Instant, viewport_height: f32) {
        let dt = if let Some(last) = self.last_tick {
            (now - last).as_secs_f32()
        } else {
            0.016
        };
        self.last_tick = Some(now);
        self.update(dt, viewport_height);
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
    }

    /// Clears all particles across all tabs.
    pub fn clear_all(&mut self) {
        self.tab_particles.clear();
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
}
