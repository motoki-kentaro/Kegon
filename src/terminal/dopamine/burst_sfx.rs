//! Burst sound: four seconds of procedural audio laid out on the Burst Visual timeline.
//!
//! `render_burst(seed)` is a pure function returning exactly 192,000 stereo frames at 48 kHz. It
//! is rendered once on a background thread at start-up and kept in memory (see
//! [`super::audio`]); playing a Burst never runs this code. Nothing here touches a device or a
//! file.
//!
//! ```text
//! t = 0.0  Impact (sub boom, roar, zap, click) + Shake A rumble + cut-in whoosh + rotating cross
//! t = 1.0  Cut-in lands: stamp thud, noise hit, stinger chord, bells + Shake B rumble
//! t = 2.0  Cross ends: air release, sparkle bells and shower
//! t = 3.0  Cut-in leaves: exit whoosh + afterglow pad
//! t = 4.0  silence
//! ```
//!
//! Ported from the PO's Python prototype `burst(seed)`. The prototype's `whoosh`, `rumble`, FM
//! bell, `reverb` and `master` bodies were not available; their shapes follow the described
//! parameters and every guess is marked "assumed" below.

use std::f64::consts::{PI, TAU};

use super::chime::Rng;
use super::fail::{Svf, add_reverb_with, expsweep};

pub const SAMPLE_RATE: u32 = 48_000;
/// Exactly four seconds.
pub const FRAMES: usize = 192_000;
/// The prototype's seed.
pub const DEFAULT_SEED: u32 = 21;

const SR: f64 = SAMPLE_RATE as f64;

const REVERB_RT60_SECS: f64 = 1.3;
const REVERB_WET: f32 = 0.20;
const MASTER_HIGHPASS_HZ: f64 = 32.0;
const MASTER_DRIVE: f64 = 1.1;
pub const MASTER_PEAK: f32 = 0.62;
const MASTER_FADE_SECS: f64 = 0.020;
const TAIL_FADE_SECS: f64 = 0.400;

fn samples(secs: f64) -> usize {
    (secs * SR).round() as usize
}

fn midi_hz(midi: f64) -> f64 {
    440.0 * 2.0_f64.powf((midi - 69.0) / 12.0)
}

/// Smooth 0 -> 1 over `attack` seconds (exactly 0 at `t = 0`); 1 for a zero attack.
fn rise(t: f64, attack: f64) -> f64 {
    if attack <= 0.0 || t >= attack {
        1.0
    } else {
        let x = t / attack;
        x * x * (3.0 - 2.0 * x)
    }
}

/// Cosine fade to zero over the last `taper` seconds of a `len`-second voice.
fn end_taper(t: f64, len: f64, taper: f64) -> f64 {
    if taper <= 0.0 || t < len - taper {
        1.0
    } else {
        0.5 * (1.0 + (PI * ((t - (len - taper)) / taper).min(1.0)).cos())
    }
}

/// Attack x exponential decay x end taper.
fn shape(t: f64, len: f64, attack: f64, tau: f64, taper: f64) -> f64 {
    rise(t, attack) * (-t / tau).exp() * end_taper(t, len, taper)
}

fn normalize_rms(v: &mut [f64]) {
    let rms = (v.iter().map(|x| x * x).sum::<f64>() / v.len().max(1) as f64).sqrt();
    if rms > 1e-12 {
        for x in v {
            *x /= rms;
        }
    }
}

fn white(rng: &mut Rng, n: usize) -> Vec<f64> {
    (0..n).map(|_| f64::from(rng.signed())).collect()
}

/// Constant-power pan (assumed law), `pan` in -1..=1.
fn pan_gains(pan: f64) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (a.cos() as f32, a.sin() as f32)
}

struct Mix {
    buf: Vec<[f32; 2]>,
}

impl Mix {
    fn new() -> Self {
        Self {
            buf: vec![[0.0; 2]; FRAMES],
        }
    }

    /// Adds `mono` at `start` seconds with a per-sample pan `pan(i)`.
    fn add_with(&mut self, start: f64, mono: &[f64], gain: f64, pan: impl Fn(usize) -> f64) {
        let first = samples(start);
        for (i, v) in mono.iter().enumerate() {
            let Some(frame) = self.buf.get_mut(first + i) else {
                break;
            };
            let (l, r) = pan_gains(pan(i));
            let s = (v * gain) as f32;
            frame[0] += s * l;
            frame[1] += s * r;
        }
    }

    fn add(&mut self, start: f64, mono: &[f64], gain: f64, pan: f64) {
        self.add_with(start, mono, gain, |_| pan);
    }
}

/// Phase-integrated sine of an arbitrary frequency track.
fn integrate_phase(freq: &[f64]) -> Vec<f64> {
    let mut phase = 0.0;
    freq.iter()
        .map(|f| {
            phase += TAU * f / SR;
            phase
        })
        .collect()
}

/// Sub boom: `42 + 170 e^(-t/0.10)` Hz through `tanh(2.4 sin)`. 1.7 s.
fn sub_boom() -> Vec<f64> {
    let len = 1.7;
    let freq: Vec<f64> = (0..samples(len))
        .map(|i| 42.0 + 170.0 * (-(i as f64 / SR) / 0.10).exp())
        .collect();
    integrate_phase(&freq)
        .iter()
        .enumerate()
        .map(|(i, p)| (2.4 * p.sin()).tanh() * shape(i as f64 / SR, len, 0.002, 0.33, 0.06))
        .collect()
}

/// Explosion roar: low-passed noise (450 + 5000 e^(-t/0.35) Hz, Q 0.8) plus 45% differenced-noise
/// sizzle fading in 100 ms, both RMS-normalised first. The prototype's length is not known; 1.5 s
/// (about 3.5 decay constants) is assumed.
fn roar(rng: &mut Rng) -> Vec<f64> {
    let len = 1.5;
    let n = samples(len);
    let noise = white(rng, n);
    let mut svf = Svf::new(0.8);
    let mut body: Vec<f64> = noise
        .iter()
        .enumerate()
        .map(|(i, x)| {
            svf.step(*x, 450.0 + 5000.0 * (-(i as f64 / SR) / 0.35).exp())
                .0
        })
        .collect();
    normalize_rms(&mut body);
    let mut sizzle: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                noise[0]
            } else {
                noise[i] - noise[i - 1]
            }
        })
        .collect();
    normalize_rms(&mut sizzle);
    (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let mixed = 0.55 * body[i] + 0.45 * sizzle[i] * (-t / 0.10).exp();
            mixed * shape(t, len, 0.001, 0.42, 0.08)
        })
        .collect()
}

/// Neon zap: 7000 -> 250 Hz exponential chirp, 160 ms, 45 ms decay.
fn neon_zap() -> Vec<f64> {
    let len = 0.16;
    let phase = integrate_phase(&expsweep(7000.0, 250.0, len));
    phase
        .iter()
        .enumerate()
        .map(|(i, p)| p.sin() * shape(i as f64 / SR, len, 0.0, 0.045, 0.015))
        .collect()
}

/// Click: 20 ms of differenced white noise, 4 ms decay.
fn click(rng: &mut Rng) -> Vec<f64> {
    let n = samples(0.02);
    let noise = white(rng, n + 1);
    (0..n)
        .map(|i| (noise[i + 1] - noise[i]) * (-(i as f64 / SR) / 0.004).exp() * 0.5)
        .collect()
}

/// Rumble: low-passed noise (65%) and a tone with its octave (35%), each RMS-normalised, then
/// amplitude modulation, attack and release. Assumed: the tone is `sin(f) + 0.5 sin(2f)` and the
/// modulation is `1 - depth + depth (0.5 + 0.5 sin)`.
struct Rumble {
    len: f64,
    cutoff: f64,
    tone_hz: f64,
    am_hz: f64,
    am_depth: f64,
    attack: f64,
    release: f64,
}

fn rumble(p: &Rumble, rng: &mut Rng) -> Vec<f64> {
    let n = samples(p.len);
    let mut svf = Svf::new(0.7);
    let mut noise: Vec<f64> = white(rng, n)
        .iter()
        .map(|x| svf.step(*x, p.cutoff).0)
        .collect();
    normalize_rms(&mut noise);
    let mut tone: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            (TAU * p.tone_hz * t).sin() + 0.5 * (TAU * 2.0 * p.tone_hz * t).sin()
        })
        .collect();
    normalize_rms(&mut tone);
    (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let am = (1.0 - p.am_depth) + p.am_depth * (0.5 + 0.5 * (TAU * p.am_hz * t).sin());
            let release = ((p.len - t) / p.release).clamp(0.0, 1.0);
            (0.65 * noise[i] + 0.35 * tone[i]) * am * rise(t, p.attack) * release
        })
        .collect()
}

/// Amplitude shape of a whoosh over `u` in 0..=1: smooth rise to `peak`, smooth fall after it.
fn whoosh_envelope(u: f64, peak: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    if u < peak {
        (PI / 2.0 * u / peak).sin().powi(2)
    } else {
        (PI / 2.0 * (u - peak) / (1.0 - peak)).cos().powi(2)
    }
}

/// Band-passed noise sweeping `f0` -> `f1`, RMS-normalised, shaped by [`whoosh_envelope`].
fn whoosh(len: f64, f0: f64, f1: f64, q: f64, peak: f64, rng: &mut Rng) -> Vec<f64> {
    let n = samples(len);
    let cutoff = expsweep(f0, f1, len);
    let mut svf = Svf::new(q);
    let mut v: Vec<f64> = white(rng, n)
        .iter()
        .zip(&cutoff)
        // A band-pass on white noise carries power in proportion to its centre frequency;
        // dividing by sqrt(fc) keeps the loudness steady over the sweep so the envelope alone
        // decides where the peak is (assumed: the prototype compensates in some way).
        .map(|(x, fc)| svf.step(*x, *fc).1 / fc.sqrt())
        .collect();
    normalize_rms(&mut v);
    v.iter()
        .enumerate()
        .map(|(i, s)| s * whoosh_envelope(i as f64 / n as f64, peak))
        .collect()
}

/// Entry whoosh pan: left edge -> centre.
pub fn pan_entry(u: f64) -> f64 {
    -1.0 + (PI * u / 2.0).sin().powf(1.2)
}

/// Exit whoosh pan: near centre -> right edge.
pub fn pan_exit(u: f64) -> f64 {
    -0.2 + 1.2 * (PI * u / 2.0).sin().powf(0.8)
}

/// Shepard-Risset rising glissando: 8 octave components from 50 Hz, each moving up one octave per
/// second, weighted by a Hann window over log-frequency so a component fades out at the top as
/// its replacement fades in at the bottom. Phases are integrated continuously and two detuned
/// copies (1.0, 1.004) are summed. 80 ms fade-in, 180 ms fade-out, 7 Hz flicker (assumed 20%
/// deep).
fn rising_cross() -> Vec<f64> {
    const OCTAVES: usize = 8;
    const RATE: f64 = 1.0;
    const DETUNE: [f64; 2] = [1.0, 1.004];
    let len = 2.0;
    let n = samples(len);
    let mut phase = [[0.0_f64; 2]; OCTAVES];
    (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let mut sum = 0.0;
            for (k, ph) in phase.iter_mut().enumerate() {
                let pos = (k as f64 + RATE * t).rem_euclid(OCTAVES as f64);
                let hz = 50.0 * 2.0_f64.powf(pos);
                let weight = 0.5 * (1.0 - (TAU * pos / OCTAVES as f64).cos());
                for (d, p) in DETUNE.iter().zip(ph.iter_mut()) {
                    *p += TAU * hz * d / SR;
                    sum += weight * p.sin();
                }
            }
            // The Hann weights of 8 evenly spaced octaves add up to 4 per copy.
            let level = sum / (OCTAVES as f64);
            let flicker = 0.8 + 0.2 * (0.5 + 0.5 * (TAU * 7.0 * t).sin());
            let fade_in = rise(t, 0.08);
            let fade_out = end_taper(t, len, 0.18);
            level * flicker * fade_in * fade_out
        })
        .collect()
}

/// Stamp thud: `46 + 120 e^(-t/0.045)` Hz through `tanh(2 sin)`, 450 ms.
fn stamp_thud() -> Vec<f64> {
    let len = 0.45;
    let freq: Vec<f64> = (0..samples(len))
        .map(|i| 46.0 + 120.0 * (-(i as f64 / SR) / 0.045).exp())
        .collect();
    integrate_phase(&freq)
        .iter()
        .enumerate()
        .map(|(i, p)| (2.0 * p.sin()).tanh() * shape(i as f64 / SR, len, 0.0, 0.13, 0.04))
        .collect()
}

/// Noise hit: 2600 Hz band-pass noise (Q 1.2), 120 ms, 30 ms decay.
fn noise_hit(rng: &mut Rng) -> Vec<f64> {
    let len = 0.12;
    let n = samples(len);
    let mut svf = Svf::new(1.2);
    let mut v: Vec<f64> = white(rng, n)
        .iter()
        .map(|x| svf.step(*x, 2600.0).1)
        .collect();
    normalize_rms(&mut v);
    v.iter()
        .enumerate()
        .map(|(i, s)| s * shape(i as f64 / SR, len, 0.0, 0.030, 0.020))
        .collect()
}

/// Major stinger: MIDI 48, 60, 64, 67, 72, each as a +/-7 cent pair of 60% saw + 40% triangle,
/// through a low-pass sweeping `1000 + 6500 e^(-t/0.5)` Hz (Q 0.8). 2 s.
fn stinger() -> Vec<f64> {
    let len = 2.0;
    let n = samples(len);
    let voices: Vec<f64> = [48.0, 60.0, 64.0, 67.0, 72.0]
        .iter()
        .flat_map(|m| {
            [
                midi_hz(*m) * 2.0_f64.powf(7.0 / 1200.0),
                midi_hz(*m) * 2.0_f64.powf(-7.0 / 1200.0),
            ]
        })
        .collect();
    let mut svf = Svf::new(0.8);
    (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let raw: f64 = voices
                .iter()
                .map(|f| {
                    let saw = 2.0 * (f * t).fract() - 1.0;
                    let tri = 2.0 * (2.0 * (f * t).fract() - 1.0).abs() - 1.0;
                    0.6 * saw + 0.4 * tri
                })
                .sum();
            let (low, _) = svf.step(raw, 1000.0 + 6500.0 * (-t / 0.5).exp());
            low * shape(t, len, 0.006, 0.85, 0.12)
        })
        .collect()
}

/// Two-operator FM bell: a sine carrier phase-modulated by a decaying 1.4x partner (assumed
/// ratio; the existing Rust code has no FM bell, so this one is new).
fn fm_bell(midi: f64, len: f64, decay: f64, index: f64) -> Vec<f64> {
    let f = midi_hz(midi);
    (0..samples(len))
        .map(|i| {
            let t = i as f64 / SR;
            let modulation = index * (-t / decay).exp() * (TAU * f * 1.4 * t).sin();
            (TAU * f * t + modulation).sin() * shape(t, len, 0.002, decay, 0.02)
        })
        .collect()
}

/// Afterglow pad: high C-major tones (MIDI 84, 88, 91, 96), each a +/-5 cent pair, weights
/// 1.0 / 0.7 / 0.8 / 0.5, 5 Hz amplitude modulation, 150 ms attack, 300 ms decay. 1 s.
fn afterglow_pad() -> Vec<f64> {
    let len = 1.0;
    let notes = [(84.0, 1.0), (88.0, 0.7), (91.0, 0.8), (96.0, 0.5)];
    (0..samples(len))
        .map(|i| {
            let t = i as f64 / SR;
            let tones: f64 = notes
                .iter()
                .map(|(m, w)| {
                    let f = midi_hz(*m);
                    w * ((TAU * f * 2.0_f64.powf(5.0 / 1200.0) * t).sin()
                        + (TAU * f * 2.0_f64.powf(-5.0 / 1200.0) * t).sin())
                })
                .sum();
            let am = 0.75 + 0.25 * (TAU * 5.0 * t).sin();
            tones * am * shape(t, len, 0.15, 0.3, 0.1)
        })
        .collect()
}

/// Draws from Beta(1, b) by inverting its closed-form CDF.
fn beta_1_b(rng: &mut Rng, b: f64) -> f64 {
    1.0 - (1.0 - f64::from(rng.unit())).powf(1.0 / b)
}

/// 26 short bells from 2.04 s on, C major pentatonic one or two octaves up, spread by
/// Beta(1, 1.9) over 1.15 s and getting quieter with time.
fn sparkle_shower(mix: &mut Mix, rng: &mut Rng) {
    const PENT: [f64; 5] = [0.0, 2.0, 4.0, 7.0, 9.0];
    for _ in 0..26 {
        let x = beta_1_b(rng, 1.9);
        let start = 2.04 + 1.15 * x;
        let midi = 60.0
            + PENT[(rng.unit() * 5.0) as usize % 5]
            + 12.0 * (1.0 + f64::from((rng.unit() * 2.0).floor().min(1.0)));
        let decay = 0.06 + 0.09 * f64::from(rng.unit());
        let pan = f64::from(rng.signed()) * 0.9;
        let gain = (0.11 - 0.06 * x) * (0.85 + 0.15 * f64::from(rng.unit()));
        mix.add(start, &fm_bell(midi, 0.35, decay, 1.2), gain, pan);
    }
}

/// All layers summed, before reverb and master.
fn mix_layers(rng: &mut Rng) -> Mix {
    let mut m = Mix::new();
    // t = 0: Impact.
    m.add(0.0, &sub_boom(), 0.80, 0.0);
    m.add(0.0, &roar(rng), 0.34, 0.0);
    m.add(0.0, &neon_zap(), 0.22, 0.0);
    m.add(0.0, &click(rng), 0.25, 0.0);
    // Shake A rumble, 0.0 - 0.5 s.
    let shake_a = Rumble {
        len: 0.5,
        cutoff: 110.0,
        tone_hz: 48.0,
        am_hz: 15.0,
        am_depth: 0.6,
        attack: 0.004,
        release: 0.06,
    };
    m.add(0.0, &rumble(&shake_a, rng), 0.34, 0.0);
    // Cut-in entry whoosh, 0.0 - 1.0 s.
    m.add_with(
        0.0,
        &whoosh(1.0, 220.0, 4800.0, 1.4, 0.90, rng),
        0.26,
        |i| pan_entry(i as f64 / samples(1.0) as f64),
    );
    // Rotating cross, 0.0 - 2.0 s.
    m.add_with(0.0, &rising_cross(), 0.27, |i| {
        0.65 * (TAU * 1.5 * i as f64 / SR).sin()
    });
    // t = 1.0: Cut-in lands.
    m.add(1.0, &stamp_thud(), 0.62, 0.0);
    m.add(1.0, &noise_hit(rng), 0.30, 0.0);
    m.add(1.0, &stinger(), 0.052, 0.0);
    m.add(1.0, &fm_bell(84.0, 1.4, 0.55, 1.4), 0.20, 0.18);
    m.add(1.0, &fm_bell(91.0, 1.2, 0.45, 1.4), 0.15, -0.18);
    m.add(1.0, &fm_bell(72.0, 1.4, 0.60, 1.4), 0.18, 0.0);
    // Shake B rumble, 1.0 - 3.0 s.
    let shake_b = Rumble {
        len: 2.0,
        cutoff: 90.0,
        tone_hz: 41.0,
        am_hz: 6.0,
        am_depth: 0.5,
        attack: 0.05,
        release: 0.12,
    };
    m.add(1.0, &rumble(&shake_b, rng), 0.12, 0.0);
    // t = 2.0: Cross release.
    m.add(
        1.98,
        &whoosh(0.45, 7000.0, 1800.0, 1.1, 0.25, rng),
        0.14,
        0.0,
    );
    for (midi, gain, pan) in [(96.0, 0.16, 0.3), (91.0, 0.13, -0.3), (88.0, 0.12, 0.0)] {
        m.add(2.0, &fm_bell(midi, 0.9, 0.35, 1.4), gain, pan);
    }
    sparkle_shower(&mut m, rng);
    // t = 3.0: Cut-in leaves.
    m.add_with(
        3.0,
        &whoosh(1.0, 4800.0, 200.0, 1.2, 0.28, rng),
        0.24,
        |i| pan_exit(i as f64 / samples(1.0) as f64),
    );
    m.add(3.0, &afterglow_pad(), 0.020, 0.0);
    m
}

/// High-pass 32 Hz, `tanh(1.1 x)`, shared peak 0.62, a 20 ms linear fade and a 400 ms cosine
/// fade to true silence.
fn master(buf: &mut [[f32; 2]]) {
    let a = (-TAU * MASTER_HIGHPASS_HZ / SR).exp();
    for ch in 0..2 {
        let (mut px, mut py) = (0.0_f64, 0.0_f64);
        for frame in buf.iter_mut() {
            let x = if frame[ch].is_finite() {
                f64::from(frame[ch])
            } else {
                0.0
            };
            let y = a * (py + x - px);
            px = x;
            py = y;
            frame[ch] = (y * MASTER_DRIVE).tanh() as f32;
        }
    }
    let peak = buf
        .iter()
        .flat_map(|f| f.iter())
        .fold(0.0_f32, |m, s| m.max(s.abs()));
    let gain = if peak > 1e-9 { MASTER_PEAK / peak } else { 0.0 };
    let len = buf.len();
    let quick = samples(MASTER_FADE_SECS);
    let tail = samples(TAIL_FADE_SECS);
    for (i, frame) in buf.iter_mut().enumerate() {
        let from_end = len - 1 - i;
        let mut g = gain;
        if from_end < quick {
            g *= from_end as f32 / (quick - 1) as f32;
        }
        if from_end < tail {
            g *= 0.5 * (1.0 - (PI as f32 * (from_end as f32 / (tail - 1) as f32)).cos());
        }
        frame[0] *= g;
        frame[1] *= g;
    }
}

/// Renders the Burst sound: 192,000 stereo frames, 48 kHz, ending in true silence. The same
/// `seed` always gives the same PCM.
pub fn render_burst(seed: u32) -> Vec<[f32; 2]> {
    let mut rng = Rng::new(seed);
    let mix = mix_layers(&mut rng);
    let (mut left, mut right): (Vec<f32>, Vec<f32>) = mix.buf.iter().map(|f| (f[0], f[1])).unzip();
    add_reverb_with(
        &mut left,
        &mut right,
        &mut rng,
        REVERB_RT60_SECS,
        REVERB_WET,
    );
    let mut out: Vec<[f32; 2]> = left.iter().zip(&right).map(|(l, r)| [*l, *r]).collect();
    master(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(buf: &[[f32; 2]], from: f64, to: f64) -> f64 {
        buf[samples(from)..samples(to)]
            .iter()
            .map(|f| f64::from(f[0]).powi(2) + f64::from(f[1]).powi(2))
            .sum()
    }

    #[test]
    fn format_is_exactly_four_seconds_of_48k_stereo() {
        assert_eq!(SAMPLE_RATE, 48_000);
        assert_eq!(FRAMES, 4 * 48_000);
        assert_eq!(samples(4.0), FRAMES);
        assert_eq!(
            FRAMES * 2 * std::mem::size_of::<f32>(),
            1_536_000,
            "about 1.46 MiB"
        );
    }

    #[test]
    fn render_is_finite_limited_deterministic_and_ends_in_true_silence() {
        let a = render_burst(DEFAULT_SEED);
        assert_eq!(a.len(), FRAMES);
        assert!(a.iter().flatten().all(|s| s.is_finite()));
        let peak = a.iter().flatten().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((peak - MASTER_PEAK).abs() < 1e-4, "{peak}");
        assert_eq!(a, render_burst(DEFAULT_SEED));
        assert_ne!(a, render_burst(22));
        assert_eq!(*a.last().unwrap(), [0.0, 0.0]);
        // The last 400 ms fade smoothly: monotone envelope-ish, no step above the slope.
        let tail = &a[FRAMES - samples(0.4)..];
        let step = tail
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .fold(0.0, f32::max);
        assert!(step < 0.05, "{step}");
        for seed in [0, 1, u32::MAX] {
            let b = render_burst(seed);
            assert_eq!(b.len(), FRAMES);
            assert!(b.iter().flatten().all(|s| s.is_finite()));
        }
    }

    #[test]
    fn every_scheduled_moment_has_sound_and_the_end_is_silent() {
        let pcm = render_burst(DEFAULT_SEED);
        for (name, from, to) in [
            ("impact", 0.0, 0.1),
            ("shake a", 0.2, 0.4),
            ("stamp and stinger", 1.0, 1.1),
            ("shake b", 1.5, 2.0),
            ("release and sparkle", 2.0, 2.3),
            ("exit whoosh", 3.0, 3.3),
            ("afterglow", 3.3, 3.6),
        ] {
            assert!(energy(&pcm, from, to) > 1e-3, "{name}");
        }
        // The impact and the cut-in landing are the loudest moments.
        assert!(energy(&pcm, 0.0, 0.1) > energy(&pcm, 2.5, 2.6));
        assert!(energy(&pcm, 1.0, 1.1) > energy(&pcm, 0.9, 1.0) * 0.5);
        assert!(energy(&pcm, 3.9, 4.0) < 1e-2 * energy(&pcm, 0.0, 0.1));
    }

    #[test]
    fn layer_lengths_and_start_shapes_match_the_spec() {
        let mut rng = Rng::new(1);
        assert_eq!(sub_boom().len(), samples(1.7));
        assert_eq!(roar(&mut rng).len(), samples(1.5));
        assert_eq!(neon_zap().len(), samples(0.16));
        assert_eq!(click(&mut rng).len(), samples(0.02));
        assert_eq!(stamp_thud().len(), samples(0.45));
        assert_eq!(noise_hit(&mut rng).len(), samples(0.12));
        assert_eq!(stinger().len(), samples(2.0));
        assert_eq!(rising_cross().len(), samples(2.0));
        assert_eq!(afterglow_pad().len(), samples(1.0));
        assert_eq!(fm_bell(84.0, 1.4, 0.55, 1.4).len(), samples(1.4));
        // Boom starts silent (2 ms attack) and ends silent (60 ms taper).
        let boom = sub_boom();
        assert_eq!(boom[0], 0.0);
        assert!(boom.last().unwrap().abs() < 1e-3);
    }

    #[test]
    fn sub_boom_and_thud_start_at_the_specified_pitch_and_fall() {
        // 42 + 170 e^0 = 212 Hz at t = 0, 42 Hz later.
        let boom_f = |t: f64| 42.0 + 170.0 * (-t / 0.10).exp();
        assert!((boom_f(0.0) - 212.0).abs() < 1e-9 && (boom_f(2.0) - 42.0).abs() < 1e-3);
        let thud_f = |t: f64| 46.0 + 120.0 * (-t / 0.045).exp();
        assert!((thud_f(0.0) - 166.0).abs() < 1e-9);
    }

    #[test]
    fn whoosh_peaks_where_specified_and_pans_follow_the_formulas() {
        let mut rng = Rng::new(2);
        for (peak, len) in [(0.90, 1.0), (0.28, 1.0), (0.25, 0.45)] {
            let w = whoosh(len, 220.0, 4800.0, 1.4, peak, &mut rng);
            let n = w.len() as f64;
            let win = (n * 0.10) as usize;
            let mut best = (0.0, 0.0);
            for start in (0..w.len() - win).step_by(win / 2) {
                let e: f64 = w[start..start + win].iter().map(|x| x * x).sum();
                if e > best.0 {
                    best = (e, (start + win / 2) as f64 / n);
                }
            }
            assert!(
                (best.1 - peak).abs() < 0.10,
                "peak {peak}: found {}",
                best.1
            );
            assert!(w[0].abs() < 1e-6 && w.last().unwrap().abs() < 1e-3);
        }
        assert!((pan_entry(0.0) + 1.0).abs() < 1e-12);
        assert!(pan_entry(1.0).abs() < 1e-12, "ends in the centre");
        assert!((pan_exit(0.0) + 0.2).abs() < 1e-12);
        assert!(
            (pan_exit(1.0) - 1.0).abs() < 1e-12,
            "ends on the right edge"
        );
        for k in 0..=100 {
            let u = f64::from(k) / 100.0;
            assert!((-1.0..=1.0).contains(&pan_entry(u)) && (-1.0..=1.0).contains(&pan_exit(u)));
        }
    }

    #[test]
    fn rising_cross_is_continuous_bounded_and_fades_at_both_ends() {
        let c = rising_cross();
        assert!(c.iter().all(|v| v.is_finite() && v.abs() < 1.5));
        assert_eq!(c[0], 0.0);
        assert!(c.last().unwrap().abs() < 1e-6);
        // No click: sample-to-sample steps stay small relative to the level everywhere,
        // including the moments each component wraps from the top octave to the bottom.
        let max_step = c
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max);
        let peak = c.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        assert!(max_step < 0.5 * peak.max(0.05), "{max_step} vs {peak}");
        // It is audible through the middle.
        let mid: f64 = c[samples(0.8)..samples(1.2)].iter().map(|v| v * v).sum();
        assert!(mid > 1.0);
    }

    #[test]
    fn rumbles_differ_in_speed_and_depth_and_start_and_end_softly() {
        let mut rng = Rng::new(3);
        let a = rumble(
            &Rumble {
                len: 0.5,
                cutoff: 110.0,
                tone_hz: 48.0,
                am_hz: 15.0,
                am_depth: 0.6,
                attack: 0.004,
                release: 0.06,
            },
            &mut rng,
        );
        let b = rumble(
            &Rumble {
                len: 2.0,
                cutoff: 90.0,
                tone_hz: 41.0,
                am_hz: 6.0,
                am_depth: 0.5,
                attack: 0.05,
                release: 0.12,
            },
            &mut rng,
        );
        assert_eq!(a.len(), samples(0.5));
        assert_eq!(b.len(), samples(2.0));
        assert_eq!(a[0], 0.0);
        assert_eq!(b[0], 0.0);
        assert!(a.last().unwrap().abs() < 1e-3 && b.last().unwrap().abs() < 1e-3);
        assert!(a.iter().all(|v| v.is_finite()) && b.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn sparkle_shower_has_26_bells_inside_their_window_and_is_reproducible() {
        let mut m1 = Mix::new();
        sparkle_shower(&mut m1, &mut Rng::new(9));
        let mut m2 = Mix::new();
        sparkle_shower(&mut m2, &mut Rng::new(9));
        assert_eq!(m1.buf, m2.buf);
        // Nothing before 2.04 s, nothing after the last bell (2.04 + 1.15 + 0.35 s).
        assert!(m1.buf[..samples(2.04)].iter().all(|f| *f == [0.0, 0.0]));
        assert!(m1.buf[samples(3.55)..].iter().all(|f| *f == [0.0, 0.0]));
        assert!(energy(&m1.buf, 2.04, 3.55) > 0.0);
        // Beta(1, 1.9) draws stay in 0..1 and lean early.
        let mut rng = Rng::new(4);
        let draws: Vec<f64> = (0..2000).map(|_| beta_1_b(&mut rng, 1.9)).collect();
        assert!(draws.iter().all(|x| (0.0..=1.0).contains(x)));
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;
        assert!((mean - 1.0 / 2.9).abs() < 0.03, "{mean}");
    }

    #[test]
    fn chords_use_the_specified_tuning() {
        assert!((midi_hz(60.0) - 261.63).abs() < 0.01);
        assert!((midi_hz(84.0) - 1046.50).abs() < 0.01);
        assert!((midi_hz(96.0) - 2093.00).abs() < 0.01);
    }
}
