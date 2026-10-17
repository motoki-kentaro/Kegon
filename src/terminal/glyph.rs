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
}

/// The rectangles a glyph is made of (never more than [`MAX_RECTS`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRects {
    rects: [Rect; MAX_RECTS],
    len: usize,
}

/// Room for the quadrant blocks and box-drawing junctions that will follow.
pub const MAX_RECTS: usize = 4;

impl GlyphRects {
    fn single(rect: Rect) -> Self {
        let mut rects = [rect; MAX_RECTS];
        rects[0] = rect;
        Self { rects, len: 1 }
    }

    pub fn as_slice(&self) -> &[Rect] {
        &self.rects[..self.len]
    }
}

/// Whether `ch` is drawn by this module.
pub fn is_builtin(ch: char) -> bool {
    matches!(
        ch,
        '\u{2588}' | '\u{2580}' | '\u{2584}' | '\u{258C}' | '\u{2590}'
    )
}

/// The rectangles of `ch` in the cell at (`col`, `line`), or `None` when the font should draw it.
///
/// Currently the full block and the four half blocks.
pub fn glyph_rects(ch: char, grid: &CellGrid, col: i32, line: i32) -> Option<GlyphRects> {
    if !is_builtin(ch) {
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
    fn exactly_the_five_block_elements_are_built_in() {
        for ch in [FULL, UPPER, LOWER, LEFT, RIGHT] {
            assert!(is_builtin(ch), "{ch:?}");
            assert!(glyph_rects(ch, &grid(8.0, 16.0, 1.0), 0, 0).is_some());
        }
        // Everything else keeps using the font: other blocks, shades, box drawing, ordinary text.
        for ch in [
            'a', ' ', '\u{2591}', '\u{2592}', '\u{2593}', '\u{2581}', '\u{2589}', '\u{2596}',
            '\u{2500}', '\u{2502}', '\u{250C}', '\u{E0B0}', '\u{3042}', '\0',
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
}
