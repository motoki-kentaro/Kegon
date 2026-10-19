//! Dopamine Gauge and Combo Transfer particle effects.
//!
//! Provides the thin horizontal gauge at the bottom of the terminal view, smooth display
//! value interpolation, and the luminous transfer particles that fly from the HIT badge
//! and converge at the gauge fill tip.
//!
//! "コンボを光に変えろ。" 🌈

use std::time::{Duration, Instant};

use iced::widget::canvas::{Frame, Path};
use iced::{Color, Point, Size, Vector};

/// Height of the horizontal Dopamine Gauge bar at the bottom of the terminal in logical pixels.
pub const GAUGE_HEIGHT: f32 = 4.0;

/// Maximum value for the gauge.
pub const MAX_GAUGE: f32 = 100.0;

/// Default duration for gauge fill interpolation.
pub const GAUGE_INTERPOLATION_DURATION: Duration = Duration::from_millis(450);

/// Flight duration for transfer particles.
#[allow(dead_code)]
pub const TRANSFER_FLIGHT_DURATION: Duration = Duration::from_millis(420);

/// Lifetime of sparks bursting on arrival at the gauge tip.
pub const SPARK_LIFETIME: Duration = Duration::from_millis(160);

/// Vertical offset (inside the overlay canvas) at which transfer particles are launched.
///
/// The HIT badge sits in the tab strip above the canvas; geometry at `y < 0` is clipped by the
/// canvas frame, so launch points are kept on the canvas' top edge instead.
pub const TRANSFER_ORIGIN_Y: f32 = 4.0;

/// Maximum particles launched in a single transfer to keep performance bounded.
pub const MAX_TRANSFER_PARTICLES: usize = 24;

/// Minimum particles launched for any non-zero transfer.
pub const MIN_TRANSFER_PARTICLES: usize = 4;

/// Computes the particle count for a given transferred combo point count.
/// Scales non-linearly with `sqrt(points)` and clamps between 4 and 24.
pub fn calculate_particle_count(points: u32) -> usize {
    if points == 0 {
        return 0;
    }
    ((points as f32).sqrt() * 3.0)
        .clamp(MIN_TRANSFER_PARTICLES as f32, MAX_TRANSFER_PARTICLES as f32) as usize
}

/// Returns a transfer launch point guaranteed to lie inside a `viewport_width` x
/// `viewport_height` overlay canvas. `badge_x` is the HIT badge centre in canvas coordinates.
pub fn transfer_origin(badge_x: f32, viewport_width: f32, viewport_height: f32) -> Point {
    let w = if viewport_width.is_finite() {
        viewport_width.max(0.0)
    } else {
        0.0
    };
    let h = if viewport_height.is_finite() {
        viewport_height.max(0.0)
    } else {
        0.0
    };
    let x = if badge_x.is_finite() {
        badge_x
    } else {
        w / 2.0
    };
    let max_x = (w - 8.0).max(0.0);
    let min_x = 8.0_f32.min(max_x);
    Point::new(x.clamp(min_x, max_x), TRANSFER_ORIGIN_Y.min(h))
}

/// Evaluates a cubic Bézier curve at parameter `t` in `[0.0, 1.0]`.
#[inline]
pub fn eval_cubic_bezier(p0: Point, p1: Point, p2: Point, p3: Point, t: f32) -> Point {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    let tt = t * t;
    let uu = u * u;
    let uuu = uu * u;
    let ttt = tt * t;

    let x = uuu * p0.x + 3.0 * uu * t * p1.x + 3.0 * u * tt * p2.x + ttt * p3.x;
    let y = uuu * p0.y + 3.0 * uu * t * p1.y + 3.0 * u * tt * p2.y + ttt * p3.y;

    Point::new(x, y)
}

/// Helper returning a color with modified alpha.
#[inline]
fn faded(mut color: Color, factor: f32) -> Color {
    color.a = (color.a * factor).clamp(0.0, 1.0);
    color
}

/// Cubic ease-out curve mapping `t` in `[0.0, 1.0]` to `[0.0, 1.0]`.
#[inline]
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Visual display state of a tab's Dopamine Gauge.
///
/// Separates the visual interpolated display value from the core ground truth gauge.
#[derive(Debug, Clone)]
pub struct TabGaugeDisplay {
    /// Currently displayed gauge fill in `0.0..=100.0`.
    displayed_value: f32,
    /// Numerical target in `0.0..=100.0`.
    target_value: f32,
    /// Value at the start of the current interpolation.
    anim_start_value: f32,
    /// Timestamp when current interpolation began.
    anim_start_time: Option<Instant>,
    /// Duration of the interpolation.
    anim_duration: Duration,
}

impl Default for TabGaugeDisplay {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TabGaugeDisplay {
    /// Creates a new gauge display initialized to `initial` (0 to 100).
    pub fn new(initial: u32) -> Self {
        let val = (initial as f32).clamp(0.0, MAX_GAUGE);
        Self {
            displayed_value: val,
            target_value: val,
            anim_start_value: val,
            anim_start_time: None,
            anim_duration: GAUGE_INTERPOLATION_DURATION,
        }
    }

    /// Sets a new target value and begins smooth interpolation from the current display value.
    pub fn set_target(&mut self, target: u32, now: Instant) {
        let new_target = (target as f32).clamp(0.0, MAX_GAUGE);
        if (new_target - self.target_value).abs() < f32::EPSILON && self.anim_start_time.is_none() {
            return;
        }

        self.anim_start_value = self.displayed_value;
        self.target_value = new_target;
        self.anim_start_time = Some(now);
    }

    /// Advances interpolation toward `target_value`.
    pub fn update(&mut self, now: Instant) {
        if let Some(start) = self.anim_start_time {
            let elapsed = now.saturating_duration_since(start).as_secs_f32();
            let total = self.anim_duration.as_secs_f32().max(0.001);
            let progress = (elapsed / total).clamp(0.0, 1.0);
            let ease = ease_out_cubic(progress);

            self.displayed_value = (self.anim_start_value
                + (self.target_value - self.anim_start_value) * ease)
                .clamp(0.0, MAX_GAUGE);

            if progress >= 1.0 {
                self.displayed_value = self.target_value;
                self.anim_start_time = None;
            }
        }
    }

    /// Re-synchronises the display with the Combo Core ground truth `canonical` (0..=100).
    ///
    /// A running interpolation is never interrupted; once it has finished, any remaining
    /// difference (for example caused by a lost transfer event) is snapped away.
    /// Returns `true` if the display was corrected.
    pub fn resync(&mut self, canonical: u32) -> bool {
        if self.anim_start_time.is_some() {
            return false;
        }
        let truth = (canonical as f32).clamp(0.0, MAX_GAUGE);
        if (self.displayed_value - truth).abs() < 0.001 && (self.target_value - truth).abs() < 0.001
        {
            return false;
        }
        self.snap_to(canonical);
        true
    }

    /// Instantly snaps the displayed value and target to `target`.
    pub fn snap_to(&mut self, target: u32) {
        let val = (target as f32).clamp(0.0, MAX_GAUGE);
        self.displayed_value = val;
        self.target_value = val;
        self.anim_start_value = val;
        self.anim_start_time = None;
    }

    /// Current visual displayed value in `0.0..=100.0`.
    pub fn displayed_value(&self) -> f32 {
        self.displayed_value
    }

    /// Numerical target value in `0.0..=100.0`.
    #[allow(dead_code)]
    pub fn target_value(&self) -> f32 {
        self.target_value
    }

    /// Whether this gauge is currently undergoing an interpolation animation.
    pub fn is_animating(&self, _now: Instant) -> bool {
        self.anim_start_time.is_some()
    }

    /// Whether the gauge bar should be drawn.
    pub fn is_visible(&self) -> bool {
        true
    }

    /// Returns the gauge fill color for a given normalized ratio `[0.0, 1.0]`.
    pub fn fill_color(ratio: f32) -> Color {
        let r = ratio.clamp(0.0, 1.0);
        if r < 0.5 {
            // Cyan (#00F0FF) to Gold (#FFD700)
            let t = r / 0.5;
            Color::from_rgb(t, 0.94 - 0.10 * t, 1.0 - 0.85 * t)
        } else {
            // Gold (#FFD700) to Hot Neon Pink (#FF007F)
            let t = (r - 0.5) / 0.5;
            Color::from_rgb(1.0, 0.84 * (1.0 - t), 0.15 + 0.35 * t)
        }
    }

    /// Paints the Dopamine Gauge bar at the bottom edge of `bounds`.
    pub fn paint(&self, frame: &mut Frame, viewport_size: Size) {
        let w = viewport_size.width;
        let h = viewport_size.height;
        if w < 1.0 || h < GAUGE_HEIGHT {
            return;
        }

        let y = h - GAUGE_HEIGHT;

        // 1. Dark translucent track background
        frame.fill_rectangle(
            Point::new(0.0, y),
            Size::new(w, GAUGE_HEIGHT),
            Color::from_rgba(0.0, 0.0, 0.0, 0.55),
        );

        // 2. Subtle top border for track definition
        frame.fill_rectangle(
            Point::new(0.0, y),
            Size::new(w, 0.75),
            Color::from_rgba(1.0, 1.0, 1.0, 0.12),
        );

        // 3. Filled portion
        let ratio = (self.displayed_value / MAX_GAUGE).clamp(0.0, 1.0);
        let fill_w = w * ratio;

        if fill_w > 0.5 {
            let color = Self::fill_color(ratio);

            // Base fill
            frame.fill_rectangle(Point::new(0.0, y), Size::new(fill_w, GAUGE_HEIGHT), color);

            // Subtle glow above the gauge fill
            frame.fill_rectangle(
                Point::new(0.0, (y - 1.5).max(0.0)),
                Size::new(fill_w, 1.5),
                faded(color, 0.28),
            );

            // Bright white tip indicator at the right edge
            let tip_w = 2.5_f32.min(fill_w);
            frame.fill_rectangle(
                Point::new(fill_w - tip_w, y),
                Size::new(tip_w, GAUGE_HEIGHT),
                Color::WHITE,
            );
        }
    }
}

/// A single glowing transfer particle travelling along a cubic Bézier trajectory.
#[derive(Debug, Clone)]
pub struct TransferParticle {
    pub start: Point,
    pub control1: Point,
    pub control2: Point,
    pub end: Point,
    pub start_time: Instant,
    pub duration: Duration,
    pub color: Color,
    pub radius: f32,
    pub arrived: bool,
}

impl TransferParticle {
    /// Evaluates the particle position at the given timestamp.
    /// Returns `None` if the particle has not yet launched.
    pub fn current_pos(&self, now: Instant) -> Option<Point> {
        if now < self.start_time {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.start_time).as_secs_f32();
        let total = self.duration.as_secs_f32().max(0.001);
        let tau = (elapsed / total).clamp(0.0, 1.0);
        Some(eval_cubic_bezier(
            self.start,
            self.control1,
            self.control2,
            self.end,
            tau,
        ))
    }

    /// Evaluates the tail position for streak rendering.
    pub fn tail_pos(&self, now: Instant) -> Option<Point> {
        if now < self.start_time {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.start_time).as_secs_f32();
        let total = self.duration.as_secs_f32().max(0.001);
        let tau = (elapsed / total).clamp(0.0, 1.0);
        let tail_tau = (tau - 0.08).max(0.0);
        Some(eval_cubic_bezier(
            self.start,
            self.control1,
            self.control2,
            self.end,
            tail_tau,
        ))
    }

    /// Progress in `[0.0, 1.0]`. Returns 0.0 if not launched.
    pub fn progress(&self, now: Instant) -> f32 {
        if now < self.start_time {
            0.0
        } else {
            let elapsed = now.saturating_duration_since(self.start_time).as_secs_f32();
            let total = self.duration.as_secs_f32().max(0.001);
            (elapsed / total).clamp(0.0, 1.0)
        }
    }
}

/// An arrival spark bursting on particle impact at the gauge fill tip.
#[derive(Debug, Clone)]
pub struct ArrivalSpark {
    pub pos: Point,
    pub vel: Vector,
    pub spawn_time: Instant,
    pub lifetime: Duration,
    pub color: Color,
    pub radius: f32,
}

impl ArrivalSpark {
    /// Returns current spark position and alpha, or `None` if expired.
    pub fn current_state(&self, now: Instant) -> Option<(Point, f32)> {
        let elapsed = now.saturating_duration_since(self.spawn_time);
        if elapsed >= self.lifetime {
            return None;
        }
        let dt = elapsed.as_secs_f32();
        let alpha = (1.0 - dt / self.lifetime.as_secs_f32()).clamp(0.0, 1.0);

        // Physics: pos = pos0 + vel * dt + 0.5 * gravity * dt^2
        let gravity = 320.0;
        let p = Point::new(
            self.pos.x + self.vel.x * dt,
            self.pos.y + self.vel.y * dt + 0.5 * gravity * dt * dt,
        );

        Some((p, alpha))
    }
}

/// Manages transfer particle animations and arrival sparks for a tab.
#[derive(Debug, Clone, Default)]
pub struct TabTransferManager {
    particles: Vec<TransferParticle>,
    sparks: Vec<ArrivalSpark>,
}

impl TabTransferManager {
    /// Creates a new, idle transfer manager.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            sparks: Vec::new(),
        }
    }

    /// Spawns transfer particles flying from `origin` (HIT badge) to `target` (Gauge fill tip).
    pub fn spawn_transfer(&mut self, points: u32, origin: Point, target: Point, now: Instant) {
        if points == 0 {
            return;
        }

        let count = calculate_particle_count(points);
        let dx = target.x - origin.x;
        let dy = target.y - origin.y;

        for i in 0..count {
            // Stagger launch times so particles stream naturally
            let delay_secs = (i as f32 / count as f32) * 0.11;
            let start_time = now + Duration::from_secs_f32(delay_secs);
            let duration = Duration::from_millis(380 + (i as u64 * 13) % 70);

            // Alternate lateral arc outward from the straight line
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let arc_dist = sign * (25.0 + ((i * 17) % 35) as f32);

            let c1 = Point::new(origin.x + arc_dist + 0.12 * dx, origin.y + 0.32 * dy);
            let c2 = Point::new(target.x + 0.25 * arc_dist, origin.y + 0.72 * dy);

            // Slight arrival jitter near the gauge tip
            let jitter_x = ((i as f32 * 3.7).sin() * 5.0).clamp(-6.0, 6.0);
            let end_jittered = Point::new((target.x + jitter_x).max(2.0), target.y);

            // Hue variation across particles: electric cyan, golden radiant, vibrant magenta
            let hue = (0.5 + (i as f32 / count as f32) * 0.5).fract();
            let color = TabGaugeDisplay::fill_color(hue);

            self.particles.push(TransferParticle {
                start: origin,
                control1: c1,
                control2: c2,
                end: end_jittered,
                start_time,
                duration,
                color,
                radius: 2.5 + ((i % 3) as f32) * 0.4,
                arrived: false,
            });
        }
    }

    /// Advances particle flight and spark lifetimes, triggering arrival bursts.
    pub fn update(&mut self, now: Instant) {
        // 1. Process particle arrivals
        for p in &mut self.particles {
            let arrival_time = p.start_time + p.duration;
            if !p.arrived && now >= arrival_time {
                p.arrived = true;

                // Spawn 3 mini-sparks at arrival point if not already expired
                if now.saturating_duration_since(arrival_time) < SPARK_LIFETIME {
                    for k in 0..3 {
                        let angle = -std::f32::consts::FRAC_PI_2 + (k as f32 - 1.0) * 0.45;
                        let speed = 60.0 + ((k * 31) % 40) as f32;
                        let vel = Vector::new(angle.cos() * speed, angle.sin() * speed);

                        self.sparks.push(ArrivalSpark {
                            pos: p.end,
                            vel,
                            spawn_time: arrival_time,
                            lifetime: SPARK_LIFETIME,
                            color: Color::WHITE,
                            radius: 1.6,
                        });
                    }
                }
            }
        }

        // 2. Prune arrived particles and dead sparks
        self.particles.retain(|p| !p.arrived);
        self.sparks
            .retain(|s| now.saturating_duration_since(s.spawn_time) < s.lifetime);
    }

    /// Whether there are active particles flying or sparks bursting.
    pub fn is_active(&self) -> bool {
        !self.particles.is_empty() || !self.sparks.is_empty()
    }

    /// Clears all active particles and sparks.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.particles.clear();
        self.sparks.clear();
    }

    /// Number of active particles.
    #[allow(dead_code)]
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    /// Number of active sparks.
    #[allow(dead_code)]
    pub fn spark_count(&self) -> usize {
        self.sparks.len()
    }

    /// Returns a slice of active particles for inspection.
    #[allow(dead_code)]
    pub fn particles(&self) -> &[TransferParticle] {
        &self.particles
    }

    /// Paints the transfer particles and arrival sparks on `frame`.
    pub fn paint(&self, frame: &mut Frame, now: Instant) {
        // Paint sparks
        for spark in &self.sparks {
            if let Some((p, alpha)) = spark.current_state(now) {
                frame.fill(
                    &Path::circle(p, spark.radius * 1.5),
                    faded(spark.color, alpha * 0.4),
                );
                frame.fill(&Path::circle(p, spark.radius), faded(spark.color, alpha));
            }
        }

        // Paint flying particles
        for p in &self.particles {
            if let Some(pos) = p.current_pos(now) {
                let tau = p.progress(now);
                // Particle shrinks slightly as it enters the gauge
                let scale = if tau > 0.8 {
                    1.0 - (tau - 0.8) / 0.2 * 0.5
                } else {
                    1.0
                };
                let r = p.radius * scale;

                // Tail streak
                if let Some(tail) = p.tail_pos(now) {
                    let (dx, dy) = (pos.x - tail.x, pos.y - tail.y);
                    let len = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
                    let half_w = r * 0.75;
                    let (nx, ny) = (-dy / len * half_w, dx / len * half_w);

                    let streak_path = Path::new(|b| {
                        b.move_to(tail);
                        b.line_to(Point::new(pos.x + nx, pos.y + ny));
                        b.line_to(Point::new(pos.x - nx, pos.y - ny));
                        b.close();
                    });
                    frame.fill(&streak_path, faded(p.color, 0.45));
                }

                // Outer glow
                frame.fill(&Path::circle(pos, r * 2.2), faded(p.color, 0.25));
                // Hot core
                frame.fill(&Path::circle(pos, r), Color::WHITE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_display_target_and_smooth_interpolation() {
        let t0 = Instant::now();
        let mut display = TabGaugeDisplay::new(20);
        assert_eq!(display.displayed_value(), 20.0);
        assert_eq!(display.target_value(), 20.0);
        assert!(!display.is_animating(t0));

        // Set target from 20 -> 48
        display.set_target(48, t0);
        assert_eq!(display.target_value(), 48.0);
        assert_eq!(display.displayed_value(), 20.0);
        assert!(display.is_animating(t0));

        // Mid-point of interpolation (225ms of 450ms)
        let t_mid = t0 + Duration::from_millis(225);
        display.update(t_mid);
        let val_mid = display.displayed_value();
        assert!(val_mid > 20.0 && val_mid < 48.0);
        // Ease-out cubic at 0.5: 1 - 0.125 = 0.875
        // 20 + 28 * 0.875 = 44.5
        assert!((val_mid - 44.5).abs() < 0.2);

        // Completion at 450ms
        let t_end = t0 + Duration::from_millis(450);
        display.update(t_end);
        assert_eq!(display.displayed_value(), 48.0);
        assert!(!display.is_animating(t_end));
    }

    #[test]
    fn resync_snaps_only_when_idle_and_different() {
        let t0 = Instant::now();
        let mut display = TabGaugeDisplay::new(20);
        assert!(!display.resync(20));

        // Running interpolation is never interrupted, even if the truth differs.
        display.set_target(48, t0);
        assert!(!display.resync(76));
        assert!(display.is_animating(t0));
        assert_eq!(display.target_value(), 48.0);

        // After completion the difference is corrected.
        display.update(t0 + Duration::from_millis(450));
        assert_eq!(display.displayed_value(), 48.0);
        assert!(display.resync(76));
        assert_eq!(display.displayed_value(), 76.0);
        assert_eq!(display.target_value(), 76.0);
        assert!(!display.resync(76));
        assert!(display.resync(500)); // truth is always treated as 0..=100
        assert_eq!(display.displayed_value(), 100.0);
    }

    #[test]
    fn transfer_origin_is_always_inside_canvas() {
        for (bx, w, h) in [
            (100.0, 800.0, 600.0),
            (-50.0, 800.0, 600.0),
            (5000.0, 800.0, 600.0),
            (f32::NAN, 800.0, 600.0),
            (10.0, 10.0, 10.0),
            (10.0, 0.0, 0.0),
            (10.0, 3.0, 2.0),
            (f32::INFINITY, f32::NAN, f32::NAN),
        ] {
            let p = transfer_origin(bx, w, h);
            let (cw, ch) = (
                if w.is_finite() { w.max(0.0) } else { 0.0 },
                if h.is_finite() { h.max(0.0) } else { 0.0 },
            );
            assert!(p.x.is_finite() && p.y.is_finite(), "{bx} {w} {h}");
            assert!(p.x >= 0.0 && p.x <= cw, "x {} not in 0..={cw}", p.x);
            assert!(p.y >= 0.0 && p.y <= ch, "y {} not in 0..={ch}", p.y);
        }
        assert_eq!(
            transfer_origin(100.0, 800.0, 600.0),
            Point::new(100.0, TRANSFER_ORIGIN_Y)
        );
    }

    #[test]
    fn gauge_saturates_at_100_and_does_not_overflow() {
        let t0 = Instant::now();
        let mut display = TabGaugeDisplay::new(90);
        display.set_target(150, t0);
        assert_eq!(display.target_value(), 100.0);

        let t_end = t0 + Duration::from_millis(500);
        display.update(t_end);
        assert_eq!(display.displayed_value(), 100.0);
    }

    #[test]
    fn gauge_snap_to_resets_immediately() {
        let mut display = TabGaugeDisplay::new(10);
        display.snap_to(75);
        assert_eq!(display.displayed_value(), 75.0);
        assert_eq!(display.target_value(), 75.0);
        assert!(!display.is_animating(Instant::now()));
    }

    #[test]
    fn particle_count_scaling_is_bounded_and_non_linear() {
        assert_eq!(calculate_particle_count(0), 0);
        assert_eq!(calculate_particle_count(1), 4);
        assert_eq!(calculate_particle_count(4), 6);
        assert_eq!(calculate_particle_count(10), 9);
        assert_eq!(calculate_particle_count(25), 15);
        assert_eq!(calculate_particle_count(100), 24);
        assert_eq!(calculate_particle_count(999), 24);
    }

    #[test]
    fn transfer_particle_bezier_trajectory_reaches_exact_start_and_end() {
        let origin = Point::new(100.0, -17.5);
        let target = Point::new(350.0, 796.0);
        let c1 = Point::new(140.0, 200.0);
        let c2 = Point::new(330.0, 600.0);

        // Parametric evaluation
        let p_start = eval_cubic_bezier(origin, c1, c2, target, 0.0);
        assert_eq!(p_start, origin);

        let p_end = eval_cubic_bezier(origin, c1, c2, target, 1.0);
        assert_eq!(p_end, target);

        let p_mid = eval_cubic_bezier(origin, c1, c2, target, 0.5);
        assert!(p_mid.x > origin.x && p_mid.x < target.x);
        assert!(p_mid.y > origin.y && p_mid.y < target.y);
    }

    #[test]
    fn transfer_manager_lifecycle_from_spawn_to_arrival_and_cleanup() {
        let t0 = Instant::now();
        let mut manager = TabTransferManager::new();
        assert!(!manager.is_active());

        let origin = Point::new(115.0, -17.5);
        let target = Point::new(400.0, 796.0);

        // 1. Spawn transfer
        manager.spawn_transfer(10, origin, target, t0);
        assert!(manager.is_active());
        assert_eq!(manager.particle_count(), 9); // sqrt(10)*3 = 9.48 -> 9
        assert_eq!(manager.spark_count(), 0);

        // 2. Flight mid-way (250ms): particles actively flying
        let t_mid = t0 + Duration::from_millis(250);
        manager.update(t_mid);
        assert!(manager.is_active());
        assert!(manager.particle_count() > 0);

        // 3. Arrival after flight duration (600ms): particles arrive, sparks burst
        let t_arrive = t0 + Duration::from_millis(600);
        manager.update(t_arrive);
        assert!(manager.is_active());
        assert_eq!(manager.particle_count(), 0); // all particles arrived
        assert!(manager.spark_count() > 0); // sparks active

        // 4. After spark lifetime (800ms): everything cleared, zero memory leak
        let t_finish = t0 + Duration::from_millis(800);
        manager.update(t_finish);
        assert!(!manager.is_active());
        assert_eq!(manager.particle_count(), 0);
        assert_eq!(manager.spark_count(), 0);
    }

    #[test]
    fn clear_resets_manager_immediately() {
        let t0 = Instant::now();
        let mut manager = TabTransferManager::new();
        manager.spawn_transfer(5, Point::new(0.0, 0.0), Point::new(100.0, 100.0), t0);
        assert!(manager.is_active());

        manager.clear();
        assert!(!manager.is_active());
        assert_eq!(manager.particle_count(), 0);
        assert_eq!(manager.spark_count(), 0);
    }
}
