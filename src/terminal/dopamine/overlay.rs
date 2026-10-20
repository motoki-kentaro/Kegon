//! Canvas overlay that draws the neon particles above the terminal text.
//!
//! iced's canvas has no additive blending, so light is approximated: a translucent halo under a
//! bright core, and a tapered tail for fast particles. At most three fills per particle.

use iced::mouse;
use iced::widget::canvas::{Frame, Geometry, Path, Program};
use iced::{Color, Point, Rectangle};

use super::border::{ActiveBorder, paint_border};
use super::gauge::{TabGaugeDisplay, TabTransferManager};
use super::params::VISUAL;
use super::particles::{Look, Particle, ParticleKind};
use std::time::Instant;

pub struct ParticleOverlayProgram<'a> {
    particles: &'a [Particle],
    border: Option<ActiveBorder>,
    gauge: Option<&'a TabGaugeDisplay>,
    transfer: Option<&'a TabTransferManager>,
    burst: Option<(&'a super::burst::Burst, f32, iced::Font)>,
    now: Instant,
    scale_factor: f32,
}

impl<'a> ParticleOverlayProgram<'a> {
    pub fn with_burst(
        mut self,
        burst: Option<&'a super::burst::Burst>,
        size: f32,
        font: iced::Font,
    ) -> Self {
        self.burst = burst.map(|b| (b, size, font));
        self
    }

    pub fn new(
        particles: &'a [Particle],
        border: Option<ActiveBorder>,
        gauge: Option<&'a TabGaugeDisplay>,
        transfer: Option<&'a TabTransferManager>,
        now: Instant,
        scale_factor: f32,
    ) -> Self {
        Self {
            burst: None,
            particles,
            border,
            gauge,
            transfer,
            now,
            scale_factor,
        }
    }
}

fn faded(mut color: Color, factor: f32) -> Color {
    color.a = (color.a * factor).clamp(0.0, 1.0);
    color
}

/// The streak as one tapered triangle: wide at the head, a point at the tail.
fn tail_path(head: Point, tail: Point, half_width: f32) -> Path {
    let (dx, dy) = (head.x - tail.x, head.y - tail.y);
    let length = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
    let (nx, ny) = (-dy / length * half_width, dx / length * half_width);
    Path::new(|b| {
        b.move_to(tail);
        b.line_to(Point::new(head.x + nx, head.y + ny));
        b.line_to(Point::new(head.x - nx, head.y - ny));
        b.close();
    })
}

fn paint(frame: &mut Frame, particle: &Particle, look: &Look) {
    let at = particle.pos;
    match particle.kind {
        ParticleKind::Flash => {
            // A bloom of three nested discs: wide and faint, then tight and hot.
            frame.fill(&Path::circle(at, look.radius), faded(look.glow, 0.16));
            frame.fill(
                &Path::circle(at, look.radius * 0.55),
                faded(look.glow, 0.34),
            );
            frame.fill(&Path::circle(at, look.radius * 0.25), faded(look.core, 0.9));
        }
        ParticleKind::Spark | ParticleKind::Mote => {
            if let Some(streak) = look.streak {
                frame.fill(
                    &tail_path(at, streak.tail, streak.half_width),
                    faded(look.glow, 0.85),
                );
            }
            frame.fill(
                &Path::circle(at, look.radius * VISUAL.halo_scale),
                faded(look.glow, look.halo_alpha),
            );
            frame.fill(
                &Path::circle(at, look.radius * VISUAL.core_scale.max(0.3)),
                look.core,
            );
        }
    }
}

impl<'a, Message> Program<Message> for ParticleOverlayProgram<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let has_particles = !self.particles.is_empty();
        let has_border = self.border.is_some();
        let has_gauge = self.gauge.is_some_and(|g| g.is_visible());
        let has_transfer = self.transfer.is_some_and(|t| t.is_active());

        if !has_particles && !has_border && !has_gauge && !has_transfer && self.burst.is_none() {
            return Vec::new();
        }

        let mut frame = Frame::new(renderer, bounds.size());

        if let Some(border) = self.border
            && border.alpha > 0.001
        {
            paint_border(&mut frame, bounds.size(), border, self.scale_factor);
        }

        if let Some(gauge) = self.gauge {
            gauge.paint(&mut frame, bounds.size());
        }

        if let Some(transfer) = self.transfer {
            transfer.paint(&mut frame, self.now);
        }

        if has_particles {
            for particle in self.particles {
                let look = particle.look(bounds.height);
                if look.alpha <= 0.01 {
                    continue;
                }
                paint(&mut frame, particle, &look);
            }
        }

        if let Some((burst, size, font)) = self.burst {
            burst.paint(&mut frame, bounds.size(), self.now, size, font);
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod burst_input_tests {
    use super::*;
    #[test]
    fn burst_canvas_never_captures_keyboard_mouse_or_ime() {
        let now = Instant::now();
        let burst = super::super::burst::Burst::new(now, &mut super::super::burst::Rng(1));
        let program = ParticleOverlayProgram::new(&[], None, None, None, now, 1.0).with_burst(
            Some(&burst),
            24.0,
            iced::Font::MONOSPACE,
        );
        for event in [
            iced::Event::InputMethod(iced::advanced::input_method::Event::Commit("日本語".into())),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Character("x".into()),
                modified_key: iced::keyboard::Key::Character("x".into()),
                physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyX),
                location: iced::keyboard::Location::Standard,
                modifiers: iced::keyboard::Modifiers::empty(),
                text: Some("x".into()),
                repeat: false,
            }),
        ] {
            let action = <ParticleOverlayProgram<'_> as Program<()>>::update(
                &program,
                &mut (),
                &event,
                Rectangle::new(Point::ORIGIN, iced::Size::new(800.0, 600.0)),
                mouse::Cursor::Unavailable,
            );
            assert!(action.is_none());
        }
    }
}
