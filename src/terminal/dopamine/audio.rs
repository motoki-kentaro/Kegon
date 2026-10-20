//! Sound for Dopamine Mode, played with Kira from buffers held in memory.
//!
//! The voices are synthesised once at start (see [`super::synth`]) and played from memory.
//! Without an audio device nothing is played and nothing fails.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Frame, Tween};

use super::params::SFX;
use super::synth::{self, Voice};
use super::{burst_sfx, chime, fail};
use crate::workbench::TabId;

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

/// Chimes being rendered on worker threads at once; further requests are dropped.
const MAX_CHIME_JOBS: usize = 3;
/// Chimes that may sound at once; the oldest is cut (with a short fade) for a new one.
const MAX_CHIME_VOICES: usize = 3;
/// Failure sounds being rendered at once; further requests are dropped.
const MAX_FAIL_JOBS: usize = 2;
/// Failure sounds that may ring at once; the oldest is cut with a short fade.
const MAX_FAIL_VOICES: usize = 2;
const FAIL_VOLUME_DB: f32 = -2.0;

/// Burst sounds that may ring at once (one per Burst; Bursts are per tab and short).
const MAX_BURST_VOICES: usize = 2;
const BURST_VOLUME_DB: f32 = -2.0;
/// Fade used when a Burst is cancelled, so the cut has no click.
const BURST_CANCEL_FADE: Duration = Duration::from_millis(40);

/// One playing Burst sound, tied to the Burst instance that started it.
struct BurstVoice {
    tab: TabId,
    id: u64,
    /// `None` only in tests that run without a device.
    handle: Option<StaticSoundHandle>,
}

/// Chimes sit a little below the Backspace sounds so that both can overlap without clipping.
const CHIME_VOLUME_DB: f32 = -3.0;
/// Older chimes make room for a newer one.
const CHIME_DUCK_DB: f32 = -9.0;

/// A rendered chime on its way to the audio thread, tagged with the generation it was requested in.
struct ChimeBuffer {
    /// A failure sound rather than a Gain Chime.
    fail: bool,
    ticket: u64,
    generation: u32,
    frames: Vec<[f32; 2]>,
}

/// Decrements the in-flight job count when the worker ends, even if it panics.
struct JobGuard(Arc<AtomicUsize>);

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
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
    /// Gain Chimes that may still be playing.
    chimes: Vec<StaticSoundHandle>,
    /// Failure sounds that may still be playing.
    fails: Vec<StaticSoundHandle>,
    /// The 4 s Burst PCM, rendered once at start-up on a worker and then shared read-only.
    /// Empty until the worker finishes (or for ever if it fails): a Burst before that is silent.
    burst_pcm: Arc<OnceLock<Arc<[Frame]>>>,
    /// Tells a still-running start-up worker to drop its result when the audio is gone.
    burst_cancel: Arc<AtomicBool>,
    burst_voices: Vec<BurstVoice>,
    /// Burst sounds started, as `(tab, burst id)`, for tests.
    #[cfg(test)]
    burst_started: Vec<(TabId, u64)>,
    fail_jobs: Arc<AtomicUsize>,
    chime_tx: Sender<ChimeBuffer>,
    chime_rx: Receiver<ChimeBuffer>,
    chime_jobs: Arc<AtomicUsize>,
    /// Bumped by [`Self::stop_all`]; buffers rendered for an older generation are discarded.
    chime_generation: u32,
    /// Next request ticket; lets the caller learn when its chime actually started.
    chime_next_ticket: u64,
    /// Every accepted request as `(delta, level, seed)`, for tests.
    #[cfg(test)]
    chime_requests: Vec<(u32, u32, u32)>,
    /// Tests: hand out tickets without a device or a worker, completed by hand.
    #[cfg(test)]
    fake_tickets: bool,
    /// Seeds of the accepted failure-sound requests, for tests.
    #[cfg(test)]
    fail_requests: Vec<u32>,
}

impl DopamineAudio {
    /// Starts the audio engine and builds every sound buffer in memory.
    pub fn new() -> Self {
        let (chime_tx, chime_rx) = mpsc::channel();
        let audio = Self {
            // Unit tests never open a real output device, so they behave the same everywhere.
            manager: if cfg!(test) {
                None
            } else {
                AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).ok()
            },
            banks: load_banks(),
            current_kind: DopamineSoundKind::default(),
            last_play: None,
            play_count: 0,
            rng: 0x2545_F491,
            last_variant: usize::MAX,
            playing: Vec::new(),
            chimes: Vec::new(),
            fails: Vec::new(),
            burst_pcm: Arc::new(OnceLock::new()),
            burst_cancel: Arc::new(AtomicBool::new(false)),
            burst_voices: Vec::new(),
            #[cfg(test)]
            burst_started: Vec::new(),
            fail_jobs: Arc::new(AtomicUsize::new(0)),
            chime_tx,
            chime_rx,
            chime_jobs: Arc::new(AtomicUsize::new(0)),
            chime_generation: 0,
            chime_next_ticket: 0,
            #[cfg(test)]
            chime_requests: Vec::new(),
            #[cfg(test)]
            fake_tickets: false,
            #[cfg(test)]
            fail_requests: Vec::new(),
        };
        // Once per application start, off the UI thread.
        audio.start_burst_prerender();
        audio
    }

    /// Renders the Burst PCM once, on a background thread, and stores it for the life of the
    /// application. Never blocks the caller; without a device there is nothing to play, so it is
    /// skipped. A panicking or failing worker leaves the cache empty and Bursts silent.
    fn start_burst_prerender(&self) {
        if self.manager.is_none() {
            return;
        }
        let cell = Arc::clone(&self.burst_pcm);
        let cancel = Arc::clone(&self.burst_cancel);
        let _ = std::thread::Builder::new()
            .name("kegon-burst-sfx".into())
            .spawn(move || {
                let pcm = burst_sfx::render_burst(burst_sfx::DEFAULT_SEED);
                if cancel.load(Ordering::Acquire) {
                    return;
                }
                let frames: Arc<[Frame]> = pcm.iter().map(|f| Frame::new(f[0], f[1])).collect();
                let _ = cell.set(frames);
            });
    }

    /// Whether the Burst PCM is ready to play.
    #[allow(dead_code)]
    pub fn burst_pcm_ready(&self) -> bool {
        self.burst_pcm.get().is_some()
    }

    /// Starts the Burst sound for Burst `id` of `tab`. Silently skipped (returns `false`) when the
    /// PCM is not ready yet, there is no device, or playing fails: never waits, never renders,
    /// never delays the visuals. The 4 s PCM is shared, not copied.
    pub fn play_burst(&mut self, tab: TabId, id: u64) -> bool {
        let Some(frames) = self.burst_pcm.get().map(Arc::clone) else {
            return false;
        };
        self.burst_voices.retain(|v| {
            v.handle
                .as_ref()
                .is_none_or(|h| h.state() != kira::sound::PlaybackState::Stopped)
        });
        let handle = match &mut self.manager {
            Some(manager) => {
                let data = StaticSoundData {
                    sample_rate: burst_sfx::SAMPLE_RATE,
                    frames,
                    settings: StaticSoundSettings::default().volume(BURST_VOLUME_DB),
                    slice: None,
                };
                match manager.play(data) {
                    Ok(handle) => Some(handle),
                    Err(_) => return false,
                }
            }
            #[cfg(test)]
            None if self.fake_tickets => None,
            None => return false,
        };
        while self.burst_voices.len() >= MAX_BURST_VOICES {
            if let Some(mut old) = self.burst_voices.remove(0).handle {
                old.stop(Tween {
                    duration: BURST_CANCEL_FADE,
                    ..Tween::default()
                });
            }
        }
        self.burst_voices.push(BurstVoice { tab, id, handle });
        #[cfg(test)]
        self.burst_started.push((tab, id));
        true
    }

    /// Stops every Burst voice for which `keep(tab, id)` is false, with a short fade. The cached
    /// PCM is untouched.
    pub fn retain_burst_voices(&mut self, keep: impl Fn(TabId, u64) -> bool) {
        let fade = Tween {
            duration: BURST_CANCEL_FADE,
            ..Tween::default()
        };
        self.burst_voices.retain_mut(|v| {
            if keep(v.tab, v.id) {
                return true;
            }
            if let Some(handle) = &mut v.handle {
                handle.stop(fade);
            }
            false
        });
    }

    /// Burst voices that may still be playing.
    #[cfg(test)]
    pub fn burst_voice_count(&self) -> usize {
        self.burst_voices.len()
    }

    /// Burst sounds started so far as `(tab, burst id)`.
    #[cfg(test)]
    pub fn burst_started_log(&self) -> &[(TabId, u64)] {
        &self.burst_started
    }

    /// Tests: installs a Burst PCM without rendering it.
    #[cfg(test)]
    pub fn install_burst_pcm_for_test(&self) {
        let _ = self
            .burst_pcm
            .set(Arc::from(vec![Frame::from_mono(0.0); 16]));
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

    /// Asks for the Gain Chime of one Combo Transfer: `delta` is the transferred combo, `level`
    /// the gauge before the transfer divided by 10. The PCM is rendered on a short-lived worker
    /// thread, so this never blocks the UI and a missing device or a failed thread is harmless.
    ///
    /// Returns a ticket when a chime will really be rendered, `None` when no sound is coming
    /// (no device, too many jobs, thread failure). [`Self::pump_chimes`] later reports the ticket
    /// at the moment the sound is handed to the audio engine, which is the shared start instant
    /// for the visuals.
    pub fn request_gain_chime(&mut self, delta: u32, level: u32, seed: u32) -> Option<u64> {
        #[cfg(test)]
        self.chime_requests.push((delta, level, seed));
        #[cfg(test)]
        if self.fake_tickets {
            self.chime_next_ticket += 1;
            return Some(self.chime_next_ticket);
        }
        if self.manager.is_none() || self.chime_jobs.load(Ordering::Acquire) >= MAX_CHIME_JOBS {
            return None;
        }
        self.chime_next_ticket = self.chime_next_ticket.wrapping_add(1);
        let ticket = self.chime_next_ticket;
        self.chime_jobs.fetch_add(1, Ordering::AcqRel);
        let guard = JobGuard(Arc::clone(&self.chime_jobs));
        let tx = self.chime_tx.clone();
        let generation = self.chime_generation;
        let spawned = std::thread::Builder::new()
            .name("kegon-gain-chime".into())
            .spawn(move || {
                let _guard = guard;
                let frames = chime::render_gain_chime(delta, level, seed);
                let _ = tx.send(ChimeBuffer {
                    fail: false,
                    ticket,
                    generation,
                    frames,
                });
            });
        // On failure the closure (and its guard) is dropped, which releases the job slot.
        spawned.ok().map(|_| ticket)
    }

    /// Tests: from now on requests return tickets that [`Self::finish_fake_chime`] completes.
    #[cfg(test)]
    pub fn enable_fake_tickets(&mut self) {
        self.fake_tickets = true;
    }

    /// Tests: the worker for `ticket` finished rendering; the next pump starts it.
    #[cfg(test)]
    pub fn finish_fake_chime(&self, ticket: u64) {
        let _ = self.chime_tx.send(ChimeBuffer {
            fail: false,
            ticket,
            generation: self.chime_generation,
            frames: Vec::new(),
        });
    }

    /// Chime requests accepted so far, as `(delta, level, seed)`.
    #[cfg(test)]
    pub fn chime_request_log(&self) -> &[(u32, u32, u32)] {
        &self.chime_requests
    }

    /// Asks for the command-failure sound. Rendered on a worker thread like the Gain Chime, so
    /// the UI never waits; with no device, too many jobs or a failed thread nothing is played
    /// and nothing else is affected.
    pub fn request_fail_sound(&mut self, seed: u32) {
        #[cfg(test)]
        self.fail_requests.push(seed);
        if self.manager.is_none() || self.fail_jobs.load(Ordering::Acquire) >= MAX_FAIL_JOBS {
            return;
        }
        self.fail_jobs.fetch_add(1, Ordering::AcqRel);
        let guard = JobGuard(Arc::clone(&self.fail_jobs));
        let tx = self.chime_tx.clone();
        let generation = self.chime_generation;
        // On failure the closure (and its guard) is dropped, which releases the job slot.
        let _ = std::thread::Builder::new()
            .name("kegon-fail-sfx".into())
            .spawn(move || {
                let _guard = guard;
                let frames = fail::render_fail(seed);
                let _ = tx.send(ChimeBuffer {
                    fail: true,
                    ticket: 0,
                    generation,
                    frames,
                });
            });
    }

    /// Failure-sound request seeds accepted so far.
    #[cfg(test)]
    pub fn fail_request_log(&self) -> &[u32] {
        &self.fail_requests
    }

    /// Plays chimes whose PCM is ready and returns their tickets (also when playing failed, so
    /// the visuals never wait for a sound that is not coming). Cheap and non-blocking; called
    /// once per UI tick. Buffers of a stopped generation are dropped silently.
    pub fn pump_chimes(&mut self) -> Vec<u64> {
        let mut started = Vec::new();
        while let Ok(buffer) = self.chime_rx.try_recv() {
            if buffer.generation != self.chime_generation {
                continue;
            }
            if buffer.fail {
                self.play_fail(&buffer.frames);
                continue;
            }
            self.play_chime(&buffer.frames);
            started.push(buffer.ticket);
        }
        started
    }

    fn play_chime(&mut self, frames: &[[f32; 2]]) {
        let Some(manager) = &mut self.manager else {
            return;
        };
        let data = StaticSoundData {
            sample_rate: chime::SAMPLE_RATE,
            frames: frames.iter().map(|f| Frame::new(f[0], f[1])).collect(),
            settings: StaticSoundSettings::default().volume(CHIME_VOLUME_DB),
            slice: None,
        };
        self.chimes
            .retain(|handle| handle.state() != kira::sound::PlaybackState::Stopped);
        let fade = |ms| Tween {
            duration: Duration::from_millis(ms),
            ..Tween::default()
        };
        while self.chimes.len() >= MAX_CHIME_VOICES {
            self.chimes.remove(0).stop(fade(30));
        }
        for older in &mut self.chimes {
            older.set_volume(CHIME_VOLUME_DB + CHIME_DUCK_DB, fade(120));
        }
        if let Ok(handle) = manager.play(data) {
            self.chimes.push(handle);
        }
    }

    fn play_fail(&mut self, frames: &[[f32; 2]]) {
        let Some(manager) = &mut self.manager else {
            return;
        };
        let data = StaticSoundData {
            sample_rate: fail::SAMPLE_RATE,
            frames: frames.iter().map(|f| Frame::new(f[0], f[1])).collect(),
            settings: StaticSoundSettings::default().volume(FAIL_VOLUME_DB),
            slice: None,
        };
        self.fails
            .retain(|handle| handle.state() != kira::sound::PlaybackState::Stopped);
        while self.fails.len() >= MAX_FAIL_VOICES {
            self.fails.remove(0).stop(Tween {
                duration: Duration::from_millis(30),
                ..Tween::default()
            });
        }
        if let Ok(handle) = manager.play(data) {
            self.fails.push(handle);
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
        for mut handle in self.chimes.drain(..).chain(self.fails.drain(..)) {
            handle.stop(quick);
        }
        for mut voice in self.burst_voices.drain(..) {
            if let Some(handle) = &mut voice.handle {
                handle.stop(quick);
            }
        }
        // Chimes still being rendered belong to the old generation and are dropped on arrival.
        self.chime_generation = self.chime_generation.wrapping_add(1);
        while self.chime_rx.try_recv().is_ok() {}
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

impl Drop for DopamineAudio {
    fn drop(&mut self) {
        // A start-up render that is still running discards its result instead of publishing it.
        self.burst_cancel.store(true, Ordering::Release);
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
        let (chime_tx, chime_rx) = mpsc::channel();
        DopamineAudio {
            chimes: Vec::new(),
            fails: Vec::new(),
            burst_pcm: Arc::new(OnceLock::new()),
            burst_cancel: Arc::new(AtomicBool::new(false)),
            burst_voices: Vec::new(),
            burst_started: Vec::new(),
            fail_jobs: Arc::new(AtomicUsize::new(0)),
            fail_requests: Vec::new(),
            chime_tx,
            chime_rx,
            chime_jobs: Arc::new(AtomicUsize::new(0)),
            chime_generation: 0,
            chime_next_ticket: 0,
            chime_requests: Vec::new(),
            fake_tickets: false,
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
    fn chime_requests_without_a_device_are_logged_but_spawn_and_leak_nothing() {
        let mut audio = silent();
        for i in 0..20 {
            audio.request_gain_chime(i + 1, 3, i);
        }
        assert_eq!(audio.chime_request_log().len(), 20);
        assert_eq!(
            audio.request_gain_chime(5, 1, 1),
            None,
            "no device, no ticket"
        );
        assert_eq!(audio.chime_jobs.load(Ordering::Acquire), 0);
        audio.pump_chimes();
        assert!(audio.chimes.is_empty());
    }

    #[test]
    fn stop_all_discards_chimes_that_were_still_rendering() {
        let mut audio = silent();
        let old = audio.chime_generation;
        audio
            .chime_tx
            .send(ChimeBuffer {
                fail: false,
                ticket: 1,
                generation: old,
                frames: vec![[0.0; 2]; 8],
            })
            .unwrap();
        audio.stop_all();
        assert_ne!(audio.chime_generation, old);
        assert!(audio.chime_rx.try_recv().is_err(), "queue emptied");
        // A buffer that arrives later with the old generation is ignored.
        audio
            .chime_tx
            .send(ChimeBuffer {
                fail: false,
                ticket: 2,
                generation: old,
                frames: vec![[0.0; 2]; 8],
            })
            .unwrap();
        assert!(
            audio.pump_chimes().is_empty(),
            "stale ticket is never reported"
        );
        assert!(audio.chimes.is_empty());
        assert!(audio.chime_rx.try_recv().is_err());
    }

    #[test]
    fn fail_requests_without_a_device_are_logged_and_never_spawn_or_leak() {
        let mut audio = silent();
        for seed in 0..10 {
            audio.request_fail_sound(seed);
        }
        assert_eq!(audio.fail_request_log().len(), 10);
        assert_eq!(audio.fail_jobs.load(Ordering::Acquire), 0);
        assert!(audio.pump_chimes().is_empty());
        assert!(audio.fails.is_empty());
    }

    #[test]
    fn a_failure_buffer_never_reports_a_chime_ticket_and_stop_all_discards_it() {
        let mut audio = silent();
        let generation = audio.chime_generation;
        audio
            .chime_tx
            .send(ChimeBuffer {
                fail: true,
                ticket: 0,
                generation,
                frames: vec![[0.0; 2]; 4],
            })
            .unwrap();
        assert!(audio.pump_chimes().is_empty(), "not a Gain Chime");
        audio
            .chime_tx
            .send(ChimeBuffer {
                fail: true,
                ticket: 0,
                generation,
                frames: vec![[0.0; 2]; 4],
            })
            .unwrap();
        audio.stop_all();
        assert!(audio.pump_chimes().is_empty());
        assert!(audio.fails.is_empty());
    }

    #[test]
    fn the_job_guard_releases_its_slot_even_when_the_worker_panics() {
        let jobs = Arc::new(AtomicUsize::new(1));
        let guard = JobGuard(Arc::clone(&jobs));
        let worker = std::thread::spawn(move || {
            let _guard = guard;
            panic!("render failed");
        });
        assert!(worker.join().is_err());
        assert_eq!(jobs.load(Ordering::Acquire), 0);
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
