//! Dopamine Mode subsystem: the Backspace "Neon Disintegration" particles and their sound.
//!
//! [`params`] holds every tunable number, [`particles`] the simulation, [`overlay`] the drawing,
//! [`synth`] the procedural sounds and [`audio`] their playback.

pub mod audio;
pub mod border;
pub mod combo;
pub mod gauge;
pub mod overlay;
pub mod params;
pub mod particles;
pub mod synth;
pub mod tab_rainbow;

use std::collections::HashMap;
use std::time::Instant;

use iced::{Color, Point};

use crate::workbench::TabId;

pub use audio::{DopamineAudio, DopamineSoundKind};
pub use border::{ActiveBorder, BORDER_DURATION, BorderEffect, BorderKind};
pub use combo::{
    COMBO_TIMEOUT, ClockState, ComboEvent, MAX_COMBO, MAX_GAUGE, TabComboState, TokenPhase,
};
pub use gauge::{
    ArrivalSpark, GAUGE_HEIGHT, GAUGE_INTERPOLATION_DURATION, SPARK_LIFETIME,
    TRANSFER_FLIGHT_DURATION, TRANSFER_ORIGIN_Y, TabGaugeDisplay, TabTransferManager,
    TransferParticle, calculate_particle_count, transfer_origin,
};
pub use overlay::ParticleOverlayProgram;
#[cfg(test)]
pub use particles::BACKSPACE_BURST_COUNT;
pub use particles::{MAX_PARTICLES_PER_TAB, Particle, ParticleKind, TabParticles};
pub use tab_rainbow::{
    HIT_BADGE_HEIGHT, HIT_BADGE_WIDTH, MAX_DISPLAY_COMBO, RAINBOW_SCROLL_SPEED, RAINBOW_WAVELENGTH,
    WAVE_AMPLITUDE, WAVE_SPATIAL_PERIOD, WAVE_TEMPORAL_HZ, active_rainbow_color, format_hit_text,
    hit_badge_element, inactive_rainbow_color, rainbow_wave_color, rainbow_wave_hue,
    rainbow_wave_phase, tab_close_button_style, tab_close_icon_color, tab_container_style,
    tab_diagonal_coord, tab_rainbow_gradient, tab_select_button_style, tab_title_color,
};

/// High-level controller managing particle simulations, border effects, audio feedback,
/// typing combo/gauge calculation, and Rainbow Wave animation across terminal tabs.
pub struct DopamineController {
    enabled: bool,
    tab_particles: HashMap<TabId, TabParticles>,
    tab_borders: HashMap<TabId, BorderEffect>,
    tab_combos: HashMap<TabId, TabComboState>,
    tab_gauges: HashMap<TabId, TabGaugeDisplay>,
    tab_transfers: HashMap<TabId, TabTransferManager>,
    audio: DopamineAudio,
    last_tick: Option<Instant>,
    animation_start: Option<Instant>,
}

impl DopamineController {
    /// Creates a new controller with the specified enabled state.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            tab_particles: HashMap::new(),
            tab_borders: HashMap::new(),
            tab_combos: HashMap::new(),
            tab_gauges: HashMap::new(),
            tab_transfers: HashMap::new(),
            audio: DopamineAudio::new(),
            last_tick: None,
            animation_start: if enabled { Some(Instant::now()) } else { None },
        }
    }

    /// Whether Dopamine Mode is active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Sets the enabled state. If disabled, all active particles, sounds, and animation
    /// timelines are terminated immediately.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.clear_all();
            self.audio.stop_all();
            self.animation_start = None;
        } else if self.animation_start.is_none() {
            self.animation_start = Some(Instant::now());
        }
    }

    /// Returns elapsed animation time in seconds for the Rainbow Wave.
    ///
    /// Monotonically advances while Dopamine Mode is enabled. Returns 0.0 when disabled.
    pub fn elapsed_animation_seconds(&self, now: Instant) -> f64 {
        if !self.enabled {
            return 0.0;
        }
        self.animation_start
            .map(|start| now.saturating_duration_since(start).as_secs_f64())
            .unwrap_or(0.0)
    }

    /// Injects an explicit animation start timestamp for deterministic testing.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn set_animation_start(&mut self, start: Instant) {
        self.animation_start = Some(start);
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

    /// Advances simulation using wall clock time and default viewport dimensions.
    #[allow(dead_code)]
    pub fn tick(&mut self, now: Instant, viewport_height: f32) {
        self.tick_with_viewport(now, 1000.0, viewport_height, None, None);
    }

    /// Advances simulation using wall clock time, active tab context, and viewport dimensions.
    pub fn tick_with_viewport(
        &mut self,
        now: Instant,
        viewport_width: f32,
        viewport_height: f32,
        active_tab: Option<TabId>,
        hit_origin: Option<Point>,
    ) {
        if !self.enabled {
            return;
        }

        let dt = if let Some(last) = self.last_tick {
            (now - last).as_secs_f32()
        } else {
            0.016
        };
        self.last_tick = Some(now);
        self.update(dt, viewport_height);

        // Prune expired border effects
        self.tab_borders.retain(|_, b| !b.is_expired(now));

        // Advance combo timers and transfer points across all tabs
        for (&tab_id, state) in self.tab_combos.iter_mut() {
            state.tick(now);
            let events = state.drain_events();
            for event in events {
                match event {
                    ComboEvent::TransferredToGauge {
                        points,
                        gauge_total,
                    } => {
                        let gauge_display = self.tab_gauges.entry(tab_id).or_insert_with(|| {
                            TabGaugeDisplay::new(gauge_total.saturating_sub(points))
                        });

                        if active_tab == Some(tab_id) {
                            gauge_display.set_target(gauge_total, now);

                            let origin = transfer_origin(
                                hit_origin.map_or(viewport_width / 2.0, |p| p.x),
                                viewport_width,
                                viewport_height,
                            );
                            let target_x = (viewport_width * (gauge_total as f32 / 100.0))
                                .clamp(8.0, (viewport_width - 8.0).max(8.0));
                            let target_y = viewport_height - GAUGE_HEIGHT / 2.0;
                            let target = Point::new(target_x, target_y);

                            self.tab_transfers
                                .entry(tab_id)
                                .or_default()
                                .spawn_transfer(points, origin, target, now);
                        } else {
                            gauge_display.snap_to(gauge_total);
                        }
                    }
                    ComboEvent::GaugeMaxReached => {}
                    ComboEvent::ComboIncremented { .. }
                    | ComboEvent::CommandSuccessDoubled { .. } => {}
                }
            }
        }

        // Advance gauge interpolation animations
        for gauge in self.tab_gauges.values_mut() {
            gauge.update(now);
        }

        // Re-synchronise the display with the Combo Core ground truth. Events only start the
        // visual effect; the Combo Core gauge is the single source of truth, so a lost event
        // (e.g. bounded queue overflow) converges here. Running interpolations are left alone.
        for (&tab_id, state) in self.tab_combos.iter() {
            let canonical = state.gauge();
            match self.tab_gauges.get_mut(&tab_id) {
                Some(display) => {
                    display.resync(canonical);
                }
                None if canonical > 0 => {
                    self.tab_gauges
                        .insert(tab_id, TabGaugeDisplay::new(canonical));
                }
                None => {}
            }
        }

        // Advance transfer particle flights and arrival sparks
        for transfer in self.tab_transfers.values_mut() {
            transfer.update(now);
        }

        // Prune inactive transfer managers to keep memory bounded
        self.tab_transfers.retain(|_, t| t.is_active());
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
    #[allow(dead_code)]
    pub fn has_active_borders(&self, now: Instant) -> bool {
        self.enabled && self.tab_borders.values().any(|b| !b.is_expired(now))
    }

    /// Whether any tab has active particles, borders, combo countdowns, transfers, or gauge interpolations.
    #[allow(dead_code)]
    pub fn has_active_animations(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let now = Instant::now();
        self.has_active_particles()
            || self.has_active_borders(now)
            || self.has_active_combos(now)
            || self.has_active_transfers()
            || self.has_active_gauge_animations(now)
    }

    /// Returns the gauge display state for `tab_id`, if any.
    pub fn gauge_display_for_tab(&self, tab_id: TabId) -> Option<&TabGaugeDisplay> {
        if !self.enabled {
            return None;
        }
        self.tab_gauges.get(&tab_id)
    }

    /// Returns the transfer manager for `tab_id`, if active and enabled.
    pub fn transfer_for_tab(&self, tab_id: TabId) -> Option<&TabTransferManager> {
        if !self.enabled {
            return None;
        }
        self.tab_transfers.get(&tab_id)
    }

    /// Returns current displayed gauge value (interpolated) for `tab_id`.
    #[allow(dead_code)]
    pub fn displayed_gauge_for_tab(&self, tab_id: TabId) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        self.tab_gauges
            .get(&tab_id)
            .map(|g| g.displayed_value())
            .unwrap_or(0.0)
    }

    /// Whether any tab has active transfer particles/sparks requiring animation frame redraws.
    pub fn has_active_transfers(&self) -> bool {
        self.enabled && self.tab_transfers.values().any(|t| t.is_active())
    }

    /// Whether any tab has an active gauge interpolation requiring animation frame redraws.
    pub fn has_active_gauge_animations(&self, now: Instant) -> bool {
        self.enabled && self.tab_gauges.values().any(|g| g.is_animating(now))
    }

    /// Whether any terminal tab currently has alive particles that require animation frames.
    #[allow(dead_code)]
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

    /// Returns current unexpired combo count for `tab_id`.
    #[allow(dead_code)]
    pub fn combo_for_tab(&self, tab_id: TabId, now: Instant) -> u32 {
        if !self.enabled {
            return 0;
        }
        self.tab_combos
            .get(&tab_id)
            .map(|s| s.combo(now))
            .unwrap_or(0)
    }

    /// Returns accumulated dopamine gauge points for `tab_id`.
    #[allow(dead_code)]
    pub fn gauge_for_tab(&self, tab_id: TabId) -> u32 {
        if !self.enabled {
            return 0;
        }
        self.tab_combos.get(&tab_id).map(|s| s.gauge()).unwrap_or(0)
    }

    /// Returns effective gauge points for `tab_id`, taking into account any pending expired combo.
    #[allow(dead_code)]
    pub fn effective_gauge_for_tab(&self, tab_id: TabId, now: Instant) -> u32 {
        if !self.enabled {
            return 0;
        }
        self.tab_combos
            .get(&tab_id)
            .map(|s| s.effective_gauge(now))
            .unwrap_or(0)
    }

    /// Whether an active combo is running for `tab_id`.
    #[allow(dead_code)]
    pub fn is_combo_active(&self, tab_id: TabId, now: Instant) -> bool {
        self.combo_for_tab(tab_id, now) > 0
    }

    /// Whether the gauge for `tab_id` is at maximum capacity (100).
    #[allow(dead_code)]
    pub fn is_gauge_max(&self, tab_id: TabId) -> bool {
        if !self.enabled {
            return false;
        }
        self.tab_combos
            .get(&tab_id)
            .map(|s| s.is_gauge_max())
            .unwrap_or(false)
    }

    /// Whether any tab has an active combo countdown ticking that requires animation frame redraws.
    ///
    /// When combo clocks are paused during command execution (C ~ D), this returns false to conserve CPU.
    #[allow(dead_code)]
    pub fn has_active_combos(&self, now: Instant) -> bool {
        self.enabled && self.tab_combos.values().any(|s| s.is_clock_ticking(now))
    }

    /// Forwards character input to `tab_id`'s combo state machine.
    pub fn on_char_input(&mut self, tab_id: TabId, c: char, now: Instant) {
        if !self.enabled {
            return;
        }
        self.tab_combos.entry(tab_id).or_default().on_char(c, now);
    }

    /// Forwards text block (paste, IME commit) to `tab_id`'s combo state machine.
    pub fn on_text_input(&mut self, tab_id: TabId, text: &str, now: Instant) {
        if !self.enabled {
            return;
        }
        self.tab_combos
            .entry(tab_id)
            .or_default()
            .on_text(text, now);
    }

    /// Forwards Enter keypress to `tab_id`'s combo state machine.
    pub fn on_enter(&mut self, tab_id: TabId, now: Instant) {
        if !self.enabled {
            return;
        }
        self.tab_combos.entry(tab_id).or_default().on_enter(now);
    }

    /// Forwards Backspace keypress to `tab_id`'s combo state machine.
    pub fn on_backspace_input(&mut self, tab_id: TabId, now: Instant) {
        if !self.enabled {
            return;
        }
        self.tab_combos.entry(tab_id).or_default().on_backspace(now);
    }

    /// Forwards CommandStarted (OSC 133 C) to `tab_id`'s combo state machine, pausing the clock.
    pub fn on_command_started(&mut self, tab_id: TabId, command_id: u64, now: Instant) {
        if !self.enabled {
            return;
        }
        self.tab_combos
            .entry(tab_id)
            .or_default()
            .on_command_started(command_id, now);
    }

    /// Forwards verified command completion result to `tab_id`'s combo state machine, resuming the clock.
    pub fn on_command_finished(
        &mut self,
        tab_id: TabId,
        command_id: u64,
        success: bool,
        valid_correlation: bool,
        now: Instant,
    ) {
        if !self.enabled {
            return;
        }
        self.tab_combos
            .entry(tab_id)
            .or_default()
            .on_command_finished(command_id, success, valid_correlation, now);
    }

    /// Forwards PromptStarted (OSC 133 A) to `tab_id`'s combo state machine for anomaly recovery.
    pub fn on_prompt_started(&mut self, tab_id: TabId, now: Instant) {
        if !self.enabled {
            return;
        }
        self.tab_combos
            .entry(tab_id)
            .or_default()
            .on_prompt_started(now);
    }

    /// Returns remaining time before combo timeout for `tab_id`.
    #[allow(dead_code)]
    pub fn remaining_time_for_tab(&self, tab_id: TabId, now: Instant) -> std::time::Duration {
        if !self.enabled {
            return std::time::Duration::ZERO;
        }
        self.tab_combos
            .get(&tab_id)
            .map(|s| s.remaining_time(now))
            .unwrap_or(std::time::Duration::ZERO)
    }

    /// Drains combo events for a specific tab.
    #[allow(dead_code)]
    pub fn drain_combo_events(&mut self, tab_id: TabId) -> Vec<ComboEvent> {
        self.tab_combos
            .get_mut(&tab_id)
            .map(|s| s.drain_events())
            .unwrap_or_default()
    }

    /// Consumes maximum gauge for a Burst effect on `tab_id`.
    #[allow(dead_code)]
    pub fn consume_gauge_burst(&mut self, tab_id: TabId) -> bool {
        if !self.enabled {
            return false;
        }
        let consumed = self
            .tab_combos
            .get_mut(&tab_id)
            .map(|s| s.consume_gauge_burst())
            .unwrap_or(false);
        if consumed && let Some(gauge) = self.tab_gauges.get_mut(&tab_id) {
            gauge.snap_to(0);
        }
        consumed
    }

    /// Cleans up only the combo counter, gauge, and pending combo events for `tab_id`,
    /// without affecting other tabs or ongoing particle/border effects.
    ///
    /// Combo transfer is NOT performed (points are discarded).
    pub fn clear_tab_combo(&mut self, tab_id: TabId) {
        self.tab_combos.remove(&tab_id);
        self.tab_gauges.remove(&tab_id);
        self.tab_transfers.remove(&tab_id);
    }

    /// Handles child shell exit for `tab_id` by safely discarding its combo state.
    pub fn on_shell_exited(&mut self, tab_id: TabId) {
        self.clear_tab_combo(tab_id);
    }

    /// Cleans up state when a terminal tab is closed.
    pub fn clear_tab(&mut self, tab_id: TabId) {
        self.tab_particles.remove(&tab_id);
        self.tab_borders.remove(&tab_id);
        self.tab_combos.remove(&tab_id);
        self.tab_gauges.remove(&tab_id);
        self.tab_transfers.remove(&tab_id);
    }

    /// Clears all particles, border effects, and combo/gauge states across all tabs.
    pub fn clear_all(&mut self) {
        self.tab_particles.clear();
        self.tab_borders.clear();
        self.tab_combos.clear();
        self.tab_gauges.clear();
        self.tab_transfers.clear();
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

    #[test]
    fn controller_combo_and_gauge_lifecycle_and_tab_isolation() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(false);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let tab2 = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        // 1. When disabled, inputs are ignored
        ctrl.on_text_input(tab1, "cargo test", t0);
        assert_eq!(ctrl.combo_for_tab(tab1, t0), 0);
        assert!(!ctrl.has_active_combos(t0));

        // 2. Enable controller
        ctrl.set_enabled(true);
        ctrl.on_text_input(tab1, "cargo test", t0);
        assert_eq!(ctrl.combo_for_tab(tab1, t0), 2);
        assert!(ctrl.is_combo_active(tab1, t0));
        assert!(ctrl.has_active_combos(t0));
        assert!(ctrl.has_active_animations());

        // Tab isolation: tab2 has 0 combo and 0 gauge
        assert_eq!(ctrl.combo_for_tab(tab2, t0), 0);
        assert_eq!(ctrl.gauge_for_tab(tab2), 0);

        // 3. Command C at t0 + 200ms pauses clock (remaining = 800ms)
        let t_c = t0 + Duration::from_millis(200);
        ctrl.on_enter(tab1, t_c);
        ctrl.on_command_started(tab1, 1, t_c);
        assert_eq!(ctrl.combo_for_tab(tab1, t_c), 2);
        assert_eq!(
            ctrl.remaining_time_for_tab(tab1, t_c),
            Duration::from_millis(1000)
        );
        // While paused, clock is not ticking frames
        assert!(!ctrl.has_active_combos(t_c));

        // Long command: 5 seconds pass while executing!
        let t_d = t_c + Duration::from_millis(5000);
        ctrl.tick(t_d, 800.0);
        assert_eq!(ctrl.combo_for_tab(tab1, t_d), 2);

        // Command finished D with Success doubles combo (2 -> 4) and resumes clock
        ctrl.on_command_finished(tab1, 1, true, true, t_d);
        assert_eq!(ctrl.combo_for_tab(tab1, t_d), 4);
        assert_eq!(ctrl.combo_for_tab(tab2, t_d), 0);
        assert!(ctrl.has_active_combos(t_d));

        // 4. Timeout after remaining time (1,000ms from D since enter was at t_c)
        let t_timeout = t_d + Duration::from_millis(1000);
        ctrl.tick(t_timeout, 800.0);
        assert_eq!(ctrl.combo_for_tab(tab1, t_timeout), 0);
        assert_eq!(ctrl.gauge_for_tab(tab1), 4);
        assert_eq!(ctrl.gauge_for_tab(tab2), 0);
        assert!(!ctrl.has_active_combos(t_timeout));

        // 5. Clear tab1 resets tab1 state
        ctrl.clear_tab(tab1);
        assert_eq!(ctrl.gauge_for_tab(tab1), 0);

        // 6. Set disabled resets all combos and gauges
        ctrl.on_text_input(tab2, "git status", t_timeout);
        let t_timeout2 = t_timeout + Duration::from_millis(1000);
        ctrl.tick(t_timeout2, 800.0);
        assert_eq!(ctrl.gauge_for_tab(tab2), 2);
        ctrl.set_enabled(false);
        assert_eq!(ctrl.gauge_for_tab(tab2), 0);
    }

    #[test]
    fn controller_n1_subscription_lifecycle_and_multi_tab_isolation() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let tab2 = tabs.add(TabKind::Terminal);
        let tab3 = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        // 1. Tab 1 Running, Tab 2 Paused, Tab 3 Idle
        ctrl.on_text_input(tab1, "cargo test", t0);
        assert_eq!(ctrl.combo_for_tab(tab1, t0), 2);

        ctrl.on_text_input(tab2, "git commit", t0);
        ctrl.on_command_started(tab2, 10, t0 + Duration::from_millis(100)); // Tab 2 Paused

        // Tab 3 is Idle (0 combo)
        assert_eq!(ctrl.combo_for_tab(tab3, t0), 0);

        // N-1 Requirement 1: Tab 1 Running & before deadline -> subscription needed
        let t_500 = t0 + Duration::from_millis(500);
        assert!(ctrl.has_active_combos(t_500));

        // N-1 Requirement 2: Tab 1 Running & exactly at deadline -> subscription needed before tick
        let t_1000 = t0 + Duration::from_millis(1000);
        assert!(ctrl.has_active_combos(t_1000));
        assert_eq!(ctrl.combo_for_tab(tab1, t_1000), 0); // read query shows expired
        assert_eq!(ctrl.effective_gauge_for_tab(tab1, t_1000), 2);
        assert_eq!(ctrl.gauge_for_tab(tab1), 0);

        // N-1 Requirement 3: Tab 1 Running & past deadline (1,500ms) -> subscription needed before tick
        let t_1500 = t0 + Duration::from_millis(1500);
        assert!(ctrl.has_active_combos(t_1500));

        // N-1 Requirement 7: Activity on other tab (tab2) doesn't lose tab1's pending transfer
        ctrl.on_char_input(tab2, 'x', t_1500);
        assert_eq!(ctrl.effective_gauge_for_tab(tab1, t_1500), 2);

        // N-1 Requirement 4: tick confirms timeout, transfers combo to gauge once
        ctrl.tick(t_1500, 800.0);
        assert_eq!(ctrl.gauge_for_tab(tab1), 2);
        assert_eq!(ctrl.combo_for_tab(tab1, t_1500), 0);

        // N-1 Requirement 5: After timeout applied, Tab 1 is Idle.
        // Tab 2 was Paused, Tab 3 is Idle.
        // Therefore, NO tabs are ticking frames!
        assert!(!ctrl.has_active_combos(t_1500));

        // Tab 2 is still Paused with its combo intact
        assert_eq!(ctrl.combo_for_tab(tab2, t_1500), 2);
    }

    #[test]
    fn controller_n2_shell_exited_cleanup() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let tab2 = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        // Setup Tab 1: Running with combo = 2, particles active, border active
        ctrl.on_text_input(tab1, "cargo test", t0);
        ctrl.on_backspace_colored(tab1, Point::new(10.0, 10.0), None, t0);
        ctrl.trigger_command_border(tab1, 1, BorderKind::Rainbow, t0);
        assert_eq!(ctrl.combo_for_tab(tab1, t0), 2);
        assert!(ctrl.has_active_particles());
        assert!(ctrl.has_active_borders(t0));

        // Setup Tab 2: Running with combo = 1
        ctrl.on_text_input(tab2, "ls", t0);
        assert_eq!(ctrl.combo_for_tab(tab2, t0), 1);

        // Shell exits on Tab 1 while Running
        ctrl.on_shell_exited(tab1);

        // N-2 Requirement 1 & 3: Tab 1 combo and gauge discarded, no transfer
        assert_eq!(ctrl.combo_for_tab(tab1, t0), 0);
        assert_eq!(ctrl.gauge_for_tab(tab1), 0);

        // N-2 Requirement 4: No pending events
        assert!(ctrl.drain_combo_events(tab1).is_empty());

        // N-2 Requirement 5: Tab 2 combo is maintained
        assert_eq!(ctrl.combo_for_tab(tab2, t0), 1);

        // N-2 Requirement 7: Tab 1 particles and borders are NOT cleared by shell exit
        assert_eq!(ctrl.particles_for_tab(tab1).len(), BACKSPACE_BURST_COUNT);
        assert!(ctrl.border_for_tab(tab1, t0).is_some());

        // N-2 Requirement 2: Shell exit while Paused
        ctrl.on_command_started(tab2, 2, t0 + Duration::from_millis(50));
        assert_eq!(ctrl.combo_for_tab(tab2, t0), 1);
        ctrl.on_shell_exited(tab2);
        assert_eq!(ctrl.combo_for_tab(tab2, t0), 0);
        assert_eq!(ctrl.gauge_for_tab(tab2), 0);

        // N-2 Requirement 6: TabClosed cleans up remaining particles/borders cleanly
        ctrl.clear_tab(tab1);
        assert_eq!(ctrl.particles_for_tab(tab1).len(), 0);
        assert!(ctrl.border_for_tab(tab1, t0).is_none());

        // N-2 Requirement 8: Disabled controller does not panic on on_shell_exited
        ctrl.set_enabled(false);
        ctrl.on_shell_exited(tab1);
    }

    #[test]
    fn controller_slice3_active_tab_transfer_and_gauge_interpolation() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        // Type a command to build combo = 5
        ctrl.on_text_input(tab1, "git commit -m foo bar", t0);
        assert_eq!(ctrl.combo_for_tab(tab1, t0), 5);
        assert_eq!(ctrl.gauge_for_tab(tab1), 0);

        // Advance past 1,000ms combo timeout
        let t_timeout = t0 + Duration::from_millis(1005);

        // Tick with tab1 as active tab
        let hit_origin = Point::new(100.0, -17.5);
        ctrl.tick_with_viewport(t_timeout, 800.0, 600.0, Some(tab1), Some(hit_origin));

        // Combo Core gauge is now 5
        assert_eq!(ctrl.gauge_for_tab(tab1), 5);
        assert_eq!(ctrl.combo_for_tab(tab1, t_timeout), 0);

        // Gauge display is created and interpolating towards 5
        let gauge = ctrl.gauge_display_for_tab(tab1).expect("gauge exists");
        assert_eq!(gauge.target_value(), 5.0);
        assert!(gauge.is_animating(t_timeout));
        assert!(ctrl.has_active_gauge_animations(t_timeout));

        // Transfer manager exists and has active flying particles
        let transfer = ctrl.transfer_for_tab(tab1).expect("transfer exists");
        assert!(transfer.is_active());
        assert!(ctrl.has_active_transfers());
        assert!(ctrl.has_active_animations());

        // Events have been drained (no leftover queue)
        assert!(ctrl.drain_combo_events(tab1).is_empty());

        // Fast forward 1 second: transfer particles finish & sparks extinguish
        let t_end = t_timeout + Duration::from_millis(1000);
        ctrl.tick_with_viewport(t_end, 800.0, 600.0, Some(tab1), Some(hit_origin));

        assert_eq!(ctrl.displayed_gauge_for_tab(tab1), 5.0);
        assert!(!ctrl.has_active_transfers());
        assert!(!ctrl.has_active_gauge_animations(t_end));
    }

    #[test]
    fn controller_slice3_inactive_tab_snaps_gauge_without_particles() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let active_tab = tabs.add(TabKind::Terminal);
        let background_tab = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        // Build combo = 3 on background tab
        ctrl.on_text_input(background_tab, "cat file.txt", t0);
        assert_eq!(ctrl.combo_for_tab(background_tab, t0), 2);

        // Advance past 1,000ms combo timeout
        let t_timeout = t0 + Duration::from_millis(1005);

        // Tick with active_tab (NOT background_tab)
        ctrl.tick_with_viewport(t_timeout, 800.0, 600.0, Some(active_tab), None);

        // Background tab's gauge value is immediately updated
        assert_eq!(ctrl.gauge_for_tab(background_tab), 2);

        // Background tab's display snapped to 2.0 without animating
        let gauge = ctrl
            .gauge_display_for_tab(background_tab)
            .expect("gauge exists");
        assert_eq!(gauge.target_value(), 2.0);
        assert_eq!(gauge.displayed_value(), 2.0);
        assert!(!gauge.is_animating(t_timeout));

        // NO transfer particles spawned for background tab
        assert!(ctrl.transfer_for_tab(background_tab).is_none());
        assert!(!ctrl.has_active_transfers());
    }

    #[test]
    fn controller_slice3_cleanup_clears_gauge_and_transfer() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab1 = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        ctrl.on_text_input(tab1, "vim test", t0);
        let t_timeout = t0 + Duration::from_millis(1005);
        ctrl.tick_with_viewport(t_timeout, 800.0, 600.0, Some(tab1), None);

        assert!(ctrl.gauge_display_for_tab(tab1).is_some());
        assert!(ctrl.transfer_for_tab(tab1).is_some());

        // clear_tab cleans up both
        ctrl.clear_tab(tab1);
        assert!(ctrl.gauge_display_for_tab(tab1).is_none());
        assert!(ctrl.transfer_for_tab(tab1).is_none());
        assert_eq!(ctrl.gauge_for_tab(tab1), 0);
    }

    fn typed_tokens(n: usize) -> String {
        (0..n).map(|_| "a").collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn slice3_f1_launch_points_and_trajectories_stay_inside_canvas() {
        use std::time::Duration;

        for (w, h) in [
            (800.0_f32, 600.0_f32),
            (120.0, 40.0),
            (10.0, 10.0),
            (1.0, 1.0),
        ] {
            let mut ctrl = DopamineController::new(true);
            let mut tabs = WorkbenchTabs::default();
            let tab = tabs.add(TabKind::Terminal);
            let t0 = Instant::now();
            ctrl.on_text_input(tab, &typed_tokens(30), t0);
            // The legacy hit origin (y = -17.5, outside the canvas) must be pulled inside.
            let t1 = t0 + Duration::from_millis(1005);
            ctrl.tick_with_viewport(t1, w, h, Some(tab), Some(Point::new(-40.0, -17.5)));

            let transfer = ctrl.transfer_for_tab(tab).expect("transfer spawned");
            assert!(!transfer.particles().is_empty());
            for p in transfer.particles() {
                assert!(
                    p.start.x >= 0.0 && p.start.x <= w,
                    "start.x {} / {w}",
                    p.start.x
                );
                assert!(
                    p.start.y >= 0.0 && p.start.y <= h,
                    "start.y {} / {h}",
                    p.start.y
                );
                assert!(p.end.x.is_finite() && p.end.y.is_finite());
                for k in 0..=20 {
                    let at = p.start_time + p.duration.mul_f32(k as f32 / 20.0);
                    let pos = p.current_pos(at).expect("launched");
                    assert!(pos.x.is_finite() && pos.y.is_finite());
                }
            }
        }
    }

    #[test]
    fn slice3_f1_launch_point_keeps_horizontal_badge_position_and_gauge_end() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();
        ctrl.on_text_input(tab, &typed_tokens(50), t0);
        let t1 = t0 + Duration::from_millis(1005);
        ctrl.tick_with_viewport(t1, 800.0, 600.0, Some(tab), Some(Point::new(412.0, -17.5)));

        let transfer = ctrl.transfer_for_tab(tab).unwrap();
        assert!(
            transfer
                .particles()
                .iter()
                .all(|p| p.start == Point::new(412.0, TRANSFER_ORIGIN_Y))
        );
        // Gauge value 50 of 100 -> the end point sits near the fill tip (x = 400), inside the bar.
        for p in transfer.particles() {
            assert!((p.end.x - 400.0).abs() <= 8.0, "end.x {}", p.end.x);
            assert!((p.end.y - (600.0 - GAUGE_HEIGHT / 2.0)).abs() < 1e-3);
        }
    }

    #[test]
    fn slice3_f2_lost_event_without_display_is_recreated_from_ground_truth() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();
        ctrl.on_text_input(tab, &typed_tokens(7), t0);
        let t1 = t0 + Duration::from_millis(1005);
        // Enter applies the timeout lazily; the resulting event is then lost.
        ctrl.on_enter(tab, t1);
        assert_eq!(ctrl.gauge_for_tab(tab), 7);
        assert!(!ctrl.drain_combo_events(tab).is_empty());

        ctrl.tick_with_viewport(t1, 800.0, 600.0, Some(tab), None);
        assert_eq!(ctrl.displayed_gauge_for_tab(tab), 7.0);
        assert_eq!(
            ctrl.gauge_for_tab(tab),
            7,
            "ground truth is never rewritten"
        );
        assert!(
            ctrl.transfer_for_tab(tab).is_none(),
            "no effect without an event"
        );
    }

    #[test]
    fn slice3_f2_lost_event_after_idle_display_snaps_to_ground_truth() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();

        ctrl.on_text_input(tab, &typed_tokens(2), t0);
        let t1 = t0 + Duration::from_millis(1005);
        ctrl.tick_with_viewport(t1, 800.0, 600.0, Some(tab), None);
        let t2 = t1 + Duration::from_millis(500);
        ctrl.tick_with_viewport(t2, 800.0, 600.0, Some(tab), None);
        assert_eq!(ctrl.displayed_gauge_for_tab(tab), 2.0);

        // Second transfer: +3, but its event never reaches the display.
        ctrl.on_text_input(tab, &typed_tokens(3), t2);
        let t3 = t2 + Duration::from_millis(1005);
        ctrl.on_enter(tab, t3);
        assert_eq!(ctrl.gauge_for_tab(tab), 5);
        ctrl.drain_combo_events(tab);

        ctrl.tick_with_viewport(t3, 800.0, 600.0, Some(tab), None);
        assert_eq!(ctrl.displayed_gauge_for_tab(tab), 5.0);
        assert_eq!(ctrl.gauge_for_tab(tab), 5);
    }

    #[test]
    fn slice3_f2_running_interpolation_is_not_interrupted_and_ends_on_truth() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let tab = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();
        ctrl.on_text_input(tab, &typed_tokens(40), t0);
        let t1 = t0 + Duration::from_millis(1005);
        ctrl.tick_with_viewport(t1, 800.0, 600.0, Some(tab), None);
        assert_eq!(ctrl.gauge_for_tab(tab), 40);

        // Mid-flight: display strictly between start and truth, still animating, not snapped.
        let t_mid = t1 + Duration::from_millis(200);
        ctrl.tick_with_viewport(t_mid, 800.0, 600.0, Some(tab), None);
        let mid = ctrl.displayed_gauge_for_tab(tab);
        assert!(mid > 0.0 && mid < 40.0, "mid = {mid}");
        assert!(ctrl.has_active_gauge_animations(t_mid));

        // Monotone, and exactly the truth at 450 ms.
        let t_late = t1 + Duration::from_millis(300);
        ctrl.tick_with_viewport(t_late, 800.0, 600.0, Some(tab), None);
        assert!(ctrl.displayed_gauge_for_tab(tab) >= mid);
        let t_end = t1 + Duration::from_millis(450);
        ctrl.tick_with_viewport(t_end, 800.0, 600.0, Some(tab), None);
        assert_eq!(ctrl.displayed_gauge_for_tab(tab), 40.0);
        assert!(!ctrl.has_active_gauge_animations(t_end));
    }

    #[test]
    fn slice3_f2_retargeting_mid_interpolation_continues_from_current_value() {
        use std::time::Duration;

        // 20 -> 48 at t=0, then 48 -> 76 at t=200ms, must reach 76 at t=650ms.
        let t0 = Instant::now();
        let mut display = TabGaugeDisplay::new(20);
        display.set_target(48, t0);
        let t200 = t0 + Duration::from_millis(200);
        display.update(t200);
        let at_200 = display.displayed_value();
        assert!(at_200 > 20.0 && at_200 < 48.0);

        display.set_target(76, t200);
        assert_eq!(display.target_value(), 76.0);
        let mut prev = at_200;
        for ms in (210..=650).step_by(10) {
            display.update(t0 + Duration::from_millis(ms));
            let v = display.displayed_value();
            assert!(
                v >= prev - 1e-4,
                "display moved backwards at {ms}ms: {prev} -> {v}"
            );
            assert!(v <= 76.0 + 1e-4);
            prev = v;
        }
        assert_eq!(display.displayed_value(), 76.0);
        assert!(!display.is_animating(t0 + Duration::from_millis(650)));
        // Re-sending the same (already reached) target does not restart the animation.
        display.set_target(76, t0 + Duration::from_millis(700));
        assert!(!display.is_animating(t0 + Duration::from_millis(700)));
    }

    #[test]
    fn slice3_f2_tabs_sync_independently_and_lifecycle_does_not_resurrect_state() {
        use std::time::Duration;

        let mut ctrl = DopamineController::new(true);
        let mut tabs = WorkbenchTabs::default();
        let a = tabs.add(TabKind::Terminal);
        let b = tabs.add(TabKind::Terminal);
        let t0 = Instant::now();
        ctrl.on_text_input(a, &typed_tokens(4), t0);
        ctrl.on_text_input(b, &typed_tokens(9), t0);
        let t1 = t0 + Duration::from_millis(1005);
        ctrl.on_enter(a, t1);
        ctrl.on_enter(b, t1);
        ctrl.drain_combo_events(a);
        ctrl.drain_combo_events(b);

        // `a` is active, `b` is in the background; both lost their events.
        ctrl.tick_with_viewport(t1, 800.0, 600.0, Some(a), None);
        assert_eq!(ctrl.displayed_gauge_for_tab(a), 4.0);
        assert_eq!(ctrl.displayed_gauge_for_tab(b), 9.0);

        // Close: nothing is re-created from stale state.
        ctrl.clear_tab(a);
        ctrl.tick_with_viewport(t1 + Duration::from_millis(16), 800.0, 600.0, Some(b), None);
        assert!(ctrl.gauge_display_for_tab(a).is_none());
        assert_eq!(ctrl.displayed_gauge_for_tab(b), 9.0);

        // ShellExited on b.
        ctrl.on_shell_exited(b);
        ctrl.tick_with_viewport(t1 + Duration::from_millis(32), 800.0, 600.0, Some(b), None);
        assert!(ctrl.gauge_display_for_tab(b).is_none());
        assert_eq!(ctrl.gauge_for_tab(b), 0);

        // Mode OFF then ON: old gauges never come back.
        ctrl.on_text_input(a, &typed_tokens(3), t1);
        ctrl.on_enter(a, t1 + Duration::from_millis(1005));
        ctrl.set_enabled(false);
        ctrl.tick_with_viewport(
            t1 + Duration::from_millis(2100),
            800.0,
            600.0,
            Some(a),
            None,
        );
        ctrl.set_enabled(true);
        ctrl.tick_with_viewport(
            t1 + Duration::from_millis(2200),
            800.0,
            600.0,
            Some(a),
            None,
        );
        assert!(ctrl.gauge_display_for_tab(a).is_none());
        assert_eq!(ctrl.gauge_for_tab(a), 0);
    }
}
