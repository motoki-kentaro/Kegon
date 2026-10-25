//! Terminal canvas rendering and geometry calculations.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use std::cell::RefCell;

use iced::advanced::svg::{Handle as SvgHandle, Svg};
use iced::alignment::Vertical;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Program, Text};
use iced::{Color, Font, Point, Rectangle, Size, Theme};

use crate::font::{TerminalCellMetrics, TerminalFontConfig};
use crate::terminal::cluster::{CellRole, RunLookup, detect_screen_runs, run_covering};
use crate::terminal::glyph::{self, CellGrid};
use crate::terminal::session::TerminalSession;
use crate::terminal::tile_cache::TileCache;
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

/// The [`Text`] every terminal text path draws with: a cell, the character under the cursor, and the
/// IME preedit.
///
/// The alignment is deliberately `Alignment::Default`, never an explicit `Left`. With an explicit
/// alignment iced's `text::align` skips the relayout a right-to-left line needs, and cosmic-text, laying
/// the line out against the unbounded `max_width`, starts it at x = infinity: Hebrew and Arabic cells
/// then draw nothing (#75). `Default` still puts left-to-right text at the left edge.
fn terminal_text(
    content: String,
    position: Point,
    color: Color,
    font: Font,
    metrics: &TerminalCellMetrics,
) -> Text {
    Text {
        content,
        position,
        color,
        size: metrics.font_size.into(),
        font,
        align_x: iced::advanced::text::Alignment::Default,
        align_y: Vertical::Top,
        line_height: iced::widget::text::LineHeight::Absolute(metrics.cell_height.into()),
        shaping: iced::widget::text::Shaping::Advanced,
        max_width: f32::INFINITY,
    }
}

/// Paints the rectangles of a built-in glyph.
fn fill_glyph_rects(frame: &mut Frame, rects: &[glyph::Rect], color: Color) {
    for rect in rects {
        frame.fill_rectangle(
            Point::new(rect.x0, rect.y0),
            Size::new(rect.width(), rect.height()),
            color,
        );
    }
}

/// Paints the curves and diagonals of a built-in glyph.
///
/// Every piece lies inside its cell (see `glyph::glyph_paths`), so nothing is clipped. Curves are
/// stroked with flat ends and diagonals are filled polygons; both are antialiased like any canvas path.
fn fill_glyph_paths(frame: &mut Frame, paths: &glyph::GlyphPaths, color: Color) {
    let point = |(x, y): glyph::Point2| Point::new(x, y);
    for shape in paths.as_slice() {
        match shape {
            glyph::PathShape::Curve {
                from,
                ctrl1,
                ctrl2,
                to,
                width,
            } => {
                let path = Path::new(|builder| {
                    builder.move_to(point(*from));
                    builder.bezier_curve_to(point(*ctrl1), point(*ctrl2), point(*to));
                });
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(*width)
                        .with_color(color)
                        .with_line_cap(canvas::LineCap::Butt),
                );
            }
            glyph::PathShape::Polygon { .. } => {
                let points = shape.polygon_points();
                if let Some((first, rest)) = points.split_first()
                    && rest.len() >= 2
                {
                    let path = Path::new(|builder| {
                        builder.move_to(point(*first));
                        for p in rest {
                            builder.line_to(point(*p));
                        }
                        builder.close();
                    });
                    frame.fill(&path, color);
                }
            }
        }
    }
}

thread_local! {
    /// The SVG tiles of the shade characters, shared by every terminal view (it is keyed by what a tile
    /// looks like, not by tab) and bounded; see `tile_cache`.
    static SHADE_TILES: RefCell<TileCache<SvgHandle>> = RefCell::new(TileCache::default());
}

/// The built-in drawing for a cell, or `None` when the font draws it: always `None` while the built-in
/// glyphs are off, for a cell with combining marks, and for any character they do not cover.
fn builtin_for(
    enabled: bool,
    ch: char,
    has_combining_marks: bool,
    grid: &CellGrid,
    col: i32,
    line: i32,
) -> Option<glyph::BuiltinGlyph> {
    if !enabled || has_combining_marks {
        return None;
    }
    glyph::builtin_glyph(ch, grid, col, line)
}

/// Paints a built-in glyph with `color`, whichever method it uses.
fn paint_builtin(frame: &mut Frame, builtin: &glyph::BuiltinGlyph, scale: f32, color: Color) {
    match builtin {
        glyph::BuiltinGlyph::Rects(rects) => fill_glyph_rects(frame, rects.as_slice(), color),
        glyph::BuiltinGlyph::Paths(paths) => fill_glyph_paths(frame, paths, color),
        glyph::BuiltinGlyph::Shade(shade) => fill_shade(frame, shade, scale, color),
    }
}

/// Drops the cached shade tiles, for when no shade will be drawn (the built-in glyphs were turned off).
pub fn clear_shade_tiles() {
    SHADE_TILES.with(|tiles| tiles.borrow_mut().clear());
}

/// Paints a shade cell: the SVG tile of its pattern, tinted with `color`.
///
/// The tile is one SVG unit per physical pixel and is placed on the cell's physically aligned bounds, so
/// the rasteriser produces it 1:1 and the dots land on the pixels the model says. The bounds are a hair
/// short of the cell (a thousandth of a physical pixel): the rasteriser sizes the image with
/// `ceil(scale × size)`, which float rounding would otherwise push one pixel too far.
///
/// Images are composited above all meshes of the frame. A selection tint or a cursor block painted as a
/// rectangle therefore lies beneath the dots, like the text of an ordinary cell, and the dots drawn in the
/// cursor's contrast color cover exactly the pixels of the dots beneath them.
fn fill_shade(frame: &mut Frame, cell: &glyph::ShadeCell, scale: f32, color: Color) {
    let handle = SHADE_TILES.with(|tiles| {
        let key = cell.key();
        tiles
            .borrow_mut()
            .get_or_insert_with(key, || SvgHandle::from_memory(key.svg().into_bytes()))
    });
    let trim = 1e-3 / scale;
    frame.draw_svg(
        Rectangle::new(
            Point::new(cell.bounds.x0, cell.bounds.y0),
            Size::new(cell.bounds.width() - trim, cell.bounds.height() - trim),
        ),
        Svg::new(handle).color(color),
    );
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
    /// Window scale factor (physical pixels per logical pixel), for physical-pixel alignment.
    pub scale_factor: f32,
    /// Draw the box-drawing and block characters (U+2500..U+259F) with built-in shapes instead of font
    /// glyphs, and place cell background rectangles on the same physically aligned edges so shapes and
    /// backgrounds share them. Off draws everything as before, with the font and unaligned backgrounds.
    pub builtin_glyphs: bool,
    /// Render-only Screen Shake translation (logical px) applied to the cell content. Layout,
    /// PTY size and input coordinates are unaffected; the background keeps covering the canvas.
    pub shake: iced::Vector,
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
        SHADE_TILES.with(|tiles| tiles.borrow_mut().begin_frame());
        let colors = self.colors;
        let metrics = &self.font_config.metrics;
        let font = self.font_config.font;

        let mut frame = Frame::new(renderer, bounds.size());
        let grid = CellGrid::new(
            (bounds.x, bounds.y),
            metrics.cell_width,
            metrics.cell_height,
            self.scale_factor,
        );

        // Fill background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.background);
        if self.shake != iced::Vector::ZERO {
            frame.translate(self.shake);
        }

        let term = self.session.term().lock();
        let content = term.renderable_content();
        let cursor_point = content.cursor.point;
        let mut cursor_cell_info: Option<(String, Flags)> = None;
        // Where the cursor character is redrawn from: the run start when the cursor is inside a run.
        let mut cursor_text_origin: Option<Point> = None;

        // Emoji sequences the grid spreads over several cells are drawn as one `Text` from their first
        // cell (#74). Detection is render-only; the grid and the logical cursor are never touched.
        let selection = content.selection;
        let runs = detect_screen_runs(term.grid().display_iter().map(|c| {
            let selected = selection.is_some_and(|r| r.contains(c.point));
            (c.point.line.0, c.point.column.0, c.cell, selected)
        }));
        let mut run_lookup = RunLookup::new(&runs);

        // 1. Draw Cell Backgrounds, Characters, and Text Line Attributes
        for cell in content.display_iter {
            let col_index = cell.point.column.0 as i32;
            let line_index = cell.point.line.0;
            let col = col_index as f32;
            let line = line_index as f32;

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
                if self.builtin_glyphs {
                    let left = grid.cell(col_index, line_index);
                    let right = grid.x_edge(col_index + if is_wide { 2 } else { 1 });
                    frame.fill_rectangle(
                        Point::new(left.x0, left.y0),
                        Size::new(right - left.x0, left.height()),
                        bg,
                    );
                } else {
                    frame.fill_rectangle(cell_pos, Size::new(cell_width, metrics.cell_height), bg);
                }
            }

            let is_wide_spacer = cell.flags.contains(Flags::WIDE_CHAR_SPACER);
            let is_hidden = cell.flags.contains(Flags::HIDDEN);
            let run_role = run_lookup.role(line_index, cell.point.column.0);

            // Draw character text unless wide char spacer or hidden. A later cell of a run has no text
            // of its own: the run's first cell drew all of it.
            if let CellRole::Start(run) = run_role
                && !is_wide_spacer
                && !is_hidden
            {
                frame.fill_text(terminal_text(
                    run.text.clone(),
                    cell_pos,
                    fg,
                    resolve_cell_font(font, run.flags),
                    metrics,
                ));
            } else if matches!(run_role, CellRole::Plain)
                && !is_wide_spacer
                && !is_hidden
                && cell.c != ' '
                && cell.c != '\0'
            {
                let mut content_str = cell.c.to_string();
                if let Some(zw) = cell.zerowidth() {
                    for &c in zw {
                        content_str.push(c);
                    }
                }

                if cell.point == cursor_point {
                    cursor_cell_info = Some((content_str.clone(), cell.flags));
                }

                // Built-in shapes are exact to the cell, so bold or italic never changes them. A cell
                // with combining marks stays with the font.
                let builtin = builtin_for(
                    self.builtin_glyphs,
                    cell.c,
                    cell.zerowidth().is_some(),
                    &grid,
                    col_index,
                    line_index,
                );
                if let Some(builtin) = builtin {
                    paint_builtin(&mut frame, &builtin, grid.scale, fg);
                } else {
                    let cell_font = resolve_cell_font(font, cell.flags);

                    frame.fill_text(terminal_text(content_str, cell_pos, fg, cell_font, metrics));
                }
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

        // A cursor inside a run keeps the run whole: its background is the logical cell, and the run's
        // text is redrawn once from the run start, exactly as a single cell's character is.
        if let Some(run) = run_covering(&runs, cursor_point.line.0, cursor_point.column.0)
            && !run.flags.contains(Flags::HIDDEN)
        {
            cursor_cell_info = Some((run.text.clone(), run.flags));
            cursor_text_origin = Some(Point::new(
                run.start_col as f32 * metrics.cell_width,
                cursor_point.line.0 as f32 * metrics.cell_height,
            ));
        }

        // 2. Draw Terminal Cursor & Cursor Character
        let cursor_pos = Point::new(
            cursor_point.column.0 as f32 * metrics.cell_width,
            cursor_point.line.0 as f32 * metrics.cell_height,
        );

        let cursor_width = metrics.cell_width;
        let cursor_height = metrics.cell_height;

        // A built-in shape under the cursor is redrawn from the same aligned edges as the cell.
        let cursor_col = cursor_point.column.0 as i32;
        let cursor_line = cursor_point.line.0;
        let cursor_builtin = cursor_cell_info.as_ref().and_then(|(content, _)| {
            let mut chars = content.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None; // a cell with combining marks stays with the font
            }
            builtin_for(
                self.builtin_glyphs,
                ch,
                false,
                &grid,
                cursor_col,
                cursor_line,
            )
        });

        if self.is_focused && cursor_builtin.is_some() {
            let cell = grid.cell(cursor_col, cursor_line);
            frame.fill_rectangle(
                Point::new(cell.x0, cell.y0),
                Size::new(cell.width(), cell.height()),
                colors.cursor,
            );
            if let Some(builtin) = cursor_builtin {
                paint_builtin(&mut frame, &builtin, grid.scale, colors.background);
            }
        } else if self.is_focused {
            frame.fill_rectangle(
                cursor_pos,
                Size::new(cursor_width, cursor_height),
                colors.cursor,
            );

            // Re-render character under focused block cursor using contrasting background color
            if let Some((content_str, cell_flags)) = cursor_cell_info {
                let cell_font = resolve_cell_font(font, cell_flags);

                frame.fill_text(terminal_text(
                    content_str,
                    cursor_text_origin.unwrap_or(cursor_pos),
                    colors.background,
                    cell_font,
                    metrics,
                ));
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
            frame.fill_text(terminal_text(
                preedit.to_string(),
                preedit_pos,
                colors.preedit_foreground,
                font,
                metrics,
            ));

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

    fn grid() -> CellGrid {
        CellGrid::new((0.0, 0.0), 9.6, 19.2, 1.0)
    }

    #[test]
    fn with_the_setting_on_every_one_of_the_160_characters_is_drawn_by_exactly_one_builtin_method()
    {
        let grid = grid();
        let (mut rects, mut paths, mut shades) = (0, 0, 0);
        for cp in 0x2500u32..=0x259F {
            let ch = char::from_u32(cp).unwrap();
            match builtin_for(true, ch, false, &grid, 2, 3) {
                Some(glyph::BuiltinGlyph::Rects(_)) => rects += 1,
                Some(glyph::BuiltinGlyph::Paths(_)) => paths += 1,
                Some(glyph::BuiltinGlyph::Shade(_)) => shades += 1,
                None => panic!("{cp:04X} falls back to the font"),
            }
        }
        assert_eq!((rects, paths, shades), (150, 7, 3));
    }

    #[test]
    fn with_the_setting_off_every_character_is_drawn_by_the_font() {
        let grid = grid();
        for cp in 0x2500u32..=0x259F {
            let ch = char::from_u32(cp).unwrap();
            assert!(
                builtin_for(false, ch, false, &grid, 2, 3).is_none(),
                "{cp:04X}"
            );
        }
    }

    #[test]
    fn other_characters_and_cells_with_combining_marks_stay_with_the_font() {
        let grid = grid();
        for ch in ['a', ' ', '\u{3042}', '\u{25A0}', '\u{E0B0}', '\u{24FF}'] {
            assert!(
                builtin_for(true, ch, false, &grid, 0, 0).is_none(),
                "{ch:?}"
            );
        }
        assert!(builtin_for(true, '\u{2588}', true, &grid, 0, 0).is_none());
        assert!(builtin_for(true, '\u{2588}', false, &grid, 0, 0).is_some());
    }

    /// Lays out `text` the way iced does for a terminal [`Text`] (the cache's buffer setup and
    /// `text::align`), and returns the x of every glyph, or `None` when there is no font to lay out with.
    fn laid_out_glyph_xs(text: &Text) -> Option<Vec<f32>> {
        use gtext::cosmic_text as ct;
        use iced::advanced::graphics::text as gtext;

        let mut fs = ct::FontSystem::new();
        if fs.db().is_empty() {
            return None;
        }
        let mut buffer = ct::Buffer::new(&mut fs, ct::Metrics::new(f32::from(text.size), 18.0));
        buffer.set_size(&mut fs, Some(text.max_width), Some(f32::INFINITY.min(1e9)));
        buffer.set_text(
            &mut fs,
            &text.content,
            &gtext::to_attributes(text.font),
            gtext::to_shaping(text.shaping, &text.content),
            None,
        );
        gtext::align(&mut buffer, &mut fs, text.align_x);
        Some(
            buffer
                .layout_runs()
                .flat_map(|run| run.glyphs.iter().map(|g| g.x).collect::<Vec<_>>())
                .collect(),
        )
    }

    fn sample_text(content: &str) -> Text {
        let metrics = TerminalCellMetrics::default();
        terminal_text(
            content.to_string(),
            Point::ORIGIN,
            Color::WHITE,
            Font::MONOSPACE,
            &metrics,
        )
    }

    #[test]
    fn terminal_text_never_pins_an_explicit_alignment() {
        let text = sample_text("A");
        assert_eq!(text.align_x, iced::advanced::text::Alignment::Default);
        assert_eq!(text.max_width, f32::INFINITY);
    }

    #[test]
    fn terminal_text_keeps_every_script_glyph_at_a_finite_x() {
        let samples = [
            ("ascii", "A"),
            ("japanese", "日本語"),
            ("armenian", "ԱԲԳ"),
            ("hebrew single", "א"),
            ("hebrew", "שלום"),
            ("arabic single", "ا"),
            ("arabic", "العربية"),
            ("persian", "فارسی"),
            ("thai", "สวัสดี"),
            ("devanagari", "नमस्ते"),
        ];
        for (name, content) in samples {
            let Some(xs) = laid_out_glyph_xs(&sample_text(content)) else {
                eprintln!("no system fonts; layout check skipped");
                return;
            };
            assert!(!xs.is_empty(), "{name}: no glyphs were laid out");
            assert!(
                xs.iter().all(|x| x.is_finite() && *x >= 0.0),
                "{name}: glyph x must be finite, got {xs:?}"
            );
        }
    }

    #[test]
    fn explicit_left_alignment_would_push_rtl_glyphs_to_infinity() {
        // Documents the failure the helper avoids; if iced fixes it, this test can go.
        let mut text = sample_text("שלום");
        text.align_x = iced::advanced::text::Alignment::Left;
        let Some(xs) = laid_out_glyph_xs(&text) else {
            return;
        };
        assert!(xs.iter().any(|x| !x.is_finite()), "{xs:?}");
    }

    #[test]
    fn left_to_right_text_starts_at_the_left_edge() {
        let Some(xs) = laid_out_glyph_xs(&sample_text("A")) else {
            return;
        };
        assert_eq!(xs, vec![0.0]);
    }
}
