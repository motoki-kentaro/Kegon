//! Procedural "electric shatter" sounds.
//!
//! Each voice is a burst of noise through a swept band-pass filter with a sharp attack and an
//! exponential decay, optionally with a few sparse crackle clicks, inharmonic ringing partials and a
//! short body tone. The buffers are rendered once, when audio starts, and played from memory; nothing
//! here runs inside the audio callback.

use std::f32::consts::TAU;

use super::params::{SFX, VoiceSpec};

/// The synthesised voices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    Zap,
    Crack,
    Burst,
}

impl Voice {
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::Zap, Self::Crack, Self::Burst];

    pub fn spec(self) -> &'static VoiceSpec {
        match self {
            Self::Zap => &super::params::ZAP,
            Self::Crack => &super::params::CRACK,
            Self::Burst => &super::params::BURST,
        }
    }
}

struct Noise(u32);

impl Noise {
    /// Uniform in -1..1.
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() + 1.0) * 0.5
    }
}

/// Smooth 0 → 1 rise over `attack` seconds: the first sample is exactly silent.
fn rise(t: f32, attack: f32) -> f32 {
    if t >= attack {
        1.0
    } else {
        let x = t / attack;
        x * x * (3.0 - 2.0 * x)
    }
}

/// Renders one voice as mono samples at `SFX.sample_rate`, peak-normalised to `SFX.peak`.
/// `seed` selects the noise, so different seeds are variations of the same sound.
pub fn render(spec: &VoiceSpec, seed: u32) -> Vec<f32> {
    let sr = SFX.sample_rate as f32;
    let n = (spec.length_ms * 0.001 * sr) as usize;
    let attack = spec.attack_ms * 0.001;
    let decay = spec.decay_ms * 0.001;
    let mut noise = Noise(seed.wrapping_mul(2_654_435_761).max(1));

    // Crackle clicks: sparse impulses fed into the filter, louder first.
    let mut impulses = vec![0.0f32; n];
    for i in 0..spec.clicks {
        let at = (noise.unit() * spec.click_span_ms * 0.001 * sr) as usize;
        let level =
            (1.0 - i as f32 / (spec.clicks.max(1) as f32 + 1.0)) * (0.5 + 0.5 * noise.unit());
        if at + 1 < n {
            impulses[at] += 6.0 * level;
            impulses[at + 1] -= 3.5 * level;
        }
    }

    // Band-pass (TPT state-variable), centre swept exponentially from start to end frequency.
    let (c0, c1) = spec.centre_hz;
    let k = 1.0 / spec.q.max(0.2);
    let (mut ic1, mut ic2) = (0.0f32, 0.0f32);
    let mut shaped = Vec::with_capacity(n);
    for (i, impulse) in impulses.iter().enumerate() {
        let t = i as f32 / sr;
        let progress = i as f32 / n.max(1) as f32;
        let centre = c0 * (c1 / c0).powf(progress);
        let g = (std::f32::consts::PI * centre.min(sr * 0.45) / sr).tan();
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let x = noise.next() + impulse;
        let v3 = x - ic2;
        let v1 = a1 * ic1 + a2 * v3;
        let v2 = ic2 + a2 * ic1 + a3 * v3;
        ic1 = 2.0 * v1 - ic1;
        ic2 = 2.0 * v2 - ic2;
        let env = rise(t, attack) * (-(t - attack).max(0.0) / decay).exp();
        shaped.push(v1 * k * env);
    }
    normalise(&mut shaped, 1.0);

    // Ringing partials and the body tone, added at their own gain relative to the noise.
    for (i, sample) in shaped.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let r = rise(t, attack);
        for (j, hz) in spec.partials.iter().enumerate() {
            let falloff = 1.0 / (1.0 + j as f32 * 0.6);
            *sample += spec.partial_gain
                * falloff
                * (TAU * hz * t + j as f32).sin()
                * r
                * (-t / (spec.partial_decay_ms * 0.001)).exp();
        }
        if spec.body_hz > 0.0 {
            *sample += spec.body_gain
                * (TAU * spec.body_hz * t).sin()
                * r
                * (-t / (spec.body_decay_ms * 0.001)).exp();
        }
    }

    // Two cascaded one-pole high-pass filters remove DC and the bass.
    let rc = 1.0 / (TAU * SFX.low_cut_hz);
    let a = rc / (rc + 1.0 / sr);
    for _ in 0..2 {
        let (mut prev_in, mut prev_out) = (0.0f32, 0.0f32);
        for sample in shaped.iter_mut() {
            let out = a * (prev_out + *sample - prev_in);
            prev_in = *sample;
            prev_out = out;
            *sample = out;
        }
    }

    // Release: the sound ends on silence.
    let release = ((SFX.release_ms * 0.001 * sr) as usize).clamp(1, n);
    for j in 0..release {
        let x = j as f32 / release as f32;
        let gain = 0.5 * (1.0 + (std::f32::consts::PI * x).cos());
        shaped[n - release + j] *= gain;
    }
    if let Some(last) = shaped.last_mut() {
        *last = 0.0;
    }

    normalise(&mut shaped, SFX.peak);
    shaped
}

fn normalise(samples: &mut [f32], target: f32) {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 1e-9 {
        let k = target / peak;
        samples.iter_mut().for_each(|s| *s *= k);
    }
}

/// Numbers that describe a rendered sound, for checking it without listening.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub duration_ms: f32,
    pub peak: f32,
    pub rms: f32,
    pub centroid_hz: f32,
    /// Share of the spectral energy below 300 Hz.
    pub low_ratio: f32,
    /// Largest absolute sample in the first tenth of a millisecond (the attack itself rises over
    /// milliseconds, so the very start must be silent) and in the last millisecond.
    pub head_abs: f32,
    pub tail_abs: f32,
    pub dc: f32,
    /// Largest sample-to-sample jump anywhere, and within the last 4 ms.
    pub max_step: f32,
    pub tail_step: f32,
}

#[cfg(test)]
pub fn analyze(samples: &[f32], sample_rate: u32) -> Metrics {
    let sr = sample_rate as f32;
    let n = samples.len();
    let ms = (sr * 0.001) as usize;
    let max_abs = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let step = |s: &[f32]| s.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));

    let size = n.next_power_of_two().max(2);
    let mut re: Vec<f32> = samples
        .iter()
        .copied()
        .chain(std::iter::repeat(0.0))
        .take(size)
        .collect();
    let mut im = vec![0.0f32; size];
    fft(&mut re, &mut im);
    let (mut total, mut low, mut weighted) = (0.0f64, 0.0f64, 0.0f64);
    for bin in 1..size / 2 {
        let hz = bin as f32 * sr / size as f32;
        let power = (re[bin] * re[bin] + im[bin] * im[bin]) as f64;
        total += power;
        weighted += power * hz as f64;
        if hz < 300.0 {
            low += power;
        }
    }

    Metrics {
        duration_ms: n as f32 / sr * 1000.0,
        peak: max_abs(samples),
        rms: (samples.iter().map(|s| s * s).sum::<f32>() / n.max(1) as f32).sqrt(),
        centroid_hz: if total > 0.0 {
            (weighted / total) as f32
        } else {
            0.0
        },
        low_ratio: if total > 0.0 {
            (low / total) as f32
        } else {
            0.0
        },
        head_abs: max_abs(&samples[..(ms / 10).max(1).min(n)]),
        tail_abs: max_abs(&samples[n.saturating_sub(ms)..]),
        dc: samples.iter().sum::<f32>() / n.max(1) as f32,
        max_step: step(samples),
        tail_step: step(&samples[n.saturating_sub(ms * 4)..]),
    }
}

/// In-place radix-2 FFT; `re.len()` must be a power of two.
#[cfg(test)]
fn fft(re: &mut [f32], im: &mut [f32]) {
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
        let angle = -TAU / len as f32;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::dopamine::params::{BURST, CRACK, ZAP};

    /// 16-bit mono PCM WAV.
    fn wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        out
    }

    #[test]
    fn every_voice_is_a_clean_short_bright_burst() {
        for voice in Voice::ALL {
            let spec = voice.spec();
            for seed in 1..=SFX.variants as u32 {
                let samples = render(spec, seed);
                let m = analyze(&samples, SFX.sample_rate);
                let label = format!("{voice:?}#{seed}: {m:?}");
                assert!((m.duration_ms - spec.length_ms).abs() < 1.0, "{label}");
                assert!((0.7..=0.801).contains(&m.peak), "{label}");
                assert!(m.rms > 0.02 && m.rms < 0.4, "{label}");
                assert!((2000.0..8500.0).contains(&m.centroid_hz), "{label}");
                assert!(m.low_ratio < 0.06, "{label}");
                assert!(m.head_abs < 0.05, "starts quietly: {label}");
                assert!(m.tail_abs < 0.01, "ends quietly: {label}");
                assert!(m.dc.abs() < 0.01, "{label}");
                assert!(m.tail_step < 0.05, "no step at the end: {label}");
                assert!(samples.iter().all(|s| s.is_finite()), "{label}");
            }
        }
    }

    #[test]
    fn variations_differ_but_the_same_seed_repeats() {
        let a = render(&ZAP, 1);
        assert_eq!(a, render(&ZAP, 1));
        assert_ne!(a, render(&ZAP, 2));
        assert_ne!(render(&ZAP, 1), render(&CRACK, 1));
        assert_ne!(render(&CRACK, 1), render(&BURST, 1));
    }

    #[test]
    fn the_energy_dies_away_instead_of_staying_flat() {
        for voice in Voice::ALL {
            let s = render(voice.spec(), 3);
            let third = s.len() / 3;
            let energy = |part: &[f32]| part.iter().map(|x| x * x).sum::<f32>();
            assert!(
                energy(&s[..third]) > 4.0 * energy(&s[2 * third..]),
                "{voice:?}"
            );
        }
    }

    #[test]
    fn the_analysis_finds_a_known_tone() {
        let sr = SFX.sample_rate;
        let tone: Vec<f32> = (0..4800)
            .map(|i| (TAU * 1000.0 * i as f32 / sr as f32).sin() * 0.5)
            .collect();
        let m = analyze(&tone, sr);
        assert!((m.centroid_hz - 1000.0).abs() < 60.0, "{}", m.centroid_hz);
        assert!((m.peak - 0.5).abs() < 1e-3);
        assert!((m.rms - 0.3536).abs() < 0.01);
        assert!(m.low_ratio < 0.02);
        let low: Vec<f32> = tone
            .iter()
            .enumerate()
            .map(|(i, _)| (TAU * 100.0 * i as f32 / sr as f32).sin())
            .collect();
        assert!(analyze(&low, sr).low_ratio > 0.9);
    }

    /// Writes the listening WAVs: `cargo test --locked export_listening_wavs -- --ignored --nocapture`
    /// with `KEGON_SFX_WAV_DIR` set to a directory outside the repository.
    #[test]
    #[ignore = "writes files; run by hand"]
    fn export_listening_wavs() {
        let dir = std::path::PathBuf::from(
            std::env::var("KEGON_SFX_WAV_DIR")
                .expect("set KEGON_SFX_WAV_DIR to an output directory"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        for voice in Voice::ALL {
            let spec = voice.spec();
            let mut all = Vec::new();
            for seed in 1..=SFX.variants as u32 {
                let samples = render(spec, seed);
                let m = analyze(&samples, SFX.sample_rate);
                println!(
                    "{voice:?}#{seed}: {:.0} ms peak {:.3} rms {:.3} centroid {:.0} Hz low {:.4} head {:.4} tail {:.5} dc {:+.5} step {:.3}",
                    m.duration_ms,
                    m.peak,
                    m.rms,
                    m.centroid_hz,
                    m.low_ratio,
                    m.head_abs,
                    m.tail_abs,
                    m.dc,
                    m.max_step
                );
                std::fs::write(
                    dir.join(format!(
                        "{}_{seed}.wav",
                        format!("{voice:?}").to_lowercase()
                    )),
                    wav_bytes(&samples, SFX.sample_rate),
                )
                .unwrap();
                all.extend_from_slice(&samples);
                all.extend(std::iter::repeat_n(0.0, (SFX.sample_rate / 5) as usize));
            }
            // Variants one after another, as a quick audition.
            std::fs::write(
                dir.join(format!("{}_all.wav", format!("{voice:?}").to_lowercase())),
                wav_bytes(&all, SFX.sample_rate),
            )
            .unwrap();
        }
        // A rapid sequence of all three, like holding Backspace.
        let mut mash = vec![0.0f32; SFX.sample_rate as usize * 2];
        for i in 0..40usize {
            let v = render(Voice::ALL[i % 3].spec(), 1 + (i % SFX.variants) as u32);
            let at = i * (SFX.sample_rate as usize * 40 / 1000);
            for (j, s) in v.iter().enumerate() {
                if let Some(slot) = mash.get_mut(at + j) {
                    *slot += s * 0.6;
                }
            }
        }
        std::fs::write(dir.join("mash_30ms.wav"), wav_bytes(&mash, SFX.sample_rate)).unwrap();
    }
}
