//! Gauge Gain Chime: a Shepard-bell arpeggio rendered from scratch for every Combo Transfer.
//!
//! `render_gain_chime(delta, level, seed)` is a pure function that returns stereo PCM. A small
//! combo is a short "シャラン"; a large combo is a fast, long "シャララララララン". The Gauge
//! level (0..=10) moves the spectral centre from 600 Hz to 2100 Hz, so a fuller Gauge sounds
//! brighter. Nothing here touches an audio device, a file or a lock: the caller renders on a
//! worker thread and only hands the finished buffer to the audio engine.
//!
//! Ported from the PO's Python prototype (`run_length`, `note_interval`, `center_freq`,
//! `shepard_note`, `gain_chime`). The prototype's `svf`, `reverb` and `master` bodies were not
//! available, so those three are re-derived from the described intent (see the notes on each).

use std::f64::consts::TAU;

use super::params::SFX;

/// C major pentatonic, in semitones above C.
pub const PENT: [i32; 5] = [0, 2, 4, 7, 9];
/// C2 in Hz: the base of the octave stack.
pub const BASE_C: f64 = 65.406;
pub const SAMPLE_RATE: u32 = SFX.sample_rate;

const MIN_NOTES: usize = 4;
const MAX_NOTES: usize = 56;
const MAX_COMPONENTS: usize = 8;
const MIN_COMPONENT_HZ: f64 = 110.0;
const MAX_COMPONENT_HZ: f64 = 9500.0;
const SIGMA_OCTAVES: f64 = 1.3;
const INHARMONIC_RATIO: f64 = 2.76;
const INHARMONIC_GAIN: f32 = 0.22;
const NOTE_ATTACK_SECS: f64 = 0.002;
/// Audible length of one arpeggio note; its decay constant gives about 200 ms of ring.
const NOTE_SECS: f64 = 0.32;
const NOTE_TAU_SECS: f64 = 0.075;
const NOTE_TAPER_SECS: f64 = 0.08;
const SHIMMER_CENTRE_HZ: f64 = 6500.0;
const SHIMMER_Q: f64 = 0.9;
const SHIMMER_GAIN: f32 = 0.09;
const LANDING_DELAY_SECS: f64 = 0.015;
const LANDING_SECS: f64 = 1.4;
const LANDING_GAIN: f32 = 0.55;
const REVERB_TAIL_SECS: f64 = 0.6;
const REVERB_RT60_SECS: f64 = 0.9;
/// Share of the mix that is reverb.
const REVERB_WET: f32 = 0.2;
const END_FADE_SECS: f64 = 0.15;
const PEAK_AT_MIN_COMBO: f32 = 0.30;
const PEAK_AT_MAX_COMBO: f32 = 0.55;
const MAX_DELTA: u32 = 999;
pub const MAX_LEVEL: u32 = 10;

/// Python-compatible rounding (ties to even), so note counts match the prototype exactly.
fn round_ties_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// Number of arpeggio notes for a combo of `delta`: 4 for 1, up to 56.
pub fn run_length(delta: u32) -> usize {
    if delta <= 1 {
        return MIN_NOTES;
    }
    let v = 4.0 + 3.2 * f64::from(delta - 1).powf(0.9);
    (round_ties_even(v) as usize).clamp(MIN_NOTES, MAX_NOTES)
}

/// Seconds between note starts; never faster than 22 ms.
pub fn note_interval(n: usize) -> f64 {
    (0.05 - 0.0085 * (n.max(1) as f64 / 4.0).log2()).max(0.022)
}

/// Spectral centre in Hz for a Gauge level 0..=10: 600 Hz at 0, about 1122 Hz at 5, 2100 Hz at 10.
pub fn center_freq(level: u32) -> f64 {
    600.0 * (2100.0_f64 / 600.0).powf(f64::from(level.min(MAX_LEVEL)) / 10.0)
}

/// Semitones above C of arpeggio note `i` of `n`; the last note (`i = n - 1`) is always C.
pub fn pitch_class(n: usize, i: usize) -> i32 {
    let start = (5 - ((n.max(1) - 1) % 5)) % 5;
    PENT[(start + i) % 5]
}

/// Octave stack of one Shepard note: up to 8 `(frequency, weight)` pairs inside
/// 110..=9500 Hz, weighted by a Gaussian (sigma 1.3 octaves) around `fc`. Weights sum to 1.
fn shepard_components(pc: i32, fc: f64) -> Vec<(f64, f32)> {
    let f0 = BASE_C * 2.0_f64.powf(f64::from(pc) / 12.0);
    let mut out: Vec<(f64, f64)> = (0..12)
        .map(|j| f0 * 2.0_f64.powi(j))
        .filter(|f| (MIN_COMPONENT_HZ..=MAX_COMPONENT_HZ).contains(f))
        .take(MAX_COMPONENTS)
        .map(|f| {
            let x = (f / fc).log2() / SIGMA_OCTAVES;
            (f, (-0.5 * x * x).exp())
        })
        .collect();
    let total: f64 = out.iter().map(|c| c.1).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    for c in &mut out {
        c.1 /= total;
    }
    out.into_iter().map(|(f, w)| (f, w as f32)).collect()
}

/// Deterministic generator (SplitMix64).
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u32) -> Self {
        Self(u64::from(seed) ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub(super) fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [-1, 1).
    pub(super) fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// Smooth 0 -> 1 over `attack` seconds; exactly 0 at `t = 0`.
fn rise(t: f64, attack: f64) -> f32 {
    if t >= attack {
        1.0
    } else {
        let x = (t / attack) as f32;
        x * x * (3.0 - 2.0 * x)
    }
}

/// Cosine fade-out over the last `taper` samples of a `len`-sample voice; 0 at the very end.
fn taper(k: usize, len: usize, taper: usize) -> f32 {
    let from = len.saturating_sub(taper);
    if k < from || taper == 0 {
        1.0
    } else {
        let x = (k - from) as f32 / taper as f32;
        0.5 * (1.0 + (std::f32::consts::PI * x).cos())
    }
}

/// Constant-power pan, `pan` in -1..=1.
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (a.cos(), a.sin())
}

fn samples(secs: f64) -> usize {
    (secs * f64::from(SAMPLE_RATE)).round() as usize
}

fn add_bell(buf: &mut [[f32; 2]], start: usize, pc: i32, fc: f64, vel: f32, pan: f32) {
    let sr = f64::from(SAMPLE_RATE);
    let comps = shepard_components(pc, fc);
    let len = samples(NOTE_SECS);
    let tail = samples(NOTE_TAPER_SECS);
    let (gl, gr) = pan_gains(pan);
    for k in 0..len {
        let Some(frame) = buf.get_mut(start + k) else {
            break;
        };
        let t = k as f64 / sr;
        let a = rise(t, NOTE_ATTACK_SECS) * taper(k, len, tail);
        let main_env = a * (-t / NOTE_TAU_SECS).exp() as f32;
        let bell_env = a * (-t / (NOTE_TAU_SECS / 2.5)).exp() as f32;
        let mut s = 0.0_f32;
        for &(f, w) in &comps {
            let phase = TAU * f * t;
            s += w * (phase.sin() as f32) * main_env;
            if f * INHARMONIC_RATIO < sr * 0.45 {
                s += w * INHARMONIC_GAIN * ((phase * INHARMONIC_RATIO).sin() as f32) * bell_env;
            }
        }
        s *= vel * 0.5;
        frame[0] += s * gl;
        frame[1] += s * gr;
    }
}

/// TPT state-variable band-pass, the same topology as the Backspace voices in `synth`.
/// (The prototype's `svf` type and coefficients were not available; band-pass with a wide Q is
/// the assumption for the "6500 Hz" shimmer.)
struct BandPass {
    ic1: f32,
    ic2: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl BandPass {
    fn new(centre_hz: f64, q: f64) -> Self {
        let g = (std::f64::consts::PI * centre_hz / f64::from(SAMPLE_RATE)).tan() as f32;
        let k = (1.0 / q) as f32;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self {
            ic1: 0.0,
            ic2: 0.0,
            k,
            a1,
            a2,
            a3: g * a2,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v1 * self.k
    }
}

/// Soft "シャ": band-passed white noise (independent per channel) under the arpeggio.
fn add_shimmer(buf: &mut [[f32; 2]], arpeggio_secs: f64, rng: &mut Rng) {
    let sr = f64::from(SAMPLE_RATE);
    let len = samples(arpeggio_secs + NOTE_SECS).min(buf.len());
    let tail = samples(0.06);
    let tau = 0.45 * (arpeggio_secs + NOTE_SECS);
    let mut filters = [
        BandPass::new(SHIMMER_CENTRE_HZ, SHIMMER_Q),
        BandPass::new(SHIMMER_CENTRE_HZ, SHIMMER_Q),
    ];
    for (k, frame) in buf.iter_mut().enumerate().take(len) {
        let t = k as f64 / sr;
        let env = rise(t, 0.010) * (-t / tau).exp() as f32 * taper(k, len, tail);
        for (ch, filter) in filters.iter_mut().enumerate() {
            frame[ch] += filter.process(rng.signed()) * env * SHIMMER_GAIN;
        }
    }
}

/// The "ラン": tonic, fifth and octave (plus a faint third harmonic) of the landing pitch
/// `261.63 * 2^round(log2(fc / 261.63))`, starting 15 ms after the last note.
fn add_landing(buf: &mut [[f32; 2]], start: usize, fc: f64, rng: &mut Rng) {
    let sr = f64::from(SAMPLE_RATE);
    let ft = 261.63 * 2.0_f64.powf((fc / 261.63).log2().round());
    let len = samples(LANDING_SECS);
    let tail = samples(0.3);
    for (ratio, gain) in [(1.0, 1.0_f32), (1.5, 0.55), (2.0, 0.6), (3.0, 0.2)] {
        let (gl, gr) = pan_gains(0.15 * rng.signed());
        let f = ft * ratio;
        for k in 0..len {
            let Some(frame) = buf.get_mut(start + k) else {
                break;
            };
            let t = k as f64 / sr;
            let env = rise(t, 0.004) * (-t / 0.4).exp() as f32 * taper(k, len, tail);
            let s = (TAU * f * t).sin() as f32 * env * gain * LANDING_GAIN * 0.5;
            frame[0] += s * gl;
            frame[1] += s * gr;
        }
    }
}

struct Comb {
    line: Vec<f32>,
    pos: usize,
    feedback: f32,
    damping: f32,
    lowpass: f32,
}

impl Comb {
    fn new(delay_secs: f64) -> Self {
        let len = samples(delay_secs).max(1);
        // Loop gain that decays by 60 dB in RT60.
        let feedback = 10.0_f64.powf(-3.0 * delay_secs / REVERB_RT60_SECS) as f32;
        Self {
            line: vec![0.0; len],
            pos: 0,
            feedback,
            damping: 0.25,
            lowpass: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.line[self.pos];
        self.lowpass = y * (1.0 - self.damping) + self.lowpass * self.damping;
        self.line[self.pos] = x + self.lowpass * self.feedback;
        self.pos = (self.pos + 1) % self.line.len();
        y
    }
}

struct AllPass {
    line: Vec<f32>,
    pos: usize,
}

impl AllPass {
    fn new(delay_secs: f64) -> Self {
        Self {
            line: vec![0.0; samples(delay_secs).max(1)],
            pos: 0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.line[self.pos];
        self.line[self.pos] = x + y * 0.5;
        self.pos = (self.pos + 1) % self.line.len();
        y - x
    }
}

/// Schroeder reverb (six damped combs into two all-passes per channel, RT60 0.9 s), mixed so
/// the reverb carries about 20% of the signal. The prototype's `reverb()` was not available,
/// so the topology and the wet calibration (reverb RMS = wet/dry of the dry RMS) are mine.
fn add_reverb(buf: &mut [[f32; 2]]) {
    const COMBS: [f64; 6] = [0.0353, 0.0367, 0.0338, 0.0322, 0.0297, 0.0277];
    let mut wet = vec![[0.0_f32; 2]; buf.len()];
    for ch in 0..2 {
        let spread = if ch == 0 { 0.0 } else { 0.0005 };
        let mut combs: Vec<Comb> = COMBS.iter().map(|d| Comb::new(d + spread)).collect();
        let mut allpass = [AllPass::new(0.005 + spread), AllPass::new(0.0017 + spread)];
        for (frame, out) in buf.iter().zip(wet.iter_mut()) {
            let mut y: f32 = combs.iter_mut().map(|c| c.process(frame[ch])).sum();
            for a in &mut allpass {
                y = a.process(y);
            }
            out[ch] = y;
        }
    }
    let rms = |v: &[[f32; 2]]| {
        let sum: f64 = v
            .iter()
            .map(|f| f64::from(f[0]).powi(2) + f64::from(f[1]).powi(2))
            .sum();
        (sum / (2.0 * v.len().max(1) as f64)).sqrt() as f32
    };
    let (dry_rms, wet_rms) = (rms(buf), rms(&wet));
    if wet_rms <= 1e-9 {
        return;
    }
    let wet_gain = dry_rms / wet_rms * REVERB_WET / (1.0 - REVERB_WET);
    for (frame, w) in buf.iter_mut().zip(&wet) {
        frame[0] += w[0] * wet_gain;
        frame[1] += w[1] * wet_gain;
    }
}

/// Peak a chime is normalised to: 0.30 for a single token up to 0.55 at the largest combo.
pub fn target_peak(delta: u32) -> f32 {
    let x = f64::from(delta.clamp(1, MAX_DELTA)).ln() / f64::from(MAX_DELTA).ln();
    PEAK_AT_MIN_COMBO + (PEAK_AT_MAX_COMBO - PEAK_AT_MIN_COMBO) * x as f32
}

/// Peak-normalises to [`target_peak`] and fades the last 150 ms to silence (no end click).
fn master(buf: &mut [[f32; 2]], delta: u32) {
    let peak = buf
        .iter()
        .flat_map(|f| f.iter())
        .fold(0.0_f32, |m, s| m.max(s.abs()));
    let gain = if peak.is_finite() && peak > 1e-9 {
        target_peak(delta) / peak
    } else {
        0.0
    };
    let fade = samples(END_FADE_SECS).min(buf.len());
    let len = buf.len();
    for (k, frame) in buf.iter_mut().enumerate() {
        let g = gain * taper(k, len, fade);
        frame[0] *= g;
        frame[1] *= g;
    }
}

/// The one timeline shared by the sound and by everything that must line up with it (Gauge
/// fill, Transfer particles). The synthesiser lays its notes out from this struct and the
/// visuals read their deadlines from it, so the two can never drift apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChimeTimeline {
    pub note_count: usize,
    /// Seconds between note starts.
    pub note_interval: f64,
    /// Start of the last arpeggio note: `(n - 1) * dt`.
    pub last_note_start: f64,
    /// Start of the landing bell ("ラン！"): `last_note_start + 15 ms`.
    pub landing: f64,
    /// Length of the rendered buffer including the landing ring and the reverb tail.
    pub total: f64,
}

impl ChimeTimeline {
    /// Timeline of the chime for a transferred combo of `delta` (clamped to 1..=999).
    pub fn for_delta(delta: u32) -> Self {
        let note_count = run_length(delta.clamp(1, MAX_DELTA));
        let note_interval = note_interval(note_count);
        let last_note_start = (note_count - 1) as f64 * note_interval;
        let landing = last_note_start + LANDING_DELAY_SECS;
        Self {
            note_count,
            note_interval,
            last_note_start,
            landing,
            total: landing + LANDING_SECS + REVERB_TAIL_SECS,
        }
    }

    /// Start of note `i` in seconds.
    pub fn note_start(&self, i: usize) -> f64 {
        i as f64 * self.note_interval
    }

    pub fn landing_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.landing)
    }
}

/// Renders the chime as stereo frames at [`SAMPLE_RATE`].
///
/// * `delta`: the combo that was transferred (clamped to 1..=999),
/// * `level`: Gauge value before the transfer divided by 10 (clamped to 0..=10),
/// * `seed`: per-event seed for the pan and noise variation.
///
/// Every sample is finite, the peak is [`target_peak`] at most, and the first and last frames
/// are silent.
pub fn render_gain_chime(delta: u32, level: u32, seed: u32) -> Vec<[f32; 2]> {
    let delta = delta.clamp(1, MAX_DELTA);
    let timeline = ChimeTimeline::for_delta(delta);
    let n = timeline.note_count;
    let fc = center_freq(level);
    let mut rng = Rng::new(seed);
    let mut buf = vec![[0.0_f32; 2]; samples(timeline.total)];
    for i in 0..n {
        let q = i as f32 / (n - 1).max(1) as f32;
        let vel = 0.45 + 0.55 * q.powf(0.8);
        let pan = -0.55 + 1.1 * q + 0.12 * rng.signed();
        add_bell(
            &mut buf,
            samples(timeline.note_start(i)),
            pitch_class(n, i),
            fc,
            vel,
            pan,
        );
    }
    add_shimmer(&mut buf, timeline.last_note_start, &mut rng);
    add_landing(&mut buf, samples(timeline.landing), fc, &mut rng);
    add_reverb(&mut buf);
    master(&mut buf, delta);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(buf: &[[f32; 2]]) -> f32 {
        buf.iter()
            .flat_map(|f| f.iter())
            .fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn note_count_is_bounded_monotone_and_matches_the_prototype() {
        assert_eq!(run_length(1), 4);
        assert_eq!(run_length(0), 4);
        let mut prev = 0;
        for d in 1..=MAX_DELTA {
            let n = run_length(d);
            assert!((4..=56).contains(&n), "{d}: {n}");
            assert!(n >= prev, "{d}");
            prev = n;
        }
        assert_eq!(run_length(MAX_DELTA), 56);
        // Python: round(4 + 3.2 * 1 ** 0.9) = round(7.2) = 7; round(4 + 3.2 * 2 ** 0.9) = 10.
        assert_eq!(run_length(2), 7);
        assert_eq!(run_length(3), 10);
        // Ties go to even, as Python's round() does.
        assert_eq!(round_ties_even(6.5), 6.0);
        assert_eq!(round_ties_even(7.5), 8.0);
    }

    #[test]
    fn interval_never_drops_below_22_ms_and_speeds_up_with_the_run() {
        assert!((note_interval(4) - 0.05).abs() < 1e-12);
        for n in 1..=200 {
            assert!(note_interval(n) >= 0.022, "{n}");
        }
        assert!(note_interval(56) < note_interval(7));
        assert_eq!(note_interval(56), 0.022, "clamped");
    }

    #[test]
    fn spectral_centre_follows_the_eleven_gauge_levels() {
        assert!((center_freq(0) - 600.0).abs() < 1e-9);
        assert!((center_freq(10) - 2100.0).abs() < 1e-9);
        assert!((center_freq(5) - 1122.5).abs() < 1.0, "{}", center_freq(5));
        assert_eq!(center_freq(99), center_freq(10));
        for l in 0..10 {
            assert!(center_freq(l) < center_freq(l + 1));
        }
    }

    #[test]
    fn the_last_note_is_always_the_tonic_for_every_run_length() {
        for n in 1..=MAX_NOTES {
            assert_eq!(pitch_class(n, n - 1), 0, "n = {n}");
            for i in 0..n {
                assert!(PENT.contains(&pitch_class(n, i)));
            }
        }
    }

    #[test]
    fn shepard_stack_is_bounded_in_count_range_and_weight() {
        for pc in PENT {
            for level in 0..=10 {
                let comps = shepard_components(pc, center_freq(level));
                assert!((1..=MAX_COMPONENTS).contains(&comps.len()));
                let sum: f32 = comps.iter().map(|c| c.1).sum();
                assert!((sum - 1.0).abs() < 1e-4);
                for (f, w) in comps {
                    assert!((110.0..=9500.0).contains(&f) && w.is_finite() && w >= 0.0);
                }
            }
        }
    }

    #[test]
    fn rendering_is_finite_limited_click_free_and_reproducible() {
        for (delta, level) in [(1, 0), (8, 2), (15, 5), (12, 9), (20, 10), (999, 10)] {
            let a = render_gain_chime(delta, level, 7);
            assert!(a.iter().flatten().all(|s| s.is_finite()), "{delta}/{level}");
            let limit = target_peak(delta);
            assert!(peak(&a) <= limit + 1e-6, "{delta}/{level}: {}", peak(&a));
            assert!(peak(&a) > limit * 0.99, "normalised to the target");
            assert_eq!(a[0], [0.0, 0.0], "silent start");
            assert!(
                a.last().unwrap().iter().all(|s| s.abs() < 1e-6),
                "silent end"
            );
            // No sudden step anywhere at the end: the last 150 ms fade smoothly.
            let tail = &a[a.len() - samples(END_FADE_SECS)..];
            let max_step = tail
                .windows(2)
                .map(|w| (w[1][0] - w[0][0]).abs())
                .fold(0.0, f32::max);
            assert!(max_step < 0.05, "{max_step}");
        }
        assert_eq!(render_gain_chime(8, 2, 42), render_gain_chime(8, 2, 42));
        assert_ne!(render_gain_chime(8, 2, 42), render_gain_chime(8, 2, 43));
    }

    #[test]
    fn bigger_combos_play_longer_and_louder_within_the_limits() {
        let small = render_gain_chime(1, 3, 1);
        let medium = render_gain_chime(20, 3, 1);
        let large = render_gain_chime(999, 3, 1);
        assert!(small.len() < medium.len() && medium.len() < large.len());
        assert_eq!(small.len(), samples(ChimeTimeline::for_delta(1).total));
        assert!((peak(&small) - 0.30).abs() < 0.01);
        assert!((peak(&large) - 0.55).abs() < 0.01);
        assert!(target_peak(1) < target_peak(20) && target_peak(20) < target_peak(999));
        // Out-of-range arguments are clamped, never panic.
        assert_eq!(render_gain_chime(0, 99, 1).len(), small.len());
    }

    #[test]
    fn higher_gauge_level_gives_a_brighter_sound() {
        // Share of energy above 2 kHz, measured by differencing (a crude high-pass).
        let brightness = |level| {
            let b = render_gain_chime(12, level, 3);
            let hi: f64 = b
                .windows(2)
                .map(|w| f64::from(w[1][0] - w[0][0]).powi(2))
                .sum();
            let all: f64 = b.iter().map(|f| f64::from(f[0]).powi(2)).sum();
            hi / all
        };
        assert!(brightness(10) > brightness(0));
    }
}
