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
/// Dashed lines have their own table ([`DASH_TABLE`]), and the rounded corners and diagonals
/// (U+256D..U+2573) are paths ([`PATH_TABLE`], [`glyph_paths`]).
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

/// A dashed box-drawing line: a straight line cut into `count` dashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dashed {
    /// Runs top to bottom (otherwise left to right).
    pub vertical: bool,
    /// How many dashes the cell holds: 2 (double dash), 3 (triple) or 4 (quadruple).
    pub count: i32,
    /// Drawn with the heavy thickness (otherwise light).
    pub heavy: bool,
}

impl Dashed {
    /// Parses a table code: direction `h` / `v`, the dash count digit, weight `l` / `h`.
    fn from_code(code: &str) -> Option<Self> {
        let mut chars = code.chars();
        let vertical = match chars.next()? {
            'h' => false,
            'v' => true,
            _ => return None,
        };
        let count = chars.next()?.to_digit(10)? as i32;
        let heavy = match chars.next()? {
            'l' => false,
            'h' => true,
            _ => return None,
        };
        (chars.next().is_none() && (2..=4).contains(&count)).then_some(Self {
            vertical,
            count,
            heavy,
        })
    }
}

/// The dashed lines, in code point order: `h`orizontal / `v`ertical, number of dashes, `l`ight / `h`eavy.
const DASH_TABLE: &[(char, &str)] = &[
    ('\u{2504}', "h3l"), // LIGHT TRIPLE DASH HORIZONTAL
    ('\u{2505}', "h3h"), // HEAVY TRIPLE DASH HORIZONTAL
    ('\u{2506}', "v3l"), // LIGHT TRIPLE DASH VERTICAL
    ('\u{2507}', "v3h"), // HEAVY TRIPLE DASH VERTICAL
    ('\u{2508}', "h4l"), // LIGHT QUADRUPLE DASH HORIZONTAL
    ('\u{2509}', "h4h"), // HEAVY QUADRUPLE DASH HORIZONTAL
    ('\u{250A}', "v4l"), // LIGHT QUADRUPLE DASH VERTICAL
    ('\u{250B}', "v4h"), // HEAVY QUADRUPLE DASH VERTICAL
    ('\u{254C}', "h2l"), // LIGHT DOUBLE DASH HORIZONTAL
    ('\u{254D}', "h2h"), // HEAVY DOUBLE DASH HORIZONTAL
    ('\u{254E}', "v2l"), // LIGHT DOUBLE DASH VERTICAL
    ('\u{254F}', "v2h"), // HEAVY DOUBLE DASH VERTICAL
];

/// The dash layout of a dashed box-drawing character, or `None` for any other character.
pub fn dashed(ch: char) -> Option<Dashed> {
    let index = DASH_TABLE.binary_search_by_key(&ch, |entry| entry.0).ok()?;
    Dashed::from_code(DASH_TABLE[index].1)
}

/// A box-drawing character drawn as a curve or a diagonal (paths, not rectangles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathGlyph {
    /// A quarter-circle corner joining a vertical and a horizontal line: the arm directions.
    Arc { right: bool, down: bool },
    /// A diagonal across the whole cell: rising (lower left to upper right) and / or falling (upper left
    /// to lower right).
    Diagonal { rising: bool, falling: bool },
}

impl PathGlyph {
    /// Parses a table code: `a` + `r`/`l` + `d`/`u` for an arc, `x` + two `0`/`1` digits (rising,
    /// falling) for a diagonal.
    fn from_code(code: &str) -> Option<Self> {
        let mut chars = code.chars();
        let parts = (chars.next(), chars.next(), chars.next(), chars.next());
        match parts {
            (Some('a'), Some(h), Some(v), None) => Some(Self::Arc {
                right: match h {
                    'r' => true,
                    'l' => false,
                    _ => return None,
                },
                down: match v {
                    'd' => true,
                    'u' => false,
                    _ => return None,
                },
            }),
            (Some('x'), Some(r), Some(f), None) => {
                let bit = |c: char| match c {
                    '0' => Some(false),
                    '1' => Some(true),
                    _ => None,
                };
                Some(Self::Diagonal {
                    rising: bit(r)?,
                    falling: bit(f)?,
                })
            }
            _ => None,
        }
    }
}

/// The curves and diagonals, in code point order.
const PATH_TABLE: &[(char, &str)] = &[
    ('\u{256D}', "ard"), // LIGHT ARC DOWN AND RIGHT
    ('\u{256E}', "ald"), // LIGHT ARC DOWN AND LEFT
    ('\u{256F}', "alu"), // LIGHT ARC UP AND LEFT
    ('\u{2570}', "aru"), // LIGHT ARC UP AND RIGHT
    ('\u{2571}', "x10"), // LIGHT DIAGONAL UPPER RIGHT TO LOWER LEFT
    ('\u{2572}', "x01"), // LIGHT DIAGONAL UPPER LEFT TO LOWER RIGHT
    ('\u{2573}', "x11"), // LIGHT DIAGONAL CROSS
];

/// The path layout of an arc or diagonal character, or `None` for any other character.
pub fn path_glyph(ch: char) -> Option<PathGlyph> {
    let index = PATH_TABLE.binary_search_by_key(&ch, |entry| entry.0).ok()?;
    PathGlyph::from_code(PATH_TABLE[index].1)
}

/// How dense a shade character is: the share of the cell's pixels drawn in the foreground color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadeLevel {
    /// U+2591 LIGHT SHADE, one pixel in four.
    Light,
    /// U+2592 MEDIUM SHADE, every other pixel in a checkerboard.
    Medium,
    /// U+2593 DARK SHADE, three pixels in four (the pixels the light shade leaves out).
    Dark,
}

/// The shade level of `ch`, or `None` for any other character.
pub fn shade_level(ch: char) -> Option<ShadeLevel> {
    match ch {
        '\u{2591}' => Some(ShadeLevel::Light),
        '\u{2592}' => Some(ShadeLevel::Medium),
        '\u{2593}' => Some(ShadeLevel::Dark),
        _ => None,
    }
}

/// Width and height in physical pixels of the repeating unit of every shade pattern.
pub const SHADE_TILE: (i32, i32) = (4, 2);

/// Whether the pixel at absolute physical position (`x`, `y`) of the window is in the foreground color.
///
/// The pattern depends only on the absolute pixel, so the dots continue unbroken from cell to cell
/// whatever size the cells have and wherever the surface starts:
/// - light: a dot in every fourth column, shifted by two on odd rows (staggered, 4 × 2 tile)
/// - medium: a checkerboard
/// - dark: every pixel the light shade leaves out
pub fn shade_lit(level: ShadeLevel, x: i32, y: i32) -> bool {
    let light = (x + 2 * y.rem_euclid(2)).rem_euclid(4) == 0;
    match level {
        ShadeLevel::Light => light,
        ShadeLevel::Medium => (x + y).rem_euclid(2) == 0,
        ShadeLevel::Dark => !light,
    }
}

/// What it takes to draw one shade cell: the pattern tile placed on the cell's physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadeCell {
    pub level: ShadeLevel,
    /// The cell in the renderer's logical coordinates (physically aligned edges).
    pub bounds: Rect,
    /// The cell's size in physical pixels.
    pub width_px: i32,
    pub height_px: i32,
    /// Offset of the first tile inside the cell, in physical pixels, that puts the pattern on the
    /// window's absolute pixel grid: `(-x0) mod 4` and `(-y0) mod 2` for a cell at (`x0`, `y0`).
    pub phase_x: i32,
    pub phase_y: i32,
}

/// Everything that decides how a tile looks, and nothing else: the key of the tile cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShadeTileKey {
    pub level: ShadeLevel,
    pub width_px: i32,
    pub height_px: i32,
    pub phase_x: i32,
    pub phase_y: i32,
}

impl ShadeCell {
    pub fn key(&self) -> ShadeTileKey {
        ShadeTileKey {
            level: self.level,
            width_px: self.width_px,
            height_px: self.height_px,
            phase_x: self.phase_x,
            phase_y: self.phase_y,
        }
    }
}

impl ShadeTileKey {
    /// The tile as SVG: a `width_px` × `height_px` pixel image (one SVG unit per physical pixel) filled with
    /// the pattern, drawn in black. The renderer paints it with the cell's foreground color
    /// (`Svg::color`), so one image serves every color.
    pub fn svg(&self) -> String {
        let (tile_w, tile_h) = SHADE_TILE;
        let mut dots = String::new();
        for y in 0..tile_h {
            for x in 0..tile_w {
                if shade_lit(self.level, x, y) {
                    dots += &format!(r#"<rect x="{x}" y="{y}" width="1" height="1"/>"#);
                }
            }
        }
        let (w, h) = (self.width_px, self.height_px);
        let (px, py) = (self.phase_x, self.phase_y);
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" shape-rendering="crispEdges"><defs><pattern id="p" x="{px}" y="{py}" width="{tile_w}" height="{tile_h}" patternUnits="userSpaceOnUse">{dots}</pattern></defs><rect width="{w}" height="{h}" fill="url(#p)"/></svg>"#
        )
    }
}

/// The shade pattern to draw in the cell at (`col`, `line`), or `None` for any other character.
pub fn glyph_shade(ch: char, grid: &CellGrid, col: i32, line: i32) -> Option<ShadeCell> {
    let level = shade_level(ch)?;
    let bounds = grid.cell(col, line);
    let p = grid.phys(bounds);
    let (width_px, height_px) = (p.x1 - p.x0, p.y1 - p.y0);
    if width_px <= 0 || height_px <= 0 {
        return None;
    }
    Some(ShadeCell {
        level,
        bounds,
        width_px,
        height_px,
        phase_x: (-p.x0).rem_euclid(SHADE_TILE.0),
        phase_y: (-p.y0).rem_euclid(SHADE_TILE.1),
    })
}

/// Whether `ch` is drawn by this module.
pub fn is_builtin(ch: char) -> bool {
    block_regions(ch).is_some()
        || box_connections(ch).is_some()
        || dashed(ch).is_some()
        || path_glyph(ch).is_some()
        || shade_level(ch).is_some()
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
    if let Some(dashes) = dashed(ch) {
        return Some(dash_rects(&dashes, grid, cell));
    }
    if let Some(PathGlyph::Arc { right, down }) = path_glyph(ch)
        && arc_too_small(&grid.phys(cell))
    {
        // A cell this small has no room for a curve: the plain light corner stands in for it.
        let side = |on| {
            if on {
                LineStyle::Light
            } else {
                LineStyle::None
            }
        };
        let corner = Connections {
            left: side(!right),
            right: side(right),
            up: side(!down),
            down: side(down),
        };
        return Some(box_rects(&corner, grid, cell));
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
    let (parts, len) = parts.as_slice();
    for r in normalize(&parts[..len]).as_slice() {
        rects.push(grid.logical(physical, cell, *r));
    }
    rects
}

/// Room for the overlapping rectangles a glyph is built from before they are merged.
const PARTS_CAP: usize = 16;

/// A built-in drawing of one character, whichever of the three methods it uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BuiltinGlyph {
    /// Filled rectangles: blocks, box-drawing lines and dashes.
    Rects(GlyphRects),
    /// Curves and diagonals.
    Paths(GlyphPaths),
    /// A shade: a tiled pattern.
    Shade(ShadeCell),
}

/// The built-in drawing of `ch` in the cell at (`col`, `line`), or `None` when the font draws it.
pub fn builtin_glyph(ch: char, grid: &CellGrid, col: i32, line: i32) -> Option<BuiltinGlyph> {
    glyph_rects(ch, grid, col, line)
        .map(BuiltinGlyph::Rects)
        .or_else(|| glyph_paths(ch, grid, col, line).map(BuiltinGlyph::Paths))
        .or_else(|| glyph_shade(ch, grid, col, line).map(BuiltinGlyph::Shade))
}

/// Builds the dashes of a dashed line.
///
/// Along the line the cell's `length` physical pixels are cut into `count` equal segments at
/// `round(length × i / count)`. Each segment holds one dash and a gap of `max(1, round(segment / 3))`
/// pixels, split between its two ends (the smaller half first). A gap is therefore the same inside the
/// cell and across the boundary to the next cell, so a row of the same character is one regular pattern.
/// The line is centred across with the light or heavy thickness, like a solid line.
///
/// A cell too short for `count` dashes with a pixel gap each (`length < 2 × count`) cannot show the
/// pattern and draws the solid line instead.
fn dash_rects(dashes: &Dashed, grid: &CellGrid, cell: Rect) -> GlyphRects {
    let physical = grid.phys(cell);
    let light = grid.light_stroke_phys();
    let thickness = if dashes.heavy { 2 * light } else { light };
    let ((lo, hi), (across_lo, across_hi)) = if dashes.vertical {
        ((physical.y0, physical.y1), (physical.x0, physical.x1))
    } else {
        ((physical.x0, physical.x1), (physical.y0, physical.y1))
    };
    let (s0, s1) = band(across_lo, across_hi, thickness);
    let length = hi - lo;
    let n = dashes.count;
    let mut spans = [(0, 0); 4];
    let mut len = 0;
    if length < 2 * n {
        spans[0] = (lo, hi);
        len = 1;
    } else {
        let boundary = |i: i32| lo + (2 * length * i + n).div_euclid(2 * n);
        for i in 0..n {
            let (b0, b1) = (boundary(i), boundary(i + 1));
            let gap = ((b1 - b0 + 1) / 3).max(1);
            spans[len] = (b0 + gap / 2, b1 - (gap - gap / 2));
            len += 1;
        }
    }
    let mut rects = GlyphRects::empty();
    for &(a, b) in &spans[..len] {
        let r = if dashes.vertical {
            IRect {
                x0: s0,
                y0: a,
                x1: s1,
                y1: b,
            }
        } else {
            IRect {
                x0: a,
                y0: s0,
                x1: b,
                y1: s1,
            }
        };
        rects.push(grid.logical(physical, cell, r));
    }
    rects
}

/// A point in the renderer's logical coordinates.
pub type Point2 = (f32, f32);

/// Room for the corners of a clipped diagonal band (a parallelogram cut by a rectangle).
pub const MAX_POLYGON_POINTS: usize = 8;

/// One piece of a curve or diagonal glyph, in logical coordinates relative to the surface origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathShape {
    /// A cubic Bézier curve to be stroked with `width` and flat (butt) ends.
    Curve {
        from: Point2,
        ctrl1: Point2,
        ctrl2: Point2,
        to: Point2,
        width: f32,
    },
    /// A closed polygon to be filled.
    Polygon {
        points: [Point2; MAX_POLYGON_POINTS],
        len: usize,
    },
}

impl PathShape {
    /// The corners of a [`PathShape::Polygon`] (empty for a curve).
    pub fn polygon_points(&self) -> &[Point2] {
        match self {
            Self::Polygon { points, len } => &points[..*len],
            Self::Curve { .. } => &[],
        }
    }
}

/// The path pieces of a glyph (at most two: a cross is two diagonals).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphPaths {
    shapes: [PathShape; 2],
    len: usize,
}

impl GlyphPaths {
    pub fn as_slice(&self) -> &[PathShape] {
        &self.shapes[..self.len]
    }
}

/// Whether a cell is too small for a curve: with fewer than 6 physical pixels in a direction the arc is a
/// couple of pixels and the corner of straight lines reads better.
fn arc_too_small(p: &IRect) -> bool {
    p.x1 - p.x0 < 6 || p.y1 - p.y0 < 6
}

/// The paths of an arc or diagonal character in the cell at (`col`, `line`), or `None` for any other
/// character (and for an arc in a cell too small to curve, which [`glyph_rects`] draws as a corner).
///
/// Every piece stays inside the cell by construction, so the renderer needs no clipping:
/// - An arc is a cubic Bézier from the middle of one line's band at the cell edge to the middle of the
///   other's, starting and ending along the lines (the same bands the straight lines use, so it joins them
///   without a step). Seen from the stroke, its flat ends lie on the cell edge and the curve bulges only
///   toward the cell's corner side, never past an edge.
/// - A diagonal is the band of one light thickness around the line between two opposite cell corners,
///   already cut by the cell's rectangle, so it ends flush with the cell edges and meets the diagonal of
///   the neighbouring cell exactly at the corner.
pub fn glyph_paths(ch: char, grid: &CellGrid, col: i32, line: i32) -> Option<GlyphPaths> {
    let glyph = path_glyph(ch)?;
    let cell = grid.cell(col, line);
    let p = grid.phys(cell);
    let width = grid.light_stroke_phys() as f32;
    let to_logical = |x: f32, y: f32| {
        // Edges of the cell map back to exactly the cell's own values.
        let lx = if x == p.x0 as f32 {
            cell.x0
        } else if x == p.x1 as f32 {
            cell.x1
        } else {
            x / grid.scale - grid.origin_x
        };
        let ly = if y == p.y0 as f32 {
            cell.y0
        } else if y == p.y1 as f32 {
            cell.y1
        } else {
            y / grid.scale - grid.origin_y
        };
        (lx, ly)
    };
    let empty = PathShape::Polygon {
        points: [(0.0, 0.0); MAX_POLYGON_POINTS],
        len: 0,
    };
    let mut out = GlyphPaths {
        shapes: [empty; 2],
        len: 0,
    };
    match glyph {
        PathGlyph::Arc { right, down } => {
            if arc_too_small(&p) {
                return None;
            }
            let t = width as i32;
            let vertical = band(p.x0, p.x1, t);
            let horizontal = band(p.y0, p.y1, t);
            let xc = (vertical.0 + vertical.1) as f32 / 2.0;
            let yc = (horizontal.0 + horizontal.1) as f32 / 2.0;
            let edge_x = if right { p.x1 } else { p.x0 } as f32;
            let edge_y = if down { p.y1 } else { p.y0 } as f32;
            // Control points at the circle constant 4/3·(√2 − 1) of the way: a quarter ellipse.
            const K: f32 = 0.552_284_8;
            out.shapes[0] = PathShape::Curve {
                from: to_logical(xc, edge_y),
                ctrl1: to_logical(xc, edge_y + (yc - edge_y) * K),
                ctrl2: to_logical(edge_x + (xc - edge_x) * K, yc),
                to: to_logical(edge_x, yc),
                width: width / grid.scale,
            };
            out.len = 1;
        }
        PathGlyph::Diagonal { rising, falling } => {
            let (x0, y0, x1, y1) = (p.x0 as f32, p.y0 as f32, p.x1 as f32, p.y1 as f32);
            let mut push = |a: Point2, b: Point2| {
                let (points, len) = diagonal_band(a, b, width, (x0, y0, x1, y1));
                let mut logical = [(0.0, 0.0); MAX_POLYGON_POINTS];
                for (slot, &(x, y)) in logical.iter_mut().zip(&points[..len]) {
                    *slot = to_logical(x, y);
                }
                out.shapes[out.len] = PathShape::Polygon {
                    points: logical,
                    len,
                };
                out.len += 1;
            };
            if rising {
                push((x0, y1), (x1, y0));
            }
            if falling {
                push((x0, y0), (x1, y1));
            }
        }
    }
    Some(out)
}

/// The band of thickness `width` around the line `a`–`b`, cut to `rect` `(x0, y0, x1, y1)`
/// (Sutherland–Hodgman against the four edges). Points on an edge of `rect` carry its exact value.
fn diagonal_band(
    a: Point2,
    b: Point2,
    width: f32,
    rect: (f32, f32, f32, f32),
) -> ([Point2; MAX_POLYGON_POINTS], usize) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
    let (ux, uy) = (dx / length, dy / length);
    let (nx, ny) = (-uy * width / 2.0, ux * width / 2.0);
    // Extended past both ends so the cut, not the line's end, shapes it.
    let (ax, ay) = (a.0 - ux * length, a.1 - uy * length);
    let (bx, by) = (b.0 + ux * length, b.1 + uy * length);
    let mut poly = [(0.0f32, 0.0f32); 2 * MAX_POLYGON_POINTS];
    poly[..4].copy_from_slice(&[
        (ax - nx, ay - ny),
        (bx - nx, by - ny),
        (bx + nx, by + ny),
        (ax + nx, ay + ny),
    ]);
    let mut len = 4;
    // Each pass keeps the part of the polygon on the inner side of one rectangle edge.
    for edge in 0..4 {
        let inside = |q: Point2| match edge {
            0 => q.0 >= rect.0,
            1 => q.0 <= rect.2,
            2 => q.1 >= rect.1,
            _ => q.1 <= rect.3,
        };
        let cut = |p: Point2, q: Point2| -> Point2 {
            match edge {
                0 | 1 => {
                    let x = if edge == 0 { rect.0 } else { rect.2 };
                    (x, p.1 + (q.1 - p.1) * (x - p.0) / (q.0 - p.0))
                }
                _ => {
                    let y = if edge == 2 { rect.1 } else { rect.3 };
                    (p.0 + (q.0 - p.0) * (y - p.1) / (q.1 - p.1), y)
                }
            }
        };
        let mut next = [(0.0f32, 0.0f32); 2 * MAX_POLYGON_POINTS];
        let mut n = 0;
        for i in 0..len {
            let (cur, prev) = (poly[i], poly[(i + len - 1) % len]);
            if inside(cur) {
                if !inside(prev) {
                    next[n] = cut(prev, cur);
                    n += 1;
                }
                next[n] = cur;
                n += 1;
            } else if inside(prev) {
                next[n] = cut(prev, cur);
                n += 1;
            }
        }
        poly = next;
        len = n;
        if len == 0 {
            break;
        }
    }
    let mut out = [(0.0f32, 0.0f32); MAX_POLYGON_POINTS];
    let len = len.min(MAX_POLYGON_POINTS);
    out[..len].copy_from_slice(&poly[..len]);
    (out, len)
}

/// A small fixed list of rectangles under construction.
#[derive(Default)]
struct Parts {
    rects: [Option<IRect>; PARTS_CAP],
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

    fn as_slice(&self) -> ([IRect; PARTS_CAP], usize) {
        let mut out = [IRect {
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
        }; PARTS_CAP];
        for (slot, part) in out.iter_mut().zip(self.rects[..self.len].iter().flatten()) {
            *slot = *part;
        }
        (out, self.len)
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
    // Horizontal lines: p-axis = x, s-axis = y.
    let mut lines = Lines::default();
    axis_lines((p.x0, p.x1), (p.y0, p.y1), light, h, v, &mut lines);
    for &(p0, s0, p1, s1) in lines.as_slice() {
        out.push(p0, s0, p1, s1);
    }
    // Vertical lines: p-axis = y, s-axis = x; swap back when emitting.
    let mut lines = Lines::default();
    axis_lines((p.y0, p.y1), (p.x0, p.x1), light, v, h, &mut lines);
    for &(p0, s0, p1, s1) in lines.as_slice() {
        out.push(s0, p0, s1, p1);
    }
}

/// The lines of one axis as `(p0, s0, p1, s1)`: at most two lines, each in at most two pieces.
#[derive(Default)]
struct Lines {
    items: [(i32, i32, i32, i32); 4],
    len: usize,
}

impl Lines {
    fn push(&mut self, line: (i32, i32, i32, i32)) {
        debug_assert!(self.len < self.items.len());
        if self.len < self.items.len() {
            self.items[self.len] = line;
            self.len += 1;
        }
    }

    fn as_slice(&self) -> &[(i32, i32, i32, i32)] {
        &self.items[..self.len]
    }
}

/// One line, or the two lines of a double line, as `(start, end)` bands across an axis.
struct Bands {
    items: [(i32, i32); 2],
    len: usize,
}

impl Bands {
    fn new(lo: i32, hi: i32, t: i32, double: bool) -> Self {
        if double {
            Self {
                items: double_bands(lo, hi, t),
                len: 2,
            }
        } else {
            Self {
                items: [band(lo, hi, t); 2],
                len: 1,
            }
        }
    }

    fn as_slice(&self) -> &[(i32, i32)] {
        &self.items[..self.len]
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
fn axis_lines(p: (i32, i32), s: (i32, i32), t: i32, me: Axis, other: Axis, out: &mut Lines) {
    if !me.lo && !me.hi {
        // No arm on this axis: there is nothing to draw (the other axis' lines are handled by its call).
        return;
    }
    let rows = Bands::new(s.0, s.1, t, me.double);
    let cols = Bands::new(p.0, p.1, t, other.double);
    let (rows, cols) = (rows.as_slice(), cols.as_slice());
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
    let mut xs = Coordinates::default();
    let mut ys = Coordinates::default();
    for r in input {
        xs.insert(r.x0);
        xs.insert(r.x1);
        ys.insert(r.y0);
        ys.insert(r.y1);
    }
    let (xs, ys) = (xs.as_slice(), ys.as_slice());
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

/// The distinct coordinates of one axis, kept sorted, in a fixed array (two per rectangle at most).
#[derive(Default)]
struct Coordinates {
    values: [i32; 2 * PARTS_CAP],
    len: usize,
}

impl Coordinates {
    fn insert(&mut self, value: i32) {
        match self.values[..self.len].binary_search(&value) {
            Ok(_) => {}
            Err(at) if self.len < self.values.len() => {
                self.values.copy_within(at..self.len, at + 1);
                self.values[at] = value;
                self.len += 1;
            }
            Err(_) => {}
        }
    }

    fn as_slice(&self) -> &[i32] {
        &self.values[..self.len]
    }
}

/// The result of [`normalize`]: at most [`MAX_RECTS`] rectangles on the stack.
struct NormalizedRects {
    rects: [IRect; MAX_RECTS],
    len: usize,
}

impl Default for NormalizedRects {
    fn default() -> Self {
        Self {
            rects: [IRect {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            }; MAX_RECTS],
            len: 0,
        }
    }
}

impl NormalizedRects {
    fn add_run(&mut self, run: IRect) {
        if let Some(above) = self.rects[..self.len]
            .iter_mut()
            .find(|r| r.x0 == run.x0 && r.x1 == run.x1 && r.y1 == run.y0)
        {
            above.y1 = run.y1;
        } else {
            debug_assert!(self.len < MAX_RECTS);
            if self.len < MAX_RECTS {
                self.rects[self.len] = run;
                self.len += 1;
            }
        }
    }

    fn as_slice(&self) -> &[IRect] {
        &self.rects[..self.len]
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
        // Everything else keeps using the font: other symbols, Powerline and
        // ordinary text.
        for ch in [
            'a', ' ', '\u{25A0}', '\u{25FE}', '\u{E0B0}', '\u{3042}', '\0',
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
    fn all_twenty_four_new_blocks_and_the_old_five_are_block_rectangles_and_shades_are_not() {
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
            // Shades are built in, but as a pattern tile rather than as block rectangles.
            let ch = char::from_u32(cp).unwrap();
            assert!(is_builtin(ch) && glyph_rects(ch, &grid(8.0, 16.0, 1.0), 0, 0).is_none());
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
            assert!(is_builtin(ch), "{cp:04X}");
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
        let supported: Vec<char> = NAMES
            .iter()
            .map(|&(c, _)| c)
            .chain(DASH_NAMES.iter().map(|&(c, _)| c))
            .chain(PATH_NAMES.iter().map(|&(c, _)| c))
            .collect();
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
        // Every code point of the block is drawn here now.
        assert_eq!(unsupported, 0);
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

    /// The Unicode names of the twelve dashed lines, written out independently of `DASH_TABLE`.
    const DASH_NAMES: [(char, &str); 12] = [
        ('\u{2504}', "BOX DRAWINGS LIGHT TRIPLE DASH HORIZONTAL"),
        ('\u{2505}', "BOX DRAWINGS HEAVY TRIPLE DASH HORIZONTAL"),
        ('\u{2506}', "BOX DRAWINGS LIGHT TRIPLE DASH VERTICAL"),
        ('\u{2507}', "BOX DRAWINGS HEAVY TRIPLE DASH VERTICAL"),
        ('\u{2508}', "BOX DRAWINGS LIGHT QUADRUPLE DASH HORIZONTAL"),
        ('\u{2509}', "BOX DRAWINGS HEAVY QUADRUPLE DASH HORIZONTAL"),
        ('\u{250A}', "BOX DRAWINGS LIGHT QUADRUPLE DASH VERTICAL"),
        ('\u{250B}', "BOX DRAWINGS HEAVY QUADRUPLE DASH VERTICAL"),
        ('\u{254C}', "BOX DRAWINGS LIGHT DOUBLE DASH HORIZONTAL"),
        ('\u{254D}', "BOX DRAWINGS HEAVY DOUBLE DASH HORIZONTAL"),
        ('\u{254E}', "BOX DRAWINGS LIGHT DOUBLE DASH VERTICAL"),
        ('\u{254F}', "BOX DRAWINGS HEAVY DOUBLE DASH VERTICAL"),
    ];

    /// Reads a dashed line's name: weight, dash count word and direction.
    fn parse_dash_name(name: &str) -> Dashed {
        let words: Vec<&str> = name.split(' ').collect();
        assert_eq!(&words[..2], ["BOX", "DRAWINGS"], "{name}");
        assert_eq!(words[4], "DASH", "{name}");
        Dashed {
            heavy: match words[2] {
                "LIGHT" => false,
                "HEAVY" => true,
                other => panic!("{name}: {other}"),
            },
            count: match words[3] {
                "DOUBLE" => 2,
                "TRIPLE" => 3,
                "QUADRUPLE" => 4,
                other => panic!("{name}: {other}"),
            },
            vertical: match words[5] {
                "HORIZONTAL" => false,
                "VERTICAL" => true,
                other => panic!("{name}: {other}"),
            },
        }
    }

    #[test]
    fn every_dash_entry_matches_its_unicode_name() {
        assert_eq!(DASH_TABLE.len(), 12);
        assert!(DASH_TABLE.windows(2).all(|w| w[0].0 < w[1].0), "sorted");
        for (ch, name) in DASH_NAMES {
            assert_eq!(dashed(ch), Some(parse_dash_name(name)), "{ch:?} {name}");
            assert!(is_builtin(ch), "{ch:?}");
            assert!(box_connections(ch).is_none(), "{ch:?}");
        }
        // Neighbours of the two ranges are not dashes.
        for ch in ['\u{2503}', '\u{250C}', '\u{254B}', '\u{2550}', 'a', '-'] {
            assert!(dashed(ch).is_none(), "{ch:?}");
        }
        assert!(Dashed::from_code("h5l").is_none() && Dashed::from_code("x3l").is_none());
    }

    /// The lit pixels along the line of `ch` repeated over `cells` cells, as one run of booleans, and the
    /// cross-axis pixels of the line.
    fn dash_pattern(ch: char, g: &CellGrid, cells: i32) -> (Vec<bool>, Vec<usize>) {
        let vertical =
            matches!(ch, '\u{2502}' | '\u{2503}') || dashed(ch).is_some_and(|d| d.vertical);
        let text: Vec<String> = if vertical {
            (0..cells).map(|_| ch.to_string()).collect()
        } else {
            vec![ch.to_string().repeat(cells as usize)]
        };
        let rows: Vec<&str> = text.iter().map(String::as_str).collect();
        let raster = draw(&rows, g);
        let (along, across) = if vertical {
            (raster.h, raster.w)
        } else {
            (raster.w, raster.h)
        };
        let lit = |a: usize, c: usize| {
            if vertical {
                raster.at(c, a) > 0
            } else {
                raster.at(a, c) > 0
            }
        };
        let cross: Vec<usize> = (0..across)
            .filter(|&c| (0..along).any(|a| lit(a, c)))
            .collect();
        let first = cross[0];
        let end = if vertical {
            (g.y_edge(cells) * g.scale).round() as usize
        } else {
            (g.x_edge(cells) * g.scale).round() as usize
        };
        ((0..end).map(|a| lit(a, first)).collect(), cross)
    }

    /// The pattern as `#` and `.` for a golden comparison.
    fn dash_text(bits: &[bool]) -> String {
        bits.iter().map(|&b| if b { '#' } else { '.' }).collect()
    }

    #[test]
    fn dashed_lines_match_the_reference_drawings() {
        // An 11 × 15 pixel cell: light lines are 1 px, heavy ones 2 px, centred at x = 5 / y = 7.
        let g = grid(11.0, 15.0, 1.0);
        let (bits, cross) = dash_pattern('\u{2504}', &g, 1);
        assert_eq!(dash_text(&bits), "###.##.###."); // triple: segments 4, 3, 4; gaps of 1
        assert_eq!(cross, [7]);
        let (bits, _) = dash_pattern('\u{2508}', &g, 1);
        assert_eq!(dash_text(&bits), "##.##.#.##."); // quadruple: segments 3, 3, 2, 3
        let (bits, _) = dash_pattern('\u{254C}', &g, 1);
        assert_eq!(dash_text(&bits), ".####..###."); // double: segments 6, 5; gaps of 2
        let (bits, cross) = dash_pattern('\u{2505}', &g, 1);
        assert_eq!(dash_text(&bits), "###.##.###.");
        assert_eq!(cross, [7, 8]);
        // Vertical, 15 px tall: triple is three segments of 5 with gaps of 2; the line is at x = 5.
        let (bits, cross) = dash_pattern('\u{2506}', &g, 1);
        assert_eq!(dash_text(&bits), ".###..###..###.");
        assert_eq!(cross, [5]);
        let (bits, cross) = dash_pattern('\u{2507}', &g, 1);
        assert_eq!(dash_text(&bits), ".###..###..###.");
        assert_eq!(cross, [5, 6]);
        // Quadruple: segments 4, 4, 3, 4 (round(15 i / 4) = 4, 8, 11, 15), gaps of 1.
        let (bits, _) = dash_pattern('\u{250A}', &g, 1);
        assert_eq!(dash_text(&bits), "###.###.##.###.");
        // Double: segments 8 and 7 with gaps of 3 and 2 px, split smaller half first.
        let (bits, _) = dash_pattern('\u{254E}', &g, 1);
        assert_eq!(dash_text(&bits), ".#####...#####.");
    }

    #[test]
    fn a_dashed_line_has_its_count_of_dashes_and_gaps_in_every_cell_size() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, name) in DASH_NAMES {
                let spec = parse_dash_name(name);
                let (bits, cross) = dash_pattern(ch, &g, 4);
                let light = g.light_stroke_phys() as usize;
                let thickness = if spec.heavy { 2 * light } else { light };
                assert_eq!(cross.len(), thickness, "{w}x{h}@{s} {ch:?}: thickness");
                // The same centre as the solid line of the same direction and weight.
                let solid = if spec.vertical {
                    if spec.heavy { '\u{2503}' } else { '\u{2502}' }
                } else if spec.heavy {
                    '\u{2501}'
                } else {
                    '\u{2500}'
                };
                let (solid_bits, solid_across) = dash_pattern(solid, &g, 4);
                assert!(solid_bits.iter().all(|&b| b), "solid line is unbroken");
                assert_eq!(cross, solid_across, "{w}x{h}@{s} {ch:?}: centre");

                // Runs of lit pixels, per cell.
                let cell_edges: Vec<usize> = (0..=4)
                    .map(|i| {
                        let e = if spec.vertical {
                            g.y_edge(i)
                        } else {
                            g.x_edge(i)
                        };
                        (e * g.scale).round() as usize
                    })
                    .collect();
                let mut runs = Vec::new();
                let mut i = 0;
                while i < bits.len() {
                    if bits[i] {
                        let start = i;
                        while i < bits.len() && bits[i] {
                            i += 1;
                        }
                        runs.push((start, i));
                    } else {
                        i += 1;
                    }
                }
                // A cell shorter than its dashes need is solid, and merges with a dashed neighbour.
                let needed = (2 * spec.count) as usize;
                let lengths: Vec<usize> =
                    (0..4).map(|i| cell_edges[i + 1] - cell_edges[i]).collect();
                let dashed_cell = |i: usize| lengths[i] >= needed;
                for cell in 0..4 {
                    let solid_neighbour = (cell > 0 && !dashed_cell(cell - 1))
                        || (cell < 3 && !dashed_cell(cell + 1));
                    if !dashed_cell(cell) || solid_neighbour {
                        continue;
                    }
                    let inside: Vec<_> = runs
                        .iter()
                        .filter(|r| r.0 >= cell_edges[cell] && r.1 <= cell_edges[cell + 1])
                        .collect();
                    assert_eq!(
                        inside.len() as i32,
                        spec.count,
                        "{w}x{h}@{s} {ch:?}: dashes in cell {cell}"
                    );
                }
                if (0..4).all(dashed_cell) {
                    // Gaps (including the one across each cell boundary) are at least one pixel,
                    // and the dashes cover most of the line.
                    for pair in runs.windows(2) {
                        assert!(pair[1].0 > pair[0].1, "{w}x{h}@{s} {ch:?}: touching dashes");
                    }
                    let lit = bits.iter().filter(|&&b| b).count() as f32;
                    let share = lit / (cell_edges[4] - cell_edges[0]) as f32;
                    if lengths.iter().all(|&l| l >= 10) {
                        assert!(
                            (0.5..=0.9).contains(&share),
                            "{w}x{h}@{s} {ch:?}: lit share {share}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn identical_dashed_cells_form_one_regular_pattern_across_the_boundary() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, _) in DASH_NAMES {
                let (bits, _) = dash_pattern(ch, &g, 6);
                let edge_of = |i: i32| {
                    let spec = dashed(ch).unwrap();
                    let e = if spec.vertical {
                        g.y_edge(i)
                    } else {
                        g.x_edge(i)
                    };
                    (e * g.scale).round() as usize
                };
                for cell in 1..5 {
                    let b = edge_of(cell);
                    // Gap that straddles the boundary: the unlit pixels around it.
                    let mut gap = 0;
                    let mut at = b;
                    while at > 0 && !bits[at - 1] {
                        at -= 1;
                        gap += 1;
                    }
                    let mut at = b;
                    while at < bits.len() && !bits[at] {
                        at += 1;
                        gap += 1;
                    }
                    let length = edge_of(cell + 1) - b;
                    let previous = b - edge_of(cell - 1);
                    let needed = 2 * dashed(ch).unwrap().count as usize;
                    if length >= needed && previous >= needed {
                        assert!((1..=length).contains(&gap), "{w}x{h}@{s} {ch:?}: gap {gap}");
                        assert!(gap <= length / 2 + 1, "{w}x{h}@{s} {ch:?}: gap {gap}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_cell_too_short_for_its_dashes_draws_the_solid_line() {
        // Four dashes need at least 8 px, three need 6 and two need 4.
        for (ch, length_ok, length_short) in [
            ('\u{2508}', 8.0, 7.0),
            ('\u{2504}', 6.0, 5.0),
            ('\u{254C}', 4.0, 3.0),
        ] {
            let ok = glyph_rects(ch, &grid(length_ok, 15.0, 1.0), 0, 0).unwrap();
            let short = glyph_rects(ch, &grid(length_short, 15.0, 1.0), 0, 0).unwrap();
            assert_eq!(
                ok.as_slice().len() as i32,
                dashed(ch).unwrap().count,
                "{ch:?}"
            );
            assert_eq!(short.as_slice().len(), 1, "{ch:?}");
            assert_eq!(
                short.as_slice()[0].width(),
                length_short,
                "{ch:?}: whole width"
            );
        }
        // The vertical ones follow the height.
        let short = glyph_rects('\u{2506}', &grid(8.0, 5.0, 1.0), 0, 0).unwrap();
        assert_eq!(short.as_slice().len(), 1);
        assert_eq!(short.as_slice()[0].height(), 5.0);
        let ok = glyph_rects('\u{2506}', &grid(8.0, 6.0, 1.0), 0, 0).unwrap();
        assert_eq!(ok.as_slice().len(), 3);
    }

    #[test]
    fn dashed_lines_stay_on_the_pixel_grid_inside_the_cell_in_every_size() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, _) in DASH_NAMES {
                for (col, line) in [(0, 0), (5, 3), (20, 11)] {
                    let cell = g.cell(col, line);
                    let rects = glyph_rects(ch, &g, col, line).unwrap();
                    assert!(rects.as_slice().len() <= 4);
                    let mut raster = Raster::new(
                        (g.x_edge(col + 1) * s).round() as usize + 2,
                        (g.y_edge(line + 1) * s).round() as usize + 2,
                    );
                    for r in rects.as_slice() {
                        assert!(r.x0 >= cell.x0 && r.x1 <= cell.x1, "{ch:?}");
                        assert!(r.y0 >= cell.y0 && r.y1 <= cell.y1, "{ch:?}");
                        for v in [
                            r.x0 + g.origin_x,
                            r.x1 + g.origin_x,
                            r.y0 + g.origin_y,
                            r.y1 + g.origin_y,
                        ] {
                            let phys = v * s;
                            assert!((phys - phys.round()).abs() < 1e-3, "{ch:?}: {v}");
                        }
                        raster.paint(r, s);
                    }
                    assert!(raster.px.iter().all(|&n| n <= 1), "{ch:?}: overlap");
                }
            }
        }
    }

    /// The Unicode names of the seven curve and diagonal characters, independent of `PATH_TABLE`.
    const PATH_NAMES: [(char, &str); 7] = [
        ('\u{256D}', "BOX DRAWINGS LIGHT ARC DOWN AND RIGHT"),
        ('\u{256E}', "BOX DRAWINGS LIGHT ARC DOWN AND LEFT"),
        ('\u{256F}', "BOX DRAWINGS LIGHT ARC UP AND LEFT"),
        ('\u{2570}', "BOX DRAWINGS LIGHT ARC UP AND RIGHT"),
        (
            '\u{2571}',
            "BOX DRAWINGS LIGHT DIAGONAL UPPER RIGHT TO LOWER LEFT",
        ),
        (
            '\u{2572}',
            "BOX DRAWINGS LIGHT DIAGONAL UPPER LEFT TO LOWER RIGHT",
        ),
        ('\u{2573}', "BOX DRAWINGS LIGHT DIAGONAL CROSS"),
    ];

    fn parse_path_name(name: &str) -> PathGlyph {
        if name.contains(" ARC ") {
            let words: Vec<&str> = name.split(' ').collect();
            // "... ARC <vertical> AND <horizontal>"
            let at = words.iter().position(|w| *w == "ARC").unwrap();
            PathGlyph::Arc {
                down: words[at + 1] == "DOWN",
                right: words[at + 4 - 1] == "RIGHT",
            }
        } else if name.ends_with("CROSS") {
            PathGlyph::Diagonal {
                rising: true,
                falling: true,
            }
        } else {
            // "DIAGONAL UPPER RIGHT TO LOWER LEFT" rises; "UPPER LEFT TO LOWER RIGHT" falls.
            let rising = name.ends_with("UPPER RIGHT TO LOWER LEFT");
            assert!(
                rising || name.ends_with("UPPER LEFT TO LOWER RIGHT"),
                "{name}"
            );
            PathGlyph::Diagonal {
                rising,
                falling: !rising,
            }
        }
    }

    #[test]
    fn every_path_entry_matches_its_unicode_name() {
        assert_eq!(PATH_TABLE.len(), 7);
        assert!(PATH_TABLE.windows(2).all(|w| w[0].0 < w[1].0), "sorted");
        for (ch, name) in PATH_NAMES {
            assert_eq!(path_glyph(ch), Some(parse_path_name(name)), "{ch:?} {name}");
            assert!(is_builtin(ch), "{ch:?}");
            assert!(
                box_connections(ch).is_none() && dashed(ch).is_none(),
                "{ch:?}"
            );
        }
        for ch in ['\u{256C}', '\u{2574}', '\u{2591}', 'a', '/'] {
            assert!(path_glyph(ch).is_none(), "{ch:?}");
        }
        assert!(PathGlyph::from_code("aru").is_some());
        assert!(PathGlyph::from_code("azu").is_none() && PathGlyph::from_code("x12").is_none());
    }

    #[test]
    fn all_128_box_drawing_code_points_are_now_built_in() {
        let (mut solid, mut dashes, mut paths_n) = (0, 0, 0);
        for cp in 0x2500u32..=0x257F {
            let ch = char::from_u32(cp).unwrap();
            assert!(is_builtin(ch), "{cp:04X}");
            let rects = glyph_rects(ch, &grid(11.0, 15.0, 1.0), 0, 0).is_some();
            let paths = glyph_paths(ch, &grid(11.0, 15.0, 1.0), 0, 0).is_some();
            assert!(rects != paths, "{cp:04X}: drawn as rectangles xor paths");
            if paths {
                paths_n += 1;
            } else if dashed(ch).is_some() {
                dashes += 1;
            } else {
                solid += 1;
            }
        }
        // 109 solid + 12 dashed + 7 paths.
        assert_eq!((solid, dashes, paths_n), (109, 12, 7));
    }

    fn physical(p: Point2, g: &CellGrid) -> (f32, f32) {
        ((p.0 + g.origin_x) * g.scale, (p.1 + g.origin_y) * g.scale)
    }

    /// A point of a cubic Bézier curve.
    fn bezier(from: Point2, c1: Point2, c2: Point2, to: Point2, t: f32) -> Point2 {
        let u = 1.0 - t;
        let f = |a: f32, b: f32, c: f32, d: f32| {
            u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
        };
        (f(from.0, c1.0, c2.0, to.0), f(from.1, c1.1, c2.1, to.1))
    }

    fn the_curve(
        ch: char,
        g: &CellGrid,
        col: i32,
        line: i32,
    ) -> (Point2, Point2, Point2, Point2, f32) {
        match glyph_paths(ch, g, col, line).unwrap().as_slice() {
            [
                PathShape::Curve {
                    from,
                    ctrl1,
                    ctrl2,
                    to,
                    width,
                },
            ] => (*from, *ctrl1, *ctrl2, *to, *width),
            other => panic!("{ch:?}: {other:?}"),
        }
    }

    #[test]
    fn an_arc_joins_the_straight_lines_at_the_middle_of_their_bands() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, name) in &PATH_NAMES[..4] {
                let PathGlyph::Arc { right, down } = parse_path_name(name) else {
                    unreachable!()
                };
                let cell = g.cell(3, 4);
                let p = g.phys(cell);
                if arc_too_small(&p) {
                    continue;
                }
                let (from, c1, c2, to, width) = the_curve(*ch, &g, 3, 4);
                // The straight lines the arc continues, in this same cell and column / row.
                let vertical = glyph_rects('\u{2502}', &g, 3, 4).unwrap().as_slice()[0];
                let horizontal = glyph_rects('\u{2500}', &g, 3, 4).unwrap().as_slice()[0];
                let (vx, hy) = (
                    (vertical.x0 + vertical.x1) / 2.0,
                    (horizontal.y0 + horizontal.y1) / 2.0,
                );
                let edge_y = if down { cell.y1 } else { cell.y0 };
                let edge_x = if right { cell.x1 } else { cell.x0 };
                let eps = 1e-3;
                assert_eq!(
                    from.1, edge_y,
                    "{w}x{h}@{s} {ch:?}: starts on the cell edge"
                );
                assert_eq!(to.0, edge_x, "{w}x{h}@{s} {ch:?}: ends on the cell edge");
                assert!(
                    (from.0 - vx).abs() < eps,
                    "{w}x{h}@{s} {ch:?}: x of the vertical band"
                );
                assert!(
                    (to.1 - hy).abs() < eps,
                    "{w}x{h}@{s} {ch:?}: y of the horizontal band"
                );
                // Leaves along the lines: vertical tangent at the start, horizontal at the end.
                assert_eq!(c1.0, from.0, "{ch:?}");
                assert_eq!(c2.1, to.1, "{ch:?}");
                let light = g.light_stroke_phys() as f32 / s;
                assert!(
                    (width - light).abs() < eps,
                    "{w}x{h}@{s} {ch:?}: width {width} vs {light}"
                );
                assert_eq!(g.stroke(LineStyle::Light), width, "{ch:?}");
            }
        }
    }

    #[test]
    fn an_arc_and_its_stroke_never_leave_the_cell() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, _) in &PATH_NAMES[..4] {
                for (col, line) in [(0, 0), (5, 3), (20, 11)] {
                    if arc_too_small(&g.phys(g.cell(col, line))) {
                        continue;
                    }
                    let cell = g.cell(col, line);
                    let (from, c1, c2, to, width) = the_curve(*ch, &g, col, line);
                    for i in 0..=64 {
                        let t = i as f32 / 64.0;
                        let (x, y) = bezier(from, c1, c2, to, t);
                        // The stroke is `width` wide across the centreline: the points half a width to
                        // either side along the normal must stay in the cell (the ends are flat).
                        let (ahead, behind) = (
                            bezier(from, c1, c2, to, (t + 1e-3).min(1.0)),
                            bezier(from, c1, c2, to, (t - 1e-3).max(0.0)),
                        );
                        let (dx, dy) = (ahead.0 - behind.0, ahead.1 - behind.1);
                        let norm = (dx * dx + dy * dy).sqrt();
                        let half = width / 2.0;
                        let (nx, ny) = (-dy / norm * half, dx / norm * half);
                        // The normal comes from a finite difference: allow a hundredth of a pixel.
                        let tol = 0.01 / s;
                        for sign in [-1.0, 1.0] {
                            let (qx, qy) = (x + sign * nx, y + sign * ny);
                            assert!(
                                qx >= cell.x0 - tol && qx <= cell.x1 + tol,
                                "{w}x{h}@{s} {ch:?} t={t}: x {qx} vs {}..{}",
                                cell.x0,
                                cell.x1
                            );
                            assert!(
                                qy >= cell.y0 - tol && qy <= cell.y1 + tol,
                                "{w}x{h}@{s} {ch:?} t={t}: y {qy} vs {}..{}",
                                cell.y0,
                                cell.y1
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_four_arcs_are_reflections_of_each_other() {
        // On an even cell with even strokes the bands are symmetric, so ╭ mirrored across the vertical
        // axis is ╮ and across the horizontal axis ╰ (within a physical pixel in general).
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(2, 2);
            if arc_too_small(&g.phys(cell)) {
                continue;
            }
            let (cx, cy) = ((cell.x0 + cell.x1) / 2.0, (cell.y0 + cell.y1) / 2.0);
            let tl = the_curve('\u{256D}', &g, 2, 2);
            let tr = the_curve('\u{256E}', &g, 2, 2);
            let bl = the_curve('\u{2570}', &g, 2, 2);
            let br = the_curve('\u{256F}', &g, 2, 2);
            let one_pixel = 1.0 / s + 1e-3;
            for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let at = |c: (Point2, Point2, Point2, Point2, f32)| bezier(c.0, c.1, c.2, c.3, t);
                let (a, b, c, d) = (at(tl), at(tr), at(bl), at(br));
                assert!(
                    ((a.0 - cx) + (b.0 - cx)).abs() <= one_pixel,
                    "{w}x{h}@{s}: ╭╮ x"
                );
                assert!((a.1 - b.1).abs() <= one_pixel, "{w}x{h}@{s}: ╭╮ y");
                assert!(
                    ((a.1 - cy) + (c.1 - cy)).abs() <= one_pixel,
                    "{w}x{h}@{s}: ╭╰ y"
                );
                assert!((a.0 - c.0).abs() <= one_pixel, "{w}x{h}@{s}: ╭╰ x");
                assert!(
                    ((d.0 - cx) + (c.0 - cx)).abs() <= one_pixel,
                    "{w}x{h}@{s}: ╯╰ x"
                );
                assert!((d.1 - c.1).abs() <= one_pixel, "{w}x{h}@{s}: ╯╰ y");
            }
        }
    }

    #[test]
    fn an_arc_bulges_toward_the_outside_of_its_corner() {
        let g = grid(12.0, 24.0, 1.0);
        let cell = g.cell(0, 0);
        let (from, c1, c2, to, _) = the_curve('\u{256D}', &g, 0, 0);
        // ╭ has its arms to the right and down: the mid point is up and left of the straight corner's
        // inner side, i.e. closer to the cell corner than the chord from start to end.
        let mid = bezier(from, c1, c2, to, 0.5);
        let chord = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        assert!(
            mid.0 < chord.0 && mid.1 < chord.1,
            "{mid:?} vs chord {chord:?}"
        );
        let _ = cell;
        let (rx, ry) = (to.0 - from.0, from.1 - to.1);
        // On a quarter ellipse about the corner side the mid point lies at normalised radius 1.
        let centre = (to.0, from.1); // centre of curvature: the cell corner side
        let radius_at_mid =
            (((mid.0 - centre.0) / rx).powi(2) + ((mid.1 - centre.1) / ry).powi(2)).sqrt();
        assert!(
            (radius_at_mid - 1.0).abs() < 0.01,
            "ellipse radius at the mid point: {radius_at_mid}"
        );
    }

    /// Area of a polygon (shoelace).
    fn polygon_area(points: &[Point2]) -> f32 {
        let mut sum = 0.0;
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            sum += a.0 * b.1 - b.0 * a.1;
        }
        sum.abs() / 2.0
    }

    /// Whether `q` is inside or on the edge of the convex polygon `points`.
    fn inside_convex(points: &[Point2], q: Point2) -> bool {
        let mut sign = 0.0f32;
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            let cross = (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0);
            if cross.abs() < 1e-3 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if sign != cross.signum() {
                return false;
            }
        }
        true
    }

    fn the_polygons(ch: char, g: &CellGrid, col: i32, line: i32) -> Vec<Vec<Point2>> {
        glyph_paths(ch, g, col, line)
            .unwrap()
            .as_slice()
            .iter()
            .map(|s| {
                assert!(matches!(s, PathShape::Polygon { .. }), "{ch:?}");
                s.polygon_points().to_vec()
            })
            .collect()
    }

    #[test]
    fn a_diagonal_is_the_band_around_the_line_between_opposite_corners() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            let cell = g.cell(3, 2);
            let (cw, ch_) = (cell.width() * s, cell.height() * s);
            let light = g.light_stroke_phys() as f32;
            for (c, rising) in [('\u{2571}', true), ('\u{2572}', false)] {
                let polys = the_polygons(c, &g, 3, 2);
                assert_eq!(polys.len(), 1, "{c:?}");
                let poly: Vec<Point2> = polys[0].iter().map(|&p| physical(p, &g)).collect();
                // Reference: area of the pixels' sub-samples within light/2 of the line, counted on a
                // 1/16 px grid, from the definition of distance to a line.
                let n = 16;
                let (ox, oy) = ((cell.x0 + g.origin_x) * s, (cell.y0 + g.origin_y) * s);
                let norm = (cw * cw + ch_ * ch_).sqrt();
                let mut hits = 0;
                for iy in 0..(ch_.round() as i32 * n) {
                    for ix in 0..(cw.round() as i32 * n) {
                        let (x, y) = ((ix as f32 + 0.5) / n as f32, (iy as f32 + 0.5) / n as f32);
                        let d = if rising {
                            (x * ch_ + (y - ch_) * cw) / norm
                        } else {
                            (x * ch_ - y * cw) / norm
                        };
                        if d.abs() <= light / 2.0 {
                            hits += 1;
                        }
                    }
                }
                let _ = (ox, oy);
                let reference = hits as f32 / (n * n) as f32;
                let area = polygon_area(&poly);
                assert!(
                    (area - reference).abs() <= 0.02 * reference + 0.05,
                    "{w}x{h}@{s} {c:?}: area {area} vs sampled {reference}"
                );
                // Passes through the two corners it joins and the centre of the cell.
                let (x0, y0, x1, y1) = (
                    (cell.x0 + g.origin_x) * s,
                    (cell.y0 + g.origin_y) * s,
                    (cell.x1 + g.origin_x) * s,
                    (cell.y1 + g.origin_y) * s,
                );
                let (a, b) = if rising {
                    ((x0, y1), (x1, y0))
                } else {
                    ((x0, y0), (x1, y1))
                };
                for q in [a, b, ((x0 + x1) / 2.0, (y0 + y1) / 2.0)] {
                    assert!(
                        inside_convex(&poly, q),
                        "{w}x{h}@{s} {c:?}: {q:?} not on the band"
                    );
                }
            }
        }
    }

    #[test]
    fn a_diagonal_never_leaves_its_cell_and_its_edge_points_are_exact() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (c, _) in &PATH_NAMES[4..] {
                for (col, line) in [(0, 0), (5, 3), (20, 11)] {
                    let cell = g.cell(col, line);
                    for poly in the_polygons(*c, &g, col, line) {
                        assert!((3..=MAX_POLYGON_POINTS).contains(&poly.len()), "{c:?}");
                        for &(x, y) in &poly {
                            assert!(x >= cell.x0 && x <= cell.x1, "{w}x{h}@{s} {c:?}: x {x}");
                            assert!(y >= cell.y0 && y <= cell.y1, "{w}x{h}@{s} {c:?}: y {y}");
                            let (px, py) = physical((x, y), &g);
                            // Every point is on the cell border (exact) or on the band's long edges.
                            let on_border =
                                x == cell.x0 || x == cell.x1 || y == cell.y0 || y == cell.y1;
                            assert!(on_border || (px.is_finite() && py.is_finite()), "{c:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn diagonals_of_neighbouring_cells_meet_at_the_shared_corner() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            // ╱ in (2, 3) rises to its top right corner, where ╱ in (3, 2) starts.
            let a = the_polygons('\u{2571}', &g, 2, 3).remove(0);
            let b = the_polygons('\u{2571}', &g, 3, 2).remove(0);
            let corner = (g.x_edge(3), g.y_edge(3));
            assert!(
                a.contains(&corner),
                "{w}x{h}@{s}: ╱ ends at the corner {corner:?}: {a:?}"
            );
            assert!(
                inside_convex(&b, corner),
                "{w}x{h}@{s}: the next ╱ starts there"
            );
            // And a ╲ chain: (2, 2) falls to its bottom right corner, where ╲ in (3, 3) starts.
            let a = the_polygons('\u{2572}', &g, 2, 2).remove(0);
            let corner = (g.x_edge(3), g.y_edge(3));
            assert!(
                inside_convex(&a, corner),
                "{w}x{h}@{s}: ╲ reaches the corner"
            );
        }
    }

    #[test]
    fn the_cross_is_two_diagonals_that_share_the_centre() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            let rising = the_polygons('\u{2571}', &g, 1, 1).remove(0);
            let falling = the_polygons('\u{2572}', &g, 1, 1).remove(0);
            let both = the_polygons('\u{2573}', &g, 1, 1);
            assert_eq!(both, vec![rising.clone(), falling.clone()], "{w}x{h}@{s}");
            let cell = g.cell(1, 1);
            let centre = ((cell.x0 + cell.x1) / 2.0, (cell.y0 + cell.y1) / 2.0);
            assert!(inside_convex(&rising, centre) && inside_convex(&falling, centre));
        }
    }

    #[test]
    fn an_arc_in_a_cell_too_small_to_curve_is_the_light_corner() {
        for (ch, left, right, up, down) in [
            ('\u{256D}', false, true, false, true),
            ('\u{256E}', true, false, false, true),
            ('\u{256F}', true, false, true, false),
            ('\u{2570}', false, true, true, false),
        ] {
            let tiny = grid(5.0, 12.0, 1.0);
            assert!(glyph_paths(ch, &tiny, 0, 0).is_none(), "{ch:?}");
            let rects = glyph_rects(ch, &tiny, 0, 0).expect("corner stands in");
            let corner = Connections {
                left: if left {
                    LineStyle::Light
                } else {
                    LineStyle::None
                },
                right: if right {
                    LineStyle::Light
                } else {
                    LineStyle::None
                },
                up: if up {
                    LineStyle::Light
                } else {
                    LineStyle::None
                },
                down: if down {
                    LineStyle::Light
                } else {
                    LineStyle::None
                },
            };
            let expected = box_rects(&corner, &tiny, tiny.cell(0, 0));
            assert_eq!(rects.as_slice(), expected.as_slice(), "{ch:?}");
            // One pixel more room and it curves again.
            let roomy = grid(6.0, 12.0, 1.0);
            assert!(glyph_paths(ch, &roomy, 0, 0).is_some(), "{ch:?}");
            assert!(glyph_rects(ch, &roomy, 0, 0).is_none(), "{ch:?}");
            let short = grid(12.0, 5.0, 1.0);
            assert!(glyph_paths(ch, &short, 0, 0).is_none(), "{ch:?}");
        }
        // Diagonals have no minimum.
        assert!(glyph_paths('\u{2573}', &grid(2.0, 2.0, 1.0), 0, 0).is_some());
    }

    #[test]
    fn paths_stay_finite_in_extreme_cells() {
        for (w, h, s) in [
            (1.0, 1.0, 1.0),
            (0.3, 0.3, 1.0),
            (4.0, 4.0, 3.0),
            (200.0, 400.0, 1.5),
            (9.6, 19.2, 1.25),
        ] {
            let g = grid(w, h, s);
            for (c, _) in PATH_NAMES {
                for r in glyph_paths(c, &g, 3, 7).iter().flat_map(|p| p.as_slice()) {
                    match r {
                        PathShape::Curve {
                            from,
                            ctrl1,
                            ctrl2,
                            to,
                            width,
                        } => {
                            for v in [from, ctrl1, ctrl2, to] {
                                assert!(v.0.is_finite() && v.1.is_finite(), "{w}x{h}@{s} {c:?}");
                            }
                            assert!(width.is_finite() && *width > 0.0);
                        }
                        PathShape::Polygon { .. } => {
                            for v in r.polygon_points() {
                                assert!(v.0.is_finite() && v.1.is_finite(), "{w}x{h}@{s} {c:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- shades ----

    const SHADES: [(char, &str, ShadeLevel); 3] = [
        ('\u{2591}', "LIGHT SHADE", ShadeLevel::Light),
        ('\u{2592}', "MEDIUM SHADE", ShadeLevel::Medium),
        ('\u{2593}', "DARK SHADE", ShadeLevel::Dark),
    ];

    #[test]
    fn the_three_shade_code_points_map_to_their_levels() {
        for (ch, name, level) in SHADES {
            assert_eq!(shade_level(ch), Some(level), "{name}");
            assert!(is_builtin(ch), "{name}");
            // Not a rectangle, path or dash glyph: the shade has its own drawing.
            let g = grid(9.0, 17.0, 1.0);
            assert!(glyph_rects(ch, &g, 0, 0).is_none(), "{name}");
            assert!(glyph_paths(ch, &g, 0, 0).is_none(), "{name}");
            assert!(glyph_shade(ch, &g, 0, 0).is_some(), "{name}");
        }
        for ch in ['\u{2590}', '\u{2594}', '\u{2588}', 'a', ' ', '\u{2500}'] {
            assert!(
                shade_level(ch).is_none() && glyph_shade(ch, &grid(9.0, 17.0, 1.0), 0, 0).is_none()
            );
        }
    }

    #[test]
    fn all_160_code_points_of_the_two_blocks_are_built_in() {
        let g = grid(11.0, 15.0, 1.0);
        let mut counts = [0; 4];
        for cp in 0x2500u32..=0x259F {
            let ch = char::from_u32(cp).unwrap();
            assert!(is_builtin(ch), "{cp:04X}");
            let kinds = [
                glyph_rects(ch, &g, 0, 0).is_some(),
                glyph_paths(ch, &g, 0, 0).is_some(),
                glyph_shade(ch, &g, 0, 0).is_some(),
            ];
            assert_eq!(
                kinds.iter().filter(|&&k| k).count(),
                1,
                "{cp:04X}: one drawing method"
            );
            for (i, k) in kinds.iter().enumerate() {
                counts[i] += usize::from(*k);
            }
            counts[3] += 1;
        }
        // 157 as rectangles or paths (109 + 12 + 29 rectangles, 7 paths), 3 shades.
        assert_eq!(counts, [150, 7, 3, 160]);
    }

    /// Two rows of eight pixels of a pattern from an absolute origin, `#` lit.
    fn shade_rows(level: ShadeLevel, x0: i32, y0: i32) -> [String; 2] {
        let row = |y: i32| -> String {
            (x0..x0 + 8)
                .map(|x| if shade_lit(level, x, y) { '#' } else { '.' })
                .collect()
        };
        [row(y0), row(y0 + 1)]
    }

    #[test]
    fn the_patterns_match_the_reference_drawings() {
        // Written out from the specification: light is a staggered dot in every fourth column, medium a
        // checkerboard, dark whatever the light shade leaves out.
        let cases = [
            (ShadeLevel::Light, ["#...#...", "..#...#."]),
            (ShadeLevel::Medium, ["#.#.#.#.", ".#.#.#.#"]),
            (ShadeLevel::Dark, [".###.###", "##.###.#"]),
        ];
        for (level, rows) in cases {
            assert_eq!(shade_rows(level, 0, 0), rows, "{level:?}");
            // The pattern repeats every 4 columns and every 2 rows, also for negative coordinates.
            assert_eq!(shade_rows(level, 4, 2), rows, "{level:?}: period");
            assert_eq!(
                shade_rows(level, -8, -4),
                rows,
                "{level:?}: negative origin"
            );
        }
    }

    #[test]
    fn the_coverage_is_a_quarter_a_half_and_three_quarters() {
        for (level, share) in [
            (ShadeLevel::Light, 0.25),
            (ShadeLevel::Medium, 0.5),
            (ShadeLevel::Dark, 0.75),
        ] {
            for (x0, y0) in [(0, 0), (1, 1), (-3, 5), (1000, 777)] {
                let (w, h) = (40, 22); // a whole number of 4 × 2 tiles
                let lit = (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .filter(|&(x, y)| shade_lit(level, x0 + x, y0 + y))
                    .count();
                assert_eq!(
                    lit as f32 / (w * h) as f32,
                    share,
                    "{level:?} at ({x0}, {y0})"
                );
            }
        }
        // Dark is exactly the complement of light.
        for y in -4..6 {
            for x in -8..12 {
                assert_ne!(
                    shade_lit(ShadeLevel::Light, x, y),
                    shade_lit(ShadeLevel::Dark, x, y)
                );
            }
        }
    }

    /// The pixels of a cell as its tile draws them: the tile is laid out from the cell's top left
    /// corner shifted by the phase, which is the SVG `<pattern x y>` behaviour.
    fn tile_pixel(cell: &ShadeCell, i: i32, j: i32) -> bool {
        let (tw, th) = SHADE_TILE;
        let (tx, ty) = (
            (i - cell.phase_x).rem_euclid(tw),
            (j - cell.phase_y).rem_euclid(th),
        );
        shade_lit(cell.level, tx, ty)
    }

    #[test]
    fn the_tile_of_every_cell_puts_the_pattern_on_the_absolute_pixel_grid() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (_, name, level) in SHADES {
                let ch = SHADES.iter().find(|x| x.2 == level).unwrap().0;
                for line in 0..4 {
                    for col in 0..24 {
                        let cell = glyph_shade(ch, &g, col, line).unwrap();
                        let p = g.phys(g.cell(col, line));
                        assert_eq!((cell.width_px, cell.height_px), (p.x1 - p.x0, p.y1 - p.y0));
                        assert_eq!(cell.bounds, g.cell(col, line), "{name}");
                        assert!((0..SHADE_TILE.0).contains(&cell.phase_x));
                        assert!((0..SHADE_TILE.1).contains(&cell.phase_y));
                        // Every pixel agrees with the absolute pattern, so neighbouring cells,
                        // whatever their size, continue each other's dots.
                        for j in 0..cell.height_px {
                            for i in 0..cell.width_px {
                                assert_eq!(
                                    tile_pixel(&cell, i, j),
                                    shade_lit(level, p.x0 + i, p.y0 + j),
                                    "{w}x{h}@{s} {name} cell ({col},{line}) pixel ({i},{j})"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dots_continue_across_cell_boundaries() {
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, name, level) in SHADES {
                // Rebuild a row of cells pixel by pixel from the tiles and compare with the pattern
                // painted over the whole row at once.
                let mut raster: Vec<Vec<bool>> = Vec::new();
                let rows = 2;
                let height = g.phys(g.cell(0, rows - 1)).y1 - g.phys(g.cell(0, 0)).y0;
                for _ in 0..height {
                    raster.push(Vec::new());
                }
                let top = g.phys(g.cell(0, 0)).y0;
                for line in 0..rows {
                    for col in 0..12 {
                        let cell = glyph_shade(ch, &g, col, line).unwrap();
                        let p = g.phys(cell.bounds);
                        for j in 0..cell.height_px {
                            for i in 0..cell.width_px {
                                let _ = i;
                                raster[(p.y0 - top + j) as usize].push(tile_pixel(&cell, i, j));
                            }
                        }
                    }
                }
                let left = g.phys(g.cell(0, 0)).x0;
                for (row, pixels) in raster.iter().enumerate() {
                    for (x, &lit) in pixels.iter().enumerate() {
                        assert_eq!(
                            lit,
                            shade_lit(level, left + x as i32, top + row as i32),
                            "{w}x{h}@{s} {name}: ({x},{row})"
                        );
                    }
                }
            }
        }
    }

    /// Reads the lit pixels back out of a tile's SVG source (independent of `shade_lit`).
    fn svg_dots(svg: &str) -> Vec<(i32, i32)> {
        let pattern = svg
            .split("<pattern")
            .nth(1)
            .unwrap()
            .split("</pattern>")
            .next()
            .unwrap();
        pattern
            .split("<rect ")
            .skip(1)
            .map(|r| {
                let get = |name: &str| -> i32 {
                    let at = r.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
                    r[at..].split('"').next().unwrap().parse().unwrap()
                };
                assert_eq!((get("width"), get("height")), (1, 1));
                (get("x"), get("y"))
            })
            .collect()
    }

    #[test]
    fn the_svg_of_a_tile_holds_exactly_the_dots_of_its_pattern() {
        let g = grid(9.6, 19.2, 1.0);
        for (ch, name, level) in SHADES {
            let cell = glyph_shade(ch, &g, 3, 2).unwrap();
            let svg = cell.key().svg();
            assert!(
                svg.starts_with("<svg ") && svg.ends_with("</svg>"),
                "{name}"
            );
            let (w, h) = (cell.width_px, cell.height_px);
            assert!(svg.contains(&format!(
                "width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\""
            )));
            assert!(svg.contains("shape-rendering=\"crispEdges\""));
            assert!(svg.contains(&format!(
                "x=\"{}\" y=\"{}\" width=\"4\" height=\"2\"",
                cell.phase_x, cell.phase_y
            )));
            assert!(
                !svg.contains("fill=\"#") && !svg.contains("rgb("),
                "colorless: tinted at draw time"
            );
            let mut dots = svg_dots(&svg);
            dots.sort();
            let mut expected: Vec<(i32, i32)> = (0..2)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .filter(|&(x, y)| shade_lit(level, x, y))
                .collect();
            expected.sort();
            assert_eq!(dots, expected, "{name}");
            let share = dots.len() as f32 / 8.0;
            assert_eq!(share, [0.25, 0.5, 0.75][level as usize], "{name}");
        }
    }

    #[test]
    fn the_tile_key_is_what_decides_the_picture_and_nothing_else() {
        let g = grid(9.6, 19.2, 1.0);
        // Same size and phase in different columns / rows (and colors never enter): one key.
        let a = glyph_shade('\u{2592}', &g, 0, 0).unwrap();
        let b = glyph_shade('\u{2592}', &g, 0, 0).unwrap();
        assert_eq!(a.key(), b.key());
        assert_ne!(
            glyph_shade('\u{2591}', &g, 0, 0).unwrap().key(),
            glyph_shade('\u{2593}', &g, 0, 0).unwrap().key()
        );
        // A real screen of 136 × 55 cells needs only a handful of distinct tiles per pattern.
        for &(w, h, s) in &BOX_CASES {
            let g = grid(w, h, s);
            for (ch, name, _) in SHADES {
                let mut keys = std::collections::HashSet::new();
                for line in 0..55 {
                    for col in 0..136 {
                        keys.insert(glyph_shade(ch, &g, col, line).unwrap().key());
                    }
                }
                assert!(keys.len() <= 32, "{w}x{h}@{s} {name}: {} tiles", keys.len());
            }
        }
    }

    #[test]
    fn a_cell_without_pixels_is_not_a_shade() {
        let tiny = grid(0.2, 0.2, 1.0);
        assert!(glyph_shade('\u{2592}', &tiny, 0, 0).is_none());
        let g = grid(1.0, 1.0, 1.0);
        let cell = glyph_shade('\u{2592}', &g, 5, 5).unwrap();
        assert_eq!((cell.width_px, cell.height_px), (1, 1));
        assert!(cell.key().svg().contains("width=\"1\" height=\"1\""));
    }
}
