//! Command completion border effect for Dopamine Mode.
//!
//! Renders a temporary 1-physical-pixel border around the outermost perimeter of
//! the active terminal content rectangle:
//! - Success (`ShellReportedStatus::Success`) -> Rainbow gradient along the four edges
//! - Failure (`ShellReportedStatus::Failure`) -> Solid vivid Red
//!
//! The border starts at 100% opacity and linearly fades to 0% over ~400ms.

use std::time::{Duration, Instant};

use iced::widget::canvas::Frame;
use iced::{Color, Point, Size};

/// Duration of the completion border effect (400ms).
pub const BORDER_DURATION: Duration = Duration::from_millis(400);

/// Visual style / kind of completion border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderKind {
    /// Rainbow gradient along the four perimeter edges.
    Rainbow,
    /// Solid vivid red border.
    Red,
}

/// Active border instance being drawn, with its current opacity (1.0 down to 0.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveBorder {
    pub kind: BorderKind,
    pub alpha: f32,
}

/// Internal state tracking a command completion border effect for a terminal tab.
#[derive(Debug, Clone)]
pub struct BorderEffect {
    pub kind: BorderKind,
    pub command_id: u64,
    pub start_time: Instant,
    pub duration: Duration,
}

impl BorderEffect {
    /// Creates a new border effect starting at `start_time` with standard duration.
    pub fn new(kind: BorderKind, command_id: u64, start_time: Instant) -> Self {
        Self {
            kind,
            command_id,
            start_time,
            duration: BORDER_DURATION,
        }
    }

    /// Returns the active rendering state if the effect has not yet expired.
    pub fn current_active(&self, now: Instant) -> Option<ActiveBorder> {
        let elapsed = now.saturating_duration_since(self.start_time);
        if elapsed >= self.duration {
            None
        } else {
            let progress = elapsed.as_secs_f32() / self.duration.as_secs_f32();
            let alpha = (1.0 - progress).clamp(0.0, 1.0);
            Some(ActiveBorder {
                kind: self.kind,
                alpha,
            })
        }
    }

    /// Whether the border effect has completed its fade-out duration.
    pub fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start_time) >= self.duration
    }
}

/// Converts HSV (Hue in [0, 360), Saturation in [0, 1], Value in [0, 1]) and alpha to an iced Color.
pub fn hsv_to_rgb(h_deg: f32, s: f32, v: f32, alpha: f32) -> Color {
    let h = ((h_deg % 360.0) + 360.0) % 360.0;
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    Color::from_rgba(r1 + m, g1 + m, b1 + m, alpha.clamp(0.0, 1.0))
}

/// Calculates line width in logical coordinates corresponding to 1 physical pixel.
pub fn calculate_line_width(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        1.0 / scale_factor
    } else {
        1.0
    }
}

/// Paints the 1-physical-pixel border on the outermost perimeter of `size`.
pub fn paint_border(frame: &mut Frame, size: Size, border: ActiveBorder, scale_factor: f32) {
    let w = size.width;
    let h = size.height;
    let line_width = calculate_line_width(scale_factor);

    if w <= line_width * 2.0 || h <= line_width * 2.0 || border.alpha <= 0.001 {
        return;
    }

    match border.kind {
        BorderKind::Red => {
            // Vivid crimson red, clearly visible on both dark and light backgrounds
            let color = Color::from_rgba(0.96, 0.16, 0.16, border.alpha);
            // Top edge
            frame.fill_rectangle(Point::ORIGIN, Size::new(w, line_width), color);
            // Bottom edge
            frame.fill_rectangle(
                Point::new(0.0, h - line_width),
                Size::new(w, line_width),
                color,
            );
            // Left edge (inset by line_width to avoid corner overlapping)
            frame.fill_rectangle(
                Point::new(0.0, line_width),
                Size::new(line_width, h - 2.0 * line_width),
                color,
            );
            // Right edge (inset by line_width to avoid corner overlapping)
            frame.fill_rectangle(
                Point::new(w - line_width, line_width),
                Size::new(line_width, h - 2.0 * line_width),
                color,
            );
        }
        BorderKind::Rainbow => {
            paint_rainbow_border(frame, w, h, line_width, border.alpha);
        }
    }
}

/// Paints a continuous rainbow gradient around the 4 perimeter edges.
fn paint_rainbow_border(frame: &mut Frame, w: f32, h: f32, line_width: f32, alpha: f32) {
    let perimeter = 2.0 * (w + h);
    if perimeter <= 0.0 {
        return;
    }

    // Step size along edges for smooth gradient transition (~20 logical px per segment)
    const TARGET_SEGMENT_LEN: f32 = 20.0;
    // Tiny overlap (0.2px) between adjacent segments in the direction of the line to prevent raster gaps
    const SEAM_OVERLAP: f32 = 0.2;

    // 1. Top edge: Left -> Right (distance: 0 -> w)
    let n_top = ((w / TARGET_SEGMENT_LEN).ceil() as usize).max(4);
    for i in 0..n_top {
        let x0 = i as f32 * w / n_top as f32;
        let x1 = ((i + 1) as f32 * w / n_top as f32).min(w);
        let mid_dist = (x0 + x1) * 0.5;
        let hue = (mid_dist / perimeter) * 360.0;
        let color = hsv_to_rgb(hue, 0.95, 1.0, alpha);
        let seg_w = (x1 - x0 + SEAM_OVERLAP).min(w - x0);
        frame.fill_rectangle(Point::new(x0, 0.0), Size::new(seg_w, line_width), color);
    }

    // 2. Right edge: Top -> Bottom (distance: w -> w + h)
    let h_inner = h - 2.0 * line_width;
    let n_right = ((h_inner / TARGET_SEGMENT_LEN).ceil() as usize).max(4);
    for i in 0..n_right {
        let y0 = line_width + i as f32 * h_inner / n_right as f32;
        let y1 = (line_width + (i + 1) as f32 * h_inner / n_right as f32).min(h - line_width);
        let mid_dist = w + (y0 + y1) * 0.5;
        let hue = (mid_dist / perimeter) * 360.0;
        let color = hsv_to_rgb(hue, 0.95, 1.0, alpha);
        let seg_h = (y1 - y0 + SEAM_OVERLAP).min(h - line_width - y0);
        frame.fill_rectangle(
            Point::new(w - line_width, y0),
            Size::new(line_width, seg_h),
            color,
        );
    }

    // 3. Bottom edge: Right -> Left (distance: w + h -> 2w + h)
    let n_bot = ((w / TARGET_SEGMENT_LEN).ceil() as usize).max(4);
    for i in 0..n_bot {
        let x0 = w - ((i + 1) as f32 * w / n_bot as f32);
        let x1 = w - (i as f32 * w / n_bot as f32);
        let mid_dist = w + h + (i as f32 + 0.5) * (w / n_bot as f32);
        let hue = (mid_dist / perimeter) * 360.0;
        let color = hsv_to_rgb(hue, 0.95, 1.0, alpha);
        let seg_w = (x1 - x0 + SEAM_OVERLAP).min(w - x0);
        frame.fill_rectangle(
            Point::new(x0, h - line_width),
            Size::new(seg_w, line_width),
            color,
        );
    }

    // 4. Left edge: Bottom -> Top (distance: 2w + h -> 2w + 2h)
    let n_left = ((h_inner / TARGET_SEGMENT_LEN).ceil() as usize).max(4);
    for i in 0..n_left {
        let y0 = h - line_width - ((i + 1) as f32 * h_inner / n_left as f32);
        let y1 = h - line_width - (i as f32 * h_inner / n_left as f32);
        let mid_dist = 2.0 * w + h + (i as f32 + 0.5) * (h_inner / n_left as f32);
        let hue = (mid_dist / perimeter) * 360.0;
        let color = hsv_to_rgb(hue, 0.95, 1.0, alpha);
        let seg_h = (y1 - y0 + SEAM_OVERLAP).min(h - line_width - y0);
        frame.fill_rectangle(Point::new(0.0, y0), Size::new(line_width, seg_h), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_to_rgb_color_conversions() {
        // Red
        let red = hsv_to_rgb(0.0, 1.0, 1.0, 1.0);
        assert!((red.r - 1.0).abs() < 1e-4);
        assert!(red.g < 1e-4);
        assert!(red.b < 1e-4);
        assert_eq!(red.a, 1.0);

        // Green
        let green = hsv_to_rgb(120.0, 1.0, 1.0, 0.8);
        assert!(green.r < 1e-4);
        assert!((green.g - 1.0).abs() < 1e-4);
        assert!(green.b < 1e-4);
        assert_eq!(green.a, 0.8);

        // Blue
        let blue = hsv_to_rgb(240.0, 1.0, 1.0, 0.5);
        assert!(blue.r < 1e-4);
        assert!(blue.g < 1e-4);
        assert!((blue.b - 1.0).abs() < 1e-4);
        assert_eq!(blue.a, 0.5);

        // 360 wraps to 0 (Red)
        let wrap = hsv_to_rgb(360.0, 1.0, 1.0, 1.0);
        assert!((wrap.r - 1.0).abs() < 1e-4);
        assert!(wrap.g < 1e-4);

        // Negative angle wrap
        let neg = hsv_to_rgb(-120.0, 1.0, 1.0, 1.0);
        let pos = hsv_to_rgb(240.0, 1.0, 1.0, 1.0);
        assert!((neg.r - pos.r).abs() < 1e-4);
        assert!((neg.g - pos.g).abs() < 1e-4);
        assert!((neg.b - pos.b).abs() < 1e-4);
    }

    #[test]
    fn line_width_scale_factor_calculations() {
        assert_eq!(calculate_line_width(1.0), 1.0);
        assert_eq!(calculate_line_width(1.25), 0.8);
        assert_eq!(calculate_line_width(2.0), 0.5);
        assert_eq!(calculate_line_width(0.0), 1.0);
        assert_eq!(calculate_line_width(-2.0), 1.0);
        assert_eq!(calculate_line_width(f32::NAN), 1.0);
    }

    #[test]
    fn border_effect_lifecycle_and_fade() {
        let t0 = Instant::now();
        let effect = BorderEffect::new(BorderKind::Rainbow, 42, t0);

        // Initially at 100% opacity
        let initial = effect.current_active(t0).unwrap();
        assert_eq!(initial.kind, BorderKind::Rainbow);
        assert!((initial.alpha - 1.0).abs() < 1e-4);
        assert!(!effect.is_expired(t0));

        // Halfway (200ms) -> ~50% opacity
        let halfway = effect
            .current_active(t0 + Duration::from_millis(200))
            .unwrap();
        assert_eq!(halfway.kind, BorderKind::Rainbow);
        assert!((halfway.alpha - 0.5).abs() < 0.05);
        assert!(!effect.is_expired(t0 + Duration::from_millis(200)));

        // At exactly 400ms -> expired
        assert!(
            effect
                .current_active(t0 + Duration::from_millis(400))
                .is_none()
        );
        assert!(effect.is_expired(t0 + Duration::from_millis(400)));

        // Beyond 400ms -> expired
        assert!(
            effect
                .current_active(t0 + Duration::from_millis(600))
                .is_none()
        );
        assert!(effect.is_expired(t0 + Duration::from_millis(600)));
    }

    #[test]
    fn border_calculation_dimensions_and_resizing() {
        // Line width scales inversely with scale factor
        let lw_1x = calculate_line_width(1.0);
        let lw_2x = calculate_line_width(2.0);
        assert_eq!(lw_1x, 1.0);
        assert_eq!(lw_2x, 0.5);

        // Check perimeter calculations for small and large sizes
        let size_small = Size::new(400.0, 300.0);
        let size_large = Size::new(1920.0, 1080.0);
        assert!(size_small.width > lw_1x * 2.0);
        assert!(size_large.width > lw_2x * 2.0);

        // Test boundary clamping on degenerate sizes
        let size_tiny = Size::new(1.0, 1.0);
        assert!(size_tiny.width <= lw_1x * 2.0);
    }
}
