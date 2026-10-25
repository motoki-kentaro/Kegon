//! Render-run detection for emoji sequences that the grid spreads over several cells.
//!
//! `alacritty_terminal` stores a ZWJ sequence such as `👨‍👩‍👧‍👦` as one wide cell per member, each
//! carrying its trailing ZWJ in `zerowidth`. Drawn cell by cell the members show up side by side. This
//! module finds those multi-cell sequences in one physical row so a renderer can draw each as a single
//! `Text`. It is a pure function over a read-only snapshot of the row: the grid, the cell widths and the
//! copy path are untouched, and nothing here draws.
//!
//! A run is produced only when all of these hold:
//!   * the Unicode extended grapheme cluster (UAX #29, `unicode-segmentation`) spans two or more
//!     adjacent drawn cells and starts and ends exactly on cell boundaries;
//!   * the cluster is an emoji sequence (every base is an emoji code point, no Regional Indicator) or
//!     a flag: exactly two Regional Indicators (#82; see `ClusterRun::is_flag`);
//!   * every character cell in it has the same foreground, background, selection state and text
//!     attributes. Any mismatch leaves the cells to the ordinary per-cell path; colours are never merged.
//!
//! Clusters that fit in one cell (`é`, `🏳️` with VS16) are not reported: they already draw correctly.

use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::Color as AnsiColor;
use unicode_segmentation::UnicodeSegmentation;

const ZWJ: char = '\u{200D}';
const VARIATION_SELECTOR_16: char = '\u{FE0F}';

/// Flags that describe grid structure rather than appearance; they never have to match across a run.
const STRUCTURAL: Flags = Flags::WIDE_CHAR
    .union(Flags::WIDE_CHAR_SPACER)
    .union(Flags::LEADING_WIDE_CHAR_SPACER)
    .union(Flags::WRAPLINE);

/// Read-only snapshot of one grid cell, as far as run detection needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterCell {
    pub col: usize,
    pub c: char,
    pub zerowidth: Vec<char>,
    pub flags: Flags,
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    /// Selection is applied by the renderer on top of the cell, so the caller supplies it.
    pub selected: bool,
}

impl ClusterCell {
    pub fn from_grid_cell(col: usize, cell: &Cell, selected: bool) -> Self {
        Self {
            col,
            c: cell.c,
            zerowidth: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
            flags: cell.flags,
            fg: cell.fg,
            bg: cell.bg,
            selected,
        }
    }

    /// The second column of a wide cell. It belongs to the wide cell before it and is skipped without
    /// ending the chain, because adjacency is decided by column arithmetic.
    fn is_wide_spacer(&self) -> bool {
        self.flags.contains(Flags::WIDE_CHAR_SPACER)
    }

    /// Cells that draw no character and so end any sequence: blanks and the row-end padding that
    /// precedes a wrapped wide character.
    fn is_blank(&self) -> bool {
        self.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) || self.c == ' ' || self.c == '\0'
    }

    fn columns(&self) -> usize {
        if self.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        }
    }

    fn appearance(&self) -> (AnsiColor, AnsiColor, Flags, bool) {
        (
            self.fg,
            self.bg,
            self.flags.difference(STRUCTURAL),
            self.selected,
        )
    }
}

/// One emoji sequence to draw as a single `Text`, covering columns `start_col..end_col`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterRun {
    pub start_col: usize,
    /// Exclusive. Includes the trailing spacer column of a wide last member.
    pub end_col: usize,
    /// The code points of the members, in order, without spacers.
    pub text: String,
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    /// Text attributes shared by every cell (structural wide-char flags removed).
    pub flags: Flags,
    pub selected: bool,
}

impl ClusterRun {
    /// Whether this run is a flag: exactly two Regional Indicators. A flag is told apart from the ZWJ and
    /// skin-tone sequences by its text alone, so a renderer can give flags a font of their own.
    pub fn is_flag(&self) -> bool {
        is_flag_pair(&self.text)
    }
}

/// Finds the multi-cell emoji sequences in one physical row. `row` must be in column order.
pub fn detect_cluster_runs(row: &[ClusterCell]) -> Vec<ClusterRun> {
    let mut runs = Vec::new();
    let mut chain: Vec<&ClusterCell> = Vec::new();
    for cell in row.iter().filter(|c| !c.is_wide_spacer()) {
        let breaks = cell.is_blank()
            || chain
                .last()
                .is_some_and(|prev| prev.col + prev.columns() != cell.col);
        if breaks {
            detect_in_chain(&chain, &mut runs);
            chain.clear();
        }
        if !cell.is_blank() {
            chain.push(cell);
        }
    }
    detect_in_chain(&chain, &mut runs);
    runs
}

/// `chain` is a stretch of adjacent drawn cells.
fn detect_in_chain(chain: &[&ClusterCell], runs: &mut Vec<ClusterRun>) {
    if chain.len() < 2 {
        return;
    }
    let mut text = String::new();
    // Byte offset in `text` where each cell starts, plus the end.
    let mut starts = Vec::with_capacity(chain.len() + 1);
    for cell in chain {
        starts.push(text.len());
        text.push(cell.c);
        text.extend(cell.zerowidth.iter());
    }
    starts.push(text.len());

    for (offset, grapheme) in text.grapheme_indices(true) {
        let end = offset + grapheme.len();
        let (Ok(first), Ok(end_index)) =
            (starts.binary_search(&offset), starts.binary_search(&end))
        else {
            continue; // Starts or ends inside a cell.
        };
        let last = end_index - 1;
        if first == last || !(is_flag_pair(grapheme) || is_emoji_sequence(grapheme)) {
            continue;
        }
        let members = &chain[first..=last];
        let appearance = members[0].appearance();
        if members.iter().any(|m| m.appearance() != appearance) {
            continue;
        }
        let tail = members[members.len() - 1];
        let (fg, bg, flags, selected) = appearance;
        runs.push(ClusterRun {
            start_col: members[0].col,
            end_col: tail.col + tail.columns(),
            text: grapheme.to_string(),
            fg,
            bg,
            flags,
            selected,
        });
    }
}

/// A run and the screen line it sits on.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedRun {
    pub line: i32,
    pub run: ClusterRun,
}

/// What a cell does in the draw loop once the runs of the frame are known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellRole<'a> {
    /// Not part of a run: drawn as before.
    Plain,
    /// First cell of a run: draws the run's whole text.
    Start(&'a ClusterRun),
    /// Later cell of a run: its text is already part of the run, so it draws none.
    Member(&'a ClusterRun),
}

/// Walks the frame's runs in step with a row-major cell sweep, so each cell costs O(1).
pub struct RunLookup<'a> {
    runs: &'a [PositionedRun],
    next: usize,
}

impl<'a> RunLookup<'a> {
    pub fn new(runs: &'a [PositionedRun]) -> Self {
        Self { runs, next: 0 }
    }

    /// Queries must come in row-major order, as `display_iter` yields them.
    pub fn role(&mut self, line: i32, col: usize) -> CellRole<'a> {
        while let Some(p) = self.runs.get(self.next) {
            if p.line < line || (p.line == line && p.run.end_col <= col) {
                self.next += 1;
            } else {
                break;
            }
        }
        match self.runs.get(self.next) {
            Some(p) if p.line == line && p.run.start_col <= col => {
                if p.run.start_col == col {
                    CellRole::Start(&p.run)
                } else {
                    CellRole::Member(&p.run)
                }
            }
            _ => CellRole::Plain,
        }
    }
}

/// The run covering a cell, for out-of-order lookups such as the cursor.
pub fn run_covering(runs: &[PositionedRun], line: i32, col: usize) -> Option<&ClusterRun> {
    runs.iter()
        .find(|p| p.line == line && (p.run.start_col..p.run.end_col).contains(&col))
        .map(|p| &p.run)
}

/// Whether a cell can be part of a multi-cell emoji sequence: it holds a ZWJ or is a skin-tone
/// modifier. Rows without such a cell cannot contain a run, so they are never converted.
fn may_join(cell: &Cell) -> bool {
    is_skin_modifier(cell.c)
        || cell
            .zerowidth()
            .is_some_and(|z| z.iter().any(|&c| c == ZWJ || is_skin_modifier(c)))
}

/// Finds the runs of a whole screen. `cells` yields `(line, column, cell, selected)` in row-major
/// order. Only rows with a joining character are converted, so ordinary text costs one cheap scan.
///
/// Flags are not reported: until a renderer can give them a colour font, a flag run would only draw its
/// two letters closer together than the per-cell path does. See `detect_screen_runs_with_flags`.
pub fn detect_screen_runs<'a>(
    cells: impl Iterator<Item = (i32, usize, &'a Cell, bool)>,
) -> Vec<PositionedRun> {
    detect_screen(cells, false)
}

/// As `detect_screen_runs`, and also reports the flag runs (`ClusterRun::is_flag`).
pub fn detect_screen_runs_with_flags<'a>(
    cells: impl Iterator<Item = (i32, usize, &'a Cell, bool)>,
) -> Vec<PositionedRun> {
    detect_screen(cells, true)
}

/// The runs a frame draws. With `flag_support` a flag is a run when the closure accepts its text (the
/// pair can really be drawn in a flag font); a flag it rejects, and every flag without `flag_support`,
/// is left to the per-cell path. ZWJ and skin-tone runs are the same either way.
pub fn detect_frame_runs<'a>(
    cells: impl Iterator<Item = (i32, usize, &'a Cell, bool)>,
    flag_support: Option<&dyn Fn(&str) -> bool>,
) -> Vec<PositionedRun> {
    let Some(supported) = flag_support else {
        return detect_screen_runs(cells);
    };
    let mut runs = detect_screen_runs_with_flags(cells);
    runs.retain(|p| !p.run.is_flag() || supported(&p.run.text));
    runs
}

fn detect_screen<'a>(
    cells: impl Iterator<Item = (i32, usize, &'a Cell, bool)>,
    flags: bool,
) -> Vec<PositionedRun> {
    fn flush(
        line: Option<i32>,
        row: &mut Vec<ClusterCell>,
        joins: bool,
        flags: bool,
        out: &mut Vec<PositionedRun>,
    ) {
        if let (Some(line), true) = (line, joins) {
            out.extend(
                detect_cluster_runs(row)
                    .into_iter()
                    .filter(|run| flags || !run.is_flag())
                    .map(|run| PositionedRun { line, run }),
            );
        }
        row.clear();
    }

    let mut out = Vec::new();
    let mut row: Vec<ClusterCell> = Vec::new();
    let mut current = None;
    let mut joins = false;
    for (line, col, cell, selected) in cells {
        if current != Some(line) {
            flush(current, &mut row, joins, flags, &mut out);
            current = Some(line);
            joins = false;
        }
        joins |= may_join(cell) || (flags && is_regional_indicator(cell.c));
        row.push(ClusterCell::from_grid_cell(col, cell, selected));
    }
    flush(current, &mut row, joins, flags, &mut out);
    out
}

/// A grapheme joined from several cells is only a run when it is an emoji sequence: every base is an
/// emoji code point, none is a Regional Indicator (flags stay per cell), and it has a joining device
/// (ZWJ, or a skin-tone modifier after a base). Other multi-cell clusters, such as scripts that
/// combine across cells, are left to the per-cell path.
fn is_emoji_sequence(grapheme: &str) -> bool {
    let mut joined = false;
    let mut bases = 0;
    for ch in grapheme.chars() {
        match ch {
            ZWJ => joined = true,
            VARIATION_SELECTOR_16 => {}
            _ if is_skin_modifier(ch) => joined |= bases > 0,
            _ if is_regional_indicator(ch) => return false,
            _ if is_emoji_base(ch) => bases += 1,
            _ => return false,
        }
    }
    joined && bases > 0
}

/// Exactly two Regional Indicators: what UAX #29 makes of a national flag (`🇯🇵`). A longer RI string is
/// cut into such pairs by the grapheme segmentation, and a lone RI is a single cell, never a run.
fn is_flag_pair(grapheme: &str) -> bool {
    let mut chars = grapheme.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(a), Some(b), None) if is_regional_indicator(a) && is_regional_indicator(b)
    )
}

fn is_skin_modifier(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Approximation of Extended_Pictographic: the emoji blocks plus the scattered BMP symbols that
/// ZWJ sequences use (e.g. ⚕ ♀ ♂ ❤ ✈). Deliberately a block list, not a full property table.
fn is_emoji_base(c: char) -> bool {
    matches!(
        c as u32,
        0x00A9
            | 0x00AE
            | 0x203C
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x2194..=0x21AA
            | 0x231A..=0x23FF
            | 0x24C2
            | 0x25AA..=0x25FE
            | 0x2600..=0x27BF
            | 0x2934..=0x2935
            | 0x2B05..=0x2B55
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x1F000..=0x1F1E5
            | 0x1F200..=0x1FAFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::{Event, EventListener};
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::{Processor, Rgb};

    struct Quiet;
    impl EventListener for Quiet {
        fn send_event(&self, _: Event) {}
    }
    struct Size(usize, usize);
    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            self.1
        }
        fn screen_lines(&self) -> usize {
            self.1
        }
        fn columns(&self) -> usize {
            self.0
        }
    }

    const FAMILY: &str = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    const TECH: &str = "👩\u{200D}💻";
    const ASTRONAUT: &str = "🧑\u{200D}🚀";
    const TECH_SKIN: &str = "👩🏽\u{200D}💻";
    const RAINBOW: &str = "🏳\u{FE0F}\u{200D}🌈";
    const THUMBS_SKIN: &str = "👍🏽";

    fn term(cols: usize, input: &str) -> Term<Quiet> {
        let mut term = Term::new(Config::default(), &Size(cols, 4), Quiet);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, input.as_bytes());
        term
    }

    fn row(term: &Term<Quiet>, line: i32) -> Vec<ClusterCell> {
        (0..term.columns())
            .map(|c| {
                let cell = &term.grid()[Point::new(Line(line), Column(c))];
                ClusterCell::from_grid_cell(c, cell, false)
            })
            .collect()
    }

    fn runs(cols: usize, input: &str) -> Vec<ClusterRun> {
        detect_cluster_runs(&row(&term(cols, input), 0))
    }

    fn spans(runs: &[ClusterRun]) -> Vec<(usize, usize, &str)> {
        runs.iter()
            .map(|r| (r.start_col, r.end_col, r.text.as_str()))
            .collect()
    }

    #[test]
    fn family_is_one_run_over_eight_columns_with_exact_code_points() {
        let r = runs(20, &format!("A{FAMILY}B"));
        assert_eq!(spans(&r), [(1, 9, FAMILY)]);
        assert!(
            !r[0].text.contains(' '),
            "spacers must not leak into the text"
        );
    }

    #[test]
    fn following_text_keeps_its_column() {
        let t = term(20, &format!("A{FAMILY}B"));
        assert_eq!(t.grid()[Point::new(Line(0), Column(9))].c, 'B');
        let cells = row(&t, 0);
        let run = &detect_cluster_runs(&cells)[0];
        assert_eq!(run.end_col, 9);
        assert_eq!(cells[run.end_col].c, 'B');
    }

    #[test]
    fn two_member_sequences() {
        assert_eq!(spans(&runs(20, &format!("{TECH} hello"))), [(0, 4, TECH)]);
        assert_eq!(spans(&runs(20, ASTRONAUT)), [(0, 4, ASTRONAUT)]);
        assert_eq!(
            spans(&runs(20, &format!("hello {TECH} world"))),
            [(6, 10, TECH)]
        );
    }

    #[test]
    fn skin_tone_before_zwj_makes_three_members() {
        assert_eq!(spans(&runs(20, TECH_SKIN)), [(0, 6, TECH_SKIN)]);
    }

    #[test]
    fn rainbow_flag_joins_a_narrow_base_with_a_wide_member() {
        assert_eq!(spans(&runs(20, &format!("x{RAINBOW}y"))), [(1, 4, RAINBOW)]);
    }

    #[test]
    fn skin_tone_modifier_without_zwj_joins() {
        assert_eq!(spans(&runs(20, THUMBS_SKIN)), [(0, 4, THUMBS_SKIN)]);
    }

    #[test]
    fn two_sequences_separated_by_a_space() {
        let r = runs(30, &format!("{TECH} {ASTRONAUT}"));
        assert_eq!(spans(&r), [(0, 4, TECH), (5, 9, ASTRONAUT)]);
    }

    #[test]
    fn adjacent_sequences_without_a_gap_are_not_merged() {
        let r = runs(30, &format!("{TECH}{ASTRONAUT}"));
        assert_eq!(spans(&r), [(0, 4, TECH), (4, 8, ASTRONAUT)]);
    }

    #[test]
    fn single_cell_clusters_are_not_runs() {
        assert!(runs(20, "e\u{0301}").is_empty());
        assert!(runs(20, "🏳\u{FE0F}").is_empty());
        assert!(runs(20, "👩").is_empty());
    }

    #[test]
    fn mixed_scripts_are_never_joined() {
        assert!(runs(30, "hello 日本語 שלום مرحبا abc").is_empty());
        let r = runs(30, &format!("日本{TECH}語 שלום"));
        assert_eq!(spans(&r), [(4, 8, TECH)]);
    }

    const JP: &str = "🇯🇵";
    const US: &str = "🇺🇸";
    const GB: &str = "🇬🇧";
    const DE: &str = "🇩🇪";
    const RI_J: &str = "\u{1F1EF}";
    const RI_P: &str = "\u{1F1F5}";

    #[test]
    fn flags_are_two_cell_runs() {
        for flag in [JP, US, GB, DE] {
            let r = runs(20, flag);
            assert_eq!(spans(&r), [(0, 2, flag)], "{flag}");
            assert!(r[0].is_flag());
        }
    }

    #[test]
    fn a_flag_keeps_the_following_text_at_its_column() {
        let t = term(20, &format!("A{JP}B"));
        let cells = row(&t, 0);
        let r = detect_cluster_runs(&cells);
        assert_eq!(spans(&r), [(1, 3, JP)]);
        assert_eq!(cells[r[0].end_col].c, 'B');
    }

    #[test]
    fn a_lone_regional_indicator_is_not_a_run() {
        assert!(runs(20, RI_J).is_empty());
        assert!(runs(20, &format!("A{RI_J}B")).is_empty());
        assert!(runs(20, &format!("{RI_J} {RI_P}")).is_empty());
    }

    #[test]
    fn regional_indicator_strings_are_paired_from_the_start() {
        let ri = |n: usize| ["🇯", "🇵", "🇺", "🇸", "🇩"][..n].concat();
        assert_eq!(spans(&runs(20, &ri(3))), [(0, 2, JP)]);
        assert_eq!(spans(&runs(20, &ri(4))), [(0, 2, JP), (2, 4, US)]);
        let five = runs(20, &ri(5));
        assert_eq!(spans(&five), [(0, 2, JP), (2, 4, US)]);
        assert!(five.iter().all(ClusterRun::is_flag));
    }

    #[test]
    fn different_flags_in_a_row_stay_separate_runs() {
        let r = runs(20, &format!("{JP}{US}{GB}"));
        assert_eq!(spans(&r), [(0, 2, JP), (2, 4, US), (4, 6, GB)]);
    }

    #[test]
    fn flags_beside_zwj_sequences_stay_separate_and_are_told_apart() {
        let r = runs(30, &format!("{JP}{TECH}{US}"));
        assert_eq!(spans(&r), [(0, 2, JP), (2, 6, TECH), (6, 8, US)]);
        let kinds: Vec<_> = r.iter().map(ClusterRun::is_flag).collect();
        assert_eq!(kinds, [true, false, true]);
    }

    #[test]
    fn emoji_sequences_are_never_flags() {
        for s in [FAMILY, TECH, ASTRONAUT, TECH_SKIN, RAINBOW, THUMBS_SKIN] {
            let r = runs(30, s);
            assert_eq!(r.len(), 1, "{s}");
            assert!(!r[0].is_flag(), "{s}");
        }
    }

    #[test]
    fn a_regional_indicator_in_a_longer_cluster_is_not_a_flag() {
        // A ZWJ joins the pair to a following emoji: one cluster of more than two code points.
        let r = runs(30, &format!("{JP}\u{200D}👩"));
        assert!(r.iter().all(|r| !r.is_flag()));
    }

    #[test]
    fn flag_colour_attribute_and_background_mismatch_fall_back_to_cells() {
        assert!(runs(20, "\x1b[38;2;255;0;0m🇯\x1b[38;2;0;255;0m🇵\x1b[0m").is_empty());
        assert!(runs(20, "\x1b[41m🇯\x1b[42m🇵\x1b[0m").is_empty());
        assert!(runs(20, "\x1b[1m🇯\x1b[0m🇵").is_empty());
        assert!(runs(20, "🇯\x1b[4m🇵\x1b[0m").is_empty());
        let uniform = runs(20, &format!("\x1b[38;2;1;2;3;48;2;4;5;6;1m{JP}\x1b[0m"));
        assert_eq!(spans(&uniform), [(0, 2, JP)]);
    }

    #[test]
    fn flag_selection_boundary_falls_back_but_a_whole_selection_stays_a_run() {
        let mut cells = row(&term(20, JP), 0);
        cells[1].selected = true;
        assert!(detect_cluster_runs(&cells).is_empty());
        cells[0].selected = true;
        let r = detect_cluster_runs(&cells);
        assert_eq!(spans(&r), [(0, 2, JP)]);
        assert!(r[0].selected);
    }

    #[test]
    fn a_flag_at_the_row_end_is_a_run_and_a_wrapped_one_is_not() {
        assert_eq!(spans(&runs(4, &format!("ab{JP}"))), [(2, 4, JP)]);
        // The first indicator ends row 0 and the second starts row 1: never joined across rows.
        let t = term(3, &format!("ab{JP}"));
        assert!(detect_cluster_runs(&row(&t, 0)).is_empty());
        assert!(detect_cluster_runs(&row(&t, 1)).is_empty());
        assert!(screen_runs_with_flags(&t, |_| false).is_empty());
    }

    #[test]
    fn flags_do_not_change_the_grid_or_the_copy_text() {
        let t = term(20, &format!("A{JP}B"));
        let cell = &t.grid()[Point::new(Line(0), Column(1))];
        assert_eq!((cell.c, cell.zerowidth()), ('\u{1F1EF}', None));
        assert!(!cell.flags.contains(Flags::WIDE_CHAR));
        let start = Point::new(Line(0), Column(0));
        let end = Point::new(Line(0), Column(3));
        assert_eq!(t.bounds_to_string(start, end), format!("A{JP}B"));
    }

    #[test]
    fn rtl_text_beside_a_flag_is_left_alone() {
        let r = runs(30, &format!("שלום {JP} مرحبا"));
        assert_eq!(spans(&r), [(5, 7, JP)]);
    }

    #[test]
    fn incomplete_zwj_does_not_swallow_the_next_character() {
        assert!(runs(20, "👩\u{200D}").is_empty());
        assert!(runs(20, "👩\u{200D}x").is_empty());
        assert!(runs(20, "👩\u{200D} x").is_empty());
    }

    #[test]
    fn leading_zwj_is_dropped_by_the_grid_and_makes_no_run() {
        assert!(runs(20, "\u{200D}💻").is_empty());
    }

    #[test]
    fn zwj_between_non_emoji_is_not_a_run() {
        assert!(runs(20, "a\u{200D}b").is_empty());
        assert!(runs(20, "日\u{200D}本").is_empty());
    }

    #[test]
    fn wrapped_sequence_is_split_across_rows_and_never_joined() {
        // Six columns: the first member (col 3) fits, the second needs a leading spacer at col 5.
        let t = term(6, &format!("abc{FAMILY}"));
        let first = row(&t, 0);
        assert!(first[5].flags.contains(Flags::LEADING_WIDE_CHAR_SPACER));
        // Row 0 holds only the first member, so there is nothing to join.
        assert!(detect_cluster_runs(&first).is_empty());
        // The continuation row holds the remaining three members as their own run.
        let next = detect_cluster_runs(&row(&t, 1));
        assert_eq!(spans(&next), [(0, 6, "👩\u{200D}👧\u{200D}👦")]);
    }

    #[test]
    fn leading_spacer_is_not_text() {
        let t = term(6, &format!("abcde{TECH}"));
        let cells = row(&t, 0);
        assert!(cells[5].flags.contains(Flags::LEADING_WIDE_CHAR_SPACER));
        assert!(detect_cluster_runs(&cells).is_empty());
    }

    #[test]
    fn foreground_change_falls_back_to_cells() {
        let input = "\x1b[38;2;255;0;0m👨\u{200D}\x1b[38;2;0;255;0m👩\x1b[0m";
        assert!(runs(20, input).is_empty());
    }

    #[test]
    fn background_change_falls_back_to_cells() {
        let input = "\x1b[41m👨\u{200D}\x1b[42m👩\x1b[0m";
        assert!(runs(20, input).is_empty());
    }

    #[test]
    fn attribute_change_falls_back_to_cells() {
        assert!(runs(20, "\x1b[1m👨\u{200D}\x1b[0m👩").is_empty());
        assert!(runs(20, "👨\u{200D}\x1b[4m👩\x1b[0m").is_empty());
        assert!(runs(20, "\x1b[9m👨\u{200D}\x1b[0m👩").is_empty());
    }

    #[test]
    fn uniform_colour_and_attributes_still_join_and_are_reported() {
        let r = runs(20, &format!("\x1b[38;2;1;2;3;48;2;4;5;6;1;4m{TECH}\x1b[0m"));
        assert_eq!(spans(&r), [(0, 4, TECH)]);
        assert_eq!(r[0].flags, Flags::BOLD | Flags::UNDERLINE);
        assert_eq!(r[0].fg, AnsiColor::Spec(Rgb { r: 1, g: 2, b: 3 }));
        assert_eq!(r[0].bg, AnsiColor::Spec(Rgb { r: 4, g: 5, b: 6 }));
    }

    #[test]
    fn spacer_attributes_do_not_block_a_run() {
        let mut cells = row(&term(20, TECH), 0);
        cells[1].bg = AnsiColor::Spec(Rgb { r: 9, g: 9, b: 9 });
        cells[3].fg = AnsiColor::Spec(Rgb { r: 9, g: 9, b: 9 });
        assert_eq!(spans(&detect_cluster_runs(&cells)), [(0, 4, TECH)]);
    }

    #[test]
    fn selection_boundary_inside_a_sequence_falls_back() {
        let mut cells = row(&term(20, TECH), 0);
        cells[2].selected = true;
        assert!(detect_cluster_runs(&cells).is_empty());
        cells[0].selected = true;
        let r = detect_cluster_runs(&cells);
        assert_eq!(spans(&r), [(0, 4, TECH)]);
        assert!(r[0].selected);
    }

    #[test]
    fn non_adjacent_cells_are_never_joined() {
        let mut cells = row(&term(20, TECH), 0);
        cells[2].col += 1; // a one-column gap between the two members
        assert!(detect_cluster_runs(&cells).is_empty());
    }

    #[test]
    fn detection_does_not_touch_the_grid_and_copy_text_is_unchanged() {
        let input = format!("A{FAMILY}B");
        let t = term(20, &input);
        let snapshot = |t: &Term<Quiet>| -> Vec<Cell> {
            (0..20)
                .map(|c| t.grid()[Point::new(Line(0), Column(c))].clone())
                .collect()
        };
        let before = snapshot(&t);
        let _ = detect_cluster_runs(&row(&t, 0));
        assert_eq!(before, snapshot(&t));
        let copied = t.bounds_to_string(
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(19)),
        );
        assert_eq!(copied.trim_end(), input);
    }

    #[test]
    fn rtl_cells_are_left_alone_and_the_run_beside_them_is_found() {
        let r = runs(30, &format!("שלום {TECH} مرحبا"));
        assert_eq!(spans(&r), [(5, 9, TECH)]);
    }

    fn screen_runs(t: &Term<Quiet>, selected: impl Fn(Point) -> bool) -> Vec<PositionedRun> {
        detect_screen_runs(
            t.grid()
                .display_iter()
                .map(|c| (c.point.line.0, c.point.column.0, c.cell, selected(c.point))),
        )
    }

    fn screen_runs_with_flags(
        t: &Term<Quiet>,
        selected: impl Fn(Point) -> bool,
    ) -> Vec<PositionedRun> {
        detect_screen_runs_with_flags(
            t.grid()
                .display_iter()
                .map(|c| (c.point.line.0, c.point.column.0, c.cell, selected(c.point))),
        )
    }

    fn roles(t: &Term<Quiet>, runs: &[PositionedRun], line: i32) -> Vec<&'static str> {
        let mut lookup = RunLookup::new(runs);
        // Earlier lines are swept first, as `display_iter` does.
        for l in 0..line {
            for c in 0..t.columns() {
                lookup.role(l, c);
            }
        }
        (0..t.columns())
            .map(|c| match lookup.role(line, c) {
                CellRole::Plain => "plain",
                CellRole::Start(_) => "start",
                CellRole::Member(_) => "member",
            })
            .collect()
    }

    #[test]
    fn only_the_first_cell_of_a_run_draws_and_the_rest_are_members() {
        let t = term(14, &format!("A{FAMILY}B"));
        let runs = screen_runs(&t, |_| false);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].line, runs[0].run.start_col, runs[0].run.end_col),
            (0, 1, 9)
        );
        let r = roles(&t, &runs, 0);
        assert_eq!(r[0], "plain"); // A
        assert_eq!(r[1], "start");
        assert!(r[2..9].iter().all(|r| *r == "member"));
        assert_eq!(r[9], "plain"); // B stays at its grid column
        assert!(r[10..].iter().all(|r| *r == "plain"));
    }

    #[test]
    fn runs_on_several_lines_are_each_found_with_their_line() {
        let t = term(14, &format!("{TECH}\r\n{ASTRONAUT} x\r\nplain"));
        let runs = screen_runs(&t, |_| false);
        let at: Vec<_> = runs.iter().map(|p| (p.line, p.run.start_col)).collect();
        assert_eq!(at, [(0, 0), (1, 0)]);
        assert_eq!(
            roles(&t, &runs, 1)[..5],
            ["start", "member", "member", "member", "plain"]
        );
        assert!(roles(&t, &runs, 2).iter().all(|r| *r == "plain"));
    }

    #[test]
    fn rows_without_a_joining_character_are_not_converted() {
        let t = term(30, "hello 日本語 שלום e\u{0301} 🇯🇵 👩 👩\u{200D}");
        assert!(screen_runs(&t, |_| false).is_empty());
    }

    #[test]
    fn cursor_lookup_finds_the_run_from_any_member_column() {
        let t = term(14, &format!("A{FAMILY}B"));
        let runs = screen_runs(&t, |_| false);
        for col in 1..9 {
            assert_eq!(
                run_covering(&runs, 0, col).map(|r| r.text.as_str()),
                Some(FAMILY)
            );
        }
        assert!(run_covering(&runs, 0, 0).is_none());
        assert!(run_covering(&runs, 0, 9).is_none());
        assert!(run_covering(&runs, 1, 3).is_none());
    }

    #[test]
    fn selection_inside_a_sequence_falls_back_but_a_whole_selection_stays_a_run() {
        let t = term(14, &format!("A{FAMILY}B"));
        let partial = screen_runs(&t, |p| p.column.0 == 3);
        assert!(partial.is_empty());
        let whole = screen_runs(&t, |p| (1..9).contains(&p.column.0));
        assert_eq!(whole.len(), 1);
        assert!(whole[0].run.selected);
    }

    #[test]
    fn a_sequence_with_a_wrapped_tail_keeps_each_row_a_separate_run() {
        let t = term(6, &format!("abc{FAMILY}"));
        let runs = screen_runs(&t, |_| false);
        let at: Vec<_> = runs
            .iter()
            .map(|p| (p.line, p.run.start_col, p.run.end_col))
            .collect();
        assert_eq!(at, [(1, 0, 6)]);
    }

    #[test]
    fn the_renderer_entry_point_still_ignores_flags() {
        let t = term(30, &format!("A{JP}B {TECH} {US}"));
        let plain = screen_runs(&t, |_| false);
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].run.text, TECH);
        let all = screen_runs_with_flags(&t, |_| false);
        let texts: Vec<_> = all.iter().map(|p| p.run.text.as_str()).collect();
        assert_eq!(texts, [JP, TECH, US]);
    }

    #[test]
    fn a_flag_is_found_from_either_of_its_cells() {
        let t = term(14, &format!("A{JP}B"));
        let runs = screen_runs_with_flags(&t, |_| false);
        assert_eq!(
            roles(&t, &runs, 0)[..4],
            ["plain", "start", "member", "plain"]
        );
        for col in 1..3 {
            let run = run_covering(&runs, 0, col).expect("covered");
            assert_eq!(run.text, JP);
            assert!(run.is_flag());
        }
        assert!(run_covering(&runs, 0, 0).is_none());
        assert!(run_covering(&runs, 0, 3).is_none());
    }

    #[test]
    fn flag_selection_at_screen_level_follows_the_same_rule() {
        let t = term(14, &format!("A{JP}B"));
        assert!(screen_runs_with_flags(&t, |p| p.column.0 == 2).is_empty());
        let whole = screen_runs_with_flags(&t, |p| (1..3).contains(&p.column.0));
        assert_eq!(whole.len(), 1);
    }

    fn frame_runs(
        t: &Term<Quiet>,
        flag_support: Option<&dyn Fn(&str) -> bool>,
    ) -> Vec<PositionedRun> {
        detect_frame_runs(
            t.grid()
                .display_iter()
                .map(|c| (c.point.line.0, c.point.column.0, c.cell, false)),
            flag_support,
        )
    }

    fn texts(runs: &[PositionedRun]) -> Vec<&str> {
        runs.iter().map(|p| p.run.text.as_str()).collect()
    }

    #[test]
    fn frame_runs_without_flag_support_leave_every_flag_to_the_cells() {
        let t = term(40, &format!("A{JP}B {TECH} {US}{GB}"));
        assert_eq!(texts(&frame_runs(&t, None)), [TECH]);
    }

    #[test]
    fn frame_runs_with_flag_support_draw_each_pair_as_its_own_run() {
        let t = term(40, &format!("A{JP}B {TECH} {US}{GB}"));
        let all = |_: &str| true;
        assert_eq!(texts(&frame_runs(&t, Some(&all))), [JP, TECH, US, GB]);
    }

    #[test]
    fn frame_runs_drop_only_the_flags_the_font_cannot_draw() {
        let t = term(40, &format!("{JP}{US} {TECH} {GB}"));
        let not_us = |text: &str| text != US;
        let r = frame_runs(&t, Some(&not_us));
        assert_eq!(texts(&r), [JP, TECH, GB]);
        // The rejected flag's cells are plain again, and the ones beside it keep their roles.
        assert_eq!(roles(&t, &r, 0)[..4], ["start", "member", "plain", "plain"]);
    }

    #[test]
    fn frame_runs_leave_zwj_and_skin_runs_alone_whatever_the_flag_font_says() {
        let t = term(60, &format!("{FAMILY} {TECH} {TECH_SKIN} {THUMBS_SKIN}"));
        let none = |_: &str| false;
        let all = |_: &str| true;
        let expected = [FAMILY, TECH, TECH_SKIN, THUMBS_SKIN];
        assert_eq!(texts(&frame_runs(&t, None)), expected);
        assert_eq!(texts(&frame_runs(&t, Some(&none))), expected);
        assert_eq!(texts(&frame_runs(&t, Some(&all))), expected);
    }

    #[test]
    fn frame_flag_runs_follow_the_selection_rule() {
        let t = term(14, &format!("A{JP}B"));
        let all = |_: &str| true;
        let detect = |sel: fn(usize) -> bool| {
            detect_frame_runs(
                t.grid().display_iter().map(|c| {
                    (
                        c.point.line.0,
                        c.point.column.0,
                        c.cell,
                        sel(c.point.column.0),
                    )
                }),
                Some(&all),
            )
        };
        assert!(detect(|c| c == 2).is_empty());
        assert_eq!(detect(|c| (1..3).contains(&c)).len(), 1);
    }
}
