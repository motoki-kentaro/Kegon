//! Sound for Dopamine Mode, played with Kira from buffers held in memory.
//!
//! The voices are synthesised once at start (see [`super::synth`]) and played from memory.
//! Without an audio device nothing is played and nothing fails.

use std::sync::Arc;
use std::time::{Duration, Instant};

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Frame, Tween};

use super::params::SFX;
use super::synth::{self, Voice};

/// Sound profile choices for Dopamine Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DopamineSoundKind {
    /// Light electric snap, synthesised ("バチッ").
    #[default]
    Zap,
    /// Metallic shatter, synthesised ("シャリッ").
    Crack,
    /// Heavier neon burst, synthesised ("ビシュッ").
    Burst,
}

impl DopamineSoundKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zap => "zap",
            Self::Crack => "crack",
            Self::Burst => "burst",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "zap" => Some(Self::Zap),
            "crack" => Some(Self::Crack),
            "burst" => Some(Self::Burst),
            _ => None,
        }
    }

    fn slot(self) -> usize {
        self as usize
    }

    #[cfg(test)]
    const COUNT: usize = 3;
}

/// The variations of one profile.
type Bank = Vec<StaticSoundData>;

/// A synthesised buffer as a Kira sound.
fn synth_sound(samples: &[f32]) -> StaticSoundData {
    let frames: Arc<[Frame]> = samples.iter().map(|&s| Frame::from_mono(s)).collect();
    StaticSoundData {
        sample_rate: SFX.sample_rate,
        frames,
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

/// Every profile's buffers, indexed by [`DopamineSoundKind::slot`].
fn load_banks() -> Vec<Bank> {
    let synthesised = |voice: Voice| -> Bank {
        (1..=SFX.variants as u32)
            .map(|seed| synth_sound(&synth::render(voice.spec(), seed)))
            .collect()
    };
    vec![
        synthesised(Voice::Zap),
        synthesised(Voice::Crack),
        synthesised(Voice::Burst),
    ]
}

/// Controller managing SFX playback for Dopamine Mode.
pub struct DopamineAudio {
    manager: Option<AudioManager<DefaultBackend>>,
    banks: Vec<Bank>,
    current_kind: DopamineSoundKind,
    last_play: Option<Instant>,
    play_count: usize,
    rng: u32,
    last_variant: usize,
    /// Sounds that may still be playing, so they can be stopped at once.
    playing: Vec<StaticSoundHandle>,
}

impl DopamineAudio {
    /// Starts the audio engine and builds every sound buffer in memory.
    pub fn new() -> Self {
        Self {
            manager: AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).ok(),
            banks: load_banks(),
            current_kind: DopamineSoundKind::default(),
            last_play: None,
            play_count: 0,
            rng: 0x2545_F491,
            last_variant: usize::MAX,
            playing: Vec::new(),
        }
    }

    pub fn set_sound_kind(&mut self, kind: DopamineSoundKind) {
        self.current_kind = kind;
    }

    #[allow(dead_code)]
    pub fn sound_kind(&self) -> DopamineSoundKind {
        self.current_kind
    }

    fn next_random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Plays the erase sound with a little pitch and level variation, unless one started less than
    /// `SFX.min_interval_ms` ago.
    pub fn play_erase(&mut self) {
        self.play_erase_at(Instant::now());
    }

    pub fn play_erase_at(&mut self, now: Instant) {
        if self.last_play.is_some_and(|last| {
            now.duration_since(last) < Duration::from_millis(SFX.min_interval_ms)
        }) {
            return;
        }
        if self.manager.is_none() {
            return;
        }
        let bank_len = self.banks[self.current_kind.slot()].len();
        if bank_len == 0 {
            return;
        }
        // A different variation from the last one whenever there is a choice.
        let mut variant = (self.next_random() * bank_len as f32) as usize % bank_len;
        if bank_len > 1 && variant == self.last_variant {
            variant = (variant + 1) % bank_len;
        }
        self.last_variant = variant;
        let rate = SFX.rate_range.0
            + (SFX.rate_range.1 - SFX.rate_range.0) * f64::from(self.next_random());
        let volume = SFX.volume_db + (self.next_random() - 0.5) * 2.0 * SFX.volume_jitter_db;

        let mut sound = self.banks[self.current_kind.slot()][variant].clone();
        sound.settings = StaticSoundSettings::default()
            .playback_rate(rate)
            .volume(volume);

        // Forget finished sounds, and cut the oldest when too many overlap.
        self.playing
            .retain(|handle| handle.state() != kira::sound::PlaybackState::Stopped);
        while self.playing.len() >= SFX.max_voices {
            self.playing.remove(0).stop(Tween::default());
        }
        let Some(manager) = &mut self.manager else {
            return;
        };
        if let Ok(handle) = manager.play(sound) {
            self.playing.push(handle);
            self.last_play = Some(now);
            self.play_count = self.play_count.wrapping_add(1);
        }
    }

    /// Stops every sound that is still playing, immediately (a few milliseconds of fade to avoid a click).
    pub fn stop_all(&mut self) {
        let quick = Tween {
            duration: Duration::from_millis(4),
            ..Tween::default()
        };
        for mut handle in self.playing.drain(..) {
            handle.stop(quick);
        }
        self.last_play = None;
    }

    /// Whether sound output started and the current profile has something to play.
    #[allow(dead_code)]
    pub fn is_available(&self) -> bool {
        self.manager.is_some() && !self.banks[self.current_kind.slot()].is_empty()
    }

    /// Number of sounds that may still be playing.
    #[cfg(test)]
    fn playing_count(&self) -> usize {
        self.playing.len()
    }
}

impl Default for DopamineAudio {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent() -> DopamineAudio {
        DopamineAudio {
            manager: None,
            banks: load_banks(),
            current_kind: DopamineSoundKind::default(),
            last_play: None,
            play_count: 0,
            rng: 1,
            last_variant: usize::MAX,
            playing: Vec::new(),
        }
    }

    #[test]
    fn every_profile_holds_its_buffers_in_memory() {
        let banks = load_banks();
        assert_eq!(banks.len(), DopamineSoundKind::COUNT);
        for kind in [
            DopamineSoundKind::Zap,
            DopamineSoundKind::Crack,
            DopamineSoundKind::Burst,
        ] {
            assert_eq!(banks[kind.slot()].len(), SFX.variants, "{kind:?}");
            for sound in &banks[kind.slot()] {
                assert!(sound.num_frames() > 1000 && sound.duration() < Duration::from_millis(200));
            }
        }
    }

    #[test]
    fn names_round_trip_and_the_default_is_a_synthesised_voice() {
        for kind in [
            DopamineSoundKind::Zap,
            DopamineSoundKind::Crack,
            DopamineSoundKind::Burst,
        ] {
            assert_eq!(DopamineSoundKind::from_str(kind.as_str()), Some(kind));
        }
        assert_eq!(DopamineSoundKind::from_str("nope"), None);
        // Names of the recorded profiles that no longer exist.
        for removed in ["plasma", "arcade", "glass"] {
            assert_eq!(DopamineSoundKind::from_str(removed), None, "{removed}");
        }
        assert_eq!(DopamineSoundKind::default(), DopamineSoundKind::Zap);
    }

    #[test]
    fn sound_kind_switching() {
        let mut audio = silent();
        audio.set_sound_kind(DopamineSoundKind::Crack);
        assert_eq!(audio.sound_kind(), DopamineSoundKind::Crack);
        audio.set_sound_kind(DopamineSoundKind::Burst);
        assert_eq!(audio.sound_kind(), DopamineSoundKind::Burst);
    }

    #[test]
    fn without_an_output_device_nothing_happens_and_nothing_fails() {
        let mut audio = silent();
        assert!(!audio.is_available());
        audio.play_erase();
        audio.play_erase_at(Instant::now() + Duration::from_secs(1));
        audio.stop_all();
        assert_eq!(audio.playing_count(), 0);
        assert_eq!(audio.play_count, 0);
    }
}
