//! Terminal canvas rendering and geometry calculations.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use iced::alignment::{Horizontal, Vertical};
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Program, Text};
use iced::{Color, Font, Point, Rectangle, Size, Theme};

use crate::terminal::session::TerminalSession;

pub const DEFAULT_CELL_WIDTH: f32 = 8.5;
pub const DEFAULT_CELL_HEIGHT: f32 = 18.0;
pub const DEFAULT_FONT_SIZE: f32 = 13.0;

pub const PALETTE_BG: Color = Color::from_rgb8(0x1e, 0x1e, 0x1e);
pub const PALETTE_FG: Color = Color::from_rgb8(0xcc, 0xcc, 0xcc);
pub const PALETTE_SELECTION: Color = Color::from_rgba8(0x26, 0x4f, 0x78, 0.6);
pub const PALETTE_CURSOR: Color = Color::from_rgb8(0xae, 0xaf, 0xad);

/// Calculates grid columns and rows for a given terminal view size.
pub fn calculate_grid_size(width: f32, height: f32) -> (u16, u16) {
    let cols = (width / DEFAULT_CELL_WIDTH).floor().max(1.0) as u16;
    let rows = (height / DEFAULT_CELL_HEIGHT).floor().max(1.0) as u16;
    (cols, rows)
}

/// Converts an ANSI terminal color into an iced [`Color`].
pub fn convert_color(color: AnsiColor, is_fg: bool) -> Color {
    match color {
        AnsiColor::Named(named) => match named {
            NamedColor::Black => Color::from_rgb8(0x1e, 0x1e, 0x1e),
            NamedColor::Red => Color::from_rgb8(0xcd, 0x31, 0x31),
            NamedColor::Green => Color::from_rgb8(0x0d, 0xbc, 0x79),
            NamedColor::Yellow => Color::from_rgb8(0xe5, 0xe5, 0x10),
            NamedColor::Blue => Color::from_rgb8(0x24, 0x72, 0xc8),
            NamedColor::Magenta => Color::from_rgb8(0xbc, 0x3f, 0xbc),
            NamedColor::Cyan => Color::from_rgb8(0x11, 0xa8, 0xcd),
            NamedColor::White => Color::from_rgb8(0xe5, 0xe5, 0xe5),
            NamedColor::BrightBlack => Color::from_rgb8(0x66, 0x66, 0x66),
            NamedColor::BrightRed => Color::from_rgb8(0xf1, 0x4c, 0x4c),
            NamedColor::BrightGreen => Color::from_rgb8(0x23, 0xd1, 0x8b),
            NamedColor::BrightYellow => Color::from_rgb8(0xf5, 0xf5, 0x43),
            NamedColor::BrightBlue => Color::from_rgb8(0x3b, 0x8e, 0xe8),
            NamedColor::BrightMagenta => Color::from_rgb8(0xd6, 0x70, 0xd6),
            NamedColor::BrightCyan => Color::from_rgb8(0x29, 0xb8, 0xdb),
            NamedColor::BrightWhite => Color::from_rgb8(0xff, 0xff, 0xff),
            NamedColor::Foreground => PALETTE_FG,
            NamedColor::Background => PALETTE_BG,
            _ => {
                if is_fg {
                    PALETTE_FG
                } else {
                    PALETTE_BG
                }
            }
        },
        AnsiColor::Spec(Rgb { r, g, b }) => Color::from_rgb8(r, g, b),
        AnsiColor::Indexed(idx) => convert_indexed_color(idx),
    }
}

fn convert_indexed_color(idx: u8) -> Color {
    if idx < 16 {
        let named = match idx {
            0 => NamedColor::Black,
            1 => NamedColor::Red,
            2 => NamedColor::Green,
            3 => NamedColor::Yellow,
            4 => NamedColor::Blue,
            5 => NamedColor::Magenta,
            6 => NamedColor::Cyan,
            7 => NamedColor::White,
            8 => NamedColor::BrightBlack,
            9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue,
            13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan,
            _ => NamedColor::BrightWhite,
        };
        convert_color(AnsiColor::Named(named), true)
    } else if idx < 232 {
        let i = idx - 16;
        let r = (i / 36) % 6;
        let g = (i / 6) % 6;
        let b = i % 6;
        let steps = [0, 95, 135, 175, 215, 255];
        Color::from_rgb8(steps[r as usize], steps[g as usize], steps[b as usize])
    } else {
        let gray = 8 + (idx - 232) * 10;
        Color::from_rgb8(gray, gray, gray)
    }
}

/// The iced [`Program`] responsible for rendering the terminal grid and preedit text.
pub struct TerminalProgram<'a> {
    pub session: &'a TerminalSession,
    pub preedit_text: Option<&'a str>,
    pub is_focused: bool,
}

impl<'a, Message> Program<Message> for TerminalProgram<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        // Fill background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), PALETTE_BG);

        let term = self.session.term().lock();
        let content = term.renderable_content();

        // 1. Draw Cell Backgrounds & Characters
        for cell in content.display_iter {
            let col = cell.point.column.0 as f32;
            let line = cell.point.line.0 as f32;

            let cell_pos = Point::new(col * DEFAULT_CELL_WIDTH, line * DEFAULT_CELL_HEIGHT);

            let mut fg = convert_color(cell.fg, true);
            let mut bg = convert_color(cell.bg, false);

            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            let is_selected = content.selection.is_some_and(|r| r.contains(cell.point));
            if is_selected {
                bg = PALETTE_SELECTION;
            }

            // Draw custom background if non-default
            if bg != PALETTE_BG {
                let cell_width = if cell.flags.contains(Flags::WIDE_CHAR) {
                    DEFAULT_CELL_WIDTH * 2.0
                } else {
                    DEFAULT_CELL_WIDTH
                };
                frame.fill_rectangle(cell_pos, Size::new(cell_width, DEFAULT_CELL_HEIGHT), bg);
            }

            // Draw character text unless wide char spacer or hidden
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                && !cell.flags.contains(Flags::HIDDEN)
                && cell.c != ' '
                && cell.c != '\0'
            {
                let text = Text {
                    content: cell.c.to_string(),
                    position: cell_pos,
                    color: fg,
                    size: DEFAULT_FONT_SIZE.into(),
                    font: Font::MONOSPACE,
                    align_x: Horizontal::Left.into(),
                    align_y: Vertical::Top,
                    line_height: iced::widget::text::LineHeight::Relative(1.0),
                    shaping: iced::widget::text::Shaping::Basic,
                    max_width: f32::INFINITY,
                };
                frame.fill_text(text);
            }
        }

        // 2. Draw Terminal Cursor
        let cursor_point = content.cursor.point;
        let cursor_pos = Point::new(
            cursor_point.column.0 as f32 * DEFAULT_CELL_WIDTH,
            cursor_point.line.0 as f32 * DEFAULT_CELL_HEIGHT,
        );

        let cursor_width = DEFAULT_CELL_WIDTH;
        let cursor_height = DEFAULT_CELL_HEIGHT;

        if self.is_focused {
            frame.fill_rectangle(
                cursor_pos,
                Size::new(cursor_width, cursor_height),
                PALETTE_CURSOR,
            );
        } else {
            let cursor_path = Path::rectangle(cursor_pos, Size::new(cursor_width, cursor_height));
            frame.stroke(
                &cursor_path,
                canvas::Stroke::default()
                    .with_color(PALETTE_CURSOR)
                    .with_width(1.0),
            );
        }

        // 3. Draw IME Preedit Text over cursor
        if let Some(preedit) = self.preedit_text
            && !preedit.is_empty()
        {
            let preedit_pos = cursor_pos;
            let preedit_width = (preedit.chars().count() as f32) * DEFAULT_CELL_WIDTH * 1.5;

            // Preedit background highlight
            frame.fill_rectangle(
                preedit_pos,
                Size::new(preedit_width.max(DEFAULT_CELL_WIDTH), DEFAULT_CELL_HEIGHT),
                Color::from_rgb8(0x3a, 0x3d, 0x41),
            );

            // Preedit text
            let text = Text {
                content: preedit.to_string(),
                position: preedit_pos,
                color: Color::WHITE,
                size: DEFAULT_FONT_SIZE.into(),
                font: Font::MONOSPACE,
                align_x: Horizontal::Left.into(),
                align_y: Vertical::Top,
                line_height: iced::widget::text::LineHeight::Relative(1.0),
                shaping: iced::widget::text::Shaping::Basic,
                max_width: f32::INFINITY,
            };
            frame.fill_text(text);

            // Preedit underline
            let line_path = Path::line(
                Point::new(preedit_pos.x, preedit_pos.y + DEFAULT_CELL_HEIGHT - 1.0),
                Point::new(
                    preedit_pos.x + preedit_width,
                    preedit_pos.y + DEFAULT_CELL_HEIGHT - 1.0,
                ),
            );
            frame.stroke(
                &line_path,
                canvas::Stroke::default()
                    .with_color(Color::WHITE)
                    .with_width(1.5),
            );
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_grid_size_computes_correct_rows_and_cols() {
        let (cols, rows) = calculate_grid_size(850.0, 360.0);
        assert_eq!(cols, 100); // 850 / 8.5 = 100
        assert_eq!(rows, 20); // 360 / 18 = 20
    }

    #[test]
    fn convert_color_handles_ansi_and_rgb() {
        let fg = convert_color(AnsiColor::Named(NamedColor::Red), true);
        assert_eq!(fg, Color::from_rgb8(0xcd, 0x31, 0x31));

        let spec = convert_color(
            AnsiColor::Spec(Rgb {
                r: 10,
                g: 20,
                b: 30,
            }),
            false,
        );
        assert_eq!(spec, Color::from_rgb8(10, 20, 30));
    }
}
