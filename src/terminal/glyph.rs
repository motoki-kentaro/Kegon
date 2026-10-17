//! Built-in drawing of terminal glyphs that must tile exactly with the cell grid.
//!
//! Block elements are drawn as rectangles derived from the cell geometry instead of from a font's
//! glyphs, so they fill the cell edge to edge whatever the font, size or scale is. This module is a
//! pure model with no GUI dependency: it maps a character and a cell to rectangles, and the renderer
//! paints them with the cell's foreground color.
//!
//! Cell edges are derived from the *index* of the edge (`origin + index × size`, then snapped to the
//! physical pixel grid), never by adding up rounded cell sizes. Two neighbouring cells therefore
//! compute the shared edge from the same number and meet exactly, with no gap or overlap.

/// An axis-aligned rectangle in the renderer's logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
}

/// The cell grid: where every cell edge falls, in the logical coordinates of the drawing surface.
///
/// `origin` is the surface's position in the window. Snapping happens on absolute window
/// coordinates, because the physical pixel grid belongs to the window, not to the surface (a surface
/// that starts at a fractional physical pixel must not snap its own local coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellGrid {
    pub origin_x: f32,
    pub origin_y: f32,
    pub cell_width: f32,
    pub cell_height: f32,
    /// Window scale factor: physical pixels per logical pixel.
    pub scale: f32,
}

impl CellGrid {
    /// A grid with sane inputs; a non-finite or non-positive scale counts as 1.
    pub fn new(origin: (f32, f32), cell_width: f32, cell_height: f32, scale: f32) -> Self {
        Self {
            origin_x: origin.0,
            origin_y: origin.1,
            cell_width,
            cell_height,
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
        }
    }

    /// Snaps an absolute logical coordinate to the nearest physical pixel boundary.
    fn snap_abs(&self, value: f32) -> f32 {
        (value * self.scale).round() / self.scale
    }

    /// Left edge of column `col`, relative to the surface origin.
    pub fn x_edge(&self, col: i32) -> f32 {
        self.snap_abs(self.origin_x + col as f32 * self.cell_width) - self.origin_x
    }

    /// Top edge of row `line`, relative to the surface origin.
    pub fn y_edge(&self, line: i32) -> f32 {
        self.snap_abs(self.origin_y + line as f32 * self.cell_height) - self.origin_y
    }

    /// The cell at (`col`, `line`) with physically aligned edges.
    pub fn cell(&self, col: i32, line: i32) -> Rect {
        Rect {
            x0: self.x_edge(col),
            y0: self.y_edge(line),
            x1: self.x_edge(col + 1),
            y1: self.y_edge(line + 1),
        }
    }

    /// The boundary between the two halves of `cell`, on the physical pixel grid.
    fn mid_x(&self, cell: Rect) -> f32 {
        self.snap_abs(self.origin_x + (cell.x0 + cell.x1) / 2.0) - self.origin_x
    }

    fn mid_y(&self, cell: Rect) -> f32 {
        self.snap_abs(self.origin_y + (cell.y0 + cell.y1) / 2.0) - self.origin_y
    }

    /// Thickness of a line style in logical px: a whole number of physical pixels.
    ///
    /// A light line is one sixteenth of the cell height, rounded to physical pixels, and never thinner
    /// than one physical pixel: `max(1, round(cell_height_px / 16))`. So a 13 px cell has 1 px lines, a
    /// 24 px cell 2 px lines, and the line is as thick on a 150% display as the same cell would need
    /// there, not thinner for being measured in logical pixels.
    pub fn stroke(&self, style: LineStyle) -> f32 {
        match style {
            LineStyle::None => 0.0,
            LineStyle::Light => {
                ((self.cell_height * self.scale / 16.0).round().max(1.0)) / self.scale
            }
        }
    }

    /// The vertical band `[x0, x1]` of a vertical line of thickness `stroke` through `cell`: centred on
    /// the cell, snapped to a physical pixel, and kept inside the cell. It depends only on the cell's
    /// column edges, so all cells of a column agree.
    fn band_x(&self, cell: Rect, stroke: f32) -> (f32, f32) {
        let start =
            self.snap_abs(self.origin_x + (cell.x0 + cell.x1) / 2.0 - stroke / 2.0) - self.origin_x;
        let start = start.min(cell.x1 - stroke).max(cell.x0);
        (start, (start + stroke).min(cell.x1))
    }

    /// The horizontal band `[y0, y1]` of a horizontal line; see [`Self::band_x`].
    fn band_y(&self, cell: Rect, stroke: f32) -> (f32, f32) {
        let start =
            self.snap_abs(self.origin_y + (cell.y0 + cell.y1) / 2.0 - stroke / 2.0) - self.origin_y;
        let start = start.min(cell.y1 - stroke).max(cell.y0);
        (start, (start + stroke).min(cell.y1))
    }
}

/// The rectangles a glyph is made of (never more than [`MAX_RECTS`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRects {
    rects: [Rect; MAX_RECTS],
    len: usize,
}

/// Room for the most rectangles any glyph needs: a light junction (┼, ├, ┤, ┬, ┴) is at most three.
pub const MAX_RECTS: usize = 4;

impl GlyphRects {
    fn single(rect: Rect) -> Self {
        let mut rects = [rect; MAX_RECTS];
        rects[0] = rect;
        Self { rects, len: 1 }
    }

    fn empty() -> Self {
        let zero = Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        };
        Self {
            rects: [zero; MAX_RECTS],
            len: 0,
        }
    }

    fn push(&mut self, rect: Rect) {
        debug_assert!(self.len < MAX_RECTS);
        if self.len < MAX_RECTS && rect.width() > 0.0 && rect.height() > 0.0 {
            self.rects[self.len] = rect;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[Rect] {
        &self.rects[..self.len]
    }
}

/// How one side of a box-drawing cell is drawn. Only light lines exist so far; heavy and double lines
/// become further variants (with their own thickness) without changing the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    None,
    Light,
}

/// The line style that leaves a box-drawing cell through each of its four sides.
///
/// This is the single source of truth for a character's shape: the rectangles are built from it,
/// never from per-character coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connections {
    pub left: LineStyle,
    pub right: LineStyle,
    pub up: LineStyle,
    pub down: LineStyle,
}

impl Connections {
    /// A cell whose listed sides carry a light line.
    const fn light(left: bool, right: bool, up: bool, down: bool) -> Self {
        const fn side(on: bool) -> LineStyle {
            if on {
                LineStyle::Light
            } else {
                LineStyle::None
            }
        }
        Self {
            left: side(left),
            right: side(right),
            up: side(up),
            down: side(down),
        }
    }

    fn has_horizontal(&self) -> bool {
        self.left != LineStyle::None || self.right != LineStyle::None
    }
}

/// The connections of a supported box-drawing character, from the directions Unicode names:
/// U+2500 LIGHT HORIZONTAL … U+253C LIGHT VERTICAL AND HORIZONTAL (the light, solid, square subset).
pub fn box_connections(ch: char) -> Option<Connections> {
    //                              left   right  up     down
    Some(match ch {
        '\u{2500}' => Connections::light(true, true, false, false), // ─
        '\u{2502}' => Connections::light(false, false, true, true), // │
        '\u{250C}' => Connections::light(false, true, false, true), // ┌ down and right
        '\u{2510}' => Connections::light(true, false, false, true), // ┐ down and left
        '\u{2514}' => Connections::light(false, true, true, false), // └ up and right
        '\u{2518}' => Connections::light(true, false, true, false), // ┘ up and left
        '\u{251C}' => Connections::light(false, true, true, true),  // ├ vertical and right
        '\u{2524}' => Connections::light(true, false, true, true),  // ┤ vertical and left
        '\u{252C}' => Connections::light(true, true, false, true),  // ┬ down and horizontal
        '\u{2534}' => Connections::light(true, true, true, false),  // ┴ up and horizontal
        '\u{253C}' => Connections::light(true, true, true, true),   // ┼
        _ => return None,
    })
}

/// Whether `ch` is drawn by this module.
pub fn is_builtin(ch: char) -> bool {
    is_block(ch) || box_connections(ch).is_some()
}

fn is_block(ch: char) -> bool {
    matches!(
        ch,
        '\u{2588}' | '\u{2580}' | '\u{2584}' | '\u{258C}' | '\u{2590}'
    )
}

/// The rectangles of `ch` in the cell at (`col`, `line`), or `None` when the font should draw it.
///
/// The full block, the four half blocks, and the light box-drawing lines.
pub fn glyph_rects(ch: char, grid: &CellGrid, col: i32, line: i32) -> Option<GlyphRects> {
    if !is_builtin(ch) {
        return None;
    }
    if let Some(connections) = box_connections(ch) {
        return Some(box_rects(&connections, grid, grid.cell(col, line)));
    }
    if !is_block(ch) {
        return None;
    }
    let cell = grid.cell(col, line);
    let rect = match ch {
        '\u{2588}' => cell,
        '\u{2580}' => Rect {
            y1: grid.mid_y(cell),
            ..cell
        },
        '\u{2584}' => Rect {
            y0: grid.mid_y(cell),
            ..cell
        },
        '\u{258C}' => Rect {
            x1: grid.mid_x(cell),
            ..cell
        },
        '\u{2590}' => Rect {
            x0: grid.mid_x(cell),
            ..cell
        },
        _ => return None,
    };
    Some(GlyphRects::single(rect))
}

/// Builds the rectangles of a box-drawing cell from its connections.
///
/// The horizontal band depends only on the cell's row edges and the vertical band only on its column
/// edges. Every cell of a row therefore draws its horizontal line at exactly the same height, and every
/// cell of a column its vertical line at exactly the same x, however the individual cells were rounded;
/// and lines run to the cell edges, which a neighbour computes from the same edge index. That is what
/// keeps a line unbroken across cell boundaries.
///
/// The rectangles never overlap: a horizontal run covers the junction, the vertical arms stop at it.
fn box_rects(connections: &Connections, grid: &CellGrid, cell: Rect) -> GlyphRects {
    let stroke = grid.stroke(LineStyle::Light);
    let (vx0, vx1) = grid.band_x(cell, stroke);
    let (hy0, hy1) = grid.band_y(cell, stroke);
    let on = |style: LineStyle| style != LineStyle::None;
    let (left, right) = (on(connections.left), on(connections.right));
    let (up, down) = (on(connections.up), on(connections.down));

    let mut rects = GlyphRects::empty();
    if connections.has_horizontal() {
        rects.push(Rect {
            x0: if left { cell.x0 } else { vx0 },
            y0: hy0,
            x1: if right { cell.x1 } else { vx1 },
            y1: hy1,
        });
        if up {
            rects.push(Rect {
                x0: vx0,
                y0: cell.y0,
                x1: vx1,
                y1: hy0,
            });
        }
        if down {
            rects.push(Rect {
                x0: vx0,
                y0: hy1,
                x1: vx1,
                y1: cell.y1,
            });
        }
    } else if up || down {
        rects.push(Rect {
            x0: vx0,
            y0: if up { cell.y0 } else { hy0 },
            x1: vx1,
            y1: if down { cell.y1 } else { hy1 },
        });
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: char = '\u{2588}';
    const UPPER: char = '\u{2580}';
    const LOWER: char = '\u{2584}';
    const LEFT: char = '\u{258C}';
    const RIGHT: char = '\u{2590}';

    fn grid(w: f32, h: f32, scale: f32) -> CellGrid {
        CellGrid::new((0.0, 0.0), w, h, scale)
    }

    fn rect_of(ch: char, grid: &CellGrid, col: i32, line: i32) -> Rect {
        let rects = glyph_rects(ch, grid, col, line).expect("builtin glyph");
        assert_eq!(rects.as_slice().len(), 1);
        rects.as_slice()[0]
    }

    /// Each cell size and scale the PoC measured, plus integer and awkward ones.
    const CASES: [(f32, f32, f32); 12] = [
        (6.5, 13.0, 1.0),
        (6.5, 13.0, 1.25),
        (6.5, 13.0, 1.5),
        (9.6, 19.2, 1.0),
        (9.6, 19.2, 1.25),
        (9.6, 19.2, 1.5),
        (10.0, 19.0, 1.0),
        (10.0, 19.0, 1.25),
        (10.0, 19.0, 1.5),
        (7.617, 15.107, 1.0),
        (7.147, 15.222, 1.25),
        (8.5, 18.0, 1.75),
    ];

    #[test]
    fn exactly_the_five_blocks_and_eleven_light_lines_are_built_in() {
        for ch in [FULL, UPPER, LOWER, LEFT, RIGHT] {
            assert!(is_builtin(ch), "{ch:?}");
            assert!(glyph_rects(ch, &grid(8.0, 16.0, 1.0), 0, 0).is_some());
        }
        for ch in BOX {
            assert!(is_builtin(ch), "{ch:?}");
            assert!(glyph_rects(ch, &grid(8.0, 16.0, 1.0), 0, 0).is_some());
        }
        // Everything else keeps using the font: other blocks, shades, heavy / double / dashed /
        // rounded / diagonal lines, mixed-weight lines, Powerline and ordinary text.
        for ch in [
            'a', ' ', '\u{2591}', '\u{2592}', '\u{2593}', '\u{2581}', '\u{2589}', '\u{2596}',
            '\u{2501}', '\u{2503}', '\u{250F}', '\u{2550}', '\u{2551}', '\u{2504}', '\u{254C}',
            '\u{256D}', '\u{2571}', '\u{253D}', '\u{2574}', '\u{E0B0}', '\u{3042}', '\0',
        ] {
            assert!(!is_builtin(ch), "{ch:?}");
            assert!(
                glyph_rects(ch, &grid(8.0, 16.0, 1.0), 0, 0).is_none(),
                "{ch:?}"
            );
        }
    }

    #[test]
    fn full_block_is_the_whole_aligned_cell() {
        let g = grid(8.0, 16.0, 1.0);
        assert_eq!(
            rect_of(FULL, &g, 3, 2),
            Rect {
                x0: 24.0,
                y0: 32.0,
                x1: 32.0,
                y1: 48.0
            }
        );
    }

    #[test]
    fn half_blocks_cover_the_expected_half_on_even_cells() {
        let g = grid(8.0, 16.0, 1.0);
        let cell = g.cell(1, 1);
        assert_eq!(rect_of(UPPER, &g, 1, 1), Rect { y1: 24.0, ..cell });
        assert_eq!(rect_of(LOWER, &g, 1, 1), Rect { y0: 24.0, ..cell });
        assert_eq!(rect_of(LEFT, &g, 1, 1), Rect { x1: 12.0, ..cell });
        assert_eq!(rect_of(RIGHT, &g, 1, 1), Rect { x0: 12.0, ..cell });
    }

    #[test]
    fn opposite_half_blocks_are_complementary_and_tile_the_cell() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            for (col, line) in [(0, 0), (1, 1), (7, 3), (135, 54)] {
                let cell = g.cell(col, line);
                let (up, lo) = (rect_of(UPPER, &g, col, line), rect_of(LOWER, &g, col, line));
                let (le, ri) = (rect_of(LEFT, &g, col, line), rect_of(RIGHT, &g, col, line));
                // Shared boundary is one value, so the halves meet exactly and span the whole cell.
                assert_eq!(up.y1, lo.y0, "{w}x{h}@{s} cell {col},{line}");
                assert_eq!((up.y0, lo.y1), (cell.y0, cell.y1));
                assert_eq!(le.x1, ri.x0, "{w}x{h}@{s} cell {col},{line}");
                assert_eq!((le.x0, ri.x1), (cell.x0, cell.x1));
                // Full-width / full-height halves.
                assert_eq!((up.x0, up.x1), (cell.x0, cell.x1));
                assert_eq!((le.y0, le.y1), (cell.y0, cell.y1));
                // Halves stay inside the cell and are never empty.
                for r in [up, lo, le, ri] {
                    assert!(r.width() > 0.0 && r.height() > 0.0, "{w}x{h}@{s}: {r:?}");
                }
            }
        }
    }

    #[test]
    fn neighbouring_cells_share_their_edge_exactly_without_gap_or_overlap() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            for col in 0..140 {
                let (a, b) = (rect_of(FULL, &g, col, 0), rect_of(FULL, &g, col + 1, 0));
                assert_eq!(a.x1, b.x0, "{w}x{h}@{s} col {col}");
            }
            for line in 0..60 {
                let (a, b) = (rect_of(FULL, &g, 0, line), rect_of(FULL, &g, 0, line + 1));
                assert_eq!(a.y1, b.y0, "{w}x{h}@{s} line {line}");
            }
        }
    }

    #[test]
    fn edges_do_not_drift_by_accumulating_rounded_cell_sizes() {
        // 136 cells of 6.5 px must end exactly at 884 px, not at 136 × round(6.5).
        let g = grid(6.5, 13.0, 1.0);
        assert_eq!(g.x_edge(136), 884.0);
        assert_eq!(g.y_edge(55), 715.0);
        // The same holds with fractional physical sizes.
        let g = grid(9.6, 19.2, 1.25);
        assert!((g.x_edge(100) - 960.0).abs() <= 0.5 / 1.25 + 1e-4);
    }

    #[test]
    fn edges_sit_on_physical_pixels_at_every_scale() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            for i in 0..140 {
                for edge in [g.x_edge(i), g.y_edge(i)] {
                    let phys = edge * s;
                    assert!(
                        (phys - phys.round()).abs() < 1e-3,
                        "{w}x{h}@{s}: {edge} → {phys}"
                    );
                }
            }
        }
    }

    #[test]
    fn alignment_follows_the_absolute_window_position_of_the_surface() {
        // A surface starting at y = 35 logical is at 43.75 physical px at 125%: not a pixel boundary.
        let g = CellGrid::new((312.0, 35.0), 9.6, 19.2, 1.25);
        for i in 0..50 {
            let abs_y = (g.origin_y + g.y_edge(i)) * 1.25;
            assert!((abs_y - abs_y.round()).abs() < 1e-3, "row {i}: {abs_y}");
            let abs_x = (g.origin_x + g.x_edge(i)) * 1.25;
            assert!((abs_x - abs_x.round()).abs() < 1e-3, "col {i}: {abs_x}");
        }
        // Neighbours still share the edge.
        let (a, b) = (g.cell(4, 4), g.cell(5, 5));
        assert_eq!((a.x1, a.y1), (b.x0, b.y0));
    }

    #[test]
    fn fractional_cells_give_cells_within_one_physical_pixel_of_the_ideal_size() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            for i in 0..100 {
                let c = g.cell(i, i);
                let tolerance = 1.0 / s + 1e-3;
                assert!((c.width() - w).abs() <= tolerance, "{w}x{h}@{s} {c:?}");
                assert!((c.height() - h).abs() <= tolerance, "{w}x{h}@{s} {c:?}");
            }
        }
    }

    #[test]
    fn halves_split_within_half_a_physical_pixel_of_the_middle() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            let cell = g.cell(5, 5);
            let tolerance = 0.5 / s + 1e-3;
            let up = rect_of(UPPER, &g, 5, 5);
            let le = rect_of(LEFT, &g, 5, 5);
            assert!(
                (up.y1 - (cell.y0 + cell.y1) / 2.0).abs() <= tolerance,
                "{w}x{h}@{s}"
            );
            assert!(
                (le.x1 - (cell.x0 + cell.x1) / 2.0).abs() <= tolerance,
                "{w}x{h}@{s}"
            );
        }
    }

    #[test]
    fn a_bad_scale_counts_as_one() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let g = CellGrid::new((0.0, 0.0), 6.5, 13.0, bad);
            assert_eq!(g.scale, 1.0);
            assert_eq!(g.x_edge(2), 13.0);
        }
    }

    #[test]
    fn the_rects_are_finite_for_extreme_cells() {
        for (w, h) in [(1.0, 1.0), (0.2 * 1.3333, 0.5 * 1.3333), (200.0, 300.0)] {
            let g = grid(w, h, 1.5);
            for ch in [FULL, UPPER, LOWER, LEFT, RIGHT] {
                let r = rect_of(ch, &g, 3, 3);
                assert!(
                    r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()
                );
                assert!(r.x1 >= r.x0 && r.y1 >= r.y0);
            }
        }
    }
    // ---- light box-drawing lines ----

    const BOX: [char; 11] = [
        '\u{2500}', '\u{2502}', '\u{250C}', '\u{2510}', '\u{2514}', '\u{2518}', '\u{251C}',
        '\u{2524}', '\u{252C}', '\u{2534}', '\u{253C}',
    ];

    /// (left, right, up, down) of every supported character, as the Unicode names give them.
    const EXPECTED: [(char, [bool; 4]); 11] = [
        ('\u{2500}', [true, true, false, false]), // ─ LIGHT HORIZONTAL
        ('\u{2502}', [false, false, true, true]), // │ LIGHT VERTICAL
        ('\u{250C}', [false, true, false, true]), // ┌ LIGHT DOWN AND RIGHT
        ('\u{2510}', [true, false, false, true]), // ┐ LIGHT DOWN AND LEFT
        ('\u{2514}', [false, true, true, false]), // └ LIGHT UP AND RIGHT
        ('\u{2518}', [true, false, true, false]), // ┘ LIGHT UP AND LEFT
        ('\u{251C}', [false, true, true, true]),  // ├ LIGHT VERTICAL AND RIGHT
        ('\u{2524}', [true, false, true, true]),  // ┤ LIGHT VERTICAL AND LEFT
        ('\u{252C}', [true, true, false, true]),  // ┬ LIGHT DOWN AND HORIZONTAL
        ('\u{2534}', [true, true, true, false]),  // ┴ LIGHT UP AND HORIZONTAL
        ('\u{253C}', [true, true, true, true]),   // ┼ LIGHT VERTICAL AND HORIZONTAL
    ];

    fn sides(c: Connections) -> [bool; 4] {
        let on = |s: LineStyle| s != LineStyle::None;
        [on(c.left), on(c.right), on(c.up), on(c.down)]
    }

    #[test]
    fn every_character_maps_to_the_connections_its_unicode_name_gives() {
        for (ch, expected) in EXPECTED {
            let c = box_connections(ch).unwrap_or_else(|| panic!("{ch:?} unsupported"));
            assert_eq!(sides(c), expected, "{ch:?}");
            for side in [c.left, c.right, c.up, c.down] {
                assert!(matches!(side, LineStyle::None | LineStyle::Light));
            }
        }
        for ch in [
            'a', '\u{2501}', '\u{2550}', '\u{250F}', '\u{2591}', '\u{2588}',
        ] {
            assert!(box_connections(ch).is_none(), "{ch:?}");
        }
    }

    /// Physical-pixel footprint of a glyph: which pixels have their centre inside a rectangle.
    /// Rectangle edges sit on the physical grid, so rounding is exact.
    struct Raster {
        w: usize,
        h: usize,
        px: Vec<u8>,
    }

    impl Raster {
        fn new(w: usize, h: usize) -> Self {
            Self {
                w,
                h,
                px: vec![0; w * h],
            }
        }

        fn paint(&mut self, r: &Rect, scale: f32) {
            let (x0, x1) = ((r.x0 * scale).round() as i64, (r.x1 * scale).round() as i64);
            let (y0, y1) = ((r.y0 * scale).round() as i64, (r.y1 * scale).round() as i64);
            for y in y0.max(0)..y1.min(self.h as i64) {
                for x in x0.max(0)..x1.min(self.w as i64) {
                    self.px[y as usize * self.w + x as usize] += 1;
                }
            }
        }

        fn at(&self, x: usize, y: usize) -> u8 {
            self.px[y * self.w + x]
        }
    }

    /// Paints `rows` of box characters (a character grid) and returns the raster.
    fn draw(rows: &[&str], g: &CellGrid) -> Raster {
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap() as i32;
        let w = (g.x_edge(cols) * g.scale).round() as usize + 2;
        let h = (g.y_edge(rows.len() as i32) * g.scale).round() as usize + 2;
        let mut raster = Raster::new(w, h);
        for (line, row) in rows.iter().enumerate() {
            for (col, ch) in row.chars().enumerate() {
                if let Some(rects) = glyph_rects(ch, g, col as i32, line as i32) {
                    for r in rects.as_slice() {
                        raster.paint(r, g.scale);
                    }
                }
            }
        }
        raster
    }

    const FRAME: [&str; 5] = [
        "┌──────────────┬──────────────┐",
        "│              │              │",
        "├──────────────┼──────────────┤",
        "│              │              │",
        "└──────────────┴──────────────┘",
    ];

    #[test]
    fn rectangles_of_one_glyph_never_overlap_and_stay_in_the_cell() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            for ch in BOX {
                for (col, line) in [(0, 0), (3, 2), (17, 9)] {
                    let cell = g.cell(col, line);
                    let rects = glyph_rects(ch, &g, col, line).unwrap();
                    assert!(!rects.as_slice().is_empty() && rects.as_slice().len() <= 3);
                    let mut raster = Raster::new(
                        (g.x_edge(col + 2) * s).round() as usize,
                        (g.y_edge(line + 2) * s).round() as usize,
                    );
                    for r in rects.as_slice() {
                        assert!(
                            r.x0 >= cell.x0 && r.x1 <= cell.x1,
                            "{ch:?} {w}x{h}@{s}: {r:?} outside {cell:?}"
                        );
                        assert!(
                            r.y0 >= cell.y0 && r.y1 <= cell.y1,
                            "{ch:?} {w}x{h}@{s}: {r:?} outside {cell:?}"
                        );
                        assert!(r.width() > 0.0 && r.height() > 0.0);
                        raster.paint(r, s);
                    }
                    assert!(
                        raster.px.iter().all(|&n| n <= 1),
                        "{ch:?} {w}x{h}@{s} overlaps"
                    );
                }
            }
        }
    }

    #[test]
    fn a_line_only_reaches_the_sides_it_connects_to() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            for (ch, [left, right, up, down]) in EXPECTED {
                let cell = g.cell(4, 4);
                let rects = glyph_rects(ch, &g, 4, 4).unwrap();
                let touches = |f: &dyn Fn(&Rect) -> bool| rects.as_slice().iter().any(f);
                assert_eq!(
                    touches(&|r| r.x0 == cell.x0),
                    left,
                    "{ch:?} left {w}x{h}@{s}"
                );
                assert_eq!(
                    touches(&|r| r.x1 == cell.x1),
                    right,
                    "{ch:?} right {w}x{h}@{s}"
                );
                assert_eq!(touches(&|r| r.y0 == cell.y0), up, "{ch:?} up {w}x{h}@{s}");
                assert_eq!(
                    touches(&|r| r.y1 == cell.y1),
                    down,
                    "{ch:?} down {w}x{h}@{s}"
                );
            }
        }
    }

    #[test]
    fn stroke_is_a_whole_number_of_physical_pixels_and_at_least_one() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            let phys = g.stroke(LineStyle::Light) * s;
            assert!(phys >= 1.0 - 1e-4, "{w}x{h}@{s}: {phys}");
            assert!((phys - phys.round()).abs() < 1e-3, "{w}x{h}@{s}: {phys}");
        }
        assert_eq!(grid(6.5, 13.0, 1.0).stroke(LineStyle::Light), 1.0);
        assert_eq!(grid(9.6, 19.2, 1.0).stroke(LineStyle::Light), 1.0);
        assert_eq!(grid(9.6, 19.2, 1.25).stroke(LineStyle::Light) * 1.25, 2.0);
        assert_eq!(grid(12.0, 24.0, 1.0).stroke(LineStyle::Light), 2.0);
        assert_eq!(grid(1.0, 1.0, 1.0).stroke(LineStyle::Light), 1.0);
        assert_eq!(grid(8.0, 16.0, 1.0).stroke(LineStyle::None), 0.0);
    }

    #[test]
    fn stroke_follows_the_cell_height_and_the_display_scale() {
        // Same logical cell, denser display: the line is not thinner in physical pixels.
        let at = |scale| grid(9.6, 19.2, scale).stroke(LineStyle::Light) * scale;
        assert!(at(1.5) >= at(1.0));
        // A taller cell never gets a thinner line.
        let mut last = 0.0;
        for h in [8.0_f32, 13.0, 16.0, 19.2, 24.0, 32.0, 48.0] {
            let t = grid(9.6, h, 1.0).stroke(LineStyle::Light);
            assert!(t >= last, "h {h}");
            last = t;
        }
    }

    #[test]
    fn a_horizontal_line_is_unbroken_and_equally_thick_across_many_cells() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            let row = ["─".repeat(140)];
            let raster = draw(&[row[0].as_str()], &g);
            let t = (g.stroke(LineStyle::Light) * s).round() as usize;
            let total = (g.x_edge(140) * s).round() as usize;
            // Rows that carry the line: the same rows, every pixel, from the first cell to the last.
            let band: Vec<usize> = (0..raster.h)
                .filter(|&y| (0..total).all(|x| raster.at(x, y) == 1))
                .collect();
            assert_eq!(band.len(), t, "{w}x{h}@{s}: thickness");
            assert!(
                band.windows(2).all(|p| p[1] == p[0] + 1),
                "{w}x{h}@{s}: contiguous"
            );
            for y in 0..raster.h {
                if !band.contains(&y) {
                    assert!(
                        (0..raster.w).all(|x| raster.at(x, y) == 0),
                        "{w}x{h}@{s}: stray row {y}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_vertical_line_is_unbroken_and_equally_thick_across_many_rows() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            let rows: Vec<&str> = std::iter::repeat_n("│", 60).collect();
            let raster = draw(&rows, &g);
            let t = (g.stroke(LineStyle::Light) * s).round() as usize;
            let total = (g.y_edge(60) * s).round() as usize;
            let band: Vec<usize> = (0..raster.w)
                .filter(|&x| (0..total).all(|y| raster.at(x, y) == 1))
                .collect();
            assert_eq!(band.len(), t, "{w}x{h}@{s}: thickness");
            assert!(
                band.windows(2).all(|p| p[1] == p[0] + 1),
                "{w}x{h}@{s}: contiguous"
            );
        }
    }

    #[test]
    fn a_frame_with_corners_tees_and_a_cross_is_one_connected_figure() {
        for (w, h, s) in CASES {
            let g = grid(w, h, s);
            let raster = draw(&FRAME, &g);
            assert!(raster.px.iter().all(|&n| n <= 1), "{w}x{h}@{s}: overlap");

            // Expected figure, derived independently of the glyph code: horizontal bands at the
            // three row centres spanning the whole frame, vertical bands at the three column
            // centres spanning the whole frame (the frame's two outer edges and the middle divider).
            let t = (g.stroke(LineStyle::Light) * s).round() as i64;
            let total_w = (g.x_edge(FRAME[0].chars().count() as i32) * s).round() as i64;
            let total_h = (g.y_edge(FRAME.len() as i32) * s).round() as i64;
            let centre_band = |lo: f32, hi: f32, origin: f32| -> i64 {
                let c = (lo + hi) / 2.0 - g.stroke(LineStyle::Light) / 2.0;
                (((origin + c) * s).round() - origin * s) as i64
            };
            let row_bands: Vec<i64> = [0, 2, 4]
                .iter()
                .map(|&r| centre_band(g.y_edge(r), g.y_edge(r + 1), g.origin_y))
                .collect();
            let col_bands: Vec<i64> = [0, 15, 30]
                .iter()
                .map(|&c| centre_band(g.x_edge(c), g.x_edge(c + 1), g.origin_x))
                .collect();
            // Horizontal lines are unbroken over the whole frame width on every one of their rows.
            for &y0 in &row_bands {
                for dy in 0..t {
                    let y = (y0 + dy) as usize;
                    let x_start = col_bands[0] as usize;
                    let x_end = (col_bands[2] + t) as usize;
                    for x in x_start..x_end {
                        assert_eq!(raster.at(x, y), 1, "{w}x{h}@{s}: hole at ({x},{y})");
                    }
                }
            }
            // Vertical lines are unbroken from the top line to the bottom line.
            for &x0 in &col_bands {
                for dx in 0..t {
                    let x = (x0 + dx) as usize;
                    let y_start = row_bands[0] as usize;
                    let y_end = (row_bands[2] + t) as usize;
                    for y in y_start..y_end {
                        assert_eq!(raster.at(x, y), 1, "{w}x{h}@{s}: hole at ({x},{y})");
                    }
                }
            }
            // And nothing else is drawn: the pixel count equals the figure's area.
            let drawn: usize = raster.px.iter().map(|&n| n as usize).sum();
            let (nh, nv) = (row_bands.len() as i64, col_bands.len() as i64);
            let horiz = nh * t * (col_bands[2] + t - col_bands[0]);
            let vert = nv * t * (row_bands[2] + t - row_bands[0]);
            let crossings = nh * nv * t * t;
            assert_eq!(drawn as i64, horiz + vert - crossings, "{w}x{h}@{s}: area");
            let _ = (total_w, total_h);
        }
    }

    #[test]
    fn odd_and_even_physical_cell_widths_both_connect() {
        // 9 and 10 physical px wide cells at 100%, and alternating 12 / 13 at 125%.
        for (w, h, s) in [
            (9.0, 17.0, 1.0),
            (10.0, 18.0, 1.0),
            (9.6, 19.2, 1.25),
            (10.0, 19.0, 1.25),
        ] {
            let g = grid(w, h, s);
            let widths: std::collections::HashSet<i64> = (0..40)
                .map(|c| ((g.x_edge(c + 1) - g.x_edge(c)) * s).round() as i64)
                .collect();
            let raster = draw(&["┼".repeat(40).as_str(), "┼".repeat(40).as_str()], &g);
            let t = (g.stroke(LineStyle::Light) * s).round() as usize;
            let total = (g.x_edge(40) * s).round() as usize;
            let band_rows = (0..raster.h)
                .filter(|&y| (0..total).all(|x| raster.at(x, y) == 1))
                .count();
            assert_eq!(band_rows, 2 * t, "{w}x{h}@{s} widths {widths:?}");
        }
    }

    #[test]
    fn lines_follow_the_absolute_window_position_of_the_surface() {
        // The tab strip puts the surface at y = 35, a fractional physical pixel at 125%.
        let g = CellGrid::new((312.0, 35.0), 9.6, 19.2, 1.25);
        for ch in BOX {
            for (col, line) in [(0, 0), (5, 3), (20, 11)] {
                for r in glyph_rects(ch, &g, col, line).unwrap().as_slice() {
                    for (v, origin) in [
                        (r.x0, g.origin_x),
                        (r.x1, g.origin_x),
                        (r.y0, g.origin_y),
                        (r.y1, g.origin_y),
                    ] {
                        let phys = (origin + v) * 1.25;
                        assert!((phys - phys.round()).abs() < 1e-3, "{ch:?}: {v} → {phys}");
                    }
                }
            }
        }
        // And neighbouring cells still share the line's band.
        let (a, b) = (
            glyph_rects('─', &g, 3, 4).unwrap().as_slice()[0],
            glyph_rects('─', &g, 4, 4).unwrap().as_slice()[0],
        );
        assert_eq!((a.y0, a.y1), (b.y0, b.y1));
        assert_eq!(a.x1, b.x0);
    }

    #[test]
    fn tiny_cells_still_produce_finite_clamped_rectangles() {
        for (w, h) in [
            (1.0, 1.0),
            (2.0, 2.0),
            (0.2 * 1.3333, 0.5 * 1.3333),
            (200.0, 300.0),
        ] {
            let g = grid(w, h, 1.5);
            for ch in BOX {
                let cell = g.cell(3, 3);
                for r in glyph_rects(ch, &g, 3, 3).unwrap().as_slice() {
                    assert!(
                        r.x0.is_finite()
                            && r.x1.is_finite()
                            && r.y0.is_finite()
                            && r.y1.is_finite()
                    );
                    assert!(
                        r.x0 >= cell.x0 && r.x1 <= cell.x1 && r.y0 >= cell.y0 && r.y1 <= cell.y1
                    );
                }
            }
        }
    }
}
