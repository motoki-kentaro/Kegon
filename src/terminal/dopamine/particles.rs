//! Particle simulation for the Backspace "Neon Disintegration" effect.
//!
//! A burst is one short flash, fast radial sparks that leave streaks, and slow motes of light. All of
//! them are pulled down by gravity; the long-lived ones fall across the whole viewport and are dropped
//! when they leave its bottom edge. The numbers live in [`super::params`].

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use iced::{Color, Point, Vector};

use super::params::{FALLBACK_PALETTE, VISUAL};

/// Particles alive at once on one tab.
pub const MAX_PARTICLES_PER_TAB: usize = VISUAL.max_particles_per_tab;

/// Particles in one burst.
#[cfg(test)]
pub const BACKSPACE_BURST_COUNT: usize =
    VISUAL.flash_count + VISUAL.spark_count + VISUAL.mote_count;

/// How far below the bottom edge a particle may go before it is dropped.
const CULL_MARGIN: f32 = 25.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    /// The brief bloom at the moment of impact.
    Flash,
    /// A fast streak of light thrown out radially, then pulled down.
    Spark,
    /// A slow grain of light that rains down the viewport.
    Mote,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub kind: ParticleKind,
    pub pos: Point,
    pub vel: Vector,
    pub age: f32,
    pub max_age: f32,
    pub radius: f32,
    /// The neon colour of the afterglow, without the white-hot start.
    pub color: Color,
    /// Share of the life after which the particle starts to fade out (0 = fades from the start).
    pub fade_from: f32,
    pub phase: f32,
}

/// The tail of a moving particle: a tapered triangle from `tail` to the head.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Streak {
    pub tail: Point,
    pub half_width: f32,
}

/// What the overlay draws for one particle at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub alpha: f32,
    pub radius: f32,
    /// Bright centre (alpha applied).
    pub core: Color,
    /// Halo and streak colour (alpha applied).
    pub glow: Color,
    pub halo_alpha: f32,
    pub streak: Option<Streak>,
}

impl Particle {
    /// Advances the motion by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        self.age += dt;
        let (drag, gravity) = match self.kind {
            ParticleKind::Flash => return,
            ParticleKind::Spark => (VISUAL.spark_drag, VISUAL.spark_gravity),
            ParticleKind::Mote => (VISUAL.mote_drag, VISUAL.mote_gravity),
        };
        let keep = (-drag * dt).exp();
        self.vel.x *= keep;
        self.vel.y = self.vel.y * keep + gravity * dt;
        let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
        if speed > VISUAL.max_speed {
            let scale = VISUAL.max_speed / speed;
            self.vel.x *= scale;
            self.vel.y *= scale;
        }
        self.pos.x += self.vel.x * dt;
        self.pos.y += self.vel.y * dt;
    }

    /// Whether the particle is still within its life and above the bottom edge.
    pub fn is_alive(&self, viewport_height: f32) -> bool {
        self.age < self.max_age && self.pos.y <= viewport_height + CULL_MARGIN
    }

    fn progress(&self) -> f32 {
        (self.age / self.max_age).clamp(0.0, 1.0)
    }

    /// Opacity: opaque while it travels, fading over the end of its life and over the bottom edge.
    pub fn alpha(&self, viewport_height: f32) -> f32 {
        let p = self.progress();
        let life = if p > self.fade_from {
            1.0 - (p - self.fade_from) / (1.0 - self.fade_from).max(0.01)
        } else {
            1.0
        };
        let bottom = if VISUAL.bottom_fade_px > 0.0 {
            ((viewport_height + CULL_MARGIN - self.pos.y) / (VISUAL.bottom_fade_px + CULL_MARGIN))
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        let fade = (life * bottom).clamp(0.0, 1.0);
        match self.kind {
            ParticleKind::Flash => fade * fade,
            _ => fade.powf(1.3),
        }
    }

    /// The colour of the particle's centre: white-hot at birth, the neon colour in flight, a dark
    /// afterglow at the end.
    pub fn color_at(&self) -> Color {
        let p = self.progress();
        let hot = 1.0 - smoothstep(0.0, VISUAL.hot_seconds, self.age);
        let cool = smoothstep(VISUAL.cool_from, 1.0, p);
        let deep = scale_rgb(self.color, VISUAL.deep_level);
        mix(mix(self.color, Color::WHITE, hot), deep, cool)
    }

    /// Everything the overlay needs to draw this particle now.
    pub fn look(&self, viewport_height: f32) -> Look {
        let alpha = self.alpha(viewport_height);
        let p = self.progress();
        let body = self.color_at();
        // The halo keeps the neon hue even while the centre is white-hot, so it never looks grey.
        let neon = mix(
            self.color,
            scale_rgb(self.color, VISUAL.deep_level),
            smoothstep(VISUAL.cool_from, 1.0, p),
        );
        let twinkle = match self.kind {
            ParticleKind::Mote => {
                1.0 - VISUAL.twinkle * 0.5 * (1.0 + (self.age * 38.0 + self.phase).sin())
            }
            _ => 1.0,
        };
        let radius = match self.kind {
            ParticleKind::Flash => VISUAL.flash_radius * (0.35 + 0.65 * p),
            _ => self.radius,
        };
        let streak = match self.kind {
            ParticleKind::Flash => None,
            _ => {
                let speed = (self.vel.x * self.vel.x + self.vel.y * self.vel.y).sqrt();
                let length =
                    (speed * VISUAL.streak_seconds * (1.0 - 0.45 * p)).min(VISUAL.streak_max_px);
                (length >= VISUAL.streak_min_px).then(|| {
                    let k = length / speed;
                    Streak {
                        tail: Point::new(self.pos.x - self.vel.x * k, self.pos.y - self.vel.y * k),
                        half_width: (self.radius * (1.0 - 0.6 * p)).max(0.4),
                    }
                })
            }
        };
        Look {
            alpha,
            radius,
            core: with_alpha(mix(body, Color::WHITE, VISUAL.core_white), alpha * twinkle),
            glow: with_alpha(neon, alpha * twinkle),
            halo_alpha: VISUAL.halo_alpha,
            streak,
        }
    }
}

/// Three neon colours for one burst.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette(pub [Color; 3]);

impl Palette {
    /// Derived from the foreground colour of the character being deleted. A missing, grey or white
    /// colour gives the fixed cyan / blue palette.
    pub fn from_base(base: Option<Color>) -> Self {
        let Some(base) = base else {
            return Self(FALLBACK_PALETTE);
        };
        let (h, s, _) = rgb_to_hsv(base);
        if s < VISUAL.min_saturation {
            return Self(FALLBACK_PALETTE);
        }
        Self([
            hsv_to_rgb(h, s.max(0.8), 1.0),
            hsv_to_rgb(h + 28.0, s.max(0.75), 1.0),
            hsv_to_rgb(h - 24.0, (s * 0.55).max(0.3), 1.0),
        ])
    }
}

/// Small deterministic generator (xorshift32); the burst shape needs variety, not quality.
#[derive(Debug, Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, (low, high): (f32, f32)) -> f32 {
        low + (high - low) * self.next()
    }
}

/// The particles of one terminal tab.
#[derive(Debug, Clone)]
pub struct TabParticles {
    particles: Vec<Particle>,
    rng: Rng,
    last_burst: Option<Instant>,
}

impl Default for TabParticles {
    fn default() -> Self {
        Self {
            particles: Vec::with_capacity(MAX_PARTICLES_PER_TAB),
            rng: Rng(0x9E37_79B9),
            last_burst: None,
        }
    }
}

impl TabParticles {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns one Backspace burst at `origin`, unless the previous one was too recent. Returns whether
    /// anything was spawned.
    pub fn spawn_erase_burst(&mut self, origin: Point, base: Option<Color>, now: Instant) -> bool {
        let gap = Duration::from_millis(VISUAL.min_burst_interval_ms);
        if self
            .last_burst
            .is_some_and(|last| now.duration_since(last) < gap)
        {
            return false;
        }
        let room = MAX_PARTICLES_PER_TAB.saturating_sub(self.particles.len());
        if room == 0 {
            return false;
        }
        self.last_burst = Some(now);
        let palette = Palette::from_base(base).0;
        let mut budget = room;

        if budget >= VISUAL.flash_count {
            for _ in 0..VISUAL.flash_count {
                let color = mix(palette[0], Color::WHITE, 0.6);
                self.particles.push(Particle {
                    kind: ParticleKind::Flash,
                    pos: origin,
                    vel: Vector::new(0.0, 0.0),
                    age: 0.0,
                    max_age: VISUAL.flash_life,
                    radius: VISUAL.flash_radius,
                    color,
                    fade_from: 0.0,
                    phase: 0.0,
                });
            }
            budget -= VISUAL.flash_count;
        }

        for i in 0..VISUAL.spark_count.min(budget) {
            let angle = self.rng.range((0.0, TAU));
            let speed = self.rng.range(VISUAL.spark_speed);
            let long = self.rng.next() < VISUAL.spark_long_share;
            let (max_age, fade_from) = if long {
                (self.rng.range(VISUAL.spark_long_life), VISUAL.fade_from)
            } else {
                (self.rng.range(VISUAL.spark_short_life), 0.0)
            };
            self.particles.push(Particle {
                kind: ParticleKind::Spark,
                pos: origin,
                vel: Vector::new(angle.cos() * speed, angle.sin() * speed),
                age: 0.0,
                max_age,
                radius: self.rng.range(VISUAL.spark_radius),
                color: palette[i % palette.len()],
                fade_from,
                phase: 0.0,
            });
        }
        budget = budget.saturating_sub(VISUAL.spark_count);

        for i in 0..VISUAL.mote_count.min(budget) {
            let angle = self.rng.range((0.0, TAU));
            let speed = self.rng.range(VISUAL.mote_speed);
            self.particles.push(Particle {
                kind: ParticleKind::Mote,
                pos: origin,
                vel: Vector::new(angle.cos() * speed, angle.sin() * speed),
                age: 0.0,
                max_age: self.rng.range(VISUAL.mote_life),
                radius: self.rng.range(VISUAL.mote_radius),
                color: palette[i % palette.len()],
                fade_from: VISUAL.fade_from,
                phase: self.rng.range((0.0, TAU)),
            });
        }
        true
    }

    /// Moves every particle by `dt` seconds and drops the dead ones.
    pub fn update(&mut self, dt: f32, viewport_height: f32) {
        for particle in &mut self.particles {
            particle.update(dt);
        }
        self.particles.retain(|p| p.is_alive(viewport_height));
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.particles.len()
    }
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        1.0,
    )
}

fn scale_rgb(c: Color, k: f32) -> Color {
    Color::from_rgba(c.r * k, c.g * k, c.b * k, 1.0)
}

fn with_alpha(mut c: Color, alpha: f32) -> Color {
    c.a = alpha.clamp(0.0, 1.0);
    c
}

/// Hue in degrees, saturation and value in 0..=1.
fn rgb_to_hsv(c: Color) -> (f32, f32, f32) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if max == c.r {
        60.0 * (((c.g - c.b) / d).rem_euclid(6.0))
    } else if max == c.g {
        60.0 * ((c.b - c.r) / d + 2.0)
    } else {
        60.0 * ((c.r - c.g) / d + 4.0)
    };
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::from_rgb(r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(origin: Point) -> TabParticles {
        let mut tp = TabParticles::new();
        assert!(tp.spawn_erase_burst(origin, None, Instant::now()));
        tp
    }

    fn count(tp: &TabParticles, kind: ParticleKind) -> usize {
        tp.particles().iter().filter(|p| p.kind == kind).count()
    }

    #[test]
    fn a_burst_is_a_flash_sparks_and_motes() {
        let tp = burst(Point::new(50.0, 50.0));
        assert_eq!(tp.len(), BACKSPACE_BURST_COUNT);
        assert_eq!(count(&tp, ParticleKind::Flash), VISUAL.flash_count);
        assert_eq!(count(&tp, ParticleKind::Spark), VISUAL.spark_count);
        assert_eq!(count(&tp, ParticleKind::Mote), VISUAL.mote_count);
        assert!(
            tp.particles()
                .iter()
                .all(|p| p.pos == Point::new(50.0, 50.0))
        );
    }

    #[test]
    fn the_start_is_radial() {
        // Over a few bursts the sparks head out in every direction, not in one fan.
        let mut tp = TabParticles::new();
        let t0 = Instant::now();
        for i in 0..6u64 {
            tp.spawn_erase_burst(Point::ORIGIN, None, t0 + Duration::from_millis(20 * i));
        }
        let sparks: Vec<_> = tp
            .particles()
            .iter()
            .filter(|p| p.kind == ParticleKind::Spark)
            .collect();
        let quadrant = |p: &&Particle| (p.vel.x >= 0.0, p.vel.y >= 0.0);
        for q in [(true, true), (true, false), (false, true), (false, false)] {
            assert!(
                sparks.iter().any(|p| quadrant(p) == q),
                "nothing heads {q:?}"
            );
        }
        for p in &sparks {
            let speed = (p.vel.x * p.vel.x + p.vel.y * p.vel.y).sqrt();
            assert!(
                (VISUAL.spark_speed.0 - 1.0..=VISUAL.spark_speed.1 + 1.0).contains(&speed),
                "{speed}"
            );
        }
    }

    #[test]
    fn bursts_closer_than_the_interval_are_skipped_and_the_cap_holds() {
        let mut tp = TabParticles::new();
        let t0 = Instant::now();
        assert!(tp.spawn_erase_burst(Point::ORIGIN, None, t0));
        assert!(!tp.spawn_erase_burst(Point::ORIGIN, None, t0 + Duration::from_millis(3)));
        assert_eq!(tp.len(), BACKSPACE_BURST_COUNT);

        for i in 1..200u64 {
            tp.spawn_erase_burst(Point::ORIGIN, None, t0 + Duration::from_millis(20 * i));
            assert!(tp.len() <= MAX_PARTICLES_PER_TAB);
        }
        assert_eq!(
            tp.len(),
            MAX_PARTICLES_PER_TAB,
            "the pool fills up and stays at the cap"
        );
        // A full pool takes no more.
        assert!(!tp.spawn_erase_burst(Point::ORIGIN, None, t0 + Duration::from_secs(60)));
    }

    #[test]
    fn the_colour_runs_from_white_through_neon_to_a_dark_afterglow() {
        let base = Color::from_rgb(0.1, 0.9, 1.0);
        let mut p = Particle {
            kind: ParticleKind::Mote,
            pos: Point::ORIGIN,
            vel: Vector::new(0.0, 0.0),
            age: 0.0,
            max_age: 2.0,
            radius: 1.5,
            color: base,
            fade_from: 0.9,
            phase: 0.0,
        };
        let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        let at = |p: &mut Particle, share: f32| {
            p.age = p.max_age * share;
            p.color_at()
        };
        let birth = at(&mut p, 0.0);
        assert!(
            birth.r > 0.97 && birth.g > 0.97 && birth.b > 0.97,
            "white-hot at birth: {birth:?}"
        );
        let flight = at(&mut p, 0.3);
        assert!(
            (flight.g - base.g).abs() < 0.02 && flight.r < 0.2,
            "neon in flight: {flight:?}"
        );
        let end = at(&mut p, 1.0);
        assert!(
            luminance(end) < luminance(flight) * 0.5,
            "dark at the end: {end:?}"
        );
        // Smooth: no step between neighbouring instants.
        let mut last = at(&mut p, 0.0);
        for i in 1..=480 {
            let c = at(&mut p, i as f32 / 480.0);
            let step = (c.r - last.r)
                .abs()
                .max((c.g - last.g).abs())
                .max((c.b - last.b).abs());
            assert!(step < 0.08, "jump of {step} at {i}/480");
            last = c;
        }
    }

    #[test]
    fn fast_particles_get_a_streak_that_shortens_and_slow_ones_do_not() {
        let mut p = Particle {
            kind: ParticleKind::Spark,
            pos: Point::new(100.0, 100.0),
            vel: Vector::new(600.0, 0.0),
            age: 0.0,
            max_age: 1.0,
            radius: 1.5,
            color: Color::WHITE,
            fade_from: 0.0,
            phase: 0.0,
        };
        let early = p.look(800.0).streak.expect("fast: streak");
        let early_len = 100.0 - early.tail.x;
        assert!(early_len > VISUAL.streak_min_px && early_len <= VISUAL.streak_max_px + 0.01);
        assert!(early.tail.y == 100.0, "the tail trails along the velocity");
        p.age = 0.8;
        let late = p.look(800.0).streak.expect("still fast: streak");
        assert!(100.0 - late.tail.x < early_len, "the tail gets shorter");
        assert!(late.half_width < early.half_width, "and thinner");
        p.vel = Vector::new(20.0, 10.0);
        assert!(p.look(800.0).streak.is_none(), "slow: no streak");
        p.kind = ParticleKind::Flash;
        p.vel = Vector::new(600.0, 0.0);
        assert!(p.look(800.0).streak.is_none(), "a flash has none");
    }

    #[test]
    fn grey_or_missing_text_colour_uses_the_cyan_palette_and_a_colourful_one_its_own() {
        assert_eq!(Palette::from_base(None).0, FALLBACK_PALETTE);
        assert_eq!(
            Palette::from_base(Some(Color::from_rgb(0.8, 0.8, 0.8))).0,
            FALLBACK_PALETTE
        );
        assert_eq!(Palette::from_base(Some(Color::WHITE)).0, FALLBACK_PALETTE);
        let red = Palette::from_base(Some(Color::from_rgb(0.9, 0.2, 0.2))).0;
        assert_ne!(red, FALLBACK_PALETTE);
        let (h, ..) = rgb_to_hsv(red[0]);
        assert!(h < 8.0 || h > 352.0, "primary keeps the hue: {h}");
        for c in red {
            let (_, _, v) = rgb_to_hsv(c);
            assert!(v > 0.99, "neon colours are fully bright");
        }
    }

    #[test]
    fn motes_fall_all_the_way_down_a_tall_viewport_before_they_expire() {
        // Spawned at the very top; every mote must leave through the bottom, not time out half way.
        for height in [600.0f32, 1000.0, 1600.0] {
            let mut tp = burst(Point::new(300.0, 0.0));
            let mut t = 0.0;
            while !tp.is_empty() && t < 10.0 {
                tp.update(1.0 / 60.0, height);
                t += 1.0 / 60.0;
            }
            assert!(
                tp.is_empty(),
                "everything ends within 10 s at height {height}"
            );
        }
        // Direct check: a mote thrown upward reaches the bottom of a 1600 px viewport alive.
        let mut mote = Particle {
            kind: ParticleKind::Mote,
            pos: Point::new(0.0, 0.0),
            vel: Vector::new(0.0, -300.0),
            age: 0.0,
            max_age: VISUAL.mote_life.0,
            radius: 1.2,
            color: Color::WHITE,
            fade_from: VISUAL.fade_from,
            phase: 0.0,
        };
        let mut reached = false;
        while mote.is_alive(1600.0) {
            mote.update(1.0 / 60.0);
            reached |= mote.pos.y > 1500.0;
        }
        assert!(reached, "reached the bottom while still alive");
        assert!(
            mote.age < mote.max_age,
            "left through the bottom edge, not by timeout"
        );
    }

    #[test]
    fn short_sparks_die_young_while_long_ones_and_motes_remain() {
        let mut tp = TabParticles::new();
        let t0 = Instant::now();
        for i in 0..10u64 {
            tp.spawn_erase_burst(
                Point::new(100.0, 50.0),
                None,
                t0 + Duration::from_millis(20 * i),
            );
        }
        tp.update(0.5, 2000.0);
        assert_eq!(count(&tp, ParticleKind::Flash), 0, "the flash is brief");
        assert!(
            count(&tp, ParticleKind::Spark) > 0,
            "some sparks keep falling"
        );
        assert!(
            count(&tp, ParticleKind::Spark) < 10 * VISUAL.spark_count,
            "short sparks are gone"
        );
        assert_eq!(count(&tp, ParticleKind::Mote), 10 * VISUAL.mote_count);
    }

    #[test]
    fn particles_below_the_viewport_are_dropped_and_the_pool_goes_idle() {
        let mut tp = burst(Point::new(50.0, 700.0));
        for _ in 0..600 {
            tp.update(1.0 / 60.0, 800.0);
        }
        assert!(tp.is_empty());
    }

    #[test]
    fn alpha_is_full_in_flight_and_zero_at_the_end_and_below_the_edge() {
        let mut p = Particle {
            kind: ParticleKind::Mote,
            pos: Point::new(10.0, 10.0),
            vel: Vector::new(0.0, 0.0),
            age: 0.5,
            max_age: 3.5,
            radius: 1.0,
            color: Color::WHITE,
            fade_from: 0.88,
            phase: 0.0,
        };
        assert!((p.alpha(800.0) - 1.0).abs() < 1e-6, "no early fade");
        p.age = p.max_age;
        assert!(p.alpha(800.0) < 1e-6);
        p.age = 1.0;
        p.pos.y = 825.0;
        assert!(p.alpha(800.0) < 1e-6, "invisible once past the edge");
        p.pos.y = 790.0;
        let a = p.alpha(800.0);
        assert!(a > 0.0 && a < 1.0, "fading over the last pixels: {a}");
    }
}
