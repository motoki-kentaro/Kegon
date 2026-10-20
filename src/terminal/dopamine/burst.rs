//! Bounded, wall-clock Burst animation. Geometry is derived from the current viewport.
//!
//! A Burst is a 4 second show that always renders every [`Effect`] at once (center
//! explosion, gauge fountain, party cracker) together with a shared cut-in text and
//! shared micro particles. Only the cut-in text is drawn at random from [`TEXT_TABLE`].
//! Everything is a pure function of `(seed pieces, elapsed time, viewport)`: painting never
//! allocates persistent state, and every piece has a finite life inside [`DURATION`].
use super::border::hsv_to_rgb;
use super::gauge::GAUGE_HEIGHT;
use iced::widget::canvas::{Frame, Path, Stroke, Text};
use iced::{Color, Font, Point, Size};
use std::time::{Duration, Instant};

/// Whole Burst, including the overlapped afterglow (no separate fifth second).
pub const DURATION: Duration = Duration::from_millis(4000);
/// The Burst only fires at a full gauge.
const MAX_ANCHOR_GAUGE: f32 = 100.0;
pub const TOTAL_SECS: f32 = 4.0;
/// Cut-in enters from the left edge to the center: 0.0..1.0 s.
pub const ENTER_SECS: f32 = 1.0;
/// Cut-in rests in the center: 1.0..3.0 s.
pub const HOLD_SECS: f32 = 2.0;
/// Cut-in leaves to the right edge: 3.0..4.0 s. The afterglow decays over this same window.
pub const EXIT_SECS: f32 = 1.0;
pub const AFTERGLOW_START_SECS: f32 = ENTER_SECS + HOLD_SECS;

// Piece budgets (the previous single-variant Burst used 180 motes, 24 spikes, 1 ring).
pub const COMMON_MOTES: usize = 540;
pub const EXPLOSION_SPIKES: usize = 72;
pub const EXPLOSION_SALVOS: usize = 3;
pub const RINGS_PER_SALVO: usize = 3;
pub const FOUNTAIN_EMITTERS: usize = 8;
pub const FOUNTAIN_SPARKS: usize = 270;
pub const CONFETTI: usize = 270;
pub const WHITE_SHOWER_EMITTERS: usize = 6;
pub const WHITE_SHOWER: usize = 240;
/// Confetti is shot from the four canvas corners (TL, TR, BL, BR) toward the center.
pub const CONFETTI_CORNERS: usize = 4;
const CORNER_INSET: f32 = 4.0;
/// Total random aim error (radians) around the corner->center direction.
const CONFETTI_SPREAD: f32 = 0.7;
const CONFETTI_DRAG: f32 = 2.2;

/// Independent white firework fountains rising from the bottom edge.
#[derive(Debug, Clone, Copy)]
pub struct FireworkConfig {
    /// Launch X as a fraction of the canvas width; one fountain per entry.
    pub x_fractions: &'static [f32],
    pub particles_each: usize,
    /// Emission window inside the Burst (seconds). Nothing is emitted outside it.
    pub start_secs: f32,
    pub end_secs: f32,
    /// Tiny per-fountain start delay.
    pub stagger_secs: f32,
    pub life_secs: (f32, f32),
}
pub const FIREWORKS: FireworkConfig = FireworkConfig {
    x_fractions: &[0.2, 0.4, 0.6, 0.8],
    particles_each: 150,
    start_secs: 0.3,
    end_secs: 2.5,
    stagger_secs: 0.04,
    life_secs: (0.9, 1.5),
};
pub const FIREWORK_PARTICLES: usize = FIREWORKS.x_fractions.len() * FIREWORKS.particles_each;
const FIREWORK_GRAVITY: f32 = 2.0;

/// Screen Shake A: the Burst-confirmed impact, at the very start of the Burst.
pub const SHAKE_A_START_SECS: f32 = 0.0;
pub const SHAKE_A_DURATION_SECS: f32 = 0.5;
/// Screen Shake B: sustained while the cut-in rests in the center. Start and length come
/// from the cut-in timeline itself, so changing `HOLD_SECS` moves/resizes the shake with it.
pub const SHAKE_B_START_SECS: f32 = ENTER_SECS;
pub const SHAKE_B_DURATION_SECS: f32 = HOLD_SECS;
/// Shake B holds full strength, then eases to zero over this last stretch of the rest.
const SHAKE_B_FADE_SECS: f32 = 0.4;
/// Maximum displacement magnitude in logical px.
pub const SHAKE_MAX_PX: f32 = 5.0;

/// Micro particles never exceed this diameter in logical px.
pub const MOTE_MAX_DIAMETER: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstText {
    pub text: &'static str,
    /// Relative draw weight; all candidates are currently equal.
    pub weight: u32,
}
const fn t(text: &'static str) -> BurstText {
    BurstText { text, weight: 1 }
}
/// Table driven: add a row to add a candidate, change `weight` to bias the draw.
pub const TEXT_TABLE: &[BurstText] = &[
    t("DOPAMINE BURST!!"),
    t("DOPA BURST!!"),
    t("MAX BURST!!"),
    t("BURST MODE!!"),
    t("DOPA RUSH!!"),
    t("RAINBOW BURST!!"),
    t("NEON BURST!!"),
    t("FULL CHARGE!!"),
    t("MAX CHARGE!!"),
    t("OVERDRIVE!!"),
    t("HYPER BURST!!"),
    t("DOPA DRIVE!!"),
    t("RAINBOW RUSH!!"),
    t("PARTY TIME!!"),
    t("DOPA FEVER!!"),
    t("FEVER BURST!!"),
    t("BOOST MAX!!"),
    t("GO WILD!!"),
    t("LET’S GOOO!!"),
    t("BRAIN SPARK!!"),
];

/// Weighted draw from [`TEXT_TABLE`]; never returns a string outside the table.
pub fn pick_text(rng: &mut Rng) -> &'static str {
    let total: u64 = TEXT_TABLE.iter().map(|e| u64::from(e.weight)).sum();
    if total == 0 {
        return TEXT_TABLE[0].text;
    }
    let mut roll = rng.index(total as usize) as u64;
    for entry in TEXT_TABLE {
        let w = u64::from(entry.weight);
        if roll < w {
            return entry.text;
        }
        roll -= w;
    }
    TEXT_TABLE[0].text
}

/// Visual effects stay separate so they can be toggled/extended; a Burst draws all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Explosion,
    Fountain,
    Party,
    /// Independent white firework fountains (see [`FIREWORKS`]).
    Fireworks,
}
pub const ALL_EFFECTS: &[Effect] = &[
    Effect::Explosion,
    Effect::Fountain,
    Effect::Party,
    Effect::Fireworks,
];

/// SplitMix64, seeded once per controller; rejection sampling avoids modulo bias.
pub struct Rng(pub u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn index(&mut self, count: usize) -> usize {
        let n = count as u64;
        let threshold = n.wrapping_neg() % n;
        loop {
            let value = self.next();
            if value >= threshold {
                return (value % n) as usize;
            }
        }
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16777216.0
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.unit() * (hi - lo)
    }
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    Square,
    Cross,
    Circle,
}

/// One finite-lived particle. `born + life <= TOTAL_SECS` always holds.
#[derive(Debug, Clone)]
struct Piece {
    born: f32,
    life: f32,
    angle: f32,
    speed: f32,
    u: f32,
    size: f32,
    shape: Shape,
    hue: f32,
}

impl Piece {
    fn new(rng: &mut Rng, born: f32, life: (f32, f32), size: (f32, f32)) -> Self {
        let life = rng.range(life.0, life.1).min(TOTAL_SECS - born);
        Self {
            born,
            life,
            angle: rng.unit() * std::f32::consts::TAU,
            speed: rng.unit(),
            u: rng.unit(),
            size: rng.range(size.0, size.1),
            shape: [Shape::Square, Shape::Cross, Shape::Circle][rng.index(3)],
            hue: rng.unit() * 360.0,
        }
    }
    /// Normalized age `0..1` while alive.
    fn q(&self, t: f32) -> Option<f32> {
        let age = t - self.born;
        (age >= 0.0 && age < self.life).then(|| age / self.life)
    }
    #[cfg(test)]
    fn alive_at(&self, t: f32) -> bool {
        self.q(t).is_some()
    }
    /// Quick bloom, long soft decay.
    fn envelope(q: f32) -> f32 {
        (q / 0.08).min(1.0) * ((1.0 - q) / 0.6).min(1.0)
    }
}

#[derive(Debug, Clone)]
pub struct Burst {
    pub started: Instant,
    /// Identifies this Burst instance for its sound, so a stale cancel can never stop a newer
    /// Burst. Assigned by the controller when the Burst is accepted; 0 until then.
    pub audio_id: u64,
    /// Gauge value (0..=100) captured when the Burst fired. The wave/fountain geometry is
    /// anchored to it as a fraction of the canvas width and never follows the shrinking display.
    anchor_gauge: f32,
    /// Effects drawn simultaneously. Always [`ALL_EFFECTS`] for now.
    pub effects: &'static [Effect],
    pub text: &'static str,
    motes: Vec<Piece>,
    sparks: Vec<Piece>,
    confetti: Vec<Piece>,
    shower: Vec<Piece>,
    fireworks: Vec<Piece>,
}

/// Oscillation shared by both shakes: two out-of-phase sines, so the magnitude is `<= amp`.
fn shake_wave(local: f32, amp: f32) -> (f32, f32) {
    let k = std::f32::consts::TAU * local;
    let norm = std::f32::consts::SQRT_2;
    (amp * (k * 17.0).sin() / norm, amp * (k * 23.0).sin() / norm)
}

/// Shake A: short and sharp, strongest at `t = 0` and decaying quadratically to zero at 0.5 s.
fn shake_a_offset(secs: f32) -> (f32, f32) {
    let local = secs - SHAKE_A_START_SECS;
    if !(0.0..SHAKE_A_DURATION_SECS).contains(&local) {
        return (0.0, 0.0);
    }
    let rest = 1.0 - local / SHAKE_A_DURATION_SECS;
    shake_wave(local, SHAKE_MAX_PX * rest * rest)
}

/// Shake B over `[start, start + duration)`: full strength, then a smoothstep fade in the
/// last [`SHAKE_B_FADE_SECS`] (or the whole span if it is shorter) down to zero.
fn shake_b_offset_with(secs: f32, start: f32, duration: f32) -> (f32, f32) {
    let local = secs - start;
    if !(0.0..duration).contains(&local) {
        return (0.0, 0.0);
    }
    let fade = SHAKE_B_FADE_SECS.min(duration);
    let left = ((duration - local) / fade).clamp(0.0, 1.0);
    shake_wave(local, SHAKE_MAX_PX * smoothstep(left))
}

/// Deterministic Screen Shake offset (logical px) `secs` after the Burst started: Shake A
/// (`0.0..0.5 s`) plus Shake B (the cut-in rest, `1.0..3.0 s`); the two never overlap and the
/// magnitude never exceeds [`SHAKE_MAX_PX`]. Zero everywhere else, including the cut-out.
pub fn shake_offset(secs: f32) -> (f32, f32) {
    if !secs.is_finite() {
        return (0.0, 0.0);
    }
    let a = shake_a_offset(secs);
    let b = shake_b_offset_with(secs, SHAKE_B_START_SECS, SHAKE_B_DURATION_SECS);
    (a.0 + b.0, a.1 + b.1)
}

pub fn text_size(terminal_size: f32) -> f32 {
    terminal_size * 2.0
}
fn ease_out(q: f32) -> f32 {
    1.0 - (1.0 - q.clamp(0.0, 1.0)).powi(3)
}
fn ease_in(q: f32) -> f32 {
    q.clamp(0.0, 1.0).powi(3)
}
/// 1.0 until the exit window starts, then falls to 0.0 exactly at the end of the Burst.
pub fn afterglow(secs: f32) -> f32 {
    ((TOTAL_SECS - secs) / EXIT_SECS).clamp(0.0, 1.0)
}
/// Gray cut-in band alpha: ~50% while shown, quick fade-in, fades with the exit.
pub fn band_alpha(secs: f32) -> f32 {
    0.5 * (secs / 0.25).clamp(0.0, 1.0).min(afterglow(secs))
}

/// Viewport-derived anchor points.
struct Geo {
    w: f32,
    h: f32,
    cx: f32,
    cy: f32,
    gauge_y: f32,
    fill_w: f32,
}

fn snapped(u: f32, count: usize) -> f32 {
    ((u * count as f32).floor().min(count as f32 - 1.0) + 0.5) / count as f32
}

impl Burst {
    pub fn new(now: Instant, rng: &mut Rng) -> Self {
        Self::with_effects(now, rng, ALL_EFFECTS)
    }

    pub fn with_effects(now: Instant, rng: &mut Rng, effects: &'static [Effect]) -> Self {
        let motes = (0..COMMON_MOTES)
            .map(|_| {
                let born = rng_born(rng, 2.4);
                Piece::new(rng, born, (0.9, 1.6), (1.5, MOTE_MAX_DIAMETER))
            })
            .collect();
        let sparks = (0..FOUNTAIN_SPARKS)
            .map(|_| {
                let born = rng_born(rng, 2.6);
                let mut p = Piece::new(rng, born, (0.6, 1.2), (1.5, 3.5));
                p.u = snapped(p.u, FOUNTAIN_EMITTERS);
                p
            })
            .collect();
        let confetti = (0..CONFETTI)
            .map(|i| {
                // Three volleys; corners are dealt round-robin so none is starved by chance.
                let volley = (i % 3) as f32 * 0.8;
                let born = volley + rng.unit() * 0.3;
                let mut p = Piece::new(rng, born, (1.6, 2.1), (6.0, 11.0));
                p.u = ((i % CONFETTI_CORNERS) as f32 + 0.5) / CONFETTI_CORNERS as f32;
                p
            })
            .collect();
        let fountains = FIREWORKS.x_fractions.len();
        let fireworks = (0..FIREWORK_PARTICLES)
            .map(|i| {
                let f = i % fountains;
                let window = FIREWORKS.end_secs
                    - FIREWORKS.start_secs
                    - fountains as f32 * FIREWORKS.stagger_secs;
                let born = FIREWORKS.start_secs
                    + f as f32 * FIREWORKS.stagger_secs
                    + rng.unit() * window.max(0.0);
                let mut p = Piece::new(rng, born, FIREWORKS.life_secs, (1.5, 4.0));
                p.u = (f as f32 + 0.5) / fountains as f32;
                p
            })
            .collect();
        let shower = (0..WHITE_SHOWER)
            .map(|_| {
                let born = rng_born(rng, 1.4);
                let mut p = Piece::new(rng, born, (0.5, 0.9), (1.5, 4.0));
                p.u = snapped(p.u, WHITE_SHOWER_EMITTERS);
                p
            })
            .collect();
        Self {
            started: now,
            audio_id: 0,
            anchor_gauge: MAX_ANCHOR_GAUGE,
            effects,
            text: pick_text(rng),
            motes,
            sparks,
            confetti,
            shower,
            fireworks,
        }
    }

    pub fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started).as_secs_f32() / DURATION.as_secs_f32())
            .clamp(0.0, 1.0)
    }
    /// Display-only gauge: 100 -> 0 linearly over the whole Burst.
    pub fn gauge(&self, now: Instant) -> f32 {
        100.0 * (1.0 - self.progress(now))
    }

    #[cfg(test)]
    /// Number of pieces ever allocated for this Burst (constant for its whole life).
    pub fn piece_count(&self) -> usize {
        self.motes.len()
            + self.sparks.len()
            + self.confetti.len()
            + self.shower.len()
            + self.fireworks.len()
    }
    #[cfg(test)]
    /// Pieces still alive at `secs` after the start.
    pub fn alive_pieces(&self, secs: f32) -> usize {
        [
            &self.motes,
            &self.sparks,
            &self.confetti,
            &self.shower,
            &self.fireworks,
        ]
        .iter()
        .map(|v| v.iter().filter(|p| p.alive_at(secs)).count())
        .sum()
    }

    /// Screen Shake offset for the terminal body at `now` (render-only; never an input offset).
    pub fn shake_offset(&self, now: Instant) -> (f32, f32) {
        shake_offset(now.saturating_duration_since(self.started).as_secs_f32())
    }

    pub fn cut_in_x(&self, now: Instant, width: f32, text_width: f32) -> f32 {
        let t = now.saturating_duration_since(self.started).as_secs_f32();
        let center = (width - text_width) / 2.0;
        if t < ENTER_SECS {
            -text_width + (center + text_width) * ease_out(t / ENTER_SECS)
        } else if t < AFTERGLOW_START_SECS {
            center
        } else {
            center + (width - center) * ease_in((t - AFTERGLOW_START_SECS) / EXIT_SECS)
        }
    }

    fn geo(&self, size: Size) -> Geo {
        Geo {
            w: size.width,
            h: size.height,
            cx: size.width / 2.0,
            cy: size.height / 2.0,
            gauge_y: size.height - GAUGE_HEIGHT / 2.0,
            fill_w: size.width * self.anchor_gauge / 100.0,
        }
    }

    pub fn paint(
        &self,
        frame: &mut Frame,
        size: Size,
        now: Instant,
        terminal_size: f32,
        font: Font,
    ) {
        if self.progress(now) >= 1.0 || size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let secs = now.saturating_duration_since(self.started).as_secs_f32();
        let glow = afterglow(secs);
        let geo = self.geo(size);
        for effect in self.effects {
            match effect {
                Effect::Explosion => paint_explosion(frame, &geo, secs, glow),
                Effect::Fountain => self.paint_fountain(frame, &geo, secs, glow),
                Effect::Party => self.paint_party(frame, &geo, secs, glow),
                Effect::Fireworks => self.paint_fireworks(frame, &geo, secs, glow),
            }
        }
        self.paint_motes(frame, &geo, secs, glow);
        self.paint_cut_in(frame, &geo, now, secs, terminal_size, font);
    }

    fn paint_motes(&self, frame: &mut Frame, g: &Geo, secs: f32, glow: f32) {
        if self.effects.is_empty() {
            return;
        }
        for (i, m) in self.motes.iter().enumerate() {
            let Some(q) = m.q(secs) else { continue };
            let at = launch(self.effects[i % self.effects.len()], m, q * m.life, g);
            let color = hsv_to_rgb(m.hue + secs * 120.0, 0.85, 1.0, 1.0);
            paint_mote(frame, at, m, color, Piece::envelope(q) * glow);
        }
    }

    fn paint_fountain(&self, frame: &mut Frame, g: &Geo, secs: f32, glow: f32) {
        if g.fill_w < 1.0 {
            return;
        }
        let top = g.h - GAUGE_HEIGHT;
        // Neon strip riding the gauge fill.
        let rise = (secs / 0.3).min(1.0) * glow;
        let pulse = 0.65 + 0.35 * (secs * 14.0).sin();
        let seg = g.fill_w / 16.0;
        for k in 0..16 {
            frame.fill_rectangle(
                Point::new(seg * k as f32, top - 10.0),
                Size::new(seg + 0.5, 10.0),
                hsv_to_rgb(
                    k as f32 * 22.0 + secs * 360.0,
                    0.9,
                    1.0,
                    0.45 * pulse * rise,
                ),
            );
        }
        // Glowing pillars: three widths, vertical fade built from stacked slices.
        for e in 0..FOUNTAIN_EMITTERS {
            let x = g.fill_w * (e as f32 + 0.5) / FOUNTAIN_EMITTERS as f32;
            let height = g.h * (0.45 + 0.2 * (secs * 9.0 + e as f32 * 1.7).sin()) * rise;
            let hue = e as f32 * 45.0 + secs * 240.0;
            for (width, alpha, white) in [(26.0, 0.07, 0.0), (14.0, 0.12, 0.2), (5.0, 0.35, 0.8)] {
                const SLICES: usize = 6;
                for j in 0..SLICES {
                    let slice_h = height / SLICES as f32;
                    let a = alpha * (1.0 - j as f32 / SLICES as f32) * rise;
                    let base = hsv_to_rgb(hue, 0.9 - white * 0.6, 1.0, a);
                    frame.fill_rectangle(
                        Point::new(x - width / 2.0, top - slice_h * (j as f32 + 1.0)),
                        Size::new(width, slice_h + 0.5),
                        base,
                    );
                }
            }
        }
        // Sparks and streaks shot upward from the pillars.
        for p in &self.sparks {
            let Some(q) = p.q(secs) else { continue };
            let age = q * p.life;
            let at = launch(Effect::Fountain, p, age, g);
            let before = launch(Effect::Fountain, p, (age - 0.05).max(0.0), g);
            if at.y > g.gauge_y {
                continue;
            }
            let a = Piece::envelope(q) * glow;
            frame.stroke(
                &Path::line(before, at),
                Stroke::default()
                    .with_width(p.size * 0.6)
                    .with_color(hsv_to_rgb(p.hue + secs * 200.0, 0.7, 1.0, a)),
            );
            paint_mote(frame, at, p, Color::WHITE, a);
        }
    }

    fn paint_party(&self, frame: &mut Frame, g: &Geo, secs: f32, glow: f32) {
        for p in &self.confetti {
            let Some(q) = p.q(secs) else { continue };
            let age = q * p.life;
            let at = launch(Effect::Party, p, age, g);
            if at.x < -16.0 || at.x > g.w + 16.0 || at.y < -16.0 || at.y > g.h + 16.0 {
                continue;
            }
            let spin = p.angle + age * (6.0 + 8.0 * p.speed);
            // Foreshortened width gives a flutter.
            let hw = p.size * 0.5;
            let hh = p.size * 0.25 * (age * 10.0 + p.angle).cos().abs().max(0.25);
            let (s, c) = spin.sin_cos();
            let corner = |x: f32, y: f32| Point::new(at.x + x * c - y * s, at.y + x * s + y * c);
            let path = Path::new(|b| {
                b.move_to(corner(-hw, -hh));
                b.line_to(corner(hw, -hh));
                b.line_to(corner(hw, hh));
                b.line_to(corner(-hw, hh));
                b.close();
            });
            let fade = (1.0 - q).min(1.0).powf(0.5) * glow;
            frame.fill(&path, hsv_to_rgb(p.hue + age * 90.0, 0.9, 1.0, fade));
        }
        // White shower jetted from several gauge points for a short time.
        for p in &self.shower {
            let Some(q) = p.q(secs) else { continue };
            let at = launch(Effect::Fountain, p, q * p.life, g);
            if at.y > g.gauge_y {
                continue;
            }
            paint_mote(frame, at, p, Color::WHITE, Piece::envelope(q) * glow);
        }
    }

    fn paint_fireworks(&self, frame: &mut Frame, g: &Geo, secs: f32, glow: f32) {
        // Muzzle glow while a fountain is emitting.
        if (FIREWORKS.start_secs..FIREWORKS.end_secs).contains(&secs) {
            let flicker = 0.7 + 0.3 * (secs * 40.0).sin();
            for frac in FIREWORKS.x_fractions {
                let base = Point::new(g.w * frac, g.gauge_y);
                for (r, a) in [(14.0, 0.12), (8.0, 0.3), (3.5, 0.8)] {
                    frame.fill(
                        &Path::circle(base, r),
                        Color::from_rgba(1.0, 1.0, 1.0, a * flicker * glow),
                    );
                }
            }
        }
        for (i, p) in self.fireworks.iter().enumerate() {
            let Some(q) = p.q(secs) else { continue };
            let age = q * p.life;
            let at = launch(Effect::Fireworks, p, age, g);
            if at.y > g.gauge_y + 2.0 || at.x < -8.0 || at.x > g.w + 8.0 {
                continue;
            }
            // Slight brightness scatter per spark.
            let bright = 0.65 + 0.35 * ((i as f32 * 12.9898).sin() * 0.5 + 0.5);
            let a = Piece::envelope(q) * glow * bright;
            let before = launch(Effect::Fireworks, p, (age - 0.04).max(0.0), g);
            frame.stroke(
                &Path::line(before, at),
                Stroke::default()
                    .with_width(p.size * 0.5)
                    .with_color(Color::from_rgba(1.0, 1.0, 1.0, a * 0.5)),
            );
            paint_mote(frame, at, p, Color::from_rgb(0.9, 0.95, 1.0), a);
        }
    }

    fn paint_cut_in(
        &self,
        frame: &mut Frame,
        g: &Geo,
        now: Instant,
        secs: f32,
        terminal_size: f32,
        font: Font,
    ) {
        let fs = text_size(terminal_size);
        let band_h = fs * 1.8;
        frame.fill_rectangle(
            Point::new(0.0, g.cy - band_h / 2.0),
            Size::new(g.w, band_h),
            Color::from_rgba(0.5, 0.5, 0.5, band_alpha(secs)),
        );
        let advance = fs * 0.68;
        let width = advance * self.text.chars().count() as f32;
        let x = self.cut_in_x(now, g.w, width);
        let bold = Font {
            weight: iced::font::Weight::Bold,
            ..font
        };
        for (i, c) in self.text.chars().enumerate() {
            // Fast rainbow sweep with per-letter flicker.
            let hue = i as f32 * 28.0 + secs * 720.0;
            let value = 0.88 + 0.12 * (secs * 30.0 + i as f32 * 1.9).sin();
            let color = hsv_to_rgb(hue, 0.9, value, 1.0);
            let pos = Point::new(x + i as f32 * advance, g.cy);
            for (dx, dy) in [(-2.5, 0.0), (2.5, 0.0), (0.0, -2.5), (0.0, 2.5), (0.0, 0.0)] {
                let mut layer = if dx == 0.0 && dy == 0.0 {
                    color
                } else {
                    hsv_to_rgb(hue + 40.0, 0.7, 1.0, 0.32)
                };
                if dx == 0.0 && dy == 0.0 {
                    layer.a = 1.0;
                }
                frame.fill_text(Text {
                    content: c.to_string(),
                    position: Point::new(pos.x + dx, pos.y + dy),
                    color: layer,
                    size: fs.into(),
                    font: bold,
                    align_y: iced::alignment::Vertical::Center,
                    ..Text::default()
                });
            }
        }
        // Twinkling glints around the text.
        for k in 0..14 {
            let phase = (secs * 5.0 + k as f32 * 0.7).sin() * 0.5 + 0.5;
            let gx = x + width * ((k as f32 * 0.6180339).fract());
            let gy = g.cy + band_h * 0.5 * ((k as f32 * 0.7548776).fract() * 2.0 - 1.0);
            let r = 3.0 + 5.0 * phase;
            let color = hsv_to_rgb(
                k as f32 * 40.0 + secs * 300.0,
                0.3,
                1.0,
                phase * band_alpha(secs) * 2.0,
            );
            frame.fill_rectangle(Point::new(gx - r, gy - 0.6), Size::new(r * 2.0, 1.2), color);
            frame.fill_rectangle(Point::new(gx - 0.6, gy - r), Size::new(1.2, r * 2.0), color);
        }
    }
}

fn rng_born(rng: &mut Rng, max: f32) -> f32 {
    rng.unit() * max
}

/// Position of a piece `age` seconds after its birth for the given emitter.
fn launch(effect: Effect, p: &Piece, age: f32, g: &Geo) -> Point {
    match effect {
        Effect::Explosion => {
            let reach = g.w.hypot(g.h) * 0.5;
            let d = reach * (0.15 + 0.85 * p.speed) * ease_out(age / p.life);
            Point::new(g.cx + p.angle.cos() * d, g.cy + p.angle.sin() * d)
        }
        Effect::Fountain => {
            let v0 = g.h * (0.7 + 0.8 * p.speed);
            Point::new(
                g.fill_w * p.u + p.angle.sin() * g.w * 0.08 * age,
                g.gauge_y - v0 * age + 0.5 * g.h * 1.6 * age * age,
            )
        }
        Effect::Party => {
            let origin = corner_origin(((p.u * CONFETTI_CORNERS as f32) as usize).min(3), g);
            let (dx, dy) = (g.cx - origin.x, g.cy - origin.y);
            let dist = dx.hypot(dy).max(1e-3);
            // Aim at the center, then scatter by a bounded random angle.
            let (sin_j, cos_j) = ((p.hue / 360.0 - 0.5) * CONFETTI_SPREAD).sin_cos();
            let (ux, uy) = (dx / dist, dy / dist);
            let (ax, ay) = (ux * cos_j - uy * sin_j, ux * sin_j + uy * cos_j);
            // Fast muzzle velocity, then air drag brings the pieces to rest around the center.
            let reach = dist * (0.8 + 0.5 * p.speed);
            let spent = 1.0 - (-CONFETTI_DRAG * age).exp();
            let lateral = reach * 0.2 * p.angle.sin() * spent;
            let flutter = (age * 6.0 + p.angle * 5.0).sin() * 8.0 * (1.0 - (-3.0 * age).exp());
            Point::new(
                origin.x + ax * reach * spent - ay * lateral + flutter,
                origin.y + ay * reach * spent + ax * lateral + 0.5 * g.h * 0.35 * age * age,
            )
        }
        Effect::Fireworks => {
            let n = FIREWORKS.x_fractions.len().max(1);
            let idx = ((p.u * n as f32) as usize).min(n - 1);
            let frac = FIREWORKS.x_fractions.get(idx).copied().unwrap_or(0.5);
            let v0 = g.h * (1.1 + 0.7 * p.speed);
            let vx = g.w * 0.22 * (p.angle / std::f32::consts::TAU - 0.5);
            Point::new(
                g.w * frac + vx * age,
                g.gauge_y - v0 * age + 0.5 * g.h * FIREWORK_GRAVITY * age * age,
            )
        }
    }
}

/// Confetti muzzle for `corner` (0 TL, 1 TR, 2 BL, 3 BR), just inside the canvas.
fn corner_origin(corner: usize, g: &Geo) -> Point {
    let ix = CORNER_INSET.min(g.w / 2.0);
    let iy = CORNER_INSET.min(g.h / 2.0);
    Point::new(
        if corner.is_multiple_of(2) {
            ix
        } else {
            g.w - ix
        },
        if corner < 2 { iy } else { g.h - iy },
    )
}

/// Number of stacked translucent discs that approximate the smooth radial halo.
pub const HALO_LAYERS: usize = 24;
/// Halo outer radius relative to the core base radius.
const HALO_REACH: f32 = 1.8;
const HALO_PEAK_ALPHA: f32 = 0.9;

#[derive(Clone, Copy, Debug)]
struct HaloLayer {
    radius: f32,
    rgb: [f32; 3],
    alpha: f32,
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// White -> warm ivory -> pale gray, by normalized radius (0 = center, 1 = rim).
fn halo_rgb(rho: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 3] = [
        (0.0, [1.0, 1.0, 1.0]),
        (0.35, [1.0, 0.97, 0.86]),
        (1.0, [0.84, 0.85, 0.88]),
    ];
    let rho = rho.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let ((r0, c0), (r1, c1)) = (pair[0], pair[1]);
        if rho <= r1 {
            let t = (rho - r0) / (r1 - r0);
            return [0, 1, 2].map(|i| c0[i] + (c1[i] - c0[i]) * t);
        }
    }
    STOPS[2].1
}

/// Outermost-first discs whose *composited* opacity follows a smooth `(1 - smoothstep)^1.5`
/// falloff, so no single boundary stands out as a ring. Per-layer alpha is derived from the
/// target opacity of the neighbouring layers, which keeps the profile monotone by construction.
fn halo_layers(base: f32) -> [HaloLayer; HALO_LAYERS] {
    let base = if base.is_finite() { base.max(0.0) } else { 0.0 };
    let target = |rho: f32| HALO_PEAK_ALPHA * (1.0 - smoothstep(rho)).powf(1.5);
    let mut prev = 0.0_f32;
    std::array::from_fn(|i| {
        // Layer i covers rho <= outer; its visible band centre sets the target opacity.
        let outer = 1.0 - i as f32 / HALO_LAYERS as f32;
        let mid = outer - 0.5 / HALO_LAYERS as f32;
        let t = target(mid).max(prev);
        let alpha = (1.0 - (1.0 - t) / (1.0 - prev)).clamp(0.0, 1.0);
        prev = t;
        HaloLayer {
            radius: base * HALO_REACH * outer,
            rgb: halo_rgb(mid),
            alpha,
        }
    })
}

/// The rotating cross flash lives for the first two seconds of the Burst.
pub const CROSS_SECS: f32 = 2.0;
const CROSS_START_DEG: f32 = 15.0;
const CROSS_SWEEP_DEG: f32 = 125.0;
const CROSS_ARM_SEGMENTS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
struct CrossFlash {
    /// Clockwise on screen (canvas y points down, so a growing angle turns clockwise).
    angle: f32,
    /// Half length of one arm, as a fraction of the shorter viewport side.
    length: f32,
    intensity: f32,
}

/// `None` once the flash is over (>= 2.0 s) or not started.
fn cross_flash(secs: f32) -> Option<CrossFlash> {
    if !secs.is_finite() || !(0.0..CROSS_SECS).contains(&secs) {
        return None;
    }
    let q = secs / CROSS_SECS;
    // Fast start, decelerating: ease-out sweep.
    let angle = (CROSS_START_DEG + CROSS_SWEEP_DEG * ease_out(q)).to_radians();
    let ignite = (secs / 0.08).clamp(0.0, 1.0);
    Some(CrossFlash {
        angle,
        length: 0.2 + 0.25 * ease_out((q * 2.5).min(1.0)),
        intensity: ignite * (1.0 - q).powf(1.5),
    })
}

/// Tapered lens-shaped arm: thickest in the middle, pointed and transparent at both tips.
fn arm_path(c: Point, angle: f32, half_len: f32, half_width: f32) -> Path {
    let (dx, dy) = (angle.cos(), angle.sin());
    let side = |t: f32, sign: f32| {
        let w = half_width * (1.0 - t.abs()).powf(1.4) * sign;
        let along = t * half_len;
        Point::new(c.x + dx * along - dy * w, c.y + dy * along + dx * w)
    };
    let n = CROSS_ARM_SEGMENTS;
    Path::new(|b| {
        b.move_to(side(-1.0, 1.0));
        for i in 1..=n {
            b.line_to(side(-1.0 + 2.0 * i as f32 / n as f32, 1.0));
        }
        for i in (0..=n).rev() {
            b.line_to(side(-1.0 + 2.0 * i as f32 / n as f32, -1.0));
        }
        b.close();
    })
}

fn paint_cross_flash(frame: &mut Frame, g: &Geo, secs: f32, glow: f32) {
    let Some(flash) = cross_flash(secs) else {
        return;
    };
    let c = Point::new(g.cx, g.cy);
    let half_len = g.w.min(g.h) * flash.length;
    let half_width = half_len * 0.09;
    // (width scale, length scale, rgb, alpha): cyan bloom -> pale blue-white -> white-hot core.
    const LAYERS: [(f32, f32, [f32; 3], f32); 3] = [
        (3.2, 1.0, [0.35, 0.8, 1.0], 0.16),
        (1.7, 0.94, [0.78, 0.95, 1.0], 0.38),
        (0.7, 0.86, [1.0, 1.0, 1.0], 0.95),
    ];
    for (ws, ls, rgb, alpha) in LAYERS {
        for k in 0..2 {
            let angle = flash.angle + k as f32 * std::f32::consts::FRAC_PI_2;
            frame.fill(
                &arm_path(c, angle, half_len * ls, half_width * ws),
                Color::from_rgba(rgb[0], rgb[1], rgb[2], alpha * flash.intensity * glow),
            );
        }
    }
}

fn paint_explosion(frame: &mut Frame, g: &Geo, secs: f32, glow: f32) {
    let c = Point::new(g.cx, g.cy);
    let reach = g.w.hypot(g.h) * 0.5;
    let salvo_age = |s: usize| secs - s as f32;
    // White-hot core that re-ignites with every salvo.
    let kick = (0..EXPLOSION_SALVOS)
        .map(|s| (1.0 - salvo_age(s) / 0.6).clamp(0.0, 1.0) * (salvo_age(s) >= 0.0) as u8 as f32)
        .fold(0.0_f32, f32::max);
    let base = g.w.min(g.h) * 0.09 * (1.0 + 0.6 * kick);
    let flicker = 0.7 + 0.3 * (secs * 12.0).sin();
    for layer in halo_layers(base) {
        frame.fill(
            &Path::circle(c, layer.radius),
            Color::from_rgba(
                layer.rgb[0],
                layer.rgb[1],
                layer.rgb[2],
                layer.alpha * flicker * glow,
            ),
        );
    }
    paint_cross_flash(frame, g, secs, glow);
    for s in 0..EXPLOSION_SALVOS {
        // Shock rings.
        for k in 0..RINGS_PER_SALVO {
            let age = salvo_age(s) - k as f32 * 0.12;
            if !(0.0..1.4).contains(&age) {
                continue;
            }
            let q = age / 1.4;
            frame.stroke(
                &Path::circle(c, reach * (0.8 + 0.1 * k as f32) * ease_out(q)),
                Stroke::default()
                    .with_width(2.0 + 6.0 * (1.0 - q))
                    .with_color(hsv_to_rgb(
                        s as f32 * 90.0 + k as f32 * 40.0 + q * 120.0,
                        0.8,
                        1.0,
                        (1.0 - q) * glow,
                    )),
            );
        }
        // Radial spikes.
        let age = salvo_age(s);
        if !(0.0..1.2).contains(&age) {
            continue;
        }
        let q = age / 1.2;
        for i in 0..EXPLOSION_SPIKES {
            let a = i as f32 / EXPLOSION_SPIKES as f32 * std::f32::consts::TAU + s as f32 * 0.07;
            let variation = 0.55 + 0.45 * ((i * 37) % 13) as f32 / 13.0;
            let inner = reach * 0.1 * q;
            let outer = reach * variation * ease_out(q);
            frame.stroke(
                &Path::line(
                    Point::new(c.x + a.cos() * inner, c.y + a.sin() * inner),
                    Point::new(c.x + a.cos() * outer, c.y + a.sin() * outer),
                ),
                Stroke::default()
                    .with_width(1.5 + 2.0 * (1.0 - q))
                    .with_color(hsv_to_rgb(
                        i as f32 * 5.0 + secs * 200.0,
                        0.85,
                        1.0,
                        (1.0 - q) * glow,
                    )),
            );
        }
    }
}

fn paint_mote(frame: &mut Frame, at: Point, m: &Piece, color: Color, alpha: f32) {
    if alpha <= 0.0 || !at.x.is_finite() || !at.y.is_finite() {
        return;
    }
    // All nested shapes fit a diameter <=5 logical px, including square diagonals.
    // Bright white center, colored halo, alpha falling off toward the outside.
    for (scale, a) in [(1.0, 0.12), (0.7, 0.35), (0.3, 0.95)] {
        let r = m.size.min(MOTE_MAX_DIAMETER) * 0.5 * scale;
        let color = if scale < 0.4 {
            Color::from_rgba(1.0, 1.0, 1.0, a * alpha)
        } else {
            Color {
                a: a * alpha,
                ..color
            }
        };
        match m.shape {
            Shape::Circle => frame.fill(&Path::circle(at, r), color),
            Shape::Square => {
                let half = r / std::f32::consts::SQRT_2;
                frame.fill_rectangle(
                    Point::new(at.x - half, at.y - half),
                    Size::new(half * 2.0, half * 2.0),
                    color,
                );
            }
            Shape::Cross => {
                frame.fill_rectangle(
                    Point::new(at.x - r * 0.9, at.y - r * 0.2),
                    Size::new(r * 1.8, r * 0.4),
                    color,
                );
                frame.fill_rectangle(
                    Point::new(at.x - r * 0.2, at.y - r * 0.9),
                    Size::new(r * 0.4, r * 1.8),
                    color,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_pieces(b: &Burst) -> impl Iterator<Item = &Piece> {
        b.motes
            .iter()
            .chain(&b.sparks)
            .chain(&b.confetti)
            .chain(&b.shower)
            .chain(&b.fireworks)
    }

    fn geo(w: f32, h: f32) -> Geo {
        Geo {
            w,
            h,
            cx: w / 2.0,
            cy: h / 2.0,
            gauge_y: h - GAUGE_HEIGHT / 2.0,
            fill_w: w,
        }
    }

    fn corner_of(p: &Piece) -> usize {
        ((p.u * CONFETTI_CORNERS as f32) as usize).min(3)
    }

    #[test]
    fn text_table_holds_all_twenty_candidates_and_draws_stay_inside_it() {
        assert_eq!(TEXT_TABLE.len(), 20);
        let expected = [
            "DOPAMINE BURST!!",
            "DOPA BURST!!",
            "MAX BURST!!",
            "BURST MODE!!",
            "DOPA RUSH!!",
            "RAINBOW BURST!!",
            "NEON BURST!!",
            "FULL CHARGE!!",
            "MAX CHARGE!!",
            "OVERDRIVE!!",
            "HYPER BURST!!",
            "DOPA DRIVE!!",
            "RAINBOW RUSH!!",
            "PARTY TIME!!",
            "DOPA FEVER!!",
            "FEVER BURST!!",
            "BOOST MAX!!",
            "GO WILD!!",
            "LET’S GOOO!!",
            "BRAIN SPARK!!",
        ];
        for (row, text) in TEXT_TABLE.iter().zip(expected) {
            assert_eq!(row.text, text);
            assert_eq!(row.weight, 1);
        }
        let mut rng = Rng(69);
        let mut counts = [0; 20];
        for _ in 0..60000 {
            let text = pick_text(&mut rng);
            let i = TEXT_TABLE
                .iter()
                .position(|e| e.text == text)
                .expect("drawn text must come from the table");
            counts[i] += 1;
        }
        for n in counts {
            assert!((2650..3350).contains(&n), "{n}");
        }
    }

    #[test]
    fn every_burst_draws_all_three_effects_regardless_of_seed() {
        for seed in 0..64 {
            let b = Burst::new(Instant::now(), &mut Rng(seed));
            assert_eq!(b.effects, ALL_EFFECTS);
            for e in [
                Effect::Explosion,
                Effect::Fountain,
                Effect::Party,
                Effect::Fireworks,
            ] {
                assert!(b.effects.contains(&e), "{e:?} missing for seed {seed}");
            }
        }
    }

    #[test]
    fn four_second_timeline_enter_hold_exit_with_overlapped_afterglow() {
        assert_eq!(DURATION, Duration::from_secs(4));
        assert_eq!(ENTER_SECS + HOLD_SECS + EXIT_SECS, TOTAL_SECS);
        assert_eq!(ENTER_SECS, 1.0);
        assert_eq!(AFTERGLOW_START_SECS, 3.0);
        let now = Instant::now();
        let b = Burst::new(now, &mut Rng(42));
        let at = |ms| now + Duration::from_millis(ms);
        // 0..1s: enters from off-screen left to the center.
        assert_eq!(b.cut_in_x(at(0), 800.0, 400.0), -400.0);
        let mid = b.cut_in_x(at(500), 800.0, 400.0);
        assert!((-400.0..200.0).contains(&mid), "{mid}");
        // 1..3s: held exactly at the center.
        for ms in [1000, 1500, 2000, 2999] {
            assert_eq!(b.cut_in_x(at(ms), 800.0, 400.0), 200.0, "{ms}");
        }
        // 3..4s: leaves to the right, fully off-screen at 4s.
        let out = b.cut_in_x(at(3500), 800.0, 400.0);
        assert!((200.0..800.0).contains(&out), "{out}");
        assert!(b.cut_in_x(at(4000), 800.0, 400.0) >= 800.0);
        // Afterglow shares the exit window and reaches zero together with it.
        assert_eq!(afterglow(0.0), 1.0);
        assert_eq!(afterglow(AFTERGLOW_START_SECS), 1.0);
        let g = afterglow(3.5);
        assert!(g > 0.0 && g < 1.0);
        assert_eq!(afterglow(TOTAL_SECS), 0.0);
        // Some pieces are still glowing inside the exit window; none survive the end.
        assert!(b.alive_pieces(3.5) > 0);
        assert_eq!(b.alive_pieces(TOTAL_SECS), 0);
        assert_eq!(b.progress(at(4000)), 1.0);
        assert_eq!(b.progress(at(2000)), 0.5);
    }

    #[test]
    fn band_is_half_transparent_gray_while_text_is_resting() {
        for secs in [0.5, 1.0, 2.0, 3.0] {
            assert_eq!(band_alpha(secs), 0.5, "{secs}");
        }
        assert!(band_alpha(0.1) < 0.5);
        assert!(band_alpha(3.5) < 0.5 && band_alpha(3.5) > 0.0);
        assert_eq!(band_alpha(4.0), 0.0);
    }

    #[test]
    fn display_gauge_shrinks_with_the_burst_to_zero() {
        let now = Instant::now();
        let b = Burst::new(now, &mut Rng(7));
        assert_eq!(b.gauge(now), 100.0);
        assert_eq!(b.gauge(now + Duration::from_secs(1)), 75.0);
        assert_eq!(b.gauge(now + Duration::from_secs(2)), 50.0);
        assert_eq!(b.gauge(now + Duration::from_secs(4)), 0.0);
        assert_eq!(b.gauge(now + Duration::from_secs(40)), 0.0);
        let mut prev = 100.0;
        for ms in (0..=4000).step_by(50) {
            let v = b.gauge(now + Duration::from_millis(ms));
            assert!(v <= prev, "gauge grew at {ms}ms");
            prev = v;
        }
    }

    #[test]
    fn density_is_about_three_times_the_previous_single_effect_burst() {
        // Previous Burst: 180 motes, 24 spikes, 1 ring, 180 party confetti/showers.
        const {
            assert!(COMMON_MOTES >= 180 * 3);
        }
        const {
            assert!(EXPLOSION_SPIKES >= 24 * 3);
        }
        const {
            assert!(EXPLOSION_SALVOS * RINGS_PER_SALVO >= 3);
        }
        const {
            assert!(CONFETTI >= 180 + 90);
        }
        const {
            assert!(WHITE_SHOWER >= 180);
        }
        let b = Burst::new(Instant::now(), &mut Rng(1));
        assert_eq!(
            b.piece_count(),
            COMMON_MOTES + FOUNTAIN_SPARKS + CONFETTI + WHITE_SHOWER + FIREWORK_PARTICLES
        );
        assert!(b.piece_count() >= 3 * 180);
    }

    #[test]
    fn pieces_have_finite_life_bounded_size_and_never_accumulate() {
        let b = Burst::new(Instant::now(), &mut Rng(42));
        let count = b.piece_count();
        for p in all_pieces(&b) {
            assert!(p.life > 0.0);
            assert!(p.born >= 0.0 && p.born + p.life <= TOTAL_SECS + 1e-4);
            assert!(!p.alive_at(TOTAL_SECS));
        }
        assert!(b.motes.iter().all(|m| m.size <= MOTE_MAX_DIAMETER));
        assert!(b.shower.iter().all(|m| m.size <= MOTE_MAX_DIAMETER));
        for shape in [Shape::Square, Shape::Cross, Shape::Circle] {
            assert!(
                b.motes
                    .iter()
                    .any(|m| std::mem::discriminant(&m.shape) == std::mem::discriminant(&shape))
            );
        }
        // Ticking time forward never changes the allocation budget.
        for ms in (0..=4000).step_by(100) {
            assert_eq!(b.piece_count(), count);
            assert!(b.alive_pieces(ms as f32 / 1000.0) <= count);
        }
    }

    #[test]
    fn launch_positions_are_finite_for_every_effect_and_viewport() {
        let b = Burst::new(Instant::now(), &mut Rng(5));
        for (w, h) in [(1280.0, 720.0), (200.0, 80.0), (1.0, 1.0)] {
            for ms in (0..=4000).step_by(250) {
                let g = b.geo(Size::new(w, h));
                for effect in ALL_EFFECTS {
                    for p in all_pieces(&b) {
                        if let Some(q) = p.q(ms as f32 / 1000.0) {
                            let pt = launch(*effect, p, q * p.life, &g);
                            assert!(pt.x.is_finite() && pt.y.is_finite());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn text_is_twice_the_terminal_font_size() {
        for size in [8.0, 12.0, 16.0, 18.5, 32.0, 64.0] {
            assert_eq!(text_size(size), size * 2.0);
        }
    }

    #[test]
    fn confetti_is_dealt_evenly_to_the_four_corners_for_every_seed() {
        for seed in [1, 2, 42, 999] {
            let b = Burst::new(Instant::now(), &mut Rng(seed));
            let mut counts = [0; 4];
            for p in &b.confetti {
                counts[corner_of(p)] += 1;
            }
            for n in counts {
                assert!(n >= CONFETTI / 4 - 1, "{counts:?}");
            }
        }
    }

    #[test]
    fn confetti_launches_from_the_corners_aimed_at_the_center() {
        for (w, h) in [
            (800.0_f32, 600.0_f32),
            (1920.0, 1000.0),
            (40.0, 30.0),
            (1.0, 1.0),
        ] {
            let g = geo(w, h);
            let b = Burst::new(Instant::now(), &mut Rng(7));
            let origins: Vec<Point> = (0..4).map(|c| corner_origin(c, &g)).collect();
            // Four in-canvas muzzles: TL, TR, BL, BR.
            assert!(origins[0].x <= origins[1].x);
            assert!(origins[0].y <= origins[2].y);
            for o in &origins {
                assert!(o.x >= 0.0 && o.x <= w && o.y >= 0.0 && o.y <= h);
            }
            for p in &b.confetti {
                let o = origins[corner_of(p)];
                let start = launch(Effect::Party, p, 0.0, &g);
                assert!((start.x - o.x).abs() < 1e-3 && (start.y - o.y).abs() < 1e-3);
                if w < 100.0 {
                    continue;
                }
                // Initial velocity points at the center within the scatter cone.
                let soon = launch(Effect::Party, p, 0.03, &g);
                let (vx, vy) = (soon.x - o.x, soon.y - o.y);
                let (cx, cy) = (g.cx - o.x, g.cy - o.y);
                let cos = (vx * cx + vy * cy) / (vx.hypot(vy) * cx.hypot(cy));
                assert!(cos >= 0.65_f32.cos(), "cos {cos}");
                assert!(vx.hypot(vy) > 1.0, "muzzle speed too low");
            }
        }
    }

    #[test]
    fn confetti_is_fast_at_first_then_slows_and_stays_finite() {
        let g = geo(800.0, 600.0);
        let b = Burst::new(Instant::now(), &mut Rng(3));
        for p in &b.confetti {
            let at = |a: f32| launch(Effect::Party, p, a, &g);
            let o = at(0.0);
            let early = (at(0.1).x - o.x).hypot(at(0.1).y - o.y);
            let late = (at(1.6).x - at(1.5).x).hypot(at(1.6).y - at(1.5).y);
            assert!(early > late, "{early} <= {late}");
            assert!(at(p.life).x.is_finite() && at(p.life).y.is_finite());
        }
    }

    #[test]
    fn four_fireworks_stand_at_20_40_60_80_percent_of_the_width() {
        assert_eq!(FIREWORKS.x_fractions, &[0.2, 0.4, 0.6, 0.8]);
        for (w, h) in [(1000.0_f32, 600.0_f32), (640.0, 360.0), (50.0, 30.0)] {
            let g = geo(w, h);
            let b = Burst::new(Instant::now(), &mut Rng(11));
            let mut per_fountain = [0; 4];
            for p in &b.fireworks {
                let f = ((p.u * 4.0) as usize).min(3);
                per_fountain[f] += 1;
                let start = launch(Effect::Fireworks, p, 0.0, &g);
                assert!((start.x - w * FIREWORKS.x_fractions[f]).abs() < 1e-3);
                assert!(
                    (start.y - g.gauge_y).abs() < 1e-3,
                    "launched from the bottom edge"
                );
            }
            assert_eq!(per_fountain, [FIREWORKS.particles_each; 4]);
        }
    }

    #[test]
    fn firework_particles_rise_then_fall_in_a_parabola_and_spread_sideways() {
        let g = geo(1000.0, 600.0);
        let b = Burst::new(Instant::now(), &mut Rng(5));
        let (mut left, mut right) = (0, 0);
        for p in &b.fireworks {
            let y = |a: f32| launch(Effect::Fireworks, p, a, &g).y;
            let x = |a: f32| launch(Effect::Fireworks, p, a, &g).x;
            assert!(y(0.05) < y(0.0), "initial velocity is upward");
            // Constant gravity: positive, equal second differences.
            let second = y(0.4) - 2.0 * y(0.3) + y(0.2);
            assert!(second > 0.0);
            assert!((second - (y(0.7) - 2.0 * y(0.6) + y(0.5))).abs() < 1e-2);
            // Apex, then descent.
            let apex = (0..150)
                .map(|k| y(k as f32 * 0.01))
                .fold(f32::MAX, f32::min);
            assert!(apex < y(0.0) - 50.0, "rises well above the muzzle");
            assert!(y(1.4) > apex);
            match x(0.5).partial_cmp(&x(0.0)) {
                Some(std::cmp::Ordering::Less) => left += 1,
                Some(std::cmp::Ordering::Greater) => right += 1,
                _ => {}
            }
        }
        assert!(left > 100 && right > 100, "{left}/{right}");
    }

    #[test]
    fn firework_emission_is_bounded_in_count_time_and_life() {
        let b = Burst::new(Instant::now(), &mut Rng(9));
        assert_eq!(b.fireworks.len(), FIREWORK_PARTICLES);
        assert_eq!(FIREWORK_PARTICLES, 600);
        for p in &b.fireworks {
            assert!(p.born >= FIREWORKS.start_secs);
            assert!(p.born <= FIREWORKS.end_secs);
            assert!(p.life <= FIREWORKS.life_secs.1);
            assert!(p.born + p.life <= TOTAL_SECS + 1e-4);
            assert!(p.size <= MOTE_MAX_DIAMETER);
            assert!(!p.alive_at(TOTAL_SECS));
        }
        // Nothing before the emission window; continuous emission inside it.
        assert!(
            b.fireworks
                .iter()
                .all(|p| !p.alive_at(FIREWORKS.start_secs - 0.01))
        );
        for secs in [0.6, 1.0, 1.5, 2.0] {
            let alive = b.fireworks.iter().filter(|p| p.alive_at(secs)).count();
            assert!(alive > 50, "{secs}: {alive}");
        }
    }

    fn mag(secs: f32) -> f32 {
        let (x, y) = shake_offset(secs);
        x.hypot(y)
    }

    fn peak(from_ms: u32, to_ms: u32) -> f32 {
        (from_ms..to_ms)
            .map(|ms| mag(ms as f32 / 1000.0))
            .fold(0.0_f32, f32::max)
    }

    #[test]
    fn shake_a_covers_the_first_half_second_and_decays_to_zero() {
        assert_eq!(SHAKE_A_START_SECS, 0.0);
        assert_eq!(SHAKE_A_DURATION_SECS, 0.5);
        assert_eq!(SHAKE_MAX_PX, 5.0);
        let (mut x_used, mut y_used) = (false, false);
        for ms in 0..500 {
            let (x, y) = shake_offset(ms as f32 / 1000.0);
            assert!(x.hypot(y) <= SHAKE_MAX_PX + 1e-4, "{ms}ms {x},{y}");
            x_used |= x.abs() > 0.5;
            y_used |= y.abs() > 0.5;
        }
        assert!(x_used && y_used);
        assert!(peak(0, 100) > 2.0, "sharp start");
        assert!(peak(0, 100) > peak(400, 500), "decays");
        // 0.5..1.0 s is quiet, exactly zero from 0.5 s.
        for ms in (500..1000).step_by(3) {
            assert_eq!(shake_offset(ms as f32 / 1000.0), (0.0, 0.0), "{ms}ms");
        }
    }

    #[test]
    fn shake_b_runs_through_the_whole_cut_in_rest_and_stops_at_cut_out() {
        assert_eq!(SHAKE_B_START_SECS, ENTER_SECS);
        assert_eq!(
            SHAKE_B_START_SECS + SHAKE_B_DURATION_SECS,
            AFTERGLOW_START_SECS
        );
        assert_eq!(SHAKE_B_DURATION_SECS, HOLD_SECS);
        assert!(
            mag(0.9999) < 1e-6 && peak(1000, 1100) > 2.0,
            "starts at 1.0 s"
        );
        // Keeps shaking all the way through the rest: no 100 ms window goes quiet.
        for from in (1000..2500).step_by(100) {
            assert!(peak(from, from + 100) > 1.5, "{from}ms");
        }
        // Smooth fade at the end: magnitude shrinks and is bounded.
        assert!(peak(2900, 3000) < peak(2000, 2100));
        for ms in 1000..3000 {
            assert!(mag(ms as f32 / 1000.0) <= SHAKE_MAX_PX + 1e-4, "{ms}ms");
        }
        // Cut-out (3.0..4.0 s) and afterwards never shake.
        for ms in (3000..6000).step_by(3) {
            assert_eq!(shake_offset(ms as f32 / 1000.0), (0.0, 0.0), "{ms}ms");
        }
        assert_eq!(shake_offset(f32::NAN), (0.0, 0.0));
    }

    #[test]
    fn shake_b_follows_a_changed_rest_duration_and_both_shakes_join_without_jumps() {
        // A 3 s rest starting at 1.0 s keeps shaking until 4.0 s, then stops.
        let b = |secs| shake_b_offset_with(secs, 1.0, 3.0);
        assert_ne!(b(3.5), (0.0, 0.0));
        assert_eq!(b(4.0), (0.0, 0.0));
        // A very short rest still fades to zero inside itself.
        assert_eq!(shake_b_offset_with(1.2, 1.0, 0.2), (0.0, 0.0));
        // Boundaries: A->quiet at 0.5 s, quiet->B at 1.0 s, B->quiet at 3.0 s are continuous.
        for edge in [0.5_f32, 1.0, 3.0] {
            assert!(
                mag(edge - 0.001) < 1.0,
                "{edge} before: {}",
                mag(edge - 0.001)
            );
            assert!(
                mag(edge + 0.001) < 1.0,
                "{edge} after: {}",
                mag(edge + 0.001)
            );
        }
        // Frame-to-frame steps stay far below the 5 px bound (no teleporting).
        let mut prev = shake_offset(0.0);
        for ms in 1..4000 {
            let cur = shake_offset(ms as f32 / 1000.0);
            assert!((cur.0 - prev.0).hypot(cur.1 - prev.1) < 3.5, "{ms}ms");
            prev = cur;
        }
    }

    #[test]
    fn shake_offset_follows_the_burst_clock_and_the_cut_in_rest_instant() {
        let now = Instant::now();
        let b = Burst::new(now, &mut Rng(1));
        let at = |ms| now + Duration::from_millis(ms);
        assert_ne!(b.shake_offset(at(20)), (0.0, 0.0));
        assert_eq!(b.shake_offset(at(500)), (0.0, 0.0));
        assert_eq!(b.shake_offset(at(999)), (0.0, 0.0));
        assert_ne!(b.shake_offset(at(1020)), (0.0, 0.0));
        assert_ne!(b.shake_offset(at(2000)), (0.0, 0.0));
        assert_eq!(b.shake_offset(at(3000)), (0.0, 0.0));
        assert_eq!(b.shake_offset(at(3500)), (0.0, 0.0));
        // The shake starts exactly when the cut-in reaches its resting position.
        assert_eq!(b.cut_in_x(at(1000), 800.0, 400.0), 200.0);
        assert!(b.cut_in_x(at(990), 800.0, 400.0) < 200.0);
        // A clock before the start (saturating) is t = 0, where the sine has not moved yet.
        assert_eq!(b.shake_offset(now), (0.0, 0.0));
    }

    #[test]
    fn wave_anchor_is_fixed_at_launch_while_the_gauge_display_shrinks() {
        let now = Instant::now();
        let b = Burst::new(now, &mut Rng(21));
        let size = Size::new(1000.0, 600.0);
        let fixed = b.geo(size);
        assert_eq!(fixed.fill_w, 1000.0, "anchored to the launch gauge (100%)");
        // The displayed gauge shrinks 100 -> 50 -> 0 ...
        assert_eq!(b.gauge(now), 100.0);
        assert_eq!(b.gauge(now + Duration::from_secs(2)), 50.0);
        assert_eq!(b.gauge(now + Duration::from_secs(4)), 0.0);
        // ... but the wave geometry never moves: geo() has no time input at all, and the
        // positions of the same piece at the same age are identical whenever it is evaluated.
        let at = |g: &Geo| {
            b.sparks
                .iter()
                .chain(&b.shower)
                .chain(&b.motes)
                .map(|p| launch(Effect::Fountain, p, 0.4, g))
                .collect::<Vec<_>>()
        };
        let reference = at(&fixed);
        for _ in [0, 2, 4] {
            let g = b.geo(size);
            assert_eq!(g.fill_w, fixed.fill_w);
            assert_eq!(at(&g), reference);
        }
        // Emitter columns span the full launch width and start inside the canvas.
        let xs: Vec<f32> = b
            .sparks
            .iter()
            .map(|p| launch(Effect::Fountain, p, 0.0, &fixed).x)
            .collect();
        assert!(xs.iter().all(|x| *x >= 0.0 && *x <= 1000.0));
        assert!(
            xs.iter().any(|x| *x > 800.0),
            "not squeezed toward the left"
        );
    }

    #[test]
    fn wave_anchor_scales_with_the_canvas_on_resize_and_animation_keeps_running() {
        let b = Burst::new(Instant::now(), &mut Rng(22));
        let (small, large) = (
            b.geo(Size::new(500.0, 300.0)),
            b.geo(Size::new(1000.0, 600.0)),
        );
        // Normalized: the same fraction of the width after a resize, no jump.
        assert_eq!(small.fill_w / small.w, large.fill_w / large.w);
        // The wave itself still moves over time (rise, then fall back).
        let p = &b.sparks[0];
        let y = |age: f32| launch(Effect::Fountain, p, age, &large).y;
        assert!(y(0.1) < y(0.0));
        assert!(y(2.0) > y(0.4));
        // Pieces live and die on their own clock, not the gauge's.
        assert!(b.alive_pieces(0.5) > 0 && b.alive_pieces(3.9) > 0);
        assert_eq!(b.alive_pieces(TOTAL_SECS), 0);
    }

    #[test]
    fn halo_is_many_stepped_smooth_and_monotone_from_center_to_rim() {
        for base in [0.0, 1.0, 12.0, 90.0, 400.0, f32::NAN, f32::INFINITY] {
            let layers = halo_layers(base);
            assert!(layers.len() >= 8);
            let mut composite = 0.0_f32;
            let mut last_radius = f32::MAX;
            let mut steps = Vec::new();
            for l in layers {
                assert!(l.radius.is_finite() && l.radius >= 0.0);
                assert!(l.radius <= last_radius, "outermost first");
                last_radius = l.radius;
                assert!((0.0..=1.0).contains(&l.alpha));
                assert!(l.rgb.iter().all(|c| (0.0..=1.0).contains(c)));
                let next = 1.0 - (1.0 - composite) * (1.0 - l.alpha);
                assert!(next >= composite - 1e-6, "opacity grows toward the center");
                steps.push(next - composite);
                composite = next;
            }
            assert!(composite > 0.8, "{composite}");
            assert!(steps.iter().all(|s| *s < 0.2), "{steps:?}");
        }
        let layers = halo_layers(50.0);
        assert!(layers[0].alpha < 0.05, "rim fades to transparent");
        assert!(layers[HALO_LAYERS - 1].rgb[2] > 0.98, "white center");
        assert!(
            layers[HALO_LAYERS - 1].rgb[2] > layers[0].rgb[2],
            "rim is grayer"
        );
    }

    #[test]
    fn cross_flash_lasts_two_seconds_and_turns_clockwise_with_deceleration() {
        assert_eq!(CROSS_SECS, 2.0);
        let at = |s| cross_flash(s).expect("alive");
        assert!(
            cross_flash(0.0).is_some(),
            "spawned together with the Burst"
        );
        assert!(cross_flash(CROSS_SECS).is_none());
        assert!(cross_flash(3.9).is_none());
        assert!(cross_flash(-0.1).is_none() && cross_flash(f32::NAN).is_none());
        let (a0, a1, a2) = (at(0.0).angle, at(1.0).angle, at(1.999).angle);
        assert!((a0 - CROSS_START_DEG.to_radians()).abs() < 1e-6);
        // Clockwise on a y-down canvas == growing angle, never a full turn.
        assert!(a0 < a1 && a1 < a2);
        let sweep = (a2 - a0).to_degrees();
        assert!((90.0..=140.0).contains(&sweep), "{sweep}");
        assert!(a1 - a0 > a2 - a1, "decelerates");
        assert!(at(1.999).intensity < 0.01);
        assert!(at(0.5).intensity > at(1.5).intensity);
        let mut s = 0.0;
        while s < 2.0 {
            let f = at(s);
            assert!(f.angle.is_finite() && f.length.is_finite() && f.length > 0.0);
            assert!((0.0..=1.0).contains(&f.intensity));
            s += 0.01;
        }
    }

    #[test]
    fn cross_flash_is_a_pure_function_of_time_so_nothing_outlives_the_burst() {
        let now = Instant::now();
        let b = Burst::new(now, &mut Rng(7));
        let secs = |ms: u64| {
            (now + Duration::from_millis(ms))
                .saturating_duration_since(b.started)
                .as_secs_f32()
        };
        assert!(cross_flash(secs(1000)).is_some());
        assert!(cross_flash(secs(2000)).is_none());
        assert!(cross_flash(secs(4000)).is_none());
        // The halo is rebuilt per frame from the viewport only; Geo has no cached state.
        let g = geo(1.0, 1.0);
        assert!(
            halo_layers(g.w.min(g.h) * 0.09)
                .iter()
                .all(|l| l.radius.is_finite())
        );
    }
}
