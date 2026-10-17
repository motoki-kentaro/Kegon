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

    /// The vertical boundary `eighth / 8` of the way across `cell`, on the physical pixel grid.
    ///
    /// The cell's own edges are already aligned; the inner boundaries are placed on the aligned cell
    /// and then snapped, so they depend only on the cell and the eighth. 0 and 8 are the cell edges
    /// themselves, and the result never decreases as `eighth` grows. Glyphs that must tile (a lower
    /// 3/8 block and the upper 5/8 above it) use the same boundary value.
    pub fn split_x(&self, cell: Rect, eighth: u8) -> f32 {
        match eighth {
            0 => cell.x0,
            8.. => cell.x1,
            k => {
                let v = cell.x0 + (cell.x1 - cell.x0) * f32::from(k) / 8.0;
                self.snap_abs(self.origin_x + v) - self.origin_x
            }
        }
    }

    /// The horizontal boundary `eighth / 8` of the way down `cell`; see [`Self::split_x`].
    pub fn split_y(&self, cell: Rect, eighth: u8) -> f32 {
        match eighth {
            0 => cell.y0,
            8.. => cell.y1,
            k => {
                let v = cell.y0 + (cell.y1 - cell.y0) * f32::from(k) / 8.0;
                self.snap_abs(self.origin_y + v) - self.origin_y
            }
        }
    }

    /// Thickness of one line of `style` in logical px: a whole number of physical pixels.
    ///
    /// A light line is one sixteenth of the cell height, rounded to physical pixels, and never thinner
    /// than one physical pixel: `max(1, round(cell_height_px / 16))`. So a 13 px cell has 1 px lines, a
    /// 24 px cell 2 px lines, and the line is as thick on a 150% display as the same cell would need
    /// there, not thinner for being measured in logical pixels.
    ///
    /// A heavy line is twice as thick as a light one. A double line is two light lines, so this is the
    /// thickness of each of them (see [`LineStyle::Double`]).
    #[cfg(test)]
    pub fn stroke(&self, style: LineStyle) -> f32 {
        let light = self.light_stroke_phys();
        let physical = match style {
            LineStyle::None => 0,
            LineStyle::Light | LineStyle::Double => light,
            LineStyle::Heavy => 2 * light,
        };
        physical as f32 / self.scale
    }

    /// Thickness of a light line in whole physical pixels (see [`Self::stroke`]).
    fn light_stroke_phys(&self) -> i32 {
        (self.cell_height * self.scale / 16.0).round().max(1.0) as i32
    }

    /// `cell` in whole physical pixels of the window (the surface origin included).
    fn phys(&self, cell: Rect) -> IRect {
        let px = |v: f32, origin: f32| ((origin + v) * self.scale).round() as i32;
        IRect {
            x0: px(cell.x0, self.origin_x),
            y0: px(cell.y0, self.origin_y),
            x1: px(cell.x1, self.origin_x),
            y1: px(cell.y1, self.origin_y),
        }
    }

    /// A physical pixel coordinate back to the surface's logical coordinates. The cell's own edges map
    /// back to exactly the values `cell` carries, so neighbouring glyphs keep sharing them bit for bit.
    fn logical(&self, physical: IRect, cell: Rect, rect: IRect) -> Rect {
        let x = |v: i32, edge: (i32, f32, i32, f32)| {
            if v == edge.0 {
                edge.1
            } else if v == edge.2 {
                edge.3
            } else {
                v as f32 / self.scale - self.origin_x
            }
        };
        let y = |v: i32, edge: (i32, f32, i32, f32)| {
            if v == edge.0 {
                edge.1
            } else if v == edge.2 {
                edge.3
            } else {
                v as f32 / self.scale - self.origin_y
            }
        };
        let ex = (physical.x0, cell.x0, physical.x1, cell.x1);
        let ey = (physical.y0, cell.y0, physical.y1, cell.y1);
        Rect {
            x0: x(rect.x0, ex),
            y0: y(rect.y0, ey),
            x1: x(rect.x1, ex),
            y1: y(rect.y1, ey),
        }
    }
}

/// A rectangle in whole physical pixels of the window; the exact arithmetic of box drawing is done here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl IRect {
    fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
}

/// The rectangles a glyph is made of (never more than [`MAX_RECTS`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRects {
    rects: [Rect; MAX_RECTS],
    len: usize,
}

/// Room for the most rectangles any glyph needs. The double cross ╬ is four separate corner pieces of
/// two rectangles each, which is eight; every other supported glyph needs fewer.
pub const MAX_RECTS: usize = 8;

impl GlyphRects {
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

/// How one side of a box-drawing cell is drawn.
///
/// - `Light`: one line of the light thickness ([`CellGrid::stroke`]).
/// - `Heavy`: one line twice as thick.
/// - `Double`: two light lines with a gap of one light thickness between them, so the pair is three
///   light thicknesses wide and centred like a single line. A cell too small for that (see
///   `double_fits`) draws its double lines as heavy lines instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    None,
    Light,
    Heavy,
    Double,
}

/// The line style that leaves a box-drawing cell through each of its four sides.
///
/// This is the single source of truth for a character's shape: the rectangles are built from it,
/// never from per-character coordinates, and each side can have its own style (a heavy line meeting a
/// light one, a double line meeting a single one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connections {
    pub left: LineStyle,
    pub right: LineStyle,
    pub up: LineStyle,
    pub down: LineStyle,
}

impl Connections {
    /// Parses a table code: one letter per side in the order left, right, up, down, each `-` (none),
    /// `l` (light), `h` (heavy) or `d` (double).
    fn from_code(code: &str) -> Option<Self> {
        let mut sides = code.chars().map(|c| match c {
            '-' => Some(LineStyle::None),
            'l' => Some(LineStyle::Light),
            'h' => Some(LineStyle::Heavy),
            'd' => Some(LineStyle::Double),
            _ => None,
        });
        let c = Self {
            left: sides.next()??,
            right: sides.next()??,
            up: sides.next()??,
            down: sides.next()??,
        };
        sides.next().is_none().then_some(c)
    }

    fn sides(&self) -> [LineStyle; 4] {
        [self.left, self.right, self.up, self.down]
    }

    fn any_double(&self) -> bool {
        self.sides().contains(&LineStyle::Double)
    }

    /// The same connections with every double side drawn heavy.
    fn double_as_heavy(&self) -> Self {
        let map = |s| {
            if s == LineStyle::Double {
                LineStyle::Heavy
            } else {
                s
            }
        };
        Self {
            left: map(self.left),
            right: map(self.right),
            up: map(self.up),
            down: map(self.down),
        }
    }
}

/// The supported box-drawing characters and their connections, in code point order (the lookup is a
/// binary search): the solid light, heavy and double lines and their mixtures, and the half lines
/// U+2574..U+257F. Each code is `left right up down` with `-` none, `l` light, `h` heavy, `d` double
/// (a light line next to a double one is Unicode's "single").
///
/// Not supported and left to the font: dashed lines (U+2504..U+250B, U+254C..U+254F), rounded corners
/// (U+256D..U+2570) and diagonals (U+2571..U+2573).
const BOX_TABLE: &[(char, &str)] = &[
    ('\u{2500}', "ll--"),
    ('\u{2501}', "hh--"),
    ('\u{2502}', "--ll"),
    ('\u{2503}', "--hh"),
    ('\u{250C}', "-l-l"),
    ('\u{250D}', "-h-l"),
    ('\u{250E}', "-l-h"),
    ('\u{250F}', "-h-h"),
    ('\u{2510}', "l--l"),
    ('\u{2511}', "h--l"),
    ('\u{2512}', "l--h"),
    ('\u{2513}', "h--h"),
    ('\u{2514}', "-ll-"),
    ('\u{2515}', "-hl-"),
    ('\u{2516}', "-lh-"),
    ('\u{2517}', "-hh-"),
    ('\u{2518}', "l-l-"),
    ('\u{2519}', "h-l-"),
    ('\u{251A}', "l-h-"),
    ('\u{251B}', "h-h-"),
    ('\u{251C}', "-lll"),
    ('\u{251D}', "-hll"),
    ('\u{251E}', "-lhl"),
    ('\u{251F}', "-llh"),
    ('\u{2520}', "-lhh"),
    ('\u{2521}', "-hhl"),
    ('\u{2522}', "-hlh"),
    ('\u{2523}', "-hhh"),
    ('\u{2524}', "l-ll"),
    ('\u{2525}', "h-ll"),
    ('\u{2526}', "l-hl"),
    ('\u{2527}', "l-lh"),
    ('\u{2528}', "l-hh"),
    ('\u{2529}', "h-hl"),
    ('\u{252A}', "h-lh"),
    ('\u{252B}', "h-hh"),
    ('\u{252C}', "ll-l"),
    ('\u{252D}', "hl-l"),
    ('\u{252E}', "lh-l"),
    ('\u{252F}', "hh-l"),
    ('\u{2530}', "ll-h"),
    ('\u{2531}', "hl-h"),
    ('\u{2532}', "lh-h"),
    ('\u{2533}', "hh-h"),
    ('\u{2534}', "lll-"),
    ('\u{2535}', "hll-"),
    ('\u{2536}', "lhl-"),
    ('\u{2537}', "hhl-"),
    ('\u{2538}', "llh-"),
    ('\u{2539}', "hlh-"),
    ('\u{253A}', "lhh-"),
    ('\u{253B}', "hhh-"),
    ('\u{253C}', "llll"),
    ('\u{253D}', "hlll"),
    ('\u{253E}', "lhll"),
    ('\u{253F}', "hhll"),
    ('\u{2540}', "llhl"),
    ('\u{2541}', "lllh"),
    ('\u{2542}', "llhh"),
    ('\u{2543}', "hlhl"),
    ('\u{2544}', "lhhl"),
    ('\u{2545}', "hllh"),
    ('\u{2546}', "lhlh"),
    ('\u{2547}', "hhhl"),
    ('\u{2548}', "hhlh"),
    ('\u{2549}', "hlhh"),
    ('\u{254A}', "lhhh"),
    ('\u{254B}', "hhhh"),
    ('\u{2550}', "dd--"),
    ('\u{2551}', "--dd"),
    ('\u{2552}', "-d-l"),
    ('\u{2553}', "-l-d"),
    ('\u{2554}', "-d-d"),
    ('\u{2555}', "d--l"),
    ('\u{2556}', "l--d"),
    ('\u{2557}', "d--d"),
    ('\u{2558}', "-dl-"),
    ('\u{2559}', "-ld-"),
    ('\u{255A}', "-dd-"),
    ('\u{255B}', "d-l-"),
    ('\u{255C}', "l-d-"),
    ('\u{255D}', "d-d-"),
    ('\u{255E}', "-dll"),
    ('\u{255F}', "-ldd"),
    ('\u{2560}', "-ddd"),
    ('\u{2561}', "d-ll"),
    ('\u{2562}', "l-dd"),
    ('\u{2563}', "d-dd"),
    ('\u{2564}', "dd-l"),
    ('\u{2565}', "ll-d"),
    ('\u{2566}', "dd-d"),
    ('\u{2567}', "ddl-"),
    ('\u{2568}', "lld-"),
    ('\u{2569}', "ddd-"),
    ('\u{256A}', "ddll"),
    ('\u{256B}', "lldd"),
    ('\u{256C}', "dddd"),
    ('\u{2574}', "l---"),
    ('\u{2575}', "--l-"),
    ('\u{2576}', "-l--"),
    ('\u{2577}', "---l"),
    ('\u{2578}', "h---"),
    ('\u{2579}', "--h-"),
    ('\u{257A}', "-h--"),
    ('\u{257B}', "---h"),
    ('\u{257C}', "lh--"),
    ('\u{257D}', "--lh"),
    ('\u{257E}', "hl--"),
    ('\u{257F}', "--hl"),
];

/// The connections of a supported box-drawing character, or `None` when the font draws it.
pub fn box_connections(ch: char) -> Option<Connections> {
    let index = BOX_TABLE.binary_search_by_key(&ch, |entry| entry.0).ok()?;
    Connections::from_code(BOX_TABLE[index].1)
}

/// Whether `ch` is drawn by this module.
pub fn is_builtin(ch: char) -> bool {
    block_regions(ch).is_some() || box_connections(ch).is_some()
}

/// A rectangle in eighths of the cell: `(x0, y0, x1, y1)`, each in `0..=8`, left/top first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eighths {
    pub x0: u8,
    pub y0: u8,
    pub x1: u8,
    pub y1: u8,
}

const fn eighths(x0: u8, y0: u8, x1: u8, y1: u8) -> Eighths {
    Eighths { x0, y0, x1, y1 }
}

/// The filled area of a block element, in eighths of the cell (at most two rectangles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRegions {
    regions: [Eighths; 2],
    len: usize,
}

impl BlockRegions {
    fn one(region: Eighths) -> Self {
        Self {
            regions: [region; 2],
            len: 1,
        }
    }

    pub fn as_slice(&self) -> &[Eighths] {
        &self.regions[..self.len]
    }
}

/// Quadrant bits: upper left, upper right, lower left, lower right.
const UL: u8 = 1;
const UR: u8 = 2;
const LL: u8 = 4;
const LR: u8 = 8;

/// The quadrants filled by U+2596 … U+259F, in code point order.
const QUADRANTS: [u8; 10] = [
    LL,           // U+2596 QUADRANT LOWER LEFT
    LR,           // U+2597 QUADRANT LOWER RIGHT
    UL,           // U+2598 QUADRANT UPPER LEFT
    UL | LL | LR, // U+2599 QUADRANT UPPER LEFT AND LOWER LEFT AND LOWER RIGHT
    UL | LR,      // U+259A QUADRANT UPPER LEFT AND LOWER RIGHT
    UL | UR | LL, // U+259B QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER LEFT
    UL | UR | LR, // U+259C QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER RIGHT
    UR,           // U+259D QUADRANT UPPER RIGHT
    UR | LL,      // U+259E QUADRANT UPPER RIGHT AND LOWER LEFT
    UR | LL | LR, // U+259F QUADRANT UPPER RIGHT AND LOWER LEFT AND LOWER RIGHT
];

/// The area a block element fills, in eighths of the cell, from its code point.
///
/// - U+2580 upper half, U+2594 upper one eighth
/// - U+2581 … U+2588 lower 1/8 … 8/8 (U+2588 is the full block)
/// - U+2589 … U+258F left 7/8 … 1/8, U+2590 right half, U+2595 right one eighth
/// - U+2596 … U+259F quadrants, from [`QUADRANTS`]
///
/// The shades (U+2591 … U+2593) are not block regions; they stay with the font for now.
pub fn block_regions(ch: char) -> Option<BlockRegions> {
    let cp = ch as u32;
    Some(match cp {
        0x2580 => BlockRegions::one(eighths(0, 0, 8, 4)),
        0x2581..=0x2588 => BlockRegions::one(eighths(0, 8 - (cp - 0x2580) as u8, 8, 8)),
        0x2589..=0x258F => BlockRegions::one(eighths(0, 0, (0x2590 - cp) as u8, 8)),
        0x2590 => BlockRegions::one(eighths(4, 0, 8, 8)),
        0x2594 => BlockRegions::one(eighths(0, 0, 8, 1)),
        0x2595 => BlockRegions::one(eighths(7, 0, 8, 8)),
        0x2596..=0x259F => quadrant_regions(QUADRANTS[(cp - 0x2596) as usize]),
        _ => return None,
    })
}

/// The rectangles of a set of quadrants: each half-row is one rectangle when both its quadrants are
/// filled, and two rows with the same horizontal extent merge into one tall rectangle.
fn quadrant_regions(mask: u8) -> BlockRegions {
    let row = |left: bool, right: bool| -> Option<(u8, u8)> {
        match (left, right) {
            (true, true) => Some((0, 8)),
            (true, false) => Some((0, 4)),
            (false, true) => Some((4, 8)),
            (false, false) => None,
        }
    };
    let top = row(mask & UL != 0, mask & UR != 0);
    let bottom = row(mask & LL != 0, mask & LR != 0);
    let mut out = BlockRegions {
        regions: [eighths(0, 0, 0, 0); 2],
        len: 0,
    };
    let mut push = |r: Eighths| {
        out.regions[out.len] = r;
        out.len += 1;
    };
    match (top, bottom) {
        (Some(t), Some(b)) if t == b => push(eighths(t.0, 0, t.1, 8)),
        (t, b) => {
            if let Some(t) = t {
                push(eighths(t.0, 0, t.1, 4));
            }
            if let Some(b) = b {
                push(eighths(b.0, 4, b.1, 8));
            }
        }
    }
    out
}

/// The rectangles of `ch` in the cell at (`col`, `line`), or `None` when the font should draw it.
///
/// Block elements (all of U+2580 … U+259F except the shades) and the light box-drawing lines.
pub fn glyph_rects(ch: char, grid: &CellGrid, col: i32, line: i32) -> Option<GlyphRects> {
    if !is_builtin(ch) {
        return None;
    }
    let cell = grid.cell(col, line);
    if let Some(connections) = box_connections(ch) {
        return Some(box_rects(&connections, grid, cell));
    }
    let regions = block_regions(ch)?;
    let mut rects = GlyphRects::empty();
    for r in regions.as_slice() {
        // Splits come from `split_x` / `split_y`, so complementary glyphs in the same cell share each
        // boundary exactly; a region that rounds to nothing is simply not drawn.
        rects.push(Rect {
            x0: grid.split_x(cell, r.x0),
            y0: grid.split_y(cell, r.y0),
            x1: grid.split_x(cell, r.x1),
            y1: grid.split_y(cell, r.y1),
        });
    }
    Some(rects)
}

/// Builds the rectangles of a box-drawing cell from its connections.
///
/// All arithmetic is in whole physical pixels. Lines are centred on the cell with
/// `start = round_half_up((lo + hi - thickness) / 2)`, which depends only on the cell's own edges: every
/// cell of a row puts its horizontal lines at exactly the same height and every cell of a column its
/// vertical lines at exactly the same x, however the individual cells were rounded, and lines run to the
/// cell edges, which a neighbour computes from the same edge index. That is what keeps a line unbroken
/// across cell boundaries.
///
/// Two builders produce a set of possibly overlapping rectangles, which [`normalize`] turns into a few
/// non-overlapping ones:
/// - [`line_parts`] for light and heavy lines: the four arms plus a hub where they meet.
/// - [`double_parts`] for anything with a double side: each line of each arm is run to a join chosen by
///   how the arms meet (corner, T, cross).
fn box_rects(connections: &Connections, grid: &CellGrid, cell: Rect) -> GlyphRects {
    let physical = grid.phys(cell);
    let light = grid.light_stroke_phys();
    let mut connections = *connections;
    if connections.any_double() && !double_fits(&physical, light) {
        connections = connections.double_as_heavy();
    }
    let mut parts = Parts::default();
    if connections.any_double() {
        double_parts(&connections, &physical, light, &mut parts);
    } else {
        line_parts(&connections, &physical, light, &mut parts);
    }

    let mut rects = GlyphRects::empty();
    for r in normalize(&parts.as_slice()).as_slice() {
        rects.push(grid.logical(physical, cell, *r));
    }
    rects
}

/// A small fixed list of rectangles under construction.
#[derive(Default)]
struct Parts {
    rects: [Option<IRect>; 16],
    len: usize,
}

impl Parts {
    fn push(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let r = IRect { x0, y0, x1, y1 };
        if !r.is_empty() && self.len < self.rects.len() {
            self.rects[self.len] = Some(r);
            self.len += 1;
        }
    }

    fn as_slice(&self) -> Vec<IRect> {
        self.rects[..self.len].iter().flatten().copied().collect()
    }
}

/// A line of thickness `t` centred in `lo..hi`, kept inside it.
fn band(lo: i32, hi: i32, t: i32) -> (i32, i32) {
    let start = (lo + hi - t + 1).div_euclid(2).clamp(lo, (hi - t).max(lo));
    (start, (start + t).min(hi))
}

/// The two lines of a double line centred in `lo..hi`: each `t` thick with a gap of `t` between.
fn double_bands(lo: i32, hi: i32, t: i32) -> [(i32, i32); 2] {
    let start = (lo + hi - 3 * t + 1)
        .div_euclid(2)
        .clamp(lo, (hi - 3 * t).max(lo));
    [(start, start + t), (start + 2 * t, start + 3 * t)]
}

/// Whether a double line (three light thicknesses wide) fits the cell in both directions with at least
/// one free pixel on each side. If not, the cell draws it as a heavy line.
fn double_fits(cell: &IRect, light: i32) -> bool {
    cell.x1 - cell.x0 >= 3 * light + 2 && cell.y1 - cell.y0 >= 3 * light + 2
}

fn style_thickness(style: LineStyle, light: i32) -> i32 {
    match style {
        LineStyle::None => 0,
        LineStyle::Light | LineStyle::Double => light,
        LineStyle::Heavy => 2 * light,
    }
}

/// Light and heavy lines. Each arm is a bar from its cell edge to the hub; the hub is the rectangle
/// spanned by the widest horizontal and vertical arms, so a thin line meeting a thick one ends flush
/// with it and no notch is left at the junction. Without arms on the other axis there is no hub and the
/// two arms meet at the centre of the cell.
fn line_parts(c: &Connections, p: &IRect, light: i32, out: &mut Parts) {
    let on = |s: LineStyle| s != LineStyle::None;
    let rows = |s: LineStyle| on(s).then(|| band(p.y0, p.y1, style_thickness(s, light)));
    let cols = |s: LineStyle| on(s).then(|| band(p.x0, p.x1, style_thickness(s, light)));
    let (left, right) = (rows(c.left), rows(c.right));
    let (up, down) = (cols(c.up), cols(c.down));
    let has_h = left.is_some() || right.is_some();
    let has_v = up.is_some() || down.is_some();

    if has_h && has_v {
        let hub_x0 = [up, down]
            .iter()
            .flatten()
            .map(|b| b.0)
            .min()
            .unwrap_or(p.x0);
        let hub_x1 = [up, down]
            .iter()
            .flatten()
            .map(|b| b.1)
            .max()
            .unwrap_or(p.x1);
        let hub_y0 = [left, right]
            .iter()
            .flatten()
            .map(|b| b.0)
            .min()
            .unwrap_or(p.y0);
        let hub_y1 = [left, right]
            .iter()
            .flatten()
            .map(|b| b.1)
            .max()
            .unwrap_or(p.y1);
        out.push(hub_x0, hub_y0, hub_x1, hub_y1);
        if let Some(b) = left {
            out.push(p.x0, b.0, hub_x0, b.1);
        }
        if let Some(b) = right {
            out.push(hub_x1, b.0, p.x1, b.1);
        }
        if let Some(b) = up {
            out.push(b.0, p.y0, b.1, hub_y0);
        }
        if let Some(b) = down {
            out.push(b.0, hub_y1, b.1, p.y1);
        }
    } else if has_h {
        // Two arms meet at the centre. A lone arm is a stub that also covers the centre square of its
        // own thickness, so a stub and its mirror image are mirror images of each other.
        let centre = (p.x0 + p.x1 + 1).div_euclid(2);
        let (stub_end, stub_start) = (
            band(p.x0, p.x1, style_thickness(c.left, light)).1,
            band(p.x0, p.x1, style_thickness(c.right, light)).0,
        );
        if let Some(b) = left {
            out.push(
                p.x0,
                b.0,
                if right.is_some() { centre } else { stub_end },
                b.1,
            );
        }
        if let Some(b) = right {
            out.push(
                if left.is_some() { centre } else { stub_start },
                b.0,
                p.x1,
                b.1,
            );
        }
    } else if has_v {
        let centre = (p.y0 + p.y1 + 1).div_euclid(2);
        let (stub_end, stub_start) = (
            band(p.y0, p.y1, style_thickness(c.up, light)).1,
            band(p.y0, p.y1, style_thickness(c.down, light)).0,
        );
        if let Some(b) = up {
            out.push(
                b.0,
                p.y0,
                b.1,
                if down.is_some() { centre } else { stub_end },
            );
        }
        if let Some(b) = down {
            out.push(
                b.0,
                if up.is_some() { centre } else { stub_start },
                b.1,
                p.y1,
            );
        }
    }
}

/// One axis of a box-drawing cell as the double-line builder sees it: whether an arm leaves through the
/// low side (left / up) and the high side (right / down), and whether the lines are double (otherwise a
/// single light line).
#[derive(Clone, Copy)]
struct Axis {
    lo: bool,
    hi: bool,
    double: bool,
}

/// Lines with a double side. Per axis, [`axis_lines`] runs each line of each arm to its join; the
/// horizontal lines are built directly and the vertical ones with the two axes swapped, then every
/// rectangle goes through [`normalize`], which also removes the overlap where two lines cover the same
/// pixels.
fn double_parts(c: &Connections, p: &IRect, light: i32, out: &mut Parts) {
    let on = |s: LineStyle| s != LineStyle::None;
    let dbl = |a: LineStyle, b: LineStyle| a == LineStyle::Double || b == LineStyle::Double;
    let h = Axis {
        lo: on(c.left),
        hi: on(c.right),
        double: dbl(c.left, c.right),
    };
    let v = Axis {
        lo: on(c.up),
        hi: on(c.down),
        double: dbl(c.up, c.down),
    };
    let mut lines = Vec::with_capacity(8);
    // Horizontal lines: p-axis = x, s-axis = y.
    axis_lines((p.x0, p.x1), (p.y0, p.y1), light, h, v, &mut lines);
    for (p0, s0, p1, s1) in lines.drain(..) {
        out.push(p0, s0, p1, s1);
    }
    // Vertical lines: p-axis = y, s-axis = x; swap back when emitting.
    axis_lines((p.y0, p.y1), (p.x0, p.x1), light, v, h, &mut lines);
    for (p0, s0, p1, s1) in lines.drain(..) {
        out.push(s0, p0, s1, p1);
    }
}

/// Emits the lines of axis `me` (running along the "p" direction, `p` is the cell's extent that way and
/// `s` its extent across) as `(p0, s0, p1, s1)` rectangles, given the perpendicular axis `other`.
///
/// Rules, read off the shapes of the Unicode double / single box characters:
/// - A line with nothing crossing it runs the whole cell.
/// - A through line (arms on both sides) is continuous, except that a double line crossed by double
///   lines is cut at the crossing on the side where the crossing arm lies (╦ cuts its lower line, ╬
///   cuts both) and each piece ends flush with the crossing's inner edge.
/// - A single arm ends where it joins: a double arm meeting a single line stops against it (╒, ╞); a
///   single arm meeting a double line stops against the nearer line of a through pair (╟) or runs to the
///   far line of a corner (╓); two double lines at a corner pair up inner-to-inner and outer-to-outer
///   (╔), and at a T the arm runs to the line of the through pair nearer to it (╠).
fn axis_lines(
    p: (i32, i32),
    s: (i32, i32),
    t: i32,
    me: Axis,
    other: Axis,
    out: &mut Vec<(i32, i32, i32, i32)>,
) {
    if !me.lo && !me.hi {
        // No arm on this axis: there is nothing to draw (the other axis' lines are handled by its call).
        return;
    }
    let rows: Vec<(i32, i32)> = if me.double {
        double_bands(s.0, s.1, t).to_vec()
    } else {
        vec![band(s.0, s.1, t)]
    };
    let cols: Vec<(i32, i32)> = if other.double {
        double_bands(p.0, p.1, t).to_vec()
    } else {
        vec![band(p.0, p.1, t)]
    };
    let (n, m) = (rows.len(), cols.len());
    let has_other = other.lo || other.hi;
    let through = me.lo && me.hi;
    let other_through = other.lo && other.hi;

    for (i, &(s0, s1)) in rows.iter().enumerate() {
        if !has_other {
            // Nothing crosses: the line runs from the edge to the edge, or to the centre from one side.
            let centre = (p.0 + p.1 + 1).div_euclid(2);
            let (a, b) = (
                if me.lo { p.0 } else { centre },
                if me.hi { p.1 } else { centre },
            );
            out.push((a, s0, b, s1));
        } else if through {
            let cut = n == 2 && m == 2 && ((i == 0 && other.lo) || (i == 1 && other.hi));
            if cut {
                out.push((p.0, s0, cols[0].1, s1));
                out.push((cols[1].0, s0, p.1, s1));
            } else {
                out.push((p.0, s0, p.1, s1));
            }
        } else {
            let at_hi = me.hi;
            let end = match (n, m) {
                (2, 2) if other_through => {
                    if at_hi {
                        cols[1].0
                    } else {
                        cols[0].1
                    }
                }
                (2, 2) => {
                    // Corner: the line on the side the other arm lies pairs with the nearer line.
                    let inner = (i == 0) == other.lo;
                    match (at_hi, inner) {
                        (true, true) => cols[1].0,
                        (true, false) => cols[0].0,
                        (false, true) => cols[0].1,
                        (false, false) => cols[1].1,
                    }
                }
                (1, 2) if other_through => {
                    if at_hi {
                        cols[1].1
                    } else {
                        cols[0].0
                    }
                }
                (1, 2) => {
                    if at_hi {
                        cols[0].0
                    } else {
                        cols[1].1
                    }
                }
                _ => {
                    if at_hi {
                        cols[0].1
                    } else {
                        cols[0].0
                    }
                }
            };
            if at_hi {
                out.push((end, s0, p.1, s1));
            } else {
                out.push((p.0, s0, end, s1));
            }
        }
    }
}

/// Turns overlapping rectangles into the union as a few non-overlapping ones: the plane is cut along
/// every rectangle edge, covered cells are collected row by row into runs, and a run directly above one
/// with the same extent is merged into it.
fn normalize(input: &[IRect]) -> NormalizedRects {
    let mut xs: Vec<i32> = input.iter().flat_map(|r| [r.x0, r.x1]).collect();
    let mut ys: Vec<i32> = input.iter().flat_map(|r| [r.y0, r.y1]).collect();
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let covered = |i: usize, j: usize| {
        input
            .iter()
            .any(|r| r.x0 <= xs[i] && xs[i + 1] <= r.x1 && r.y0 <= ys[j] && ys[j + 1] <= r.y1)
    };
    let mut out = NormalizedRects::default();
    for j in 0..ys.len().saturating_sub(1) {
        let mut i = 0;
        while i + 1 < xs.len() {
            if !covered(i, j) {
                i += 1;
                continue;
            }
            let start = i;
            while i + 1 < xs.len() && covered(i, j) {
                i += 1;
            }
            let run = IRect {
                x0: xs[start],
                y0: ys[j],
                x1: xs[i],
                y1: ys[j + 1],
            };
            out.add_run(run);
        }
    }
    out
}

/// The result of [`normalize`].
#[derive(Default)]
struct NormalizedRects {
    rects: Vec<IRect>,
}

impl NormalizedRects {
    fn add_run(&mut self, run: IRect) {
        if let Some(above) = self
            .rects
            .iter_mut()
            .find(|r| r.x0 == run.x0 && r.x1 == run.x1 && r.y1 == run.y0)
        {
            above.y1 = run.y1;
        } else {
            self.rects.push(run);
        }
    }

    fn as_slice(&self) -> &[IRect] {
        &self.rects
    }
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
            'a', ' ', '\u{2591}', '\u{2592}', '\u{2593}', '\u{25A0}', '\u{25FE}', '\u{2504}',
            '\u{250B}', '\u{254C}', '\u{254F}', '\u{256D}', '\u{2570}', '\u{2571}', '\u{2573}',
            '\u{E0B0}', '\u{3042}', '\0',
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
                // A half of a one-pixel cell may round to nothing; whatever is drawn is sane.
                for r in glyph_rects(ch, &g, 3, 3).unwrap().as_slice() {
                    assert!(
                        r.x0.is_finite()
                            && r.y0.is_finite()
                            && r.x1.is_finite()
                            && r.y1.is_finite()
                    );
                    assert!(r.x1 > r.x0 && r.y1 > r.y0);
                }
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
                assert!(matches!(side, LineStyle::None | LineStyle::Light), "{ch:?}");
            }
        }
        for ch in [
            'a', '\u{2504}', '\u{256D}', '\u{2571}', '\u{2591}', '\u{2588}',
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
            // A line is centred on its cell: start = round_half_up((lo + hi - t) / 2) in whole
            // physical pixels, written here from the specification rather than from the code.
            let centre_band = |lo: f32, hi: f32| -> i64 {
                let (lo, hi) = ((lo * s).round() as i64, (hi * s).round() as i64);
                (lo + hi - t + 1).div_euclid(2)
            };
            let row_bands: Vec<i64> = [0, 2, 4]
                .iter()
                .map(|&r| centre_band(g.y_edge(r), g.y_edge(r + 1)))
                .collect();
            let col_bands: Vec<i64> = [0, 15, 30]
                .iter()
                .map(|&c| centre_band(g.x_edge(c), g.x_edge(c + 1)))
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
    // ---- partial blocks and quadrants ----

    /// (code point, expected filled area in eighths: x0, y0, x1, y1) of every single-rectangle block,
    /// written from the Unicode names, independently of the code under test.
    const SINGLE: [(char, [u8; 4]); 23] = [
        ('\u{2580}', [0, 0, 8, 4]), // UPPER HALF BLOCK
        ('\u{2581}', [0, 7, 8, 8]), // LOWER ONE EIGHTH BLOCK
        ('\u{2582}', [0, 6, 8, 8]), // LOWER ONE QUARTER BLOCK
        ('\u{2583}', [0, 5, 8, 8]), // LOWER THREE EIGHTHS BLOCK
        ('\u{2584}', [0, 4, 8, 8]), // LOWER HALF BLOCK
        ('\u{2585}', [0, 3, 8, 8]), // LOWER FIVE EIGHTHS BLOCK
        ('\u{2586}', [0, 2, 8, 8]), // LOWER THREE QUARTERS BLOCK
        ('\u{2587}', [0, 1, 8, 8]), // LOWER SEVEN EIGHTHS BLOCK
        ('\u{2588}', [0, 0, 8, 8]), // FULL BLOCK
        ('\u{2589}', [0, 0, 7, 8]), // LEFT SEVEN EIGHTHS BLOCK
        ('\u{258A}', [0, 0, 6, 8]), // LEFT THREE QUARTERS BLOCK
        ('\u{258B}', [0, 0, 5, 8]), // LEFT FIVE EIGHTHS BLOCK
        ('\u{258C}', [0, 0, 4, 8]), // LEFT HALF BLOCK
        ('\u{258D}', [0, 0, 3, 8]), // LEFT THREE EIGHTHS BLOCK
        ('\u{258E}', [0, 0, 2, 8]), // LEFT ONE QUARTER BLOCK
        ('\u{258F}', [0, 0, 1, 8]), // LEFT ONE EIGHTH BLOCK
        ('\u{2590}', [4, 0, 8, 8]), // RIGHT HALF BLOCK
        ('\u{2594}', [0, 0, 8, 1]), // UPPER ONE EIGHTH BLOCK
        ('\u{2595}', [7, 0, 8, 8]), // RIGHT ONE EIGHTH BLOCK
        ('\u{2598}', [0, 0, 4, 4]), // QUADRANT UPPER LEFT
        ('\u{259D}', [4, 0, 8, 4]), // QUADRANT UPPER RIGHT
        ('\u{2596}', [0, 4, 4, 8]), // QUADRANT LOWER LEFT
        ('\u{2597}', [4, 4, 8, 8]), // QUADRANT LOWER RIGHT
    ];

    /// Quadrant characters and the quadrants each fills: (upper left, upper right, lower left, lower right).
    const QUADRANT_CHARS: [(char, [bool; 4]); 10] = [
        ('\u{2596}', [false, false, true, false]),
        ('\u{2597}', [false, false, false, true]),
        ('\u{2598}', [true, false, false, false]),
        ('\u{2599}', [true, false, true, true]),
        ('\u{259A}', [true, false, false, true]),
        ('\u{259B}', [true, true, true, false]),
        ('\u{259C}', [true, true, false, true]),
        ('\u{259D}', [false, true, false, false]),
        ('\u{259E}', [false, true, true, false]),
        ('\u{259F}', [false, true, true, true]),
    ];

    /// Every block character this module draws (shades excluded).
    fn all_blocks() -> Vec<char> {
        (0x2580u32..=0x259F)
            .filter_map(char::from_u32)
            .filter(|&ch| block_regions(ch).is_some())
            .collect()
    }

    /// Cells of whole physical pixels, odd and even, plus the fractional ones used elsewhere.
    const BLOCK_CASES: [(f32, f32, f32); 16] = [
        (8.0, 16.0, 1.0),
        (7.0, 15.0, 1.0),
        (9.0, 17.0, 1.0),
        (10.0, 19.0, 1.0),
        (6.5, 13.0, 1.0),
        (6.5, 13.0, 1.25),
        (6.5, 13.0, 1.5),
        (9.6, 19.2, 1.0),
        (9.6, 19.2, 1.25),
        (9.6, 19.2, 1.5),
        (10.0, 19.0, 1.25),
        (10.0, 19.0, 1.5),
        (7.617, 15.107, 1.0),
        (7.147, 15.222, 1.25),
        (12.0, 24.0, 1.0),
        (8.5, 18.0, 1.75),
    ];

    fn raster_of(ch: char, g: &CellGrid, col: i32, line: i32, into: &mut Raster) {
        let rects = glyph_rects(ch, g, col, line).expect("builtin");
        for r in rects.as_slice() {
            into.paint(r, g.scale);
        }
    }

    /// Physical pixels covered by `ch` in cell (1, 1), as a raster of the 3x3 cell block around it.
    fn cell_raster(ch: char, g: &CellGrid) -> Raster {
        let mut raster = Raster::new(
            (g.x_edge(3) * g.scale).round() as usize + 1,
            (g.y_edge(3) * g.scale).round() as usize + 1,
        );
        raster_of(ch, g, 1, 1, &mut raster);
        raster
    }

    fn covered(r: &Raster) -> usize {
        r.px.iter().filter(|&&n| n > 0).count()
    }

    #[test]
    fn the_filled_area_of_every_block_matches_its_unicode_name() {
        for &(ch, expected) in SINGLE.iter() {
            let regions = block_regions(ch).unwrap_or_else(|| panic!("{ch:?}"));
            assert_eq!(regions.as_slice().len(), 1, "{ch:?}");
            let r = regions.as_slice()[0];
            assert_eq!([r.x0, r.y0, r.x1, r.y1], expected, "{ch:?}");
        }
    }

    #[test]
    fn quadrant_characters_fill_exactly_their_named_quadrants() {
        let g = grid(16.0, 32.0, 1.0);
        let cell = g.cell(1, 1);
        let (mx, my) = (g.split_x(cell, 4), g.split_y(cell, 4));
        for (ch, [ul, ur, ll, lr]) in QUADRANT_CHARS {
            let raster = cell_raster(ch, &g);
            // Probe the centre of each quadrant.
            let probe =
                |x: f32, y: f32| raster.at((x * g.scale) as usize, (y * g.scale) as usize) > 0;
            let ul_p = probe((cell.x0 + mx) / 2.0, (cell.y0 + my) / 2.0);
            let ur_p = probe((mx + cell.x1) / 2.0, (cell.y0 + my) / 2.0);
            let ll_p = probe((cell.x0 + mx) / 2.0, (my + cell.y1) / 2.0);
            let lr_p = probe((mx + cell.x1) / 2.0, (my + cell.y1) / 2.0);
            assert_eq!([ul_p, ur_p, ll_p, lr_p], [ul, ur, ll, lr], "{ch:?}");
            // Whole quadrants only: covered area is the number of quadrants times a quarter.
            let n = [ul, ur, ll, lr].iter().filter(|&&q| q).count();
            let cell_px =
                ((cell.x1 - cell.x0) * g.scale * (cell.y1 - cell.y0) * g.scale).round() as usize;
            assert_eq!(covered(&raster), cell_px * n / 4, "{ch:?}");
        }
    }

    #[test]
    fn quadrant_shapes_use_at_most_two_rectangles() {
        for (ch, _) in QUADRANT_CHARS {
            let n = block_regions(ch).unwrap().as_slice().len();
            assert!((1..=2).contains(&n), "{ch:?}: {n}");
        }
        for ch in all_blocks() {
            assert!(block_regions(ch).unwrap().as_slice().len() <= MAX_RECTS);
        }
        // 2×2 quadrant masks that are a whole row or column collapse to one rectangle.
        assert_eq!(block_regions('\u{2599}').unwrap().as_slice().len(), 2); // UL + bottom row
        assert_eq!(block_regions('\u{259B}').unwrap().as_slice().len(), 2); // top row + LL
        assert_eq!(block_regions('\u{259A}').unwrap().as_slice().len(), 2); // diagonal
        assert_eq!(block_regions('\u{259E}').unwrap().as_slice().len(), 2); // diagonal
    }

    #[test]
    fn all_twenty_four_new_blocks_and_the_old_five_are_built_in_and_shades_are_not() {
        let blocks = all_blocks();
        assert_eq!(blocks.len(), 29, "{blocks:?}");
        for ch in &blocks {
            assert!(is_builtin(*ch));
        }
        for cp in [0x2591u32, 0x2592, 0x2593, 0x2593 + 0x100] {
            let ch = char::from_u32(cp).unwrap();
            assert!(block_regions(ch).is_none() || cp > 0x259F, "{ch:?}");
        }
        for cp in [0x2591u32, 0x2592, 0x2593] {
            let ch = char::from_u32(cp).unwrap();
            assert!(!is_builtin(ch), "{ch:?} must stay with the font");
        }
        // Neighbours just outside U+2580..=U+259F.
        for ch in ['\u{257F}', '\u{25A0}'] {
            assert!(block_regions(ch).is_none());
        }
    }

    #[test]
    fn rectangles_have_positive_area_stay_in_the_cell_and_never_overlap() {
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(1, 1);
            for ch in all_blocks() {
                let mut raster = Raster::new(
                    (g.x_edge(3) * s).round() as usize + 1,
                    (g.y_edge(3) * s).round() as usize + 1,
                );
                for r in glyph_rects(ch, &g, 1, 1).unwrap().as_slice() {
                    assert!(
                        r.width() > 0.0 && r.height() > 0.0,
                        "{ch:?} {w}x{h}@{s}: {r:?}"
                    );
                    assert!(
                        r.x0 >= cell.x0 && r.x1 <= cell.x1 && r.y0 >= cell.y0 && r.y1 <= cell.y1,
                        "{ch:?} {w}x{h}@{s}: {r:?} outside {cell:?}"
                    );
                    raster.paint(r, s);
                }
                assert!(
                    raster.px.iter().all(|&n| n <= 1),
                    "{ch:?} {w}x{h}@{s}: overlap"
                );
            }
        }
    }

    #[test]
    fn split_boundaries_are_ordered_on_the_pixel_grid_and_shared() {
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            for (col, line) in [(0, 0), (1, 1), (7, 3), (135, 54)] {
                let cell = g.cell(col, line);
                assert_eq!(g.split_x(cell, 0), cell.x0);
                assert_eq!(g.split_x(cell, 8), cell.x1);
                assert_eq!(g.split_y(cell, 0), cell.y0);
                assert_eq!(g.split_y(cell, 8), cell.y1);
                let mut prev = (cell.x0, cell.y0);
                for k in 1..=8u8 {
                    let (x, y) = (g.split_x(cell, k), g.split_y(cell, k));
                    assert!(x >= prev.0 && y >= prev.1, "{w}x{h}@{s} k={k}: order");
                    assert!(x <= cell.x1 && y <= cell.y1);
                    for v in [x * s, y * s] {
                        // Snapping happens on absolute window coordinates; origin is 0 here.
                        assert!((v - v.round()).abs() < 1e-3, "{w}x{h}@{s} k={k}: {v}");
                    }
                    // Within half a physical pixel of the ideal split.
                    let ideal_x = cell.x0 + (cell.x1 - cell.x0) * f32::from(k) / 8.0;
                    let ideal_y = cell.y0 + (cell.y1 - cell.y0) * f32::from(k) / 8.0;
                    assert!((x - ideal_x).abs() <= 0.5 / s + 1e-3, "{w}x{h}@{s} k={k}");
                    assert!((y - ideal_y).abs() <= 0.5 / s + 1e-3, "{w}x{h}@{s} k={k}");
                    prev = (x, y);
                }
            }
        }
    }

    /// Pairs of blocks that together fill the cell exactly: (a, b).
    const TILING_PAIRS: [(char, char); 9] = [
        ('\u{2587}', '\u{2594}'), // lower 7/8 + upper 1/8
        ('\u{2584}', '\u{2580}'), // lower half + upper half
        ('\u{2589}', '\u{2595}'), // left 7/8 + right 1/8
        ('\u{258C}', '\u{2590}'), // left half + right half
        ('\u{2598}', '\u{259F}'), // UL + (UR LL LR)
        ('\u{2596}', '\u{259C}'), // LL + (UL UR LR)
        ('\u{2597}', '\u{259B}'), // LR + (UL UR LL)
        ('\u{259D}', '\u{2599}'), // UR + (UL LL LR)
        ('\u{259A}', '\u{259E}'), // UL LR + UR LL
    ];

    #[test]
    fn complementary_blocks_tile_the_cell_with_no_gap_and_no_overlap() {
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(1, 1);
            let full = {
                let mut r = Raster::new(
                    (g.x_edge(3) * s).round() as usize + 1,
                    (g.y_edge(3) * s).round() as usize + 1,
                );
                raster_of('\u{2588}', &g, 1, 1, &mut r);
                r
            };
            for &(a, b) in &TILING_PAIRS {
                let mut r = Raster::new(full.w, full.h);
                raster_of(a, &g, 1, 1, &mut r);
                raster_of(b, &g, 1, 1, &mut r);
                assert!(
                    r.px.iter().all(|&n| n <= 1),
                    "{a:?}+{b:?} {w}x{h}@{s}: overlap"
                );
                assert_eq!(r.px, full.px, "{a:?}+{b:?} {w}x{h}@{s}: gap");
            }
            let _ = cell;
        }
    }

    #[test]
    fn lower_and_left_eighths_grow_monotonically_and_fill_the_named_share() {
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(1, 1);
            let (cw, ch_) = ((cell.x1 - cell.x0) * s, (cell.y1 - cell.y0) * s);
            let mut last_lower = 0usize;
            let mut last_left = 0usize;
            for n in 1..=8u32 {
                let lower = char::from_u32(0x2580 + n).unwrap();
                let left = char::from_u32(0x2590 - n).unwrap();
                let (lo, le) = (
                    covered(&cell_raster(lower, &g)),
                    covered(&cell_raster(left, &g)),
                );
                assert!(
                    lo >= last_lower && le >= last_left,
                    "{w}x{h}@{s} n={n}: not monotonic"
                );
                // Share of the cell within one physical pixel row / column of n/8.
                let lo_expected = ch_ * n as f32 / 8.0;
                let le_expected = cw * n as f32 / 8.0;
                let lo_rows = lo as f32 / cw.round();
                let le_cols = le as f32 / ch_.round();
                assert!(
                    (lo_rows - lo_expected).abs() <= 1.0,
                    "{w}x{h}@{s} lower {n}/8: {lo_rows} vs {lo_expected}"
                );
                assert!(
                    (le_cols - le_expected).abs() <= 1.0,
                    "{w}x{h}@{s} left {n}/8: {le_cols} vs {le_expected}"
                );
                last_lower = lo;
                last_left = le;
            }
        }
    }

    #[test]
    fn mirrored_blocks_are_mirror_images_within_one_physical_pixel() {
        // lower n/8 vs upper n/8 and left vs right only exist for 1/8 and 1/2; compare those.
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(1, 1);
            let up = glyph_rects('\u{2594}', &g, 1, 1).unwrap().as_slice()[0];
            let lo = glyph_rects('\u{2581}', &g, 1, 1).unwrap().as_slice()[0];
            let (top, bottom) = ((up.y1 - up.y0) * s, (lo.y1 - lo.y0) * s);
            assert!(
                (top - bottom).abs() <= 1.0 + 1e-3,
                "{w}x{h}@{s}: {top} vs {bottom}"
            );
            let le = glyph_rects('\u{258F}', &g, 1, 1).unwrap().as_slice()[0];
            let ri = glyph_rects('\u{2595}', &g, 1, 1).unwrap().as_slice()[0];
            let (l, r) = ((le.x1 - le.x0) * s, (ri.x1 - ri.x0) * s);
            assert!((l - r).abs() <= 1.0 + 1e-3, "{w}x{h}@{s}: {l} vs {r}");
            assert_eq!(
                (up.y0, lo.y1, le.x0, ri.x1),
                (cell.y0, cell.y1, cell.x0, cell.x1)
            );
        }
    }

    #[test]
    fn quadrant_unions_equal_their_single_quadrants() {
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            let single = |ch| cell_raster(ch, &g);
            let (ul, ur, ll, lr) = (
                single('\u{2598}'),
                single('\u{259D}'),
                single('\u{2596}'),
                single('\u{2597}'),
            );
            for (ch, [qul, qur, qll, qlr]) in QUADRANT_CHARS {
                let mut expected = Raster::new(ul.w, ul.h);
                for (on, q) in [(qul, &ul), (qur, &ur), (qll, &ll), (qlr, &lr)] {
                    if on {
                        for (e, p) in expected.px.iter_mut().zip(&q.px) {
                            *e += p;
                        }
                    }
                }
                assert_eq!(cell_raster(ch, &g).px, expected.px, "{ch:?} {w}x{h}@{s}");
            }
        }
    }

    #[test]
    fn neighbouring_block_cells_meet_without_a_seam() {
        // A row of left-seven-eighths blocks next to right-one-eighth blocks, and stacked lower blocks.
        for (w, h, s) in BLOCK_CASES {
            let g = grid(w, h, s);
            for ch in all_blocks() {
                for col in 0..30 {
                    let (a, b) = (
                        glyph_rects(ch, &g, col, 0).unwrap(),
                        glyph_rects(ch, &g, col + 1, 0).unwrap(),
                    );
                    // Same shape in the same row: heights match exactly, and each lies in its own cell.
                    for (ra, rb) in a.as_slice().iter().zip(b.as_slice()) {
                        assert_eq!((ra.y0, ra.y1), (rb.y0, rb.y1), "{ch:?} {w}x{h}@{s}");
                    }
                }
            }
            // Full blocks in a column abut exactly, including the slice a lower 1/8 leaves for the next row.
            for line in 0..30 {
                let a = glyph_rects('\u{2581}', &g, 0, line).unwrap().as_slice()[0];
                let b = glyph_rects('\u{2594}', &g, 0, line + 1).unwrap().as_slice()[0];
                assert_eq!(a.y1, b.y0, "{w}x{h}@{s} line {line}");
            }
        }
    }

    #[test]
    fn collapsing_eighths_of_tiny_cells_stay_ordered_and_never_negative() {
        for (w, h, s) in [
            (3.0, 3.0, 1.0),
            (5.0, 5.0, 1.0),
            (2.0, 4.0, 1.5),
            (0.5, 0.7, 1.0),
        ] {
            let g = grid(w, h, s);
            for ch in all_blocks() {
                let cell = g.cell(2, 2);
                for r in glyph_rects(ch, &g, 2, 2).unwrap().as_slice() {
                    assert!(r.x1 > r.x0 && r.y1 > r.y0, "{ch:?} {w}x{h}@{s}: {r:?}");
                    assert!(
                        r.x0 >= cell.x0 && r.x1 <= cell.x1 && r.y0 >= cell.y0 && r.y1 <= cell.y1
                    );
                }
            }
            // The complement of whatever collapsed still tiles: lower 7/8 + upper 1/8.
            let mut r = Raster::new(
                (g.x_edge(5) * s).round() as usize + 1,
                (g.y_edge(5) * s).round() as usize + 1,
            );
            raster_of('\u{2587}', &g, 2, 2, &mut r);
            raster_of('\u{2594}', &g, 2, 2, &mut r);
            assert!(r.px.iter().all(|&n| n <= 1), "{w}x{h}@{s}: overlap");
        }
    }

    #[test]
    fn the_thirty_two_code_points_split_into_five_existing_twenty_four_new_and_three_shades() {
        let existing: [u32; 5] = [0x2580, 0x2584, 0x2588, 0x258C, 0x2590];
        let new: Vec<u32> = [
            0x2581, 0x2582, 0x2583, 0x2585, 0x2586, 0x2587, 0x2589, 0x258A, 0x258B,
        ]
        .into_iter()
        .chain([0x258D, 0x258E, 0x258F, 0x2594, 0x2595])
        .chain(0x2596..=0x259F)
        .collect();
        let shades: [u32; 3] = [0x2591, 0x2592, 0x2593];
        assert_eq!(new.len(), 24);
        for cp in 0x2580u32..=0x259F {
            let ch = char::from_u32(cp).unwrap();
            let class = [
                existing.contains(&cp),
                new.contains(&cp),
                shades.contains(&cp),
            ];
            assert_eq!(
                class.iter().filter(|&&c| c).count(),
                1,
                "{cp:04X} is in exactly one class"
            );
            assert_eq!(block_regions(ch).is_some(), !class[2], "{cp:04X}");
            assert_eq!(is_builtin(ch), !class[2], "{cp:04X}");
        }
        // Every one of the 29 is covered by an expected-area table (single rectangles) or the quadrants.
        let table: Vec<u32> = SINGLE.iter().map(|&(ch, _)| ch as u32).collect();
        let quads: Vec<u32> = QUADRANT_CHARS.iter().map(|&(ch, _)| ch as u32).collect();
        for cp in existing.iter().chain(&new) {
            assert!(
                table.contains(cp) || quads.contains(cp),
                "{cp:04X} has no expectation"
            );
        }
    }

    // ---- heavy, double and mixed box-drawing lines ----

    /// Unicode names of every supported box-drawing character. This is independent data: the table in
    /// the code is checked against it through a parser of the names, not against itself.
    const NAMES: [(char, &str); 109] = [
        ('\u{2500}', "BOX DRAWINGS LIGHT HORIZONTAL"),
        ('\u{2501}', "BOX DRAWINGS HEAVY HORIZONTAL"),
        ('\u{2502}', "BOX DRAWINGS LIGHT VERTICAL"),
        ('\u{2503}', "BOX DRAWINGS HEAVY VERTICAL"),
        ('\u{250C}', "BOX DRAWINGS LIGHT DOWN AND RIGHT"),
        ('\u{250D}', "BOX DRAWINGS DOWN LIGHT AND RIGHT HEAVY"),
        ('\u{250E}', "BOX DRAWINGS DOWN HEAVY AND RIGHT LIGHT"),
        ('\u{250F}', "BOX DRAWINGS HEAVY DOWN AND RIGHT"),
        ('\u{2510}', "BOX DRAWINGS LIGHT DOWN AND LEFT"),
        ('\u{2511}', "BOX DRAWINGS DOWN LIGHT AND LEFT HEAVY"),
        ('\u{2512}', "BOX DRAWINGS DOWN HEAVY AND LEFT LIGHT"),
        ('\u{2513}', "BOX DRAWINGS HEAVY DOWN AND LEFT"),
        ('\u{2514}', "BOX DRAWINGS LIGHT UP AND RIGHT"),
        ('\u{2515}', "BOX DRAWINGS UP LIGHT AND RIGHT HEAVY"),
        ('\u{2516}', "BOX DRAWINGS UP HEAVY AND RIGHT LIGHT"),
        ('\u{2517}', "BOX DRAWINGS HEAVY UP AND RIGHT"),
        ('\u{2518}', "BOX DRAWINGS LIGHT UP AND LEFT"),
        ('\u{2519}', "BOX DRAWINGS UP LIGHT AND LEFT HEAVY"),
        ('\u{251A}', "BOX DRAWINGS UP HEAVY AND LEFT LIGHT"),
        ('\u{251B}', "BOX DRAWINGS HEAVY UP AND LEFT"),
        ('\u{251C}', "BOX DRAWINGS LIGHT VERTICAL AND RIGHT"),
        ('\u{251D}', "BOX DRAWINGS VERTICAL LIGHT AND RIGHT HEAVY"),
        ('\u{251E}', "BOX DRAWINGS UP HEAVY AND RIGHT DOWN LIGHT"),
        ('\u{251F}', "BOX DRAWINGS DOWN HEAVY AND RIGHT UP LIGHT"),
        ('\u{2520}', "BOX DRAWINGS VERTICAL HEAVY AND RIGHT LIGHT"),
        ('\u{2521}', "BOX DRAWINGS DOWN LIGHT AND RIGHT UP HEAVY"),
        ('\u{2522}', "BOX DRAWINGS UP LIGHT AND RIGHT DOWN HEAVY"),
        ('\u{2523}', "BOX DRAWINGS HEAVY VERTICAL AND RIGHT"),
        ('\u{2524}', "BOX DRAWINGS LIGHT VERTICAL AND LEFT"),
        ('\u{2525}', "BOX DRAWINGS VERTICAL LIGHT AND LEFT HEAVY"),
        ('\u{2526}', "BOX DRAWINGS UP HEAVY AND LEFT DOWN LIGHT"),
        ('\u{2527}', "BOX DRAWINGS DOWN HEAVY AND LEFT UP LIGHT"),
        ('\u{2528}', "BOX DRAWINGS VERTICAL HEAVY AND LEFT LIGHT"),
        ('\u{2529}', "BOX DRAWINGS DOWN LIGHT AND LEFT UP HEAVY"),
        ('\u{252A}', "BOX DRAWINGS UP LIGHT AND LEFT DOWN HEAVY"),
        ('\u{252B}', "BOX DRAWINGS HEAVY VERTICAL AND LEFT"),
        ('\u{252C}', "BOX DRAWINGS LIGHT DOWN AND HORIZONTAL"),
        ('\u{252D}', "BOX DRAWINGS LEFT HEAVY AND RIGHT DOWN LIGHT"),
        ('\u{252E}', "BOX DRAWINGS RIGHT HEAVY AND LEFT DOWN LIGHT"),
        ('\u{252F}', "BOX DRAWINGS DOWN LIGHT AND HORIZONTAL HEAVY"),
        ('\u{2530}', "BOX DRAWINGS DOWN HEAVY AND HORIZONTAL LIGHT"),
        ('\u{2531}', "BOX DRAWINGS RIGHT LIGHT AND LEFT DOWN HEAVY"),
        ('\u{2532}', "BOX DRAWINGS LEFT LIGHT AND RIGHT DOWN HEAVY"),
        ('\u{2533}', "BOX DRAWINGS HEAVY DOWN AND HORIZONTAL"),
        ('\u{2534}', "BOX DRAWINGS LIGHT UP AND HORIZONTAL"),
        ('\u{2535}', "BOX DRAWINGS LEFT HEAVY AND RIGHT UP LIGHT"),
        ('\u{2536}', "BOX DRAWINGS RIGHT HEAVY AND LEFT UP LIGHT"),
        ('\u{2537}', "BOX DRAWINGS UP LIGHT AND HORIZONTAL HEAVY"),
        ('\u{2538}', "BOX DRAWINGS UP HEAVY AND HORIZONTAL LIGHT"),
        ('\u{2539}', "BOX DRAWINGS RIGHT LIGHT AND LEFT UP HEAVY"),
        ('\u{253A}', "BOX DRAWINGS LEFT LIGHT AND RIGHT UP HEAVY"),
        ('\u{253B}', "BOX DRAWINGS HEAVY UP AND HORIZONTAL"),
        ('\u{253C}', "BOX DRAWINGS LIGHT VERTICAL AND HORIZONTAL"),
        (
            '\u{253D}',
            "BOX DRAWINGS LEFT HEAVY AND RIGHT VERTICAL LIGHT",
        ),
        (
            '\u{253E}',
            "BOX DRAWINGS RIGHT HEAVY AND LEFT VERTICAL LIGHT",
        ),
        (
            '\u{253F}',
            "BOX DRAWINGS VERTICAL LIGHT AND HORIZONTAL HEAVY",
        ),
        (
            '\u{2540}',
            "BOX DRAWINGS UP HEAVY AND DOWN HORIZONTAL LIGHT",
        ),
        (
            '\u{2541}',
            "BOX DRAWINGS DOWN HEAVY AND UP HORIZONTAL LIGHT",
        ),
        (
            '\u{2542}',
            "BOX DRAWINGS VERTICAL HEAVY AND HORIZONTAL LIGHT",
        ),
        (
            '\u{2543}',
            "BOX DRAWINGS LEFT UP HEAVY AND RIGHT DOWN LIGHT",
        ),
        (
            '\u{2544}',
            "BOX DRAWINGS RIGHT UP HEAVY AND LEFT DOWN LIGHT",
        ),
        (
            '\u{2545}',
            "BOX DRAWINGS LEFT DOWN HEAVY AND RIGHT UP LIGHT",
        ),
        (
            '\u{2546}',
            "BOX DRAWINGS RIGHT DOWN HEAVY AND LEFT UP LIGHT",
        ),
        (
            '\u{2547}',
            "BOX DRAWINGS DOWN LIGHT AND UP HORIZONTAL HEAVY",
        ),
        (
            '\u{2548}',
            "BOX DRAWINGS UP LIGHT AND DOWN HORIZONTAL HEAVY",
        ),
        (
            '\u{2549}',
            "BOX DRAWINGS RIGHT LIGHT AND LEFT VERTICAL HEAVY",
        ),
        (
            '\u{254A}',
            "BOX DRAWINGS LEFT LIGHT AND RIGHT VERTICAL HEAVY",
        ),
        ('\u{254B}', "BOX DRAWINGS HEAVY VERTICAL AND HORIZONTAL"),
        ('\u{2550}', "BOX DRAWINGS DOUBLE HORIZONTAL"),
        ('\u{2551}', "BOX DRAWINGS DOUBLE VERTICAL"),
        ('\u{2552}', "BOX DRAWINGS DOWN SINGLE AND RIGHT DOUBLE"),
        ('\u{2553}', "BOX DRAWINGS DOWN DOUBLE AND RIGHT SINGLE"),
        ('\u{2554}', "BOX DRAWINGS DOUBLE DOWN AND RIGHT"),
        ('\u{2555}', "BOX DRAWINGS DOWN SINGLE AND LEFT DOUBLE"),
        ('\u{2556}', "BOX DRAWINGS DOWN DOUBLE AND LEFT SINGLE"),
        ('\u{2557}', "BOX DRAWINGS DOUBLE DOWN AND LEFT"),
        ('\u{2558}', "BOX DRAWINGS UP SINGLE AND RIGHT DOUBLE"),
        ('\u{2559}', "BOX DRAWINGS UP DOUBLE AND RIGHT SINGLE"),
        ('\u{255A}', "BOX DRAWINGS DOUBLE UP AND RIGHT"),
        ('\u{255B}', "BOX DRAWINGS UP SINGLE AND LEFT DOUBLE"),
        ('\u{255C}', "BOX DRAWINGS UP DOUBLE AND LEFT SINGLE"),
        ('\u{255D}', "BOX DRAWINGS DOUBLE UP AND LEFT"),
        ('\u{255E}', "BOX DRAWINGS VERTICAL SINGLE AND RIGHT DOUBLE"),
        ('\u{255F}', "BOX DRAWINGS VERTICAL DOUBLE AND RIGHT SINGLE"),
        ('\u{2560}', "BOX DRAWINGS DOUBLE VERTICAL AND RIGHT"),
        ('\u{2561}', "BOX DRAWINGS VERTICAL SINGLE AND LEFT DOUBLE"),
        ('\u{2562}', "BOX DRAWINGS VERTICAL DOUBLE AND LEFT SINGLE"),
        ('\u{2563}', "BOX DRAWINGS DOUBLE VERTICAL AND LEFT"),
        ('\u{2564}', "BOX DRAWINGS DOWN SINGLE AND HORIZONTAL DOUBLE"),
        ('\u{2565}', "BOX DRAWINGS DOWN DOUBLE AND HORIZONTAL SINGLE"),
        ('\u{2566}', "BOX DRAWINGS DOUBLE DOWN AND HORIZONTAL"),
        ('\u{2567}', "BOX DRAWINGS UP SINGLE AND HORIZONTAL DOUBLE"),
        ('\u{2568}', "BOX DRAWINGS UP DOUBLE AND HORIZONTAL SINGLE"),
        ('\u{2569}', "BOX DRAWINGS DOUBLE UP AND HORIZONTAL"),
        (
            '\u{256A}',
            "BOX DRAWINGS VERTICAL SINGLE AND HORIZONTAL DOUBLE",
        ),
        (
            '\u{256B}',
            "BOX DRAWINGS VERTICAL DOUBLE AND HORIZONTAL SINGLE",
        ),
        ('\u{256C}', "BOX DRAWINGS DOUBLE VERTICAL AND HORIZONTAL"),
        ('\u{2574}', "BOX DRAWINGS LIGHT LEFT"),
        ('\u{2575}', "BOX DRAWINGS LIGHT UP"),
        ('\u{2576}', "BOX DRAWINGS LIGHT RIGHT"),
        ('\u{2577}', "BOX DRAWINGS LIGHT DOWN"),
        ('\u{2578}', "BOX DRAWINGS HEAVY LEFT"),
        ('\u{2579}', "BOX DRAWINGS HEAVY UP"),
        ('\u{257A}', "BOX DRAWINGS HEAVY RIGHT"),
        ('\u{257B}', "BOX DRAWINGS HEAVY DOWN"),
        ('\u{257C}', "BOX DRAWINGS LIGHT LEFT AND HEAVY RIGHT"),
        ('\u{257D}', "BOX DRAWINGS LIGHT UP AND HEAVY DOWN"),
        ('\u{257E}', "BOX DRAWINGS HEAVY LEFT AND LIGHT RIGHT"),
        ('\u{257F}', "BOX DRAWINGS HEAVY UP AND LIGHT DOWN"),
    ];

    /// Reads a Unicode box-drawing name into (left, right, up, down).
    ///
    /// A name is one or two phrases joined by AND; a phrase names directions (HORIZONTAL is left and
    /// right, VERTICAL is up and down) and may name a style (SINGLE means light). A style given in only
    /// one phrase applies to all of them ("LIGHT DOWN AND RIGHT").
    fn parse_name(name: &str) -> [LineStyle; 4] {
        let rest = name.strip_prefix("BOX DRAWINGS ").expect("prefix");
        let phrases: Vec<&str> = rest.split(" AND ").collect();
        let parsed: Vec<(Option<LineStyle>, Vec<usize>)> = phrases
            .iter()
            .map(|phrase| {
                let mut style = None;
                let mut dirs = Vec::new();
                for word in phrase.split(' ') {
                    match word {
                        "LIGHT" | "SINGLE" => style = Some(LineStyle::Light),
                        "HEAVY" => style = Some(LineStyle::Heavy),
                        "DOUBLE" => style = Some(LineStyle::Double),
                        "LEFT" => dirs.push(0),
                        "RIGHT" => dirs.push(1),
                        "UP" => dirs.push(2),
                        "DOWN" => dirs.push(3),
                        "HORIZONTAL" => dirs.extend([0, 1]),
                        "VERTICAL" => dirs.extend([2, 3]),
                        other => panic!("unknown word {other:?} in {name:?}"),
                    }
                }
                (style, dirs)
            })
            .collect();
        let styled: Vec<LineStyle> = parsed.iter().filter_map(|(s, _)| *s).collect();
        let mut out = [LineStyle::None; 4];
        for (style, dirs) in &parsed {
            let style = style
                .or_else(|| (styled.len() == 1).then(|| styled[0]))
                .expect("style");
            for &d in dirs {
                out[d] = style;
            }
        }
        out
    }

    fn sides4(c: Connections) -> [LineStyle; 4] {
        [c.left, c.right, c.up, c.down]
    }

    #[test]
    fn every_table_entry_matches_its_unicode_name() {
        assert_eq!(NAMES.len(), 109);
        assert_eq!(BOX_TABLE.len(), 109);
        for (ch, name) in NAMES {
            let c = box_connections(ch).unwrap_or_else(|| panic!("{ch:?} {name} unsupported"));
            assert_eq!(sides4(c), parse_name(name), "{:04X} {name}", ch as u32);
        }
        // The table is sorted (the lookup is a binary search) and holds nothing else.
        let mut previous = '\0';
        for &(ch, _) in BOX_TABLE {
            assert!(ch > previous, "{ch:?} out of order");
            previous = ch;
            assert!(
                NAMES.iter().any(|&(n, _)| n == ch),
                "{ch:?} not in the name list"
            );
        }
    }

    #[test]
    fn the_name_parser_reads_the_tricky_names_correctly() {
        use LineStyle::{Double as D, Heavy as H, Light as L, None as N};
        assert_eq!(
            parse_name("BOX DRAWINGS LIGHT DOWN AND RIGHT"),
            [N, L, N, L]
        );
        assert_eq!(
            parse_name("BOX DRAWINGS DOWN LIGHT AND RIGHT HEAVY"),
            [N, H, N, L]
        );
        assert_eq!(
            parse_name("BOX DRAWINGS LEFT UP HEAVY AND RIGHT DOWN LIGHT"),
            [H, L, H, L]
        );
        assert_eq!(
            parse_name("BOX DRAWINGS UP HEAVY AND DOWN HORIZONTAL LIGHT"),
            [L, L, H, L]
        );
        assert_eq!(
            parse_name("BOX DRAWINGS RIGHT LIGHT AND LEFT VERTICAL HEAVY"),
            [H, L, H, H]
        );
        assert_eq!(
            parse_name("BOX DRAWINGS DOWN SINGLE AND RIGHT DOUBLE"),
            [N, D, N, L]
        );
        assert_eq!(
            parse_name("BOX DRAWINGS DOUBLE VERTICAL AND HORIZONTAL"),
            [D, D, D, D]
        );
        assert_eq!(parse_name("BOX DRAWINGS HEAVY LEFT"), [H, N, N, N]);
    }

    #[test]
    fn the_unsupported_ranges_stay_with_the_font() {
        let supported: Vec<char> = NAMES.iter().map(|&(c, _)| c).collect();
        let mut unsupported = 0;
        for cp in 0x2500u32..=0x257F {
            let ch = char::from_u32(cp).unwrap();
            if supported.contains(&ch) {
                assert!(is_builtin(ch));
            } else {
                unsupported += 1;
                assert!(box_connections(ch).is_none() && !is_builtin(ch), "{cp:04X}");
            }
        }
        // 8 dashes + 4 dashes + 4 arcs + 3 diagonals.
        assert_eq!(unsupported, 19);
        assert_eq!(unsupported + supported.len(), 128);
    }

    /// Cells for the box-drawing tests: odd and even physical sizes, fractional logical sizes, scales.
    const BOX_CASES: [(f32, f32, f32); 14] = [
        (8.0, 16.0, 1.0),
        (9.0, 17.0, 1.0),
        (11.0, 15.0, 1.0),
        (10.0, 19.0, 1.0),
        (6.5, 13.0, 1.0),
        (6.5, 13.0, 1.25),
        (6.5, 13.0, 1.5),
        (9.6, 19.2, 1.0),
        (9.6, 19.2, 1.25),
        (9.6, 19.2, 1.5),
        (10.0, 19.0, 1.25),
        (7.617, 15.107, 1.0),
        (7.147, 15.222, 1.25),
        (12.0, 24.0, 1.0),
    ];

    /// The pixels of a glyph in cell (1, 1), as a raster of the 3×3 cells around it.
    fn glyph_raster(ch: char, g: &CellGrid) -> (Raster, [i32; 4]) {
        let mut raster = Raster::new(
            (g.x_edge(3) * g.scale).round() as usize + 1,
            (g.y_edge(3) * g.scale).round() as usize + 1,
        );
        raster_of(ch, g, 1, 1, &mut raster);
        let c = g.cell(1, 1);
        let p = |v: f32| (v * g.scale).round() as i32;
        (raster, [p(c.x0), p(c.y0), p(c.x1), p(c.y1)])
    }

    /// Lines of a style centred in `lo..hi`, written from the specification: start =
    /// round_half_up((lo + hi - width) / 2), a double line is two light lines with a light gap.
    fn expected_bands(
        style: LineStyle,
        lo: i32,
        hi: i32,
        light: i32,
        double_fits: bool,
    ) -> Vec<(i32, i32)> {
        let centred = |width: i32| {
            let start = (lo + hi - width + 1).div_euclid(2);
            (start, start + width)
        };
        match style {
            LineStyle::None => vec![],
            LineStyle::Light => vec![centred(light)],
            LineStyle::Heavy => vec![centred(2 * light)],
            LineStyle::Double if double_fits => {
                let (start, _) = centred(3 * light);
                vec![
                    (start, start + light),
                    (start + 2 * light, start + 3 * light),
                ]
            }
            LineStyle::Double => vec![centred(2 * light)],
        }
    }

    fn covered_set(values: impl Iterator<Item = (i32, bool)>) -> Vec<i32> {
        values.filter(|&(_, on)| on).map(|(v, _)| v).collect()
    }

    fn band_set(bands: &[(i32, i32)]) -> Vec<i32> {
        bands.iter().flat_map(|&(a, b)| a..b).collect()
    }

    /// One cell side under test: its name, style, the lit pixels along the edge, and the side's extent.
    type EdgeCase = (&'static str, LineStyle, Vec<i32>, (i32, i32));

    #[test]
    fn every_arm_reaches_its_cell_edge_with_exactly_the_lines_of_its_style() {
        for (w, h, s) in BOX_CASES {
            let g = grid(w, h, s);
            let light = g.light_stroke_phys();
            for (ch, _) in NAMES {
                let c = box_connections(ch).unwrap();
                let (r, [x0, y0, x1, y1]) = glyph_raster(ch, &g);
                let fits = x1 - x0 >= 3 * light + 2 && y1 - y0 >= 3 * light + 2;
                let at = |x: i32, y: i32| r.at(x as usize, y as usize) > 0;
                // Left edge column: rows lit must be the left arm's lines.
                let cases: [EdgeCase; 4] = [
                    (
                        "left",
                        c.left,
                        covered_set((y0..y1).map(|y| (y, at(x0, y)))),
                        (y0, y1),
                    ),
                    (
                        "right",
                        c.right,
                        covered_set((y0..y1).map(|y| (y, at(x1 - 1, y)))),
                        (y0, y1),
                    ),
                    (
                        "up",
                        c.up,
                        covered_set((x0..x1).map(|x| (x, at(x, y0)))),
                        (x0, x1),
                    ),
                    (
                        "down",
                        c.down,
                        covered_set((x0..x1).map(|x| (x, at(x, y1 - 1)))),
                        (x0, x1),
                    ),
                ];
                for (side, style, lit, (lo, hi)) in cases {
                    let expected = band_set(&expected_bands(style, lo, hi, light, fits));
                    assert_eq!(
                        lit, expected,
                        "{:04X} {side} {style:?} {w}x{h}@{s}",
                        ch as u32
                    );
                }
            }
        }
    }

    #[test]
    fn glyphs_stay_inside_the_cell_never_overlap_and_fit_the_rectangle_budget() {
        let mut most = 0;
        for (w, h, s) in BOX_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(1, 1);
            for (ch, _) in NAMES {
                let rects = glyph_rects(ch, &g, 1, 1).unwrap();
                let list = rects.as_slice();
                assert!(
                    !list.is_empty() && list.len() <= MAX_RECTS,
                    "{:04X}",
                    ch as u32
                );
                most = most.max(list.len());
                let mut raster = Raster::new(
                    (g.x_edge(3) * s).round() as usize + 1,
                    (g.y_edge(3) * s).round() as usize + 1,
                );
                for r in list {
                    assert!(r.width() > 0.0 && r.height() > 0.0);
                    assert!(
                        r.x0 >= cell.x0 && r.x1 <= cell.x1 && r.y0 >= cell.y0 && r.y1 <= cell.y1,
                        "{:04X} {w}x{h}@{s}: {r:?} outside {cell:?}",
                        ch as u32
                    );
                    raster.paint(r, s);
                }
                assert!(
                    raster.px.iter().all(|&n| n <= 1),
                    "{:04X} {w}x{h}@{s}: overlap",
                    ch as u32
                );
            }
        }
        // The double cross is four separate two-rectangle corners: it is what sets the budget.
        assert_eq!(most, MAX_RECTS);
        assert_eq!(
            glyph_rects('\u{256C}', &grid(11.0, 15.0, 1.0), 1, 1)
                .unwrap()
                .as_slice()
                .len(),
            8
        );
    }

    #[test]
    fn heavy_is_twice_light_and_each_double_line_is_light() {
        for (w, h, s) in BOX_CASES {
            let g = grid(w, h, s);
            let light = g.stroke(LineStyle::Light);
            assert_eq!(g.stroke(LineStyle::Heavy), 2.0 * light, "{w}x{h}@{s}");
            assert_eq!(g.stroke(LineStyle::Double), light);
            assert_eq!(g.stroke(LineStyle::None), 0.0);
        }
    }

    #[test]
    fn heavy_light_and_double_lines_are_visibly_different() {
        // Same cell: light is t thick, heavy 2t, double two t-lines with a t gap (3t overall).
        let g = grid(11.0, 15.0, 1.0);
        let thickness = |ch: char| {
            let (r, [x0, y0, _x1, y1]) = glyph_raster(ch, &g);
            let col = x0 + 1;
            (y0..y1)
                .filter(|&y| r.at(col as usize, y as usize) > 0)
                .count()
        };
        assert_eq!(thickness('\u{2500}'), 1);
        assert_eq!(thickness('\u{2501}'), 2);
        assert_eq!(thickness('\u{2550}'), 2); // two 1 px lines, 1 px apart
        let (r, [x0, y0, _, y1]) = glyph_raster('\u{2550}', &g);
        let rows: Vec<bool> = (y0..y1)
            .map(|y| r.at((x0 + 1) as usize, y as usize) > 0)
            .collect();
        let first = rows.iter().position(|&b| b).unwrap();
        assert_eq!(&rows[first..first + 3], &[true, false, true]);
    }

    #[test]
    fn neighbouring_cells_of_every_style_connect_across_the_boundary() {
        for (w, h, s) in BOX_CASES {
            let g = grid(w, h, s);
            let all: Vec<char> = NAMES.iter().map(|&(c, _)| c).collect();
            for style in [LineStyle::Light, LineStyle::Heavy, LineStyle::Double] {
                let rights: Vec<char> = all
                    .iter()
                    .copied()
                    .filter(|&c| box_connections(c).unwrap().right == style)
                    .collect();
                let lefts: Vec<char> = all
                    .iter()
                    .copied()
                    .filter(|&c| box_connections(c).unwrap().left == style)
                    .collect();
                let downs: Vec<char> = all
                    .iter()
                    .copied()
                    .filter(|&c| box_connections(c).unwrap().down == style)
                    .collect();
                let ups: Vec<char> = all
                    .iter()
                    .copied()
                    .filter(|&c| box_connections(c).unwrap().up == style)
                    .collect();
                assert!(
                    !rights.is_empty() && !lefts.is_empty() && !downs.is_empty() && !ups.is_empty()
                );
                // Horizontal neighbours: A at column 0, B at column 1, same row.
                for &a in &rights {
                    for &b in lefts.iter().take(6) {
                        let mut r = Raster::new(
                            (g.x_edge(3) * s).round() as usize + 1,
                            (g.y_edge(3) * s).round() as usize + 1,
                        );
                        raster_of(a, &g, 0, 1, &mut r);
                        raster_of(b, &g, 1, 1, &mut r);
                        let cell_a = g.cell(0, 1);
                        let boundary = (cell_a.x1 * s).round() as usize;
                        let (y0, y1) = (
                            (cell_a.y0 * s).round() as usize,
                            (cell_a.y1 * s).round() as usize,
                        );
                        for y in y0..y1 {
                            // A's last column and B's first column carry the same rows: no step.
                            assert_eq!(
                                r.at(boundary - 1, y) > 0,
                                r.at(boundary, y) > 0,
                                "{:04X}|{:04X} {style:?} {w}x{h}@{s} row {y}",
                                a as u32,
                                b as u32
                            );
                        }
                    }
                }
                // Vertical neighbours: A above B.
                for &a in &downs {
                    for &b in ups.iter().take(6) {
                        let mut r = Raster::new(
                            (g.x_edge(3) * s).round() as usize + 1,
                            (g.y_edge(3) * s).round() as usize + 1,
                        );
                        raster_of(a, &g, 1, 0, &mut r);
                        raster_of(b, &g, 1, 1, &mut r);
                        let cell_a = g.cell(1, 0);
                        let boundary = (cell_a.y1 * s).round() as usize;
                        let (x0, x1) = (
                            (cell_a.x0 * s).round() as usize,
                            (cell_a.x1 * s).round() as usize,
                        );
                        for x in x0..x1 {
                            assert_eq!(
                                r.at(x, boundary - 1) > 0,
                                r.at(x, boundary) > 0,
                                "{:04X}/{:04X} {style:?} {w}x{h}@{s} col {x}",
                                a as u32,
                                b as u32
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mirrored_glyphs_are_mirror_images_when_the_cell_parity_allows() {
        // A line of odd thickness (light 1, double 3) centres exactly in a cell of odd size, so there the
        // left/right and up/down mirror of a glyph must be the mirror image of its pixels.
        for (w, h, s) in [(9.0, 17.0, 1.0), (11.0, 15.0, 1.0), (7.0, 15.0, 1.0)] {
            let g = grid(w, h, s);
            for (ch, _) in NAMES {
                let c = box_connections(ch).unwrap();
                if c.sides().contains(&LineStyle::Heavy) {
                    continue;
                }
                let mirror_h = Connections {
                    left: c.right,
                    right: c.left,
                    ..c
                };
                let mirror_v = Connections {
                    up: c.down,
                    down: c.up,
                    ..c
                };
                let (a, [x0, y0, x1, y1]) = glyph_raster(ch, &g);
                for (m, flip_x) in [(mirror_h, true), (mirror_v, false)] {
                    let Some(&(other, _)) =
                        NAMES.iter().find(|&&(n, _)| box_connections(n) == Some(m))
                    else {
                        continue;
                    };
                    let (b, _) = glyph_raster(other, &g);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let (mx, my) = if flip_x {
                                (x0 + x1 - 1 - x, y)
                            } else {
                                (x, y0 + y1 - 1 - y)
                            };
                            assert_eq!(
                                a.at(x as usize, y as usize) > 0,
                                b.at(mx as usize, my as usize) > 0,
                                "{:04X} vs {:04X} {w}x{h} at ({x},{y})",
                                ch as u32,
                                other as u32
                            );
                        }
                    }
                }
            }
        }
    }

    /// The pixels of `ch` in an 11×15 cell at 100% (light line 1 px, double bands 3 wide) as text.
    fn ascii(ch: char) -> Vec<String> {
        let g = grid(11.0, 15.0, 1.0);
        let mut raster = Raster::new(11, 15);
        let rects = glyph_rects(ch, &g, 0, 0).unwrap();
        for r in rects.as_slice() {
            raster.paint(r, 1.0);
        }
        (0..15)
            .map(|y| {
                (0..11)
                    .map(|x| if raster.at(x, y) > 0 { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    /// Expands `(rows, pattern)` pairs into the 15 rows of a glyph.
    fn rows(spec: &[(std::ops::Range<usize>, &str)]) -> Vec<String> {
        spec.iter()
            .flat_map(|(range, pattern)| range.clone().map(move |_| pattern.to_string()))
            .collect()
    }

    #[test]
    fn double_line_shapes_match_the_reference_drawings() {
        // Double lines are 1 px with a 1 px gap here: bands at x = 4, 6 and y = 6, 8 (single line: 5, 7).
        let cases: [(char, Vec<String>); 9] = [
            // ╬: four separate corners around an empty centre.
            (
                '\u{256C}',
                rows(&[
                    (0..6, "....#.#...."),
                    (6..7, "#####.#####"),
                    (7..8, "..........."),
                    (8..9, "#####.#####"),
                    (9..15, "....#.#...."),
                ]),
            ),
            // ╔: outer corner at the upper left, inner corner one line inside.
            (
                '\u{2554}',
                rows(&[
                    (0..6, "..........."),
                    (6..7, "....#######"),
                    (7..8, "....#......"),
                    (8..9, "....#.#####"),
                    (9..15, "....#.#...."),
                ]),
            ),
            // ╠: the left line runs through, the right one is cut where the branch leaves.
            (
                '\u{2560}',
                rows(&[
                    (0..6, "....#.#...."),
                    (6..7, "....#.#####"),
                    (7..8, "....#......"),
                    (8..9, "....#.#####"),
                    (9..15, "....#.#...."),
                ]),
            ),
            // ╒: a single line down joins the upper of two double lines going right.
            (
                '\u{2552}',
                rows(&[
                    (0..6, "..........."),
                    (6..7, ".....######"),
                    (7..8, ".....#....."),
                    (8..9, ".....######"),
                    (9..15, ".....#....."),
                ]),
            ),
            // ╓: a double line down meets one single line going right.
            (
                '\u{2553}',
                rows(&[
                    (0..7, "..........."),
                    (7..8, "....#######"),
                    (8..15, "....#.#...."),
                ]),
            ),
            // ╪: a single vertical crosses two double horizontals.
            (
                '\u{256A}',
                rows(&[
                    (0..6, ".....#....."),
                    (6..7, "###########"),
                    (7..8, ".....#....."),
                    (8..9, "###########"),
                    (9..15, ".....#....."),
                ]),
            ),
            // ╫: two double verticals cross one single horizontal.
            (
                '\u{256B}',
                rows(&[
                    (0..7, "....#.#...."),
                    (7..8, "###########"),
                    (8..15, "....#.#...."),
                ]),
            ),
            // ╤: a single line hangs below two double horizontals.
            (
                '\u{2564}',
                rows(&[
                    (0..6, "..........."),
                    (6..7, "###########"),
                    (7..8, "..........."),
                    (8..9, "###########"),
                    (9..15, ".....#....."),
                ]),
            ),
            // ╥: a double line hangs below one single horizontal.
            (
                '\u{2565}',
                rows(&[
                    (0..7, "..........."),
                    (7..8, "###########"),
                    (8..15, "....#.#...."),
                ]),
            ),
        ];
        for (ch, expected) in cases {
            assert_eq!(ascii(ch), expected, "{:04X}", ch as u32);
        }
    }

    #[test]
    fn heavy_and_mixed_shapes_match_the_reference_drawings() {
        // Light line 1 px, heavy 2 px, in an 11×15 cell: light band x = 5, y = 7; heavy x = 5..7, y = 7..9?
        // (heavy starts at round_half_up((0 + 15 - 2) / 2) = 7 and (0 + 11 - 2) / 2 = 5).
        let cases: [(char, Vec<String>); 4] = [
            // ┏: heavy corner.
            (
                '\u{250F}',
                rows(&[
                    (0..7, "..........."),
                    (7..9, ".....######"),
                    (9..15, ".....##...."),
                ]),
            ),
            // ┍: light line down, heavy line right: the heavy bar covers the hub, the light one stops at it.
            (
                '\u{250D}',
                rows(&[
                    (0..7, "..........."),
                    (7..9, ".....######"),
                    (9..15, ".....#....."),
                ]),
            ),
            // ╋: heavy cross.
            (
                '\u{254B}',
                rows(&[
                    (0..7, ".....##...."),
                    (7..9, "###########"),
                    (9..15, ".....##...."),
                ]),
            ),
            // ┿: light vertical crossed by a heavy horizontal.
            (
                '\u{253F}',
                rows(&[
                    (0..7, ".....#....."),
                    (7..9, "###########"),
                    (9..15, ".....#....."),
                ]),
            ),
        ];
        for (ch, expected) in cases {
            assert_eq!(ascii(ch), expected, "{:04X}", ch as u32);
        }
    }

    #[test]
    fn double_lines_degrade_to_heavy_when_the_cell_is_too_small() {
        // A double line needs 3 light thicknesses plus a free pixel each side (5 px at 1 px lines).
        for (w, h) in [(4.0, 8.0), (8.0, 4.0), (3.0, 3.0)] {
            let g = grid(w, h, 1.0);
            for (double, heavy) in [
                ('\u{2550}', '\u{2501}'),
                ('\u{2551}', '\u{2503}'),
                ('\u{2554}', '\u{250F}'),
                ('\u{2557}', '\u{2513}'),
                ('\u{256C}', '\u{254B}'),
                ('\u{2566}', '\u{2533}'),
                ('\u{2552}', '\u{250D}'), // down single, right double → down light, right heavy
            ] {
                let a = glyph_rects(double, &g, 1, 1).unwrap();
                let b = glyph_rects(heavy, &g, 1, 1).unwrap();
                assert_eq!(a, b, "{double:?} vs {heavy:?} in {w}x{h}");
            }
        }
        // A cell that fits keeps the double line.
        let g = grid(11.0, 15.0, 1.0);
        assert_ne!(
            glyph_rects('\u{2550}', &g, 1, 1),
            glyph_rects('\u{2501}', &g, 1, 1)
        );
    }

    #[test]
    fn all_supported_box_glyphs_stay_finite_in_tiny_cells() {
        for (w, h, s) in [
            (1.0, 1.0, 1.0),
            (2.0, 2.0, 1.0),
            (3.0, 5.0, 1.5),
            (0.5, 0.7, 1.0),
        ] {
            let g = grid(w, h, s);
            let cell = g.cell(2, 2);
            for (ch, _) in NAMES {
                for r in glyph_rects(ch, &g, 2, 2).unwrap().as_slice() {
                    assert!(
                        r.x0.is_finite()
                            && r.x1.is_finite()
                            && r.y0.is_finite()
                            && r.y1.is_finite()
                    );
                    assert!(r.x1 > r.x0 && r.y1 > r.y0);
                    assert!(
                        r.x0 >= cell.x0 && r.x1 <= cell.x1 && r.y0 >= cell.y0 && r.y1 <= cell.y1
                    );
                }
            }
        }
    }

    #[test]
    fn a_heavy_frame_is_one_connected_figure_of_the_expected_area() {
        const HEAVY_FRAME: [&str; 5] = [
            "┏━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━┓",
            "┃              ┃              ┃",
            "┣━━━━━━━━━━━━━━╋━━━━━━━━━━━━━━┫",
            "┃              ┃              ┃",
            "┗━━━━━━━━━━━━━━┻━━━━━━━━━━━━━━┛",
        ];
        for (w, h, s) in BOX_CASES {
            let g = grid(w, h, s);
            let raster = draw(&HEAVY_FRAME, &g);
            assert!(raster.px.iter().all(|&n| n <= 1), "{w}x{h}@{s}: overlap");
            let t = 2 * g.light_stroke_phys() as i64;
            let band = |lo: f32, hi: f32| -> i64 {
                let (lo, hi) = ((lo * s).round() as i64, (hi * s).round() as i64);
                (lo + hi - t + 1).div_euclid(2)
            };
            let rows: Vec<i64> = [0, 2, 4]
                .iter()
                .map(|&r| band(g.y_edge(r), g.y_edge(r + 1)))
                .collect();
            let cols: Vec<i64> = [0, 15, 30]
                .iter()
                .map(|&c| band(g.x_edge(c), g.x_edge(c + 1)))
                .collect();
            for &y0 in &rows {
                for dy in 0..t {
                    for x in cols[0]..cols[2] + t {
                        assert_eq!(
                            raster.at(x as usize, (y0 + dy) as usize),
                            1,
                            "{w}x{h}@{s}: hole ({x},{})",
                            y0 + dy
                        );
                    }
                }
            }
            for &x0 in &cols {
                for dx in 0..t {
                    for y in rows[0]..rows[2] + t {
                        assert_eq!(
                            raster.at((x0 + dx) as usize, y as usize),
                            1,
                            "{w}x{h}@{s}: hole ({},{y})",
                            x0 + dx
                        );
                    }
                }
            }
            let drawn: i64 = raster.px.iter().map(|&n| n as i64).sum();
            let horiz = 3 * t * (cols[2] + t - cols[0]);
            let vert = 3 * t * (rows[2] + t - rows[0]);
            assert_eq!(drawn, horiz + vert - 9 * t * t, "{w}x{h}@{s}: area");
        }
    }
}
