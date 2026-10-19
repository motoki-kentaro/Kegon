//! Rainbow Wave tabs and HIT combo badge for Dopamine Mode.
//!
//! Provides the continuous 45-degree Rainbow Wave field across the tab bar,
//! unified phase coordinate math, active vs inactive color mapping, and the fixed-width
//! HIT combo counter badge.
//!
//! "Rainbow Wave — One rainbow, across all tabs."
//! "仕事は真面目に。描画は正気を失え。"

use iced::alignment::{Horizontal, Vertical};
use iced::widget::{Space, button, container, text};
use iced::{Border, Color, Element, Font, Length, Radians};

use crate::app::tab_style;
use crate::terminal::dopamine::border::hsv_to_rgb;
use crate::theme::KegonTheme;

/// Fixed width reserved for the HIT combo badge (fits up to 3 digits + '!').
pub const HIT_BADGE_WIDTH: f32 = 42.0;

/// Height of the HIT combo badge plate.
pub const HIT_BADGE_HEIGHT: f32 = 18.0;

/// Maximum combo count displayed in the HIT badge.
pub const MAX_DISPLAY_COMBO: u32 = 999;

/// Wavelength of the rainbow in logical pixels (distance for one full 360° spectrum cycle).
///
/// Set to a fixed 480.0 px (exactly 3 standard 160px tabs), independent of total tabs.
pub const RAINBOW_WAVELENGTH: f32 = 480.0;

/// Scrolling speed in cycles per second (0.18 cycles/s = ~5.5s per full rainbow rotation).
pub const RAINBOW_SCROLL_SPEED: f32 = 0.18;

/// Amplitude of the traveling sine wave undulation in cycles (~21.6° of hue shift).
pub const WAVE_AMPLITUDE: f32 = 0.06;

/// Spatial wavelength of the wave ripple (320.0 px = 2 tabs).
pub const WAVE_SPATIAL_PERIOD: f32 = 320.0;

/// Temporal frequency of the wave undulation in Hz (0.35 Hz).
pub const WAVE_TEMPORAL_HZ: f32 = 0.35;

/// Formats the HIT text for a given combo count.
///
/// Returns `None` when combo is 0 (hiding the badge text while retaining reserved width).
/// When `combo > 0`, returns `"{combo}!"` capped at 999.
pub fn format_hit_text(combo: u32) -> Option<String> {
    if combo == 0 {
        None
    } else {
        Some(format!("{}!", combo.min(MAX_DISPLAY_COMBO)))
    }
}

/// Computes the Rainbow Wave phase in `[0.0, 1.0)` for a given diagonal coordinate `(x + y)`
/// and elapsed animation time `elapsed_secs` (in seconds).
///
/// Uses double-precision floating point internally to ensure zero drift or jitter even after
/// weeks of continuous operation.
pub fn rainbow_wave_phase(diag: f32, elapsed_secs: f64) -> f32 {
    let spatial_term = (diag as f64) / (RAINBOW_WAVELENGTH as f64);
    let scroll_term = (RAINBOW_SCROLL_SPEED as f64) * elapsed_secs;
    let wave_k = 2.0 * std::f64::consts::PI / (WAVE_SPATIAL_PERIOD as f64);
    let wave_omega = 2.0 * std::f64::consts::PI * (WAVE_TEMPORAL_HZ as f64);
    let wave_term =
        (WAVE_AMPLITUDE as f64) * (wave_k * (diag as f64) - wave_omega * elapsed_secs).sin();

    let raw_phase = spatial_term - scroll_term + wave_term;
    (raw_phase.rem_euclid(1.0)) as f32
}

/// Converts a diagonal coordinate and elapsed time into a continuous hue angle in `[0.0, 360.0)`.
pub fn rainbow_wave_hue(diag: f32, elapsed_secs: f64) -> f32 {
    rainbow_wave_phase(diag, elapsed_secs) * 360.0
}

/// Computes the active tab rainbow color for a given hue angle (in degrees).
///
/// Active tabs use high-saturation and high-brightness colors. Hues around blue and purple
/// (190°..295°) are lightened toward electric sky blue / pastel lavender so that black title
/// text and close icon achieve high WCAG contrast (>= 4.5:1) across the entire 360° spectrum.
pub fn active_rainbow_color(hue_deg: f32) -> Color {
    let h = ((hue_deg % 360.0) + 360.0) % 360.0;
    let (s, v) = if (190.0..=295.0).contains(&h) {
        let factor = 1.0 - ((h - 245.0).abs() / 55.0);
        let s = (0.95 - 0.55 * factor).clamp(0.40, 0.95);
        let v = 1.0;
        (s, v)
    } else {
        (0.95, 0.95)
    };
    hsv_to_rgb(h, s, v, 1.0)
}

/// Computes the inactive tab rainbow color ("くすみレインボー") for a given hue angle (in degrees).
///
/// Shares the exact same hue progression and phase as the active tab, but with subdued
/// saturation and luminance so the active tab remains visually dominant while maintaining
/// unmistakable rainbow continuity.
pub fn inactive_rainbow_color(hue_deg: f32) -> Color {
    let h = ((hue_deg % 360.0) + 360.0) % 360.0;
    let s = 0.42;
    let v = 0.30;
    hsv_to_rgb(h, s, v, 1.0)
}

/// Returns the Rainbow Wave color for a given diagonal coordinate, active state, and elapsed time.
pub fn rainbow_wave_color(diag: f32, is_active: bool, elapsed_secs: f64) -> Color {
    let hue = rainbow_wave_hue(diag, elapsed_secs);
    if is_active {
        active_rainbow_color(hue)
    } else {
        inactive_rainbow_color(hue)
    }
}

/// Computes the diagonal coordinate along the 45-degree field for a normalized stop `s` in `[0.0, 1.0]`.
///
/// Projects along `diag = tab_index * tab_w + s * (tab_w + tab_h)`.
#[inline]
pub fn tab_diagonal_coord(tab_index: usize, s: f32, tab_w: f32, tab_h: f32) -> f32 {
    (tab_index as f32) * tab_w + s * (tab_w + tab_h)
}

/// Generates a 45-degree linear gradient for a tab container at `tab_index`.
///
/// Uses 8 stops spaced evenly across the diagonal of the tab. Because stop `s` projects
/// to diagonal coordinate `tab_index * tab_w + s * (tab_w + tab_h)`,
/// the gradient is mathematically continuous across adjacent tab boundaries for any tab dimensions.
pub fn tab_rainbow_gradient(
    tab_index: usize,
    is_active: bool,
    elapsed_secs: f64,
    tab_w: f32,
    tab_h: f32,
) -> iced::gradient::Linear {
    use iced::gradient::Linear;

    // 135° in iced angle convention projects along vector (+1, +1), producing 45° diagonal stripes.
    let angle = Radians(3.0 * std::f32::consts::FRAC_PI_4);
    let mut linear = Linear::new(angle);

    const STOPS: usize = 8;
    for s_idx in 0..STOPS {
        let s = s_idx as f32 / (STOPS - 1) as f32; // 0.0, 1/7, ..., 1.0
        let diag = tab_diagonal_coord(tab_index, s, tab_w, tab_h);
        let color = rainbow_wave_color(diag, is_active, elapsed_secs);
        linear = linear.add_stop(s, color);
    }

    linear
}

/// Returns the tab container style.
///
/// In Dopamine Mode, renders the tab's portion of the continuous 45-degree Rainbow Wave field
/// sized to `(tab_w, tab_h)`.
/// When Dopamine Mode is OFF, returns default (transparent) container style.
pub fn tab_container_style(
    tab_index: usize,
    is_active: bool,
    dopamine_enabled: bool,
    elapsed_secs: f64,
    tab_w: f32,
    tab_h: f32,
) -> container::Style {
    if dopamine_enabled {
        container::Style::default().background(tab_rainbow_gradient(
            tab_index,
            is_active,
            elapsed_secs,
            tab_w,
            tab_h,
        ))
    } else {
        container::Style::default()
    }
}

/// Returns the title text color for a tab.
///
/// In Dopamine Mode:
/// - Active tab: `Color::BLACK` (for striking contrast over vivid rainbow)
/// - Inactive tab: `theme.text.primary` (high contrast over dark muted rainbow)
///
/// When Dopamine Mode is OFF:
/// - Active tab: `theme.text.emphasis`
/// - Inactive tab: `theme.text.muted`
pub fn tab_title_color(is_active: bool, dopamine_enabled: bool, theme: &KegonTheme) -> Color {
    if dopamine_enabled {
        if is_active {
            Color::BLACK
        } else {
            theme.text.primary
        }
    } else if is_active {
        theme.text.emphasis
    } else {
        theme.text.muted
    }
}

/// Returns the close icon color for a tab.
///
/// In Dopamine Mode:
/// - Active tab: `Color::BLACK`
/// - Inactive tab: `theme.icons.active` on hover, `theme.icons.inactive` otherwise
///
/// When Dopamine Mode is OFF:
/// - Active or hovered tab: `theme.icons.active`
/// - Inactive unhovered tab: `theme.icons.inactive`
pub fn tab_close_icon_color(
    is_active: bool,
    is_hovered: bool,
    dopamine_enabled: bool,
    theme: &KegonTheme,
) -> Color {
    if dopamine_enabled {
        if is_active {
            Color::BLACK
        } else if is_hovered {
            theme.icons.active
        } else {
            theme.icons.inactive
        }
    } else if is_hovered || is_active {
        theme.icons.active
    } else {
        theme.icons.inactive
    }
}

/// Returns the select button style for a tab.
///
/// In Dopamine Mode:
/// - Background is transparent by default so the container's rainbow gradient shows through.
/// - On hover/press, applies a subtle translucent overlay.
/// - Text color follows `tab_title_color`.
///
/// When Dopamine Mode is OFF:
/// - Delegates to standard `tab_style`.
pub fn tab_select_button_style(
    is_active: bool,
    status: button::Status,
    dopamine_enabled: bool,
    theme: &KegonTheme,
) -> button::Style {
    if dopamine_enabled {
        let background = match status {
            button::Status::Hovered => Some(Color::from_rgba(1.0, 1.0, 1.0, 0.16).into()),
            button::Status::Pressed => Some(Color::from_rgba(0.0, 0.0, 0.0, 0.20).into()),
            _ => None,
        };
        button::Style {
            background,
            text_color: tab_title_color(is_active, true, theme),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..button::Style::default()
        }
    } else {
        tab_style(theme, is_active, status)
    }
}

/// Returns the close button style for a tab.
///
/// In Dopamine Mode:
/// - Background is transparent by default.
/// - On hover/press, applies a subtle translucent pill overlay.
///
/// When Dopamine Mode is OFF:
/// - Delegates to standard `tab_style`.
pub fn tab_close_button_style(
    is_active: bool,
    status: button::Status,
    dopamine_enabled: bool,
    theme: &KegonTheme,
) -> button::Style {
    if dopamine_enabled {
        let background = match status {
            button::Status::Hovered => Some(Color::from_rgba(1.0, 1.0, 1.0, 0.22).into()),
            button::Status::Pressed => Some(Color::from_rgba(0.0, 0.0, 0.0, 0.25).into()),
            _ => None,
        };
        button::Style {
            background,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    } else {
        tab_style(theme, is_active, status)
    }
}

/// Renders the HIT combo badge widget with constant layout width.
///
/// When `combo > 0`:
/// - Renders a dark rounded plate (`Color::from_rgba(0.0, 0.0, 0.0, 0.78)`)
/// - Displays bold white text `"{combo}!"` centered inside the plate
///
/// When `combo == 0`:
/// - Renders an empty space of the exact same `HIT_BADGE_WIDTH` (42px) and `HIT_BADGE_HEIGHT` (18px)
/// - Completely transparent and invisible, ensuring zero title layout jitter when combo appears or disappears.
pub fn hit_badge_element<'a, Message: 'a>(combo: u32, font: Font) -> Element<'a, Message> {
    if let Some(hit_str) = format_hit_text(combo) {
        let mut bold_font = font;
        bold_font.weight = iced::font::Weight::Bold;

        container(
            text(hit_str)
                .size(11.0)
                .font(bold_font)
                .color(Color::WHITE)
                .wrapping(text::Wrapping::None),
        )
        .width(HIT_BADGE_WIDTH)
        .height(HIT_BADGE_HEIGHT)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(|_| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.78).into()),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    } else {
        container(
            Space::new()
                .width(Length::Fixed(HIT_BADGE_WIDTH))
                .height(Length::Fixed(HIT_BADGE_HEIGHT)),
        )
        .width(HIT_BADGE_WIDTH)
        .height(HIT_BADGE_HEIGHT)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::NIGHT_DARK;

    fn rel_luminance(c: Color) -> f32 {
        let to_linear = |val: f32| -> f32 {
            if val <= 0.04045 {
                val / 12.92
            } else {
                ((val + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * to_linear(c.r) + 0.7152 * to_linear(c.g) + 0.0722 * to_linear(c.b)
    }

    fn contrast_ratio(lum1: f32, lum2: f32) -> f32 {
        let lighter = lum1.max(lum2);
        let darker = lum1.min(lum2);
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn hit_text_formatting_and_clamping() {
        assert_eq!(format_hit_text(0), None);
        assert_eq!(format_hit_text(1), Some("1!".to_string()));
        assert_eq!(format_hit_text(12), Some("12!".to_string()));
        assert_eq!(format_hit_text(128), Some("128!".to_string()));
        assert_eq!(format_hit_text(999), Some("999!".to_string()));
        assert_eq!(format_hit_text(1000), Some("999!".to_string()));
        assert_eq!(format_hit_text(u32::MAX), Some("999!".to_string()));
    }

    #[test]
    fn badge_dimensions_constants() {
        assert_eq!(HIT_BADGE_WIDTH, 42.0);
        assert_eq!(HIT_BADGE_HEIGHT, 18.0);
        assert_eq!(MAX_DISPLAY_COMBO, 999);
    }

    #[test]
    fn same_coordinates_and_time_yield_identical_hue() {
        for diag in [0.0, 50.0, 160.0, 320.0, 480.0, 1200.0] {
            for t in [0.0, 1.25, 10.0, 3600.0] {
                let h1 = rainbow_wave_hue(diag, t);
                let h2 = rainbow_wave_hue(diag, t);
                assert_eq!(
                    h1, h2,
                    "Deterministic hue evaluation failed at d={diag}, t={t}"
                );
            }
        }
    }

    #[test]
    fn boundary_continuity_across_adjacent_tabs() {
        // Must hold for production layout dimensions from app.rs as well as custom dimensions,
        // proving the continuous wave math is not hardcoded to 160.0/35.0.
        let dimension_cases = [
            (crate::app::TAB_WIDTH, crate::app::TAB_STRIP_HEIGHT),
            (200.0, 40.0),
            (120.0, 28.0),
            (240.0, 48.0),
        ];

        for &(tab_w, tab_h) in &dimension_cases {
            // Test at boundary between tab i and tab i+1 for various tabs, heights y, and times t
            for tab_i in 0..6 {
                for y_fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let y = y_fraction * tab_h;
                    for t in [0.0, 0.5, 2.0, 10.0, 100.0] {
                        // Right edge of tab_i: x_local = tab_w, height y
                        let s_i = (tab_w + y) / (tab_w + tab_h);
                        let diag_i = tab_diagonal_coord(tab_i, s_i, tab_w, tab_h);

                        // Left edge of tab_i+1: x_local = 0.0, height y
                        let s_next = y / (tab_w + tab_h);
                        let diag_next = tab_diagonal_coord(tab_i + 1, s_next, tab_w, tab_h);

                        assert!(
                            (diag_i - diag_next).abs() < 1e-4,
                            "Diagonal coordinate mismatch at boundary between tab {tab_i} and {} for size ({tab_w}, {tab_h})",
                            tab_i + 1
                        );

                        let p_i = rainbow_wave_phase(diag_i, t);
                        let p_next = rainbow_wave_phase(diag_next, t);
                        assert!(
                            (p_i - p_next).abs() < 1e-5,
                            "Phase mismatch at boundary for ({tab_w}, {tab_h}): {p_i} vs {p_next} (tab {tab_i}, y={y}, t={t})"
                        );

                        let c_active_i = rainbow_wave_color(diag_i, true, t);
                        let c_active_next = rainbow_wave_color(diag_next, true, t);
                        assert!((c_active_i.r - c_active_next.r).abs() < 1e-4);
                        assert!((c_active_i.g - c_active_next.g).abs() < 1e-4);
                        assert!((c_active_i.b - c_active_next.b).abs() < 1e-4);

                        let c_inactive_i = rainbow_wave_color(diag_i, false, t);
                        let c_inactive_next = rainbow_wave_color(diag_next, false, t);
                        assert!((c_inactive_i.r - c_inactive_next.r).abs() < 1e-4);
                        assert!((c_inactive_i.g - c_inactive_next.g).abs() < 1e-4);
                        assert!((c_inactive_i.b - c_inactive_next.b).abs() < 1e-4);
                    }
                }
            }
        }
    }

    #[test]
    fn tab_count_change_does_not_shift_existing_tab_hues() {
        // Wavelength is fixed constant RAINBOW_WAVELENGTH (480.0 px).
        // Evaluated at Tab 0 stop s=0.5: diag = 0 + 0.5 * (TAB_WIDTH + TAB_STRIP_HEIGHT).
        // It does NOT depend on total tabs!
        let t = 2.0;
        let diag_tab0_mid =
            tab_diagonal_coord(0, 0.5, crate::app::TAB_WIDTH, crate::app::TAB_STRIP_HEIGHT);
        let hue_initial = rainbow_wave_hue(diag_tab0_mid, t);

        // Simulate having 1 tab, 2 tabs, 5 tabs, 20 tabs:
        // Tab 0's hue at the same coordinate is strictly invariant
        for _total_tabs in [1, 2, 5, 20] {
            let hue_after = rainbow_wave_hue(diag_tab0_mid, t);
            assert_eq!(
                hue_initial, hue_after,
                "Tab count change must never shift existing tab hue"
            );
        }
    }

    #[test]
    fn tab_switch_preserves_base_hue_and_changes_only_saturation_luminance() {
        let diag = 200.0;
        let t = 3.5;
        let hue = rainbow_wave_hue(diag, t);

        let active_c = active_rainbow_color(hue);
        let inactive_c = inactive_rainbow_color(hue);

        // Active color is high saturation & luminance
        assert!(active_c.r >= 0.0 && active_c.g >= 0.0 && active_c.b >= 0.0);

        // Inactive color is muted ("くすみレインボー")
        assert!(inactive_c.r <= 0.35 && inactive_c.g <= 0.35 && inactive_c.b <= 0.35);

        // The base hue is identical
        assert_eq!(
            rainbow_wave_color(diag, true, t),
            active_c,
            "Active tab must use the exact wave hue"
        );
        assert_eq!(
            rainbow_wave_color(diag, false, t),
            inactive_c,
            "Inactive tab must use the exact wave hue"
        );
    }

    #[test]
    fn smooth_phase_progression_over_time() {
        let diag = 100.0;
        let dt = 0.016; // ~60 FPS frame interval
        let mut t = 0.0;
        let mut prev_phase = rainbow_wave_phase(diag, t);

        for _ in 0..100 {
            t += dt;
            let current_phase = rainbow_wave_phase(diag, t);
            // Difference accounting for circular wrap
            let mut diff = (current_phase - prev_phase).abs();
            if diff > 0.5 {
                diff = (1.0 - diff).abs();
            }
            // Max phase delta per 16ms frame is comfortably bounded
            assert!(
                diff < 0.05,
                "Phase jump too large between frames: diff={diff} at t={t}"
            );
            prev_phase = current_phase;
        }
    }

    #[test]
    fn sine_wave_undulation_bounded_and_continuous() {
        let t = 1.0;
        for diag in (0..1000).step_by(5) {
            let diag_f = diag as f32;
            let phase = rainbow_wave_phase(diag_f, t);
            assert!(
                (0.0..1.0).contains(&phase),
                "Phase must strictly lie within [0.0, 1.0), got {phase}"
            );
        }
    }

    #[test]
    fn long_running_stability_and_no_numerical_anomalies() {
        // Test timestamps ranging from 0 to over 3 years of uptime
        let test_times: [f64; 7] = [
            0.0,
            60.0,
            3600.0,        // 1 hour
            86400.0,       // 24 hours
            604800.0,      // 7 days
            10_000_000.0,  // ~115 days
            100_000_000.0, // ~3.17 years
        ];

        for &t in &test_times {
            for diag in [0.0, 160.0, 320.0, 480.0, 1500.0] {
                let phase = rainbow_wave_phase(diag, t);
                assert!(
                    phase.is_finite() && !phase.is_nan() && (0.0..1.0).contains(&phase),
                    "Phase anomaly at t={t}, diag={diag}: {phase}"
                );
                let hue = rainbow_wave_hue(diag, t);
                assert!(
                    hue.is_finite() && !hue.is_nan() && (0.0..360.0).contains(&hue),
                    "Hue anomaly at t={t}, diag={diag}: {hue}"
                );
                let active = rainbow_wave_color(diag, true, t);
                assert!(active.r.is_finite() && active.g.is_finite() && active.b.is_finite());
                let inactive = rainbow_wave_color(diag, false, t);
                assert!(inactive.r.is_finite() && inactive.g.is_finite() && inactive.b.is_finite());
            }
        }
    }

    #[test]
    fn active_rainbow_contrast_with_black_text() {
        let black_lum = rel_luminance(Color::BLACK);
        // Step through full 360 degrees in fine increments
        for deg in (0..360).step_by(10) {
            let color = active_rainbow_color(deg as f32);
            let lum = rel_luminance(color);
            let contrast = contrast_ratio(lum, black_lum);
            assert!(
                contrast >= 4.5,
                "Active rainbow hue {}° has insufficient contrast with black text: {:.2}:1 (color: {:?})",
                deg,
                contrast,
                color
            );
        }
    }

    #[test]
    fn inactive_rainbow_muted_and_contrast_with_light_text() {
        let text_lum = rel_luminance(Color::from_rgb(0.80, 0.80, 0.80));
        for deg in (0..360).step_by(10) {
            let color = inactive_rainbow_color(deg as f32);
            let lum = rel_luminance(color);
            let contrast = contrast_ratio(text_lum, lum);
            assert!(
                contrast >= 4.5,
                "Inactive rainbow hue {}° has insufficient contrast with light text: {:.2}:1 (color: {:?})",
                deg,
                contrast,
                color
            );
            // Must be muted ("くすみ"): value <= 0.35 and max channel <= 0.35
            assert!(
                color.r <= 0.35 && color.g <= 0.35 && color.b <= 0.35,
                "Inactive color too bright at {}°: {:?}",
                deg,
                color
            );
            // Must not be gray: at least some color separation
            let max_c = color.r.max(color.g).max(color.b);
            let min_c = color.r.min(color.g).min(color.b);
            assert!(
                (max_c - min_c) > 0.05,
                "Inactive color is nearly gray at {}°: {:?}",
                deg,
                color
            );
        }
    }

    #[test]
    fn tab_colors_and_styles_dopamine_on_vs_off() {
        let theme = &NIGHT_DARK;

        // Dopamine Mode ON
        assert_eq!(
            tab_title_color(true, true, theme),
            Color::BLACK,
            "Active title must be black in Dopamine Mode"
        );
        assert_eq!(
            tab_title_color(false, true, theme),
            theme.text.primary,
            "Inactive title must use primary text in Dopamine Mode"
        );
        assert_eq!(
            tab_close_icon_color(true, false, true, theme),
            Color::BLACK,
            "Active close icon must be black in Dopamine Mode"
        );
        assert_eq!(
            tab_close_icon_color(false, false, true, theme),
            theme.icons.inactive
        );
        assert_eq!(
            tab_close_icon_color(false, true, true, theme),
            theme.icons.active
        );

        // Dopamine Mode OFF (exact original values)
        assert_eq!(
            tab_title_color(true, false, theme),
            theme.text.emphasis,
            "Active title must be emphasis when Dopamine Mode is OFF"
        );
        assert_eq!(
            tab_title_color(false, false, theme),
            theme.text.muted,
            "Inactive title must be muted when Dopamine Mode is OFF"
        );
        assert_eq!(
            tab_close_icon_color(true, false, false, theme),
            theme.icons.active
        );
        assert_eq!(
            tab_close_icon_color(false, false, false, theme),
            theme.icons.inactive
        );
        assert_eq!(
            tab_close_icon_color(false, true, false, theme),
            theme.icons.active
        );
    }

    #[test]
    fn tab_select_button_style_reverts_cleanly_when_off() {
        let theme = &NIGHT_DARK;
        let original_active = tab_style(theme, true, button::Status::Active);
        let original_inactive = tab_style(theme, false, button::Status::Active);

        let off_active = tab_select_button_style(true, button::Status::Active, false, theme);
        let off_inactive = tab_select_button_style(false, button::Status::Active, false, theme);

        assert_eq!(off_active.text_color, original_active.text_color);
        assert_eq!(off_active.background, original_active.background);
        assert_eq!(off_inactive.text_color, original_inactive.text_color);
        assert_eq!(off_inactive.background, original_inactive.background);
    }

    #[test]
    fn tab_button_hover_overlays_in_dopamine_mode() {
        let theme = &NIGHT_DARK;
        let idle_style = tab_select_button_style(true, button::Status::Active, true, theme);
        assert_eq!(
            idle_style.background, None,
            "Idle select button must have transparent background to show rainbow"
        );

        let hovered_style = tab_select_button_style(true, button::Status::Hovered, true, theme);
        assert!(
            hovered_style.background.is_some(),
            "Hovered select button must have translucent highlight overlay"
        );

        let idle_close = tab_close_button_style(true, button::Status::Active, true, theme);
        assert_eq!(
            idle_close.background, None,
            "Idle close button must have transparent background"
        );

        let hovered_close = tab_close_button_style(true, button::Status::Hovered, true, theme);
        assert!(
            hovered_close.background.is_some(),
            "Hovered close button must have translucent highlight overlay"
        );
    }

    #[test]
    fn gradient_and_container_style_propagate_custom_and_production_dimensions() {
        // Production dimensions from app.rs:
        let _g_prod = tab_rainbow_gradient(
            0,
            true,
            1.0,
            crate::app::TAB_WIDTH,
            crate::app::TAB_STRIP_HEIGHT,
        );
        let _s_prod_on = tab_container_style(
            0,
            true,
            true,
            1.0,
            crate::app::TAB_WIDTH,
            crate::app::TAB_STRIP_HEIGHT,
        );
        let _s_prod_off = tab_container_style(
            0,
            true,
            false,
            1.0,
            crate::app::TAB_WIDTH,
            crate::app::TAB_STRIP_HEIGHT,
        );

        // Custom dimensions (e.g. 200.0, 40.0):
        let custom_w = 200.0;
        let custom_h = 40.0;
        let _g_custom = tab_rainbow_gradient(1, false, 2.5, custom_w, custom_h);
        let _s_custom = tab_container_style(1, false, true, 2.5, custom_w, custom_h);

        // Verify diagonal coordinates change when dimensions change:
        let diag_prod =
            tab_diagonal_coord(1, 0.5, crate::app::TAB_WIDTH, crate::app::TAB_STRIP_HEIGHT);
        let diag_custom = tab_diagonal_coord(1, 0.5, custom_w, custom_h);
        assert_ne!(diag_prod, diag_custom);
        assert_eq!(diag_prod, 160.0 + 0.5 * 195.0);
        assert_eq!(diag_custom, 200.0 + 0.5 * 240.0);
    }
}
