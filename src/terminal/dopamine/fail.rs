//! Command-failure sound: "グシャッ！ ヒュウウウン……ガガガ", rendered from scratch.
//!
//! `render_fail(seed)` is a pure function returning 1.3 s of 48 kHz stereo PCM. Four layers
//! (a crunch of noise, a falling saw, a dissonant triangle cluster and a glitch stutter that
//! repeats the first 100 ms through a sample-and-hold) go through a short noise reverb and a
//! master stage. Nothing here touches a device or a file; the caller renders on a worker
//! thread.
//!
//! Ported from the PO's Python prototype (`expsweep`, the Chamberlin `svf`, `fail`). The
//! prototype's reverb and master were described but not available as code, so the parts that
//! the description leaves open are noted where they are decided.

use std::f64::consts::PI;

use super::chime::Rng;

pub const SAMPLE_RATE: u32 = 48_000;
/// Total length: 1.3 s.
pub const LENGTH_SAMPLES: usize = 62_400;

const SR: f64 = SAMPLE_RATE as f64;

const REVERB_RT60_SECS: f64 = 0.5;
const REVERB_WET: f32 = 0.12;
const REVERB_PREDELAY_SECS: f64 = 0.012;
const MASTER_HIGHPASS_HZ: f64 = 120.0;
const MASTER_DRIVE: f64 = 1.1;
pub const MASTER_PEAK: f32 = 0.45;
const MASTER_FADE_SECS: f64 = 0.12;

const GLITCH_SOURCE_SECS: f64 = 0.10;
const GLITCH_HOLD: usize = 14;
/// (start in seconds, gain) of the three stutter repeats; the gain already includes the 0.6.
const GLITCH_REPEATS: [(f64, f32); 3] = [(0.14, 0.9 * 0.6), (0.23, 0.6 * 0.6), (0.30, 0.35 * 0.6)];

fn samples(secs: f64) -> usize {
    (SR * secs) as usize
}

/// Frequency of each sample of a `secs`-long exponential sweep from `f0` to `f1`.
pub(super) fn expsweep(f0: f64, f1: f64, secs: f64) -> Vec<f64> {
    (0..samples(secs))
        .map(|i| f0 * (f1 / f0).powf((i as f64 / SR) / secs))
        .collect()
}

/// Chamberlin state-variable filter, as in the prototype:
/// `f = 2 sin(pi min(fc, 0.22 SR) / SR); low += f band; high = x - low - band / q; band += f high`.
/// Every layer owns its own state.
pub(super) struct Svf {
    low: f64,
    band: f64,
    inv_q: f64,
}

impl Svf {
    pub(super) fn new(q: f64) -> Self {
        Self {
            low: 0.0,
            band: 0.0,
            inv_q: 1.0 / q,
        }
    }

    /// Advances one sample and returns `(low, band)`. Non-finite state is reset to silence, so a
    /// numerical blow-up can never leave the filter.
    pub(super) fn step(&mut self, x: f64, cutoff_hz: f64) -> (f64, f64) {
        let f = 2.0 * (PI * cutoff_hz.min(SR * 0.22) / SR).sin();
        self.low += f * self.band;
        let high = x - self.low - self.band * self.inv_q;
        self.band += f * high;
        if !(self.low.is_finite() && self.band.is_finite()) {
            self.low = 0.0;
            self.band = 0.0;
        }
        (self.low, self.band)
    }
}

/// Layer A: white noise through a band-pass sweeping 4200 -> 1200 Hz (Q 2), decaying in 50 ms.
fn crunch(rng: &mut Rng) -> Vec<f64> {
    let d = 0.14;
    let mut svf = Svf::new(2.0);
    expsweep(4200.0, 1200.0, d)
        .iter()
        .enumerate()
        .map(|(i, &fc)| {
            let (_, band) = svf.step(f64::from(rng.signed()), fc);
            band * (-(i as f64 / SR) / 0.05).exp() * 0.35
        })
        .collect()
}

/// Layer B: a saw falling 700 -> 90 Hz through a low-pass sweeping 3000 -> 300 Hz (Q 0.9).
fn falling_saw() -> Vec<f64> {
    let d = 0.70;
    let freq = expsweep(700.0, 90.0, d);
    let cutoff = expsweep(3000.0, 300.0, d);
    let mut svf = Svf::new(0.9);
    let mut phase = 0.0_f64;
    (0..freq.len())
        .map(|i| {
            phase += freq[i] / SR;
            let saw = 2.0 * phase.fract() - 1.0;
            let (low, _) = svf.step(saw, cutoff[i]);
            low * (-(i as f64 / SR) / 0.35).exp() * 0.22
        })
        .collect()
}

/// Layer C: C4 + C#4 + F#4 triangles (minor second and tritone) through a low-pass
/// sweeping 2500 -> 500 Hz (Q 0.8), decaying in 400 ms. Starts at 0.04 s.
fn dissonant_cluster() -> Vec<f64> {
    let d = 1.0;
    let notes = [60, 61, 66].map(|m: i32| 440.0 * 2.0_f64.powf(f64::from(m - 69) / 12.0));
    let cutoff = expsweep(2500.0, 500.0, d);
    let mut svf = Svf::new(0.8);
    (0..cutoff.len())
        .map(|i| {
            let t = i as f64 / SR;
            let tri: f64 = notes
                .iter()
                .map(|f| 2.0 * (2.0 * (f * t).fract() - 1.0).abs() - 1.0)
                .sum();
            let (low, _) = svf.step(tri, cutoff[i]);
            low * (-t / 0.40).exp() * 0.08
        })
        .collect()
}

/// The three layers mixed, centred, at full length and before any glitch, reverb or master.
fn mix_layers(rng: &mut Rng) -> Vec<f32> {
    let mut mix = vec![0.0_f32; LENGTH_SAMPLES];
    let mut add = |layer: Vec<f64>, start: usize| {
        for (i, v) in layer.into_iter().enumerate() {
            if let Some(slot) = mix.get_mut(start + i) {
                *slot += v as f32;
            }
        }
    };
    add(crunch(rng), 0);
    add(falling_saw(), 0);
    add(dissonant_cluster(), samples(0.04));
    mix
}

/// Layer D: the first 100 ms of the layer mix, held every 14 samples (sample-and-hold), laid
/// over 0.14 / 0.23 / 0.30 s. Every repeat is cut from the untouched original, never from an
/// earlier repeat.
fn add_glitch_stutter(buf: &mut [f32]) {
    let source = &buf[..samples(GLITCH_SOURCE_SECS)];
    let held: Vec<f32> = (0..source.len())
        .map(|i| source[i / GLITCH_HOLD * GLITCH_HOLD])
        .collect();
    for (start, gain) in GLITCH_REPEATS {
        let start = samples(start);
        for (i, v) in held.iter().enumerate() {
            if let Some(slot) = buf.get_mut(start + i) {
                *slot += v * gain;
            }
        }
    }
}

/// In-place iterative radix-2 FFT over `re`/`im` (length a power of two).
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = 2.0 * PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0_f64, 0.0_f64);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
    if inverse {
        let scale = 1.0 / n as f64;
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r *= scale;
            *i *= scale;
        }
    }
}

/// `x * h`, truncated to `x.len()`.
fn convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
    let size = (x.len() + h.len()).next_power_of_two();
    let load = |v: &[f32]| {
        let mut re = vec![0.0_f64; size];
        for (slot, s) in re.iter_mut().zip(v) {
            *slot = f64::from(*s);
        }
        (re, vec![0.0_f64; size])
    };
    let (mut xr, mut xi) = load(x);
    let (mut hr, mut hi) = load(h);
    fft(&mut xr, &mut xi, false);
    fft(&mut hr, &mut hi, false);
    for i in 0..size {
        let (r, im) = (xr[i] * hr[i] - xi[i] * hi[i], xr[i] * hi[i] + xi[i] * hr[i]);
        xr[i] = r;
        xi[i] = im;
    }
    fft(&mut xr, &mut xi, true);
    xr[..x.len()].iter().map(|v| *v as f32).collect()
}

/// Gaussian noise, from two uniform draws (Box-Muller).
fn gaussian(rng: &mut Rng) -> f64 {
    let u1 = f64::from(rng.unit()).max(1e-9);
    let u2 = f64::from(rng.unit());
    (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
}

/// Decaying-noise impulse response (60 dB in RT60), differenced to remove low frequencies and
/// scaled to unit energy. An independent response per channel gives the stereo width.
fn reverb_impulse(rng: &mut Rng, rt60: f64) -> Vec<f32> {
    let n = samples(rt60);
    let noise: Vec<f64> = (0..n)
        .map(|i| gaussian(rng) * (-6.907_755 * (i as f64 / SR) / rt60).exp())
        .collect();
    let mut ir: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                noise[0]
            } else {
                noise[i] - noise[i - 1]
            }
        })
        .collect();
    let energy = ir.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-12);
    for v in &mut ir {
        *v /= energy;
    }
    ir.into_iter().map(|v| v as f32).collect()
}

/// Reverb: noise IR convolved by FFT, 12 ms pre-delay, 12% wet added to the dry signal. The
/// prototype's IR normalisation was not available; unit energy keeps the wet level close to the
/// dry level before the 0.12 gain.
fn add_reverb(left: &mut [f32], right: &mut [f32], rng: &mut Rng) {
    add_reverb_with(left, right, rng, REVERB_RT60_SECS, REVERB_WET);
}

/// [`add_reverb`] with its own decay time and wet amount (also used by the Burst sound).
pub(super) fn add_reverb_with(
    left: &mut [f32],
    right: &mut [f32],
    rng: &mut Rng,
    rt60: f64,
    wet_amount: f32,
) {
    let predelay = samples(REVERB_PREDELAY_SECS);
    for ch in [left, right] {
        let ir = reverb_impulse(rng, rt60);
        let wet = convolve(ch, &ir);
        for (i, w) in wet.iter().enumerate() {
            if let Some(slot) = ch.get_mut(i + predelay) {
                *slot += w * wet_amount;
            }
        }
    }
}

/// Renders the failure sound: 1.3 s, 48 kHz, stereo. The same `seed` always gives the same PCM.
pub fn render_fail(seed: u32) -> Vec<[f32; 2]> {
    let mut rng = Rng::new(seed);
    let mut left = mix_layers(&mut rng);
    add_glitch_stutter(&mut left);
    let mut right = left.clone();
    add_reverb(&mut left, &mut right, &mut rng);
    // Both channels share the peak so the stereo image is not skewed by the normalisation.
    let mut joined: Vec<f32> = left.iter().chain(right.iter()).copied().collect();
    master_stereo(&mut joined);
    let (l, r) = joined.split_at(LENGTH_SAMPLES);
    l.iter().zip(r).map(|(l, r)| [*l, *r]).collect()
}

/// Master: 120 Hz one-pole high-pass, `tanh(1.1 x)`, one shared peak of 0.45 over both
/// channels (two concatenated), then a linear 120 ms fade to exactly zero. Non-finite samples
/// become silence.
fn master_stereo(joined: &mut [f32]) {
    let (l, r) = joined.split_at_mut(LENGTH_SAMPLES);
    for ch in [&mut *l, &mut *r] {
        let a = (-2.0 * PI * MASTER_HIGHPASS_HZ / SR).exp();
        let (mut prev_x, mut prev_y) = (0.0_f64, 0.0_f64);
        for s in ch.iter_mut() {
            let x = if s.is_finite() { f64::from(*s) } else { 0.0 };
            let y = a * (prev_y + x - prev_x);
            prev_x = x;
            prev_y = y;
            *s = (y * MASTER_DRIVE).tanh() as f32;
        }
    }
    let peak = l
        .iter()
        .chain(r.iter())
        .fold(0.0_f32, |m, s| m.max(s.abs()));
    let gain = if peak > 1e-9 { MASTER_PEAK / peak } else { 0.0 };
    let fade = samples(MASTER_FADE_SECS);
    let from = LENGTH_SAMPLES - fade;
    for ch in [l, r] {
        for (i, s) in ch.iter_mut().enumerate() {
            let f = if i < from {
                1.0
            } else {
                (LENGTH_SAMPLES - 1 - i) as f32 / (fade - 1) as f32
            };
            *s *= gain * f;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expsweep_runs_exponentially_from_start_towards_end() {
        let s = expsweep(700.0, 90.0, 0.70);
        assert_eq!(s.len(), samples(0.70));
        assert_eq!(s[0], 700.0);
        assert!(s.windows(2).all(|w| w[1] < w[0]));
        // Geometric: the midpoint is the geometric mean.
        let mid = s[s.len() / 2];
        assert!((mid - (700.0_f64 * 90.0).sqrt()).abs() < 1.0, "{mid}");
        assert!(*s.last().unwrap() > 90.0);
    }

    #[test]
    fn svf_band_and_low_pass_behave_and_stay_finite() {
        let run = |hz: f64, low: bool| {
            let mut svf = Svf::new(2.0);
            let mut energy = 0.0;
            for i in 0..48_000 {
                let x = (2.0 * PI * hz * i as f64 / SR).sin();
                let (lo, band) = svf.step(x, 2000.0);
                if i > 4_800 {
                    let y = if low { lo } else { band };
                    energy += y * y;
                }
            }
            energy
        };
        assert!(run(2000.0, false) > 10.0 * run(200.0, false));
        assert!(run(200.0, true) > 10.0 * run(8000.0, true));
        // Hostile input and cutoffs never leave NaN or Inf behind.
        let mut svf = Svf::new(0.8);
        for i in 0..100_000 {
            let x = if i % 7 == 0 { 1e6 } else { -3e5 };
            let (lo, band) = svf.step(x, 1e9);
            assert!(lo.is_finite() && band.is_finite());
        }
    }

    #[test]
    fn layers_have_the_specified_lengths_and_start_times() {
        let mut rng = Rng::new(1);
        assert_eq!(crunch(&mut rng).len(), samples(0.14));
        assert_eq!(falling_saw().len(), samples(0.70));
        assert_eq!(dissonant_cluster().len(), samples(1.0));
        let mix = mix_layers(&mut Rng::new(1));
        assert_eq!(mix.len(), LENGTH_SAMPLES);
        assert_eq!(LENGTH_SAMPLES, samples(1.3));
        // The cluster (0.04 s + 1.0 s) is the longest layer: nothing is left after 1.04 s.
        assert!(mix[samples(1.04)..].iter().all(|v| *v == 0.0));
        assert!(mix[samples(1.0)..samples(1.04)].iter().any(|v| *v != 0.0));
    }

    #[test]
    fn dissonant_cluster_uses_the_three_specified_pitches() {
        let f = |m: i32| 440.0 * 2.0_f64.powf(f64::from(m - 69) / 12.0);
        assert!((f(60) - 261.63).abs() < 0.01);
        assert!((f(61) - 277.18).abs() < 0.01);
        assert!((f(66) - 369.99).abs() < 0.01);
    }

    #[test]
    fn glitch_repeats_come_from_the_original_with_a_14_sample_hold() {
        let mut buf = vec![0.0_f32; LENGTH_SAMPLES];
        for (i, v) in buf.iter_mut().enumerate().take(samples(0.10)) {
            *v = i as f32 + 1.0;
        }
        let original = buf.clone();
        add_glitch_stutter(&mut buf);
        let held = |k: usize| original[k / 14 * 14];
        // Untouched where nothing is added.
        assert_eq!(&buf[..4800], &original[..4800]);
        // First repeat (0.14 s, 0.54) alone until the second starts at 0.23 s.
        let s1 = samples(0.14);
        for k in [0, 1, 13, 14, 15, 1000, 4000] {
            assert!((buf[s1 + k] - 0.54 * held(k)).abs() < 1e-3, "{k}");
        }
        // Third repeat (0.30 s, 0.21) after the second ended: still the original, held, not a
        // re-sampled copy of an earlier repeat.
        let s3 = samples(0.30);
        for k in [1500, 2000, 3000, 4799] {
            assert!((buf[s3 + k] - 0.21 * held(k)).abs() < 1e-3, "{k}");
        }
        // The three repeats end inside the buffer: the last one finishes at 0.40 s.
        assert_eq!(s3 + 4800, samples(0.40));
        assert!(buf[samples(0.40)..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn fft_convolution_matches_direct_convolution() {
        let x: Vec<f32> = (0..50).map(|i| ((i * 7) % 11) as f32 - 5.0).collect();
        let h: Vec<f32> = (0..9).map(|i| 1.0 / (i as f32 + 1.0)).collect();
        let fast = convolve(&x, &h);
        for n in 0..x.len() {
            let direct: f32 = (0..h.len())
                .filter(|k| *k <= n)
                .map(|k| h[k] * x[n - k])
                .sum();
            assert!((fast[n] - direct).abs() < 1e-3, "{n}");
        }
    }

    #[test]
    fn reverb_response_decays_and_has_unit_energy_without_low_content() {
        let ir = reverb_impulse(&mut Rng::new(3), REVERB_RT60_SECS);
        assert_eq!(ir.len(), samples(0.5));
        let energy: f32 = ir.iter().map(|v| v * v).sum();
        assert!((energy - 1.0).abs() < 1e-3);
        let early: f32 = ir[..2400].iter().map(|v| v * v).sum();
        let late: f32 = ir[ir.len() - 2400..].iter().map(|v| v * v).sum();
        assert!(early > 100.0 * late, "decays");
        let dc: f32 = ir.iter().sum();
        assert!(dc.abs() < 0.2, "differencing removes the low end: {dc}");
    }

    #[test]
    fn fail_is_finite_limited_stereo_and_ends_in_silence() {
        for seed in [0, 1, 42, u32::MAX] {
            let pcm = render_fail(seed);
            assert_eq!(pcm.len(), LENGTH_SAMPLES);
            assert!(pcm.iter().flatten().all(|s| s.is_finite()));
            let peak = pcm.iter().flatten().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!((peak - MASTER_PEAK).abs() < 1e-4, "{seed}: {peak}");
            assert_eq!(*pcm.last().unwrap(), [0.0, 0.0]);
            // The last 120 ms fade out smoothly: no step bigger than the fade slope allows.
            let tail = &pcm[LENGTH_SAMPLES - samples(0.12)..];
            let step = tail
                .windows(2)
                .map(|w| (w[1][0] - w[0][0]).abs())
                .fold(0.0, f32::max);
            assert!(step < 0.05, "{step}");
        }
        // Left and right differ (independent reverb), the dry part is centred.
        let pcm = render_fail(7);
        assert!(pcm.iter().any(|f| (f[0] - f[1]).abs() > 1e-4));
    }

    #[test]
    fn fail_is_reproducible_per_seed_and_varies_between_seeds() {
        assert_eq!(render_fail(5), render_fail(5));
        assert_ne!(render_fail(5), render_fail(6));
    }
}
