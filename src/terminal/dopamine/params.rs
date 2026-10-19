//! Every tunable number of the Backspace "Neon Disintegration" effect, in one place.
//!
//! All values are provisional starting points to be settled by looking and listening, not a
//! specification. Nothing here is exposed in `settings.toml` yet.

use iced::Color;

/// Look and motion of the particles.
pub struct VisualParams {
    /// Particles alive at once on one tab; a burst that does not fit is trimmed.
    pub max_particles_per_tab: usize,
    /// Shortest time between two bursts on one tab (key repeat is slower than this).
    pub min_burst_interval_ms: u64,

    /// Particles per burst.
    pub flash_count: usize,
    pub spark_count: usize,
    pub mote_count: usize,

    /// Initial flash: radius in px and lifetime in seconds.
    pub flash_radius: f32,
    pub flash_life: f32,

    /// Sparks: fast, radial, dragged, then pulled down. A share of them lives longer and keeps falling.
    pub spark_speed: (f32, f32),
    pub spark_drag: f32,
    pub spark_gravity: f32,
    pub spark_short_life: (f32, f32),
    pub spark_long_life: (f32, f32),
    /// Probability that a spark gets the long life.
    pub spark_long_share: f32,
    pub spark_radius: (f32, f32),

    /// Motes: slow grains of light that rain down the whole viewport.
    pub mote_speed: (f32, f32),
    pub mote_drag: f32,
    pub mote_gravity: f32,
    /// Longer than a fall across a tall viewport, so they do not vanish half way.
    pub mote_life: (f32, f32),
    pub mote_radius: (f32, f32),

    /// Speed limit, so a long fall does not turn into an endless streak.
    pub max_speed: f32,

    /// Streak (comet tail) length: seconds of travel, capped in px; shorter than this is not drawn.
    pub streak_seconds: f32,
    pub streak_max_px: f32,
    pub streak_min_px: f32,

    /// Glow approximation: a translucent disc of `halo_scale` × radius under a bright core.
    pub halo_scale: f32,
    pub halo_alpha: f32,
    pub core_scale: f32,

    /// Seconds spent white-hot after birth, and the share of the life where the afterglow starts to
    /// darken.
    pub hot_seconds: f32,
    pub cool_from: f32,
    /// Brightness of the darkest afterglow, relative to the neon colour.
    pub deep_level: f32,
    /// Motes and long-lived sparks stay opaque until this share of their life has passed.
    pub fade_from: f32,
    /// Fade across the last pixels above the bottom edge.
    pub bottom_fade_px: f32,
    /// Twinkle depth of the motes (0 = steady).
    pub twinkle: f32,
    /// How much white the bright centre mixes in (0 = the neon colour itself).
    pub core_white: f32,

    /// Text colours with less saturation than this (white, grey) use the fallback palette.
    pub min_saturation: f32,
}

pub const VISUAL: VisualParams = VisualParams {
    max_particles_per_tab: 480,
    min_burst_interval_ms: 14,

    flash_count: 1,
    spark_count: 10,
    mote_count: 14,

    flash_radius: 30.0,
    flash_life: 0.10,

    spark_speed: (380.0, 980.0),
    spark_drag: 3.4,
    spark_gravity: 640.0,
    spark_short_life: (0.20, 0.45),
    spark_long_life: (1.2, 2.0),
    spark_long_share: 0.4,
    spark_radius: (1.1, 2.0),

    mote_speed: (70.0, 420.0),
    mote_drag: 0.5,
    mote_gravity: 1000.0,
    mote_life: (3.2, 4.0),
    mote_radius: (1.2, 2.3),

    max_speed: 1100.0,

    streak_seconds: 0.034,
    streak_max_px: 38.0,
    streak_min_px: 2.5,

    halo_scale: 3.3,
    halo_alpha: 0.22,
    core_scale: 0.62,

    hot_seconds: 0.10,
    cool_from: 0.55,
    deep_level: 0.35,
    fade_from: 0.88,
    bottom_fade_px: 60.0,
    twinkle: 0.28,
    core_white: 0.35,

    min_saturation: 0.25,
};

/// Neon colours used when the text colour near the cursor is unknown or colourless.
pub const FALLBACK_PALETTE: [Color; 3] = [
    Color::from_rgb(0.10, 0.92, 1.00), // cyan
    Color::from_rgb(0.38, 0.58, 1.00), // electric blue
    Color::from_rgb(0.78, 0.92, 1.00), // blue-white
];

/// One synthesised voice: noise through a swept band-pass, plus optional partials and a body tone.
pub struct VoiceSpec {
    pub length_ms: f32,
    /// Attack time of the noise burst.
    pub attack_ms: f32,
    /// Exponential decay time constant of the noise burst.
    pub decay_ms: f32,
    /// Band-pass centre at the start and the end of the sound, and its resonance.
    pub centre_hz: (f32, f32),
    pub q: f32,
    /// Number of sparse crackle clicks and how long they are spread over.
    pub clicks: usize,
    pub click_span_ms: f32,
    /// Inharmonic ringing partials (Hz) and their decay.
    pub partials: &'static [f32],
    pub partial_gain: f32,
    pub partial_decay_ms: f32,
    /// A short tone for weight: frequency, gain and decay. Kept at 350 Hz or above.
    pub body_hz: f32,
    pub body_gain: f32,
    pub body_decay_ms: f32,
}

/// Shared synthesis and playback settings.
pub struct SfxParams {
    pub sample_rate: u32,
    /// Pre-rendered variations per voice (different noise), cycled so repeats do not sound alike.
    pub variants: usize,
    /// Low cut applied to every voice (Hz).
    pub low_cut_hz: f32,
    /// Fade across the last milliseconds, so no sound ends on a step.
    pub release_ms: f32,
    /// Peak of the rendered buffers (before the playback volume).
    pub peak: f32,
    /// Playback volume in dB and the spread added per play.
    pub volume_db: f32,
    pub volume_jitter_db: f32,
    /// Playback-rate range (pitch) chosen per play.
    pub rate_range: (f64, f64),
    /// Shortest time between two sounds and how many may overlap.
    pub min_interval_ms: u64,
    pub max_voices: usize,
}

pub const SFX: SfxParams = SfxParams {
    sample_rate: 48_000,
    variants: 5,
    low_cut_hz: 320.0,
    release_ms: 4.0,
    peak: 0.8,
    volume_db: -4.0,
    volume_jitter_db: 1.2,
    rate_range: (0.94, 1.08),
    min_interval_ms: 28,
    max_voices: 6,
};

/// Light electric snap ("バチッ").
pub const ZAP: VoiceSpec = VoiceSpec {
    length_ms: 70.0,
    attack_ms: 1.5,
    decay_ms: 13.0,
    centre_hz: (5600.0, 2800.0),
    q: 2.2,
    clicks: 5,
    click_span_ms: 38.0,
    partials: &[],
    partial_gain: 0.0,
    partial_decay_ms: 1.0,
    body_hz: 0.0,
    body_gain: 0.0,
    body_decay_ms: 1.0,
};

/// Metallic shatter ("シャリッ").
pub const CRACK: VoiceSpec = VoiceSpec {
    length_ms: 110.0,
    attack_ms: 2.0,
    decay_ms: 22.0,
    centre_hz: (5600.0, 4200.0),
    q: 5.0,
    clicks: 3,
    click_span_ms: 25.0,
    partials: &[3130.0, 4710.0, 6370.0],
    partial_gain: 0.5,
    partial_decay_ms: 30.0,
    body_hz: 0.0,
    body_gain: 0.0,
    body_decay_ms: 1.0,
};

/// Heavier neon burst ("ビシュッ").
pub const BURST: VoiceSpec = VoiceSpec {
    length_ms: 120.0,
    attack_ms: 2.0,
    decay_ms: 30.0,
    centre_hz: (5200.0, 1800.0),
    q: 1.6,
    clicks: 2,
    click_span_ms: 20.0,
    partials: &[],
    partial_gain: 0.0,
    partial_decay_ms: 1.0,
    body_hz: 380.0,
    body_gain: 0.9,
    body_decay_ms: 22.0,
};
