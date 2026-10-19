//! Combo Counter and Dopamine Gauge calculation foundation.
//!
//! Tracks typing combo count and accumulated dopamine gauge points on a per-tab basis,
//! completely independent of UI rendering.
//!
//! # Specification Rules & Philosophy
//! - Exhilaration of typing over game score fairness ("スコアの公平性より、操作の爽快感").
//! - Non-empty token detection: ASCII half-width space (`' '`, `0x20`) is used as the sole delimiter.
//! - Consecutive spaces do not add combo points.
//! - Enter, Backspace, IME Commit, Paste are combo clock activities refreshing remaining time to 1,000ms.
//! - Non-character keys (Arrow keys, Home/End, Function keys) and IME Preedit do NOT update the clock.
//! - Ctrl/Alt shortcuts do not count as character typing.
//! - Combo clock pauses on CommandStarted (OSC 133 C), preserving remaining time.
//! - Combo clock stays paused during execution (C ~ D), including long commands and TUI.
//! - On verified CommandFinished (OSC 133 D):
//!   - Success doubles ongoing combo (saturating at 999).
//!   - Failure / Unknown maintains combo without doubling.
//!   - Clock resumes with preserved remaining time (or 1,000ms if typed during execution).
//! - On D missing or mismatched correlation, clock stays stopped until PromptStarted (OSC 133 A)
//!   safely recovers by discarding old combo (no gauge transfer) and re-initializing.
//! - Timeout at 1,000ms cumulative inactivity transfers 1 COMBO = 1 point to gauge (max 100).
//! - Simultaneous event and timeout prioritizes timeout.
//! - Events buffer is bounded (max 32 capacity) to prevent unbounded memory growth.
//! - Command strings themselves are never stored or logged.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Maximum saturation value for the combo counter.
pub const MAX_COMBO: u32 = 999;

/// Maximum saturation value for the dopamine gauge.
pub const MAX_GAUGE: u32 = 100;

/// Inactivity duration before an active combo expires and transfers to the gauge.
pub const COMBO_TIMEOUT: Duration = Duration::from_millis(1000);

/// Maximum event queue capacity to guarantee bounded memory.
pub const MAX_EVENT_CAPACITY: usize = 32;

/// Current token parsing phase for space-delimited non-empty token detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPhase {
    /// Awaiting the first non-space character of the next token.
    Whitespace,
    /// Currently accumulating non-space characters of a token.
    InToken,
}

/// The running or paused state of the 1,000ms combo inactivity countdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockState {
    /// No active countdown (e.g. combo is 0).
    Idle,
    /// Clock is actively counting down.
    Running {
        running_since: Instant,
        remaining_at_start: Duration,
    },
    /// Clock is paused (e.g. during command execution C ~ D, or waiting for recovery).
    Paused { remaining: Duration },
}

/// Events emitted by the combo state machine for future visual/audio integration
/// (Rainbow Tabs, Transfer particles, Burst effect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComboEvent {
    /// A new non-empty token started and combo was incremented.
    ComboIncremented { new_combo: u32 },
    /// Ongoing combo was doubled due to verified command completion success.
    CommandSuccessDoubled { old_combo: u32, new_combo: u32 },
    /// Combo timed out and points were transferred to the gauge.
    TransferredToGauge { points: u32, gauge_total: u32 },
    /// Gauge reached maximum saturation (100).
    GaugeMaxReached,
}

/// Tab-isolated combo and gauge state machine.
#[derive(Debug, Clone)]
pub struct TabComboState {
    combo: u32,
    gauge: u32,
    token_phase: TokenPhase,
    clock_state: ClockState,
    is_executing: bool,
    needs_recovery: bool,
    last_completed_command_id: Option<u64>,
    transfer_paused: bool,
    pending_transfer_points: u32,
    events: VecDeque<ComboEvent>,
}

impl Default for TabComboState {
    fn default() -> Self {
        Self::new()
    }
}

impl TabComboState {
    /// Creates a new, blank combo and gauge state.
    pub fn new() -> Self {
        Self {
            combo: 0,
            gauge: 0,
            token_phase: TokenPhase::Whitespace,
            clock_state: ClockState::Idle,
            is_executing: false,
            needs_recovery: false,
            last_completed_command_id: None,
            transfer_paused: false,
            pending_transfer_points: 0,
            events: VecDeque::with_capacity(MAX_EVENT_CAPACITY),
        }
    }

    /// Current unexpired combo count. Returns 0 if expired.
    pub fn combo(&self, now: Instant) -> u32 {
        if let ClockState::Running {
            running_since,
            remaining_at_start,
        } = self.clock_state
            && now.saturating_duration_since(running_since) >= remaining_at_start
        {
            return 0;
        }
        self.combo
    }

    /// Current accumulated dopamine gauge points (0..=100).
    #[allow(dead_code)]
    pub fn gauge(&self) -> u32 {
        self.gauge
    }

    /// Effective gauge points taking into account any pending expired combo that hasn't
    /// been ticked yet.
    #[allow(dead_code)]
    pub fn effective_gauge(&self, now: Instant) -> u32 {
        if let ClockState::Running {
            running_since,
            remaining_at_start,
        } = self.clock_state
            && now.saturating_duration_since(running_since) >= remaining_at_start
            && self.combo > 0
            && !self.transfer_paused
        {
            return (self.gauge + self.combo).min(MAX_GAUGE);
        }
        self.gauge
    }

    /// Returns current clock state.
    #[allow(dead_code)]
    pub fn clock_state(&self) -> ClockState {
        self.clock_state
    }

    /// Returns the remaining duration before combo timeout.
    pub fn remaining_time(&self, now: Instant) -> Duration {
        match self.clock_state {
            ClockState::Idle => Duration::ZERO,
            ClockState::Paused { remaining } => remaining,
            ClockState::Running {
                running_since,
                remaining_at_start,
            } => {
                let elapsed = now.saturating_duration_since(running_since);
                remaining_at_start.saturating_sub(elapsed)
            }
        }
    }

    /// Whether an active combo is running or held paused.
    #[allow(dead_code)]
    pub fn is_active(&self, now: Instant) -> bool {
        self.combo(now) > 0
    }

    /// Whether the combo clock is in the `Running` state and requires animation frame redraws
    /// until timeout processing is applied by `tick`.
    ///
    /// When paused during command execution (C ~ D), or when idle after timeout, this returns false.
    #[allow(dead_code)]
    pub fn is_clock_ticking(&self, _now: Instant) -> bool {
        matches!(self.clock_state, ClockState::Running { .. }) && self.combo > 0
    }

    /// Whether a command is currently executing between C and D.
    #[allow(dead_code)]
    pub fn is_executing(&self) -> bool {
        self.is_executing
    }

    /// Whether the state machine is pending recovery on next PromptStarted (A).
    #[allow(dead_code)]
    pub fn needs_recovery(&self) -> bool {
        self.needs_recovery
    }

    /// Whether the gauge has reached maximum capacity (100 points).
    pub fn is_gauge_max(&self) -> bool {
        self.gauge >= MAX_GAUGE
    }

    /// Checks and applies the 1,000ms idle timeout if the clock is running and has reached or
    /// exceeded its deadline.
    pub fn check_timeout(&mut self, now: Instant) {
        if let ClockState::Running {
            running_since,
            remaining_at_start,
        } = self.clock_state
        {
            let elapsed = now.saturating_duration_since(running_since);
            if elapsed >= remaining_at_start {
                self.apply_timeout();
            }
        }
    }

    fn apply_timeout(&mut self) {
        if self.combo > 0 {
            let points = self.combo;
            self.combo = 0;
            self.token_phase = TokenPhase::Whitespace;

            if self.transfer_paused {
                self.pending_transfer_points = self.pending_transfer_points.saturating_add(points);
            } else {
                let old_gauge = self.gauge;
                self.gauge = (self.gauge + points).min(MAX_GAUGE);
                self.record_event(ComboEvent::TransferredToGauge {
                    points,
                    gauge_total: self.gauge,
                });
                if old_gauge < MAX_GAUGE && self.gauge == MAX_GAUGE {
                    self.record_event(ComboEvent::GaugeMaxReached);
                }
            }
        }
        self.clock_state = ClockState::Idle;
    }

    /// Helper called on user input activity (typing, enter, backspace, paste, IME commit).
    ///
    /// Refreshes remaining time to 1,000ms. If paused during execution, keeps paused with 1,000ms.
    fn on_clock_activity(&mut self, now: Instant) {
        if self.combo > 0 {
            if self.is_executing {
                self.clock_state = ClockState::Paused {
                    remaining: COMBO_TIMEOUT,
                };
            } else {
                self.clock_state = ClockState::Running {
                    running_since: now,
                    remaining_at_start: COMBO_TIMEOUT,
                };
            }
        } else {
            self.clock_state = ClockState::Idle;
        }
    }

    /// Handles a single incoming character from user typing.
    ///
    /// - ASCII half-width space (`' '`, 0x20) acts as the sole token delimiter.
    /// - Consecutive spaces do not increment combo.
    /// - Enter / newline maintains the current combo.
    /// - Non-control, non-space characters start a new token if in `Whitespace` phase.
    /// - Command text is not stored or logged.
    pub fn on_char(&mut self, c: char, now: Instant) {
        self.check_timeout(now);

        if c == ' ' {
            self.token_phase = TokenPhase::Whitespace;
            self.on_clock_activity(now);
        } else if c == '\r' || c == '\n' {
            self.on_enter(now);
        } else if !c.is_control() {
            if self.token_phase == TokenPhase::Whitespace {
                self.combo = (self.combo + 1).min(MAX_COMBO);
                self.token_phase = TokenPhase::InToken;
                self.record_event(ComboEvent::ComboIncremented {
                    new_combo: self.combo,
                });
            }
            self.on_clock_activity(now);
        }
    }

    /// Handles a block of text (e.g. from clipboard paste or IME commit).
    ///
    /// Tokenizes solely on ASCII half-width space and preserves combo rules.
    pub fn on_text(&mut self, text: &str, now: Instant) {
        for c in text.chars() {
            self.on_char(c, now);
        }
    }

    /// Handles Enter keypress.
    ///
    /// Maintains the current combo count, resets token phase to `Whitespace` (ready for
    /// next command's first token), and refreshes remaining time to 1,000ms.
    pub fn on_enter(&mut self, now: Instant) {
        self.check_timeout(now);
        self.token_phase = TokenPhase::Whitespace;
        self.on_clock_activity(now);
    }

    /// Handles Backspace keypress.
    ///
    /// Maintains current combo, refreshes remaining time to 1,000ms.
    pub fn on_backspace(&mut self, now: Instant) {
        self.check_timeout(now);
        self.on_clock_activity(now);
    }

    /// Handles CommandStarted (OSC 133 C).
    ///
    /// Pauses the combo clock, capturing remaining countdown time. If combo is 0,
    /// no clock state is retained.
    pub fn on_command_started(&mut self, _command_id: u64, now: Instant) {
        self.check_timeout(now);
        self.is_executing = true;
        self.needs_recovery = false;

        if self.combo > 0 {
            match self.clock_state {
                ClockState::Running {
                    running_since,
                    remaining_at_start,
                } => {
                    let elapsed = now.saturating_duration_since(running_since);
                    let remaining = remaining_at_start.saturating_sub(elapsed);
                    if remaining.is_zero() {
                        self.apply_timeout();
                    } else {
                        self.clock_state = ClockState::Paused { remaining };
                    }
                }
                ClockState::Paused { .. } => {
                    // Already paused (duplicate C or nested)
                }
                ClockState::Idle => {
                    self.clock_state = ClockState::Paused {
                        remaining: COMBO_TIMEOUT,
                    };
                }
            }
        } else {
            self.clock_state = ClockState::Idle;
        }
    }

    /// Handles CommandFinished (OSC 133 D).
    ///
    /// - If C/D correlated properly (`valid_correlation == true`):
    ///   - If `success == true`, ongoing combo is doubled (max 999).
    ///   - Resumes clock using held remaining time (or 1,000ms if input occurred during execution).
    /// - If C/D missing, mismatched, or anomalous:
    ///   - Clock stays stopped until safe recovery via PromptStarted (A).
    pub fn on_command_finished(
        &mut self,
        command_id: u64,
        success: bool,
        valid_correlation: bool,
        now: Instant,
    ) {
        self.check_timeout(now);

        // F-6: Duplicate CommandFinished defense
        if Some(command_id) == self.last_completed_command_id && valid_correlation {
            return;
        }
        self.last_completed_command_id = Some(command_id);
        self.is_executing = false;

        if valid_correlation {
            self.needs_recovery = false;

            if success && self.combo > 0 {
                let old_combo = self.combo;
                self.combo = (self.combo * 2).min(MAX_COMBO);
                self.record_event(ComboEvent::CommandSuccessDoubled {
                    old_combo,
                    new_combo: self.combo,
                });
            }

            if self.combo > 0 {
                let remaining = match self.clock_state {
                    ClockState::Paused { remaining } => remaining,
                    ClockState::Running {
                        running_since,
                        remaining_at_start,
                    } => {
                        let elapsed = now.saturating_duration_since(running_since);
                        remaining_at_start.saturating_sub(elapsed)
                    }
                    ClockState::Idle => COMBO_TIMEOUT,
                };

                if remaining.is_zero() {
                    self.apply_timeout();
                } else {
                    self.clock_state = ClockState::Running {
                        running_since: now,
                        remaining_at_start: remaining,
                    };
                }
            } else {
                self.clock_state = ClockState::Idle;
            }
        } else {
            // Anomaly: keep stopped until recovery via PromptStarted (A)
            self.needs_recovery = true;
        }
    }

    /// Handles PromptStarted (OSC 133 A).
    ///
    /// If recovering from an anomaly (MissingD, Mismatched correlation), discards old combo
    /// without gauge transfer and restores clean state. On normal C -> D -> A, combo is preserved.
    pub fn on_prompt_started(&mut self, now: Instant) {
        self.check_timeout(now);

        if self.needs_recovery || self.is_executing {
            self.combo = 0;
            self.token_phase = TokenPhase::Whitespace;
            self.clock_state = ClockState::Idle;
            self.is_executing = false;
            self.needs_recovery = false;
        }
    }

    /// Advances simulation time, applying any pending timeout and point transfers.
    pub fn tick(&mut self, now: Instant) {
        self.check_timeout(now);
    }

    /// Records an event with bounded capacity (max 32), dropping oldest when full.
    fn record_event(&mut self, event: ComboEvent) {
        if self.events.len() >= MAX_EVENT_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Pauses or unpauses immediate transfer of points to the gauge upon timeout.
    ///
    /// When paused, points from expired combos accumulate in `pending_transfer_points`
    /// to allow transfer particle animations before updating the gauge.
    #[allow(dead_code)]
    pub fn set_transfer_paused(&mut self, paused: bool) {
        self.transfer_paused = paused;
    }

    /// Whether point transfer to the gauge is currently paused.
    #[allow(dead_code)]
    pub fn is_transfer_paused(&self) -> bool {
        self.transfer_paused
    }

    /// Number of points currently pending transfer during animation.
    #[allow(dead_code)]
    pub fn pending_transfer_points(&self) -> u32 {
        self.pending_transfer_points
    }

    /// Applies points from `pending_transfer_points` into `gauge`.
    #[allow(dead_code)]
    pub fn apply_pending_transfer(&mut self, points: u32) {
        let to_apply = points.min(self.pending_transfer_points);
        self.pending_transfer_points -= to_apply;
        let old_gauge = self.gauge;
        self.gauge = (self.gauge + to_apply).min(MAX_GAUGE);
        self.record_event(ComboEvent::TransferredToGauge {
            points: to_apply,
            gauge_total: self.gauge,
        });
        if old_gauge < MAX_GAUGE && self.gauge == MAX_GAUGE {
            self.record_event(ComboEvent::GaugeMaxReached);
        }
    }

    /// Consumes the maximum gauge for a Burst effect, resetting the gauge to 0.
    /// Returns true if a burst was triggered (gauge was >= 100).
    pub fn consume_gauge_burst(&mut self) -> bool {
        if self.gauge >= MAX_GAUGE {
            self.gauge = 0;
            true
        } else {
            false
        }
    }

    /// Drains all accumulated combo events.
    pub fn drain_events(&mut self) -> Vec<ComboEvent> {
        self.events.drain(..).collect()
    }

    /// Resets all state for this tab.
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.combo = 0;
        self.gauge = 0;
        self.token_phase = TokenPhase::Whitespace;
        self.clock_state = ClockState::Idle;
        self.is_executing = false;
        self.needs_recovery = false;
        self.last_completed_command_id = None;
        self.transfer_paused = false;
        self.pending_transfer_points = 0;
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_zero() {
        let state = TabComboState::new();
        let now = Instant::now();
        assert_eq!(state.combo(now), 0);
        assert_eq!(state.gauge(), 0);
        assert!(!state.is_active(now));
        assert!(!state.is_gauge_max());
        assert_eq!(state.pending_transfer_points(), 0);
        assert_eq!(state.clock_state(), ClockState::Idle);
        assert!(!state.is_clock_ticking(now));
    }

    #[test]
    fn single_token_increments_combo() {
        let mut state = TabComboState::new();
        let now = Instant::now();

        state.on_char('c', now);
        assert_eq!(state.combo(now), 1);
        assert!(state.is_active(now));
        assert!(state.is_clock_ticking(now));

        state.on_char('a', now);
        state.on_char('r', now);
        state.on_char('g', now);
        state.on_char('o', now);
        assert_eq!(state.combo(now), 1);

        let events = state.drain_events();
        assert_eq!(events, vec![ComboEvent::ComboIncremented { new_combo: 1 }]);
    }

    #[test]
    fn space_delimited_tokens_increment_combo() {
        let mut state = TabComboState::new();
        let now = Instant::now();

        for c in "cargo".chars() {
            state.on_char(c, now);
        }
        assert_eq!(state.combo(now), 1);

        state.on_char(' ', now);
        assert_eq!(state.combo(now), 1);

        for c in "test".chars() {
            state.on_char(c, now);
        }
        assert_eq!(state.combo(now), 2);

        let events = state.drain_events();
        assert_eq!(
            events,
            vec![
                ComboEvent::ComboIncremented { new_combo: 1 },
                ComboEvent::ComboIncremented { new_combo: 2 }
            ]
        );
    }

    #[test]
    fn consecutive_and_leading_trailing_spaces_do_not_add_combo() {
        let mut state = TabComboState::new();
        let now = Instant::now();

        state.on_text("   cargo   test   --release   ", now);
        assert_eq!(state.combo(now), 3);

        let events = state.drain_events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], ComboEvent::ComboIncremented { new_combo: 1 });
        assert_eq!(events[1], ComboEvent::ComboIncremented { new_combo: 2 });
        assert_eq!(events[2], ComboEvent::ComboIncremented { new_combo: 3 });
    }

    #[test]
    fn enter_maintains_combo_and_allows_next_command() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("git status", t0);
        assert_eq!(state.combo(t0), 2);

        // Press Enter
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        // Press Enter again on empty prompt
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        // Next command within 1,000ms
        let t1 = t0 + Duration::from_millis(500);
        state.on_text("cargo test", t1);
        assert_eq!(state.combo(t1), 4);
    }

    #[test]
    fn clock_boundaries_999ms_1000ms_1001ms() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        assert_eq!(state.combo(t0), 2);

        // 999ms: Not expired
        let t_999 = t0 + Duration::from_millis(999);
        assert_eq!(state.combo(t_999), 2);
        assert_eq!(state.gauge(), 0);
        assert_eq!(state.remaining_time(t_999), Duration::from_millis(1));
        assert!(state.is_clock_ticking(t_999));

        // 1000ms: Timed out exactly at boundary
        let t_1000 = t0 + Duration::from_millis(1000);
        // N-1: Before tick is applied, is_clock_ticking remains true so frames continue
        assert!(state.is_clock_ticking(t_1000));
        state.tick(t_1000);
        assert_eq!(state.combo(t_1000), 0);
        assert_eq!(state.gauge(), 2);
        assert_eq!(state.remaining_time(t_1000), Duration::ZERO);
        assert!(!state.is_clock_ticking(t_1000));

        // 1001ms: Remains timed out
        let t_1001 = t0 + Duration::from_millis(1001);
        state.tick(t_1001);
        assert_eq!(state.combo(t_1001), 0);
        assert_eq!(state.gauge(), 2);
    }

    #[test]
    fn clock_subscription_maintained_past_deadline_until_tick_resolves_timeout() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        assert_eq!(state.combo(t0), 2);
        assert_eq!(state.gauge(), 0);

        // 1. Running and before deadline (500ms): subscription needed
        let t_500 = t0 + Duration::from_millis(500);
        assert!(state.is_clock_ticking(t_500));

        // 2. Running and exactly at boundary (1,000ms) before tick: subscription needed
        let t_1000 = t0 + Duration::from_millis(1000);
        assert_eq!(state.combo(t_1000), 0);
        assert_eq!(state.effective_gauge(t_1000), 2);
        assert_eq!(state.gauge(), 0);
        assert!(state.is_clock_ticking(t_1000));

        // 3. Running and past deadline (1,500ms) before tick: subscription still needed
        let t_1500 = t0 + Duration::from_millis(1500);
        assert!(state.is_clock_ticking(t_1500));
        assert_eq!(state.gauge(), 0);

        // 4. tick resolves timeout and transfers combo to gauge once
        state.tick(t_1500);
        assert_eq!(state.gauge(), 2);
        assert_eq!(state.combo(t_1500), 0);
        assert_eq!(state.effective_gauge(t_1500), 2);

        // 5. After tick, state is Idle: subscription is no longer needed
        assert!(!state.is_clock_ticking(t_1500));
        assert_eq!(state.clock_state(), ClockState::Idle);

        // 6. Further ticks do not double-transfer
        state.tick(t0 + Duration::from_millis(2000));
        assert_eq!(state.gauge(), 2);
        assert!(!state.is_clock_ticking(t0 + Duration::from_millis(2000)));
    }

    #[test]
    fn reset_clears_combo_and_gauge_without_transfer() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        assert_eq!(state.combo(t0), 2);

        state.reset();
        assert_eq!(state.combo(t0), 0);
        assert_eq!(state.gauge(), 0);
        assert_eq!(state.clock_state(), ClockState::Idle);
        assert!(!state.is_clock_ticking(t0));
        assert!(state.drain_events().is_empty());
    }

    #[test]
    fn timeout_and_event_at_exact_same_time_prioritizes_timeout() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        assert_eq!(state.combo(t0), 2);

        // Event arrives at exactly t0 + 1000ms
        let t_boundary = t0 + Duration::from_millis(1000);
        state.on_command_finished(1, true, true, t_boundary);

        // Timeout took priority: combo was transferred, not doubled!
        assert_eq!(state.combo(t_boundary), 0);
        assert_eq!(state.gauge(), 2);
    }

    #[test]
    fn command_started_c_pauses_clock_and_preserves_remaining_time() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        // C arrives at t = 200ms
        let t_c = t0 + Duration::from_millis(200);
        state.on_command_started(1, t_c);

        assert_eq!(state.combo(t_c), 2);
        assert_eq!(state.remaining_time(t_c), Duration::from_millis(800));
        assert_eq!(
            state.clock_state(),
            ClockState::Paused {
                remaining: Duration::from_millis(800)
            }
        );
        // Clock is paused, so it is NOT ticking frames
        assert!(!state.is_clock_ticking(t_c));

        // Long execution: 30 seconds pass!
        let t_long = t_c + Duration::from_secs(30);
        state.tick(t_long);

        // Did NOT time out!
        assert_eq!(state.combo(t_long), 2);
        assert_eq!(state.gauge(), 0);
        assert_eq!(state.remaining_time(t_long), Duration::from_millis(800));

        // D arrives with Success at 30 seconds
        state.on_command_finished(1, true, true, t_long);
        // Combo doubled to 4
        assert_eq!(state.combo(t_long), 4);
        // Resumes with remaining 800ms (NOT reset to 1000ms)
        assert_eq!(state.remaining_time(t_long), Duration::from_millis(800));
        assert!(state.is_clock_ticking(t_long));

        // 799ms after D: still alive
        let t_almost = t_long + Duration::from_millis(799);
        assert_eq!(state.combo(t_almost), 4);
        assert_eq!(state.gauge(), 0);

        // 800ms after D: expires!
        let t_timeout = t_long + Duration::from_millis(800);
        state.tick(t_timeout);
        assert_eq!(state.combo(t_timeout), 0);
        assert_eq!(state.gauge(), 4);
    }

    #[test]
    fn c_when_combo_is_zero_does_not_create_clock_state() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_command_started(1, t0);
        assert_eq!(state.clock_state(), ClockState::Idle);
        assert_eq!(state.combo(t0), 0);
    }

    #[test]
    fn tui_input_during_execution_updates_combo_and_paused_remaining() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("fzf", t0);
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 1);

        // C at 200ms -> remaining = 800ms
        let t_c = t0 + Duration::from_millis(200);
        state.on_command_started(1, t_c);
        assert_eq!(state.remaining_time(t_c), Duration::from_millis(800));

        // User types inside fzf at 10 seconds
        let t_type = t_c + Duration::from_secs(10);
        state.on_text("query item", t_type);
        assert_eq!(state.combo(t_type), 3);
        // Paused remaining is refreshed to 1,000ms
        assert_eq!(state.remaining_time(t_type), Duration::from_millis(1000));
        assert_eq!(
            state.clock_state(),
            ClockState::Paused {
                remaining: Duration::from_millis(1000)
            }
        );

        // fzf finishes at 20 seconds
        let t_d = t_type + Duration::from_secs(10);
        state.on_command_finished(1, true, true, t_d);
        // Doubled: 3 * 2 = 6
        assert_eq!(state.combo(t_d), 6);
        // Resumes with 1,000ms (because of input during execution)
        assert_eq!(state.remaining_time(t_d), Duration::from_millis(1000));
    }

    #[test]
    fn c_to_d_failure_or_unknown_resumes_without_doubling() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        // C at 300ms -> remaining = 700ms
        let t_c = t0 + Duration::from_millis(300);
        state.on_command_started(1, t_c);

        // D Failure at 1 second
        let t_d = t_c + Duration::from_millis(700);
        state.on_command_finished(1, false, true, t_d);

        // Combo maintained at 2 (not doubled)
        assert_eq!(state.combo(t_d), 2);
        // Resumes with remaining 700ms
        assert_eq!(state.remaining_time(t_d), Duration::from_millis(700));

        // Times out 700ms after D
        let t_timeout = t_d + Duration::from_millis(700);
        state.tick(t_timeout);
        assert_eq!(state.combo(t_timeout), 0);
        assert_eq!(state.gauge(), 2);
    }

    #[test]
    fn normal_c_to_d_to_a_preserves_combo() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        state.on_enter(t0);
        let t_c = t0 + Duration::from_millis(100);
        state.on_command_started(1, t_c);

        let t_d = t_c + Duration::from_millis(200);
        state.on_command_finished(1, true, true, t_d);
        assert_eq!(state.combo(t_d), 4);

        // Normal PromptStarted (A) after D
        let t_a = t_d + Duration::from_millis(50);
        state.on_prompt_started(t_a);

        // Combo is NOT reset!
        assert_eq!(state.combo(t_a), 4);
    }

    #[test]
    fn missing_d_recovers_on_prompt_started_a_by_discarding_combo() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        let t_c = t0 + Duration::from_millis(100);
        state.on_command_started(1, t_c);

        // D was lost completely! Shell interrupts and sends PromptStarted (A) directly
        let t_a = t_c + Duration::from_secs(2);
        state.on_prompt_started(t_a);

        // Combo is discarded to 0, no gauge transfer
        assert_eq!(state.combo(t_a), 0);
        assert_eq!(state.gauge(), 0);
        assert_eq!(state.clock_state(), ClockState::Idle);
    }

    #[test]
    fn mismatched_c_d_recovers_on_prompt_started_a_by_discarding_combo() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        let t_c = t0 + Duration::from_millis(100);
        state.on_command_started(1, t_c);

        // D finishes with mismatched correlation
        let t_d = t_c + Duration::from_millis(200);
        state.on_command_finished(1, true, false, t_d);
        assert!(state.needs_recovery());

        // PromptStarted (A) arrives
        let t_a = t_d + Duration::from_millis(50);
        state.on_prompt_started(t_a);

        // Combo discarded to 0, no gauge transfer
        assert_eq!(state.combo(t_a), 0);
        assert_eq!(state.gauge(), 0);
        assert_eq!(state.clock_state(), ClockState::Idle);
        assert!(!state.needs_recovery());
    }

    #[test]
    fn duplicate_command_finished_defense() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo test", t0);
        state.on_enter(t0);
        assert_eq!(state.combo(t0), 2);

        let t_c = t0 + Duration::from_millis(100);
        state.on_command_started(1, t_c);

        let t_d = t_c + Duration::from_millis(200);
        state.on_command_finished(1, true, true, t_d);
        assert_eq!(state.combo(t_d), 4);

        // Duplicate D with same command_id (1)
        state.on_command_finished(1, true, true, t_d);
        // Combo is NOT doubled again!
        assert_eq!(state.combo(t_d), 4);
    }

    #[test]
    fn events_queue_is_bounded_at_32() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        // Generate 50 token increments
        for _ in 0..50 {
            state.on_char('a', t0);
            state.on_char(' ', t0);
        }

        // Must be capped at MAX_EVENT_CAPACITY (32)
        assert_eq!(state.events.len(), MAX_EVENT_CAPACITY);
        let drained = state.drain_events();
        assert_eq!(drained.len(), MAX_EVENT_CAPACITY);
        assert!(state.events.is_empty());
    }

    #[test]
    fn combo_saturates_at_999() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.combo = 600;
        state.clock_state = ClockState::Running {
            running_since: t0,
            remaining_at_start: COMBO_TIMEOUT,
        };

        state.on_command_started(1, t0);
        state.on_command_finished(1, true, true, t0);
        assert_eq!(state.combo(t0), MAX_COMBO);
    }

    #[test]
    fn gauge_saturates_at_100_and_emits_max_reached_event() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.gauge = 95;
        state.on_text("one two three four five six seven eight nine ten", t0);
        assert_eq!(state.combo(t0), 10);

        let t_timeout = t0 + Duration::from_millis(1000);
        state.tick(t_timeout);

        assert_eq!(state.combo(t_timeout), 0);
        assert_eq!(state.gauge(), MAX_GAUGE);
        assert!(state.is_gauge_max());

        let events = state.drain_events();
        assert!(events.contains(&ComboEvent::GaugeMaxReached));
    }

    #[test]
    fn burst_consumption_resets_gauge() {
        let mut state = TabComboState::new();
        state.gauge = 100;
        assert!(state.is_gauge_max());

        assert!(state.consume_gauge_burst());
        assert_eq!(state.gauge(), 0);
        assert!(!state.is_gauge_max());
        assert!(!state.consume_gauge_burst());
    }

    #[test]
    fn backspace_maintains_combo_and_refreshes_timer() {
        let mut state = TabComboState::new();
        let t0 = Instant::now();

        state.on_text("cargo", t0);
        assert_eq!(state.combo(t0), 1);

        let t1 = t0 + Duration::from_millis(500);
        state.on_backspace(t1);
        assert_eq!(state.combo(t1), 1);
        // Refreshed to 1,000ms from t1
        assert_eq!(state.remaining_time(t1), Duration::from_millis(1000));

        let t2 = t1 + Duration::from_millis(700);
        assert_eq!(state.combo(t2), 1);

        let t_timeout = t1 + Duration::from_millis(1000);
        state.tick(t_timeout);
        assert_eq!(state.combo(t_timeout), 0);
        assert_eq!(state.gauge(), 1);
    }
}
