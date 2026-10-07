//! Terminal canvas rendering and geometry calculations.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use iced::alignment::{Horizontal, Vertical};
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Program, Text};
use iced::{Color, Font, Point, Rectangle, Size, Theme};

use crate::font::{TerminalCellMetrics, TerminalFontConfig};
use crate::terminal::session::TerminalSession;
use crate::theme::TerminalColors;

#[allow(dead_code)]
pub const DEFAULT_CELL_WIDTH: f32 = 8.5;
#[allow(dead_code)]
pub const DEFAULT_CELL_HEIGHT: f32 = 18.0;
#[allow(dead_code)]
pub const DEFAULT_FONT_SIZE: f32 = 13.0;

/// Opacity of the selection highlight, drawn over the terminal background.
const SELECTION_ALPHA: f32 = 0.6;

/// Calculates grid columns and rows for a given terminal view size using active cell metrics.
#[allow(dead_code)]
pub fn calculate_grid_size(width: f32, height: f32, metrics: &TerminalCellMetrics) -> (u16, u16) {
    metrics.grid_size(width, height)
}

/// Converts an ANSI terminal color into an iced [`Color`].
///
/// Named and indexed 0–15 colors come from the theme. Explicit RGB colors
/// sent by the application, and the generated 256-color cube and grayscale
/// ramp, are used as-is.
pub fn convert_color(color: AnsiColor, is_fg: bool, colors: &TerminalColors) -> Color {
    let ansi = &colors.ansi;
    match color {
        AnsiColor::Named(named) => match named {
            NamedColor::Black => ansi.black,
            NamedColor::Red => ansi.red,
            NamedColor::Green => ansi.green,
            NamedColor::Yellow => ansi.yellow,
            NamedColor::Blue => ansi.blue,
            NamedColor::Magenta => ansi.magenta,
            NamedColor::Cyan => ansi.cyan,
            NamedColor::White => ansi.white,
            NamedColor::BrightBlack => ansi.bright_black,
            NamedColor::BrightRed => ansi.bright_red,
            NamedColor::BrightGreen => ansi.bright_green,
            NamedColor::BrightYellow => ansi.bright_yellow,
            NamedColor::BrightBlue => ansi.bright_blue,
            NamedColor::BrightMagenta => ansi.bright_magenta,
            NamedColor::BrightCyan => ansi.bright_cyan,
            NamedColor::BrightWhite => ansi.bright_white,
            NamedColor::Foreground => colors.foreground,
            NamedColor::Background => colors.background,
            _ => {
                if is_fg {
                    colors.foreground
                } else {
                    colors.background
                }
            }
        },
        AnsiColor::Spec(Rgb { r, g, b }) => Color::from_rgb8(r, g, b),
        AnsiColor::Indexed(idx) => convert_indexed_color(idx, colors),
    }
}

fn convert_indexed_color(idx: u8, colors: &TerminalColors) -> Color {
    if let Some(color) = colors.ansi.indexed(idx) {
        color
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
    /// The active theme's terminal colors.
    pub colors: &'a TerminalColors,
    /// Active terminal font and cell metrics.
    pub font_config: &'a TerminalFontConfig,
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
        let colors = self.colors;
        let metrics = &self.font_config.metrics;
        let font = self.font_config.font;

        let selection = Color {
            a: SELECTION_ALPHA,
            ..colors.selection
        };

        let mut frame = Frame::new(renderer, bounds.size());

        // Fill background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.background);

        let term = self.session.term().lock();
        let content = term.renderable_content();

        // 1. Draw Cell Backgrounds & Characters
        for cell in content.display_iter {
            let col = cell.point.column.0 as f32;
            let line = cell.point.line.0 as f32;

            let cell_pos = Point::new(col * metrics.cell_width, line * metrics.cell_height);

            let mut fg = convert_color(cell.fg, true, colors);
            let mut bg = convert_color(cell.bg, false, colors);

            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            let is_selected = content.selection.is_some_and(|r| r.contains(cell.point));
            if is_selected {
                bg = selection;
            }

            // Draw custom background if non-default
            if bg != colors.background {
                let cell_width = if cell.flags.contains(Flags::WIDE_CHAR) {
                    metrics.cell_width * 2.0
                } else {
                    metrics.cell_width
                };
                frame.fill_rectangle(cell_pos, Size::new(cell_width, metrics.cell_height), bg);
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
                    size: metrics.font_size.into(),
                    font,
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
            cursor_point.column.0 as f32 * metrics.cell_width,
            cursor_point.line.0 as f32 * metrics.cell_height,
        );

        let cursor_width = metrics.cell_width;
        let cursor_height = metrics.cell_height;

        if self.is_focused {
            frame.fill_rectangle(
                cursor_pos,
                Size::new(cursor_width, cursor_height),
                colors.cursor,
            );
        } else {
            let cursor_path = Path::rectangle(cursor_pos, Size::new(cursor_width, cursor_height));
            frame.stroke(
                &cursor_path,
                canvas::Stroke::default()
                    .with_color(colors.cursor)
                    .with_width(1.0),
            );
        }

        // 3. Draw IME Preedit Text over cursor
        if let Some(preedit) = self.preedit_text
            && !preedit.is_empty()
        {
            let preedit_pos = cursor_pos;
            let preedit_width = (preedit.chars().count() as f32) * metrics.cell_width * 1.5;

            // Preedit background highlight
            frame.fill_rectangle(
                preedit_pos,
                Size::new(preedit_width.max(metrics.cell_width), metrics.cell_height),
                colors.preedit_background,
            );

            // Preedit text
            let text = Text {
                content: preedit.to_string(),
                position: preedit_pos,
                color: colors.preedit_foreground,
                size: metrics.font_size.into(),
                font,
                align_x: Horizontal::Left.into(),
                align_y: Vertical::Top,
                line_height: iced::widget::text::LineHeight::Relative(1.0),
                shaping: iced::widget::text::Shaping::Basic,
                max_width: f32::INFINITY,
            };
            frame.fill_text(text);

            // Preedit underline
            let line_path = Path::line(
                Point::new(preedit_pos.x, preedit_pos.y + metrics.cell_height - 1.0),
                Point::new(
                    preedit_pos.x + preedit_width,
                    preedit_pos.y + metrics.cell_height - 1.0,
                ),
            );
            frame.stroke(
                &line_path,
                canvas::Stroke::default()
                    .with_color(colors.preedit_foreground)
                    .with_width(1.5),
            );
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::NIGHT_DARK;

    const COLORS: &TerminalColors = &NIGHT_DARK.terminal;

    fn named(color: NamedColor, is_fg: bool) -> Color {
        convert_color(AnsiColor::Named(color), is_fg, COLORS)
    }

    fn indexed(idx: u8) -> Color {
        convert_color(AnsiColor::Indexed(idx), true, COLORS)
    }

    #[test]
    fn calculate_grid_size_computes_correct_rows_and_cols() {
        let default_metrics = TerminalCellMetrics::default();
        let (cols, rows) = calculate_grid_size(850.0, 360.0, &default_metrics);
        assert_eq!(cols, 100); // 850 / 8.5 = 100
        assert_eq!(rows, 20); // 360 / 18 = 20

        let custom_metrics = TerminalCellMetrics::new(10.0, 20.0, 13.0);
        let (cols, rows) = calculate_grid_size(1000.0, 400.0, &custom_metrics);
        assert_eq!(cols, 100);
        assert_eq!(rows, 20);
    }

    #[test]
    fn named_colors_come_from_the_theme_palette() {
        assert_eq!(named(NamedColor::Black, true), COLORS.ansi.black);
        assert_eq!(named(NamedColor::Red, true), COLORS.ansi.red);
        assert_eq!(
            named(NamedColor::Red, true),
            Color::from_rgb8(0xcd, 0x31, 0x31)
        );
        assert_eq!(
            named(NamedColor::BrightWhite, false),
            COLORS.ansi.bright_white
        );
    }

    #[test]
    fn default_foreground_and_background_come_from_the_theme() {
        assert_eq!(named(NamedColor::Foreground, true), COLORS.foreground);
        assert_eq!(named(NamedColor::Background, false), COLORS.background);
        // Other named colors (cursor, dim variants) keep falling back by role.
        assert_eq!(named(NamedColor::Cursor, true), COLORS.foreground);
        assert_eq!(named(NamedColor::Cursor, false), COLORS.background);
    }

    #[test]
    fn explicit_rgb_is_never_themed() {
        let spec = AnsiColor::Spec(Rgb {
            r: 10,
            g: 20,
            b: 30,
        });
        assert_eq!(
            convert_color(spec, false, COLORS),
            Color::from_rgb8(10, 20, 30)
        );

        let mut other = *COLORS;
        other.foreground = Color::WHITE;
        other.background = Color::BLACK;
        assert_eq!(
            convert_color(spec, false, &other),
            Color::from_rgb8(10, 20, 30)
        );
    }

    #[test]
    fn indexed_0_to_15_use_the_theme_palette() {
        assert_eq!(indexed(0), COLORS.ansi.black);
        assert_eq!(indexed(1), COLORS.ansi.red);
        assert_eq!(indexed(8), COLORS.ansi.bright_black);
        assert_eq!(indexed(15), COLORS.ansi.bright_white);
    }

    #[test]
    fn indexed_color_cube_is_unchanged() {
        assert_eq!(indexed(16), Color::from_rgb8(0, 0, 0));
        assert_eq!(indexed(17), Color::from_rgb8(0, 0, 95));
        assert_eq!(indexed(196), Color::from_rgb8(255, 0, 0));
        assert_eq!(indexed(231), Color::from_rgb8(255, 255, 255));
    }

    #[test]
    fn indexed_grayscale_is_unchanged() {
        assert_eq!(indexed(232), Color::from_rgb8(8, 8, 8));
        assert_eq!(indexed(244), Color::from_rgb8(128, 128, 128));
        assert_eq!(indexed(255), Color::from_rgb8(238, 238, 238));
    }

    #[test]
    fn conversion_follows_the_supplied_palette() {
        let mut other = *COLORS;
        other.ansi.red = Color::from_rgb8(1, 2, 3);
        other.foreground = Color::from_rgb8(4, 5, 6);

        assert_eq!(
            convert_color(AnsiColor::Named(NamedColor::Red), true, &other),
            Color::from_rgb8(1, 2, 3)
        );
        assert_eq!(
            convert_color(AnsiColor::Indexed(1), true, &other),
            Color::from_rgb8(1, 2, 3)
        );
        assert_eq!(
            convert_color(AnsiColor::Named(NamedColor::Foreground), true, &other),
            Color::from_rgb8(4, 5, 6)
        );
    }

    #[test]
    fn selection_keeps_the_previous_translucent_color() {
        let selection = Color {
            a: SELECTION_ALPHA,
            ..COLORS.selection
        };
        assert_eq!(selection, Color::from_rgba8(0x26, 0x4f, 0x78, 0.6));
    }
}
