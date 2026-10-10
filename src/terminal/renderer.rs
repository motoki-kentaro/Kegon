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
pub fn convert_color(color: AnsiColor, _is_fg: bool, colors: &TerminalColors) -> Color {
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
            NamedColor::DimBlack => dim_color(ansi.black),
            NamedColor::DimRed => dim_color(ansi.red),
            NamedColor::DimGreen => dim_color(ansi.green),
            NamedColor::DimYellow => dim_color(ansi.yellow),
            NamedColor::DimBlue => dim_color(ansi.blue),
            NamedColor::DimMagenta => dim_color(ansi.magenta),
            NamedColor::DimCyan => dim_color(ansi.cyan),
            NamedColor::DimWhite => dim_color(ansi.white),
            NamedColor::Foreground => colors.foreground,
            NamedColor::Background => colors.background,
            NamedColor::BrightForeground => ansi.bright_white,
            NamedColor::DimForeground => dim_color(colors.foreground),
            NamedColor::Cursor => colors.cursor,
        },
        AnsiColor::Spec(Rgb { r, g, b }) => Color::from_rgb8(r, g, b),
        AnsiColor::Indexed(idx) => convert_indexed_color(idx, colors),
    }
}

pub fn dim_color(color: Color) -> Color {
    Color {
        r: color.r * 0.66,
        g: color.g * 0.66,
        b: color.b * 0.66,
        a: color.a,
    }
}

fn is_already_dim_named_color(color: AnsiColor) -> bool {
    matches!(
        color,
        AnsiColor::Named(
            NamedColor::DimBlack
                | NamedColor::DimRed
                | NamedColor::DimGreen
                | NamedColor::DimYellow
                | NamedColor::DimBlue
                | NamedColor::DimMagenta
                | NamedColor::DimCyan
                | NamedColor::DimWhite
                | NamedColor::DimForeground
        )
    )
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

/// Computes effective foreground and background colors for a cell considering ANSI palette,
/// `DIM`, `INVERSE`, and selection status.
pub fn compute_cell_colors(
    cell_fg: AnsiColor,
    cell_bg: AnsiColor,
    flags: Flags,
    is_selected: bool,
    colors: &TerminalColors,
) -> (Color, Color) {
    let mut fg = convert_color(cell_fg, true, colors);
    let mut bg = convert_color(cell_bg, false, colors);

    if flags.contains(Flags::DIM) && !is_already_dim_named_color(cell_fg) {
        fg = dim_color(fg);
    }

    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    if is_selected {
        bg = Color {
            a: SELECTION_ALPHA,
            ..colors.selection
        };
    }

    (fg, bg)
}

/// Derives the iced [`Font`] attributes for a cell based on bold and italic flags.
pub fn resolve_cell_font(base_font: Font, flags: Flags) -> Font {
    let mut cell_font = base_font;
    if flags.contains(Flags::BOLD) {
        cell_font.weight = iced::font::Weight::Bold;
    }
    if flags.contains(Flags::ITALIC) {
        cell_font.style = iced::font::Style::Italic;
    }
    cell_font
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

        let mut frame = Frame::new(renderer, bounds.size());

        // Fill background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.background);

        let term = self.session.term().lock();
        let content = term.renderable_content();
        let cursor_point = content.cursor.point;
        let mut cursor_cell_info: Option<(String, Flags)> = None;

        // 1. Draw Cell Backgrounds, Characters, and Text Line Attributes
        for cell in content.display_iter {
            let col = cell.point.column.0 as f32;
            let line = cell.point.line.0 as f32;

            let cell_pos = Point::new(col * metrics.cell_width, line * metrics.cell_height);

            let is_selected = content.selection.is_some_and(|r| r.contains(cell.point));
            let (fg, bg) = compute_cell_colors(cell.fg, cell.bg, cell.flags, is_selected, colors);

            let is_wide = cell.flags.contains(Flags::WIDE_CHAR);
            let cell_width = if is_wide {
                metrics.cell_width * 2.0
            } else {
                metrics.cell_width
            };

            // Draw custom background if non-default
            if bg != colors.background {
                frame.fill_rectangle(cell_pos, Size::new(cell_width, metrics.cell_height), bg);
            }

            let is_wide_spacer = cell.flags.contains(Flags::WIDE_CHAR_SPACER);
            let is_hidden = cell.flags.contains(Flags::HIDDEN);

            // Draw character text unless wide char spacer or hidden
            if !is_wide_spacer && !is_hidden && cell.c != ' ' && cell.c != '\0' {
                let mut content_str = cell.c.to_string();
                if let Some(zw) = cell.zerowidth() {
                    for &c in zw {
                        content_str.push(c);
                    }
                }

                if cell.point == cursor_point {
                    cursor_cell_info = Some((content_str.clone(), cell.flags));
                }

                let cell_font = resolve_cell_font(font, cell.flags);

                let text = Text {
                    content: content_str,
                    position: cell_pos,
                    color: fg,
                    size: metrics.font_size.into(),
                    font: cell_font,
                    align_x: Horizontal::Left.into(),
                    align_y: Vertical::Top,
                    line_height: iced::widget::text::LineHeight::Absolute(
                        metrics.cell_height.into(),
                    ),
                    shaping: iced::widget::text::Shaping::Advanced,
                    max_width: f32::INFINITY,
                };
                frame.fill_text(text);
            }

            // Draw text line attributes (underline, double underline, strikeout)
            if !is_wide_spacer && !is_hidden {
                if cell.flags.contains(Flags::UNDERLINE) {
                    let line_y = cell_pos.y + metrics.cell_height - 1.5;
                    let line_path = Path::line(
                        Point::new(cell_pos.x, line_y),
                        Point::new(cell_pos.x + cell_width, line_y),
                    );
                    frame.stroke(
                        &line_path,
                        canvas::Stroke::default().with_color(fg).with_width(1.0),
                    );
                } else if cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
                    let line_y1 = cell_pos.y + metrics.cell_height - 2.5;
                    let line_y2 = cell_pos.y + metrics.cell_height - 1.0;
                    let line_path1 = Path::line(
                        Point::new(cell_pos.x, line_y1),
                        Point::new(cell_pos.x + cell_width, line_y1),
                    );
                    let line_path2 = Path::line(
                        Point::new(cell_pos.x, line_y2),
                        Point::new(cell_pos.x + cell_width, line_y2),
                    );
                    frame.stroke(
                        &line_path1,
                        canvas::Stroke::default().with_color(fg).with_width(1.0),
                    );
                    frame.stroke(
                        &line_path2,
                        canvas::Stroke::default().with_color(fg).with_width(1.0),
                    );
                }

                if cell.flags.contains(Flags::STRIKEOUT) {
                    let line_y = cell_pos.y + metrics.cell_height * 0.5;
                    let line_path = Path::line(
                        Point::new(cell_pos.x, line_y),
                        Point::new(cell_pos.x + cell_width, line_y),
                    );
                    frame.stroke(
                        &line_path,
                        canvas::Stroke::default().with_color(fg).with_width(1.0),
                    );
                }
            }
        }

        // 2. Draw Terminal Cursor & Cursor Character
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

            // Re-render character under focused block cursor using contrasting background color
            if let Some((content_str, cell_flags)) = cursor_cell_info {
                let cell_font = resolve_cell_font(font, cell_flags);

                let text = Text {
                    content: content_str,
                    position: cursor_pos,
                    color: colors.background,
                    size: metrics.font_size.into(),
                    font: cell_font,
                    align_x: Horizontal::Left.into(),
                    align_y: Vertical::Top,
                    line_height: iced::widget::text::LineHeight::Absolute(
                        metrics.cell_height.into(),
                    ),
                    shaping: iced::widget::text::Shaping::Advanced,
                    max_width: f32::INFINITY,
                };
                frame.fill_text(text);
            }
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
                line_height: iced::widget::text::LineHeight::Absolute(metrics.cell_height.into()),
                shaping: iced::widget::text::Shaping::Advanced,
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
        assert_eq!(named(NamedColor::Cursor, true), COLORS.cursor);
        assert_eq!(
            named(NamedColor::BrightForeground, true),
            COLORS.ansi.bright_white
        );
    }

    #[test]
    fn dim_named_colors_scale_rgb_components() {
        let dim_red = named(NamedColor::DimRed, true);
        assert_eq!(dim_red.r, COLORS.ansi.red.r * 0.66);
        assert_eq!(dim_red.g, COLORS.ansi.red.g * 0.66);
        assert_eq!(dim_red.b, COLORS.ansi.red.b * 0.66);

        let dim_fg = named(NamedColor::DimForeground, true);
        assert_eq!(dim_fg.r, COLORS.foreground.r * 0.66);
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

    #[test]
    fn compute_cell_colors_handles_dim_inverse_and_selection() {
        let (fg, bg) = compute_cell_colors(
            AnsiColor::Named(NamedColor::Foreground),
            AnsiColor::Named(NamedColor::Background),
            Flags::empty(),
            false,
            COLORS,
        );
        assert_eq!(fg, COLORS.foreground);
        assert_eq!(bg, COLORS.background);

        // Dim scales foreground RGB components while keeping alpha intact
        let (dim_fg, _) = compute_cell_colors(
            AnsiColor::Named(NamedColor::Foreground),
            AnsiColor::Named(NamedColor::Background),
            Flags::DIM,
            false,
            COLORS,
        );
        assert_eq!(dim_fg.r, COLORS.foreground.r * 0.66);
        assert_eq!(dim_fg.g, COLORS.foreground.g * 0.66);
        assert_eq!(dim_fg.b, COLORS.foreground.b * 0.66);
        assert_eq!(dim_fg.a, COLORS.foreground.a);

        // DimRed with Flags::DIM is not double dimmed
        let (dim_red_fg, _) = compute_cell_colors(
            AnsiColor::Named(NamedColor::DimRed),
            AnsiColor::Named(NamedColor::Background),
            Flags::DIM,
            false,
            COLORS,
        );
        assert_eq!(dim_red_fg.r, COLORS.ansi.red.r * 0.66);

        // Inverse swaps fg and bg
        let (inv_fg, inv_bg) = compute_cell_colors(
            AnsiColor::Named(NamedColor::Foreground),
            AnsiColor::Named(NamedColor::Background),
            Flags::INVERSE,
            false,
            COLORS,
        );
        assert_eq!(inv_fg, COLORS.background);
        assert_eq!(inv_bg, COLORS.foreground);

        // Selection overrides bg
        let (_, sel_bg) = compute_cell_colors(
            AnsiColor::Named(NamedColor::Foreground),
            AnsiColor::Named(NamedColor::Background),
            Flags::empty(),
            true,
            COLORS,
        );
        assert_eq!(
            sel_bg,
            Color {
                a: SELECTION_ALPHA,
                ..COLORS.selection
            }
        );
    }

    #[test]
    fn resolve_cell_font_applies_bold_and_italic_attributes() {
        let base = Font::MONOSPACE;
        let font_normal = resolve_cell_font(base, Flags::empty());
        assert_eq!(font_normal.weight, iced::font::Weight::Normal);
        assert_eq!(font_normal.style, iced::font::Style::Normal);

        let font_bold = resolve_cell_font(base, Flags::BOLD);
        assert_eq!(font_bold.weight, iced::font::Weight::Bold);
        assert_eq!(font_bold.style, iced::font::Style::Normal);

        let font_italic = resolve_cell_font(base, Flags::ITALIC);
        assert_eq!(font_italic.weight, iced::font::Weight::Normal);
        assert_eq!(font_italic.style, iced::font::Style::Italic);

        let font_bold_italic = resolve_cell_font(base, Flags::BOLD | Flags::ITALIC);
        assert_eq!(font_bold_italic.weight, iced::font::Weight::Bold);
        assert_eq!(font_bold_italic.style, iced::font::Style::Italic);
    }
}
