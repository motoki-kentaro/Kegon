//! Slice 0 render experiment for Kegon #74: per-cell drawing vs one `Text` per ZWJ run.
//!
//! Feeds samples through a real `Term` and draws each twice on a canvas, using the same cell metrics,
//! `Alignment::Default` and `Shaping::Advanced` as `terminal::renderer::terminal_text`:
//!   * `cell` rows: one `Text` per cell (what Kegon does today),
//!   * `run`  rows: cells joined into one `Text` while the previous cell ends in U+200D.
//! The grid is never altered; the faint vertical lines are cell boundaries, the red block is a cursor.
//!
//! `cargo run --example zwj_render -- <out.png> [scale]` renders, saves a screenshot, and exits.
//! Without arguments it just stays open.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as GridPoint};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, Processor};
use iced::advanced::text::Alignment;
use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Text};
use iced::widget::text::{LineHeight, Shaping};
use iced::{Color, Element, Font, Length, Point, Rectangle, Renderer, Size, Task, Theme};
use iced::alignment::Vertical;

const ZWJ: char = '\u{200D}';
const COLS: usize = 24;

struct Quiet;
impl EventListener for Quiet {
    fn send_event(&self, _: Event) {}
}
struct Dim(usize, usize);
impl Dimensions for Dim {
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

/// One drawable unit on a row: a cell, or a ZWJ run covering several cells.
struct Seg {
    col: usize,
    cells: usize,
    text: String,
    fg: Color,
    mixed_fg: bool,
}

fn fg_of(c: AnsiColor) -> Color {
    match c {
        AnsiColor::Spec(rgb) => Color::from_rgb8(rgb.r, rgb.g, rgb.b),
        _ => Color::from_rgb8(0xe0, 0xe0, 0xe0),
    }
}

/// Per-cell segments, exactly like the current renderer (spacers and blanks skipped).
fn cell_segs(term: &Term<Quiet>, line: i32) -> Vec<Seg> {
    let mut out = vec![];
    for col in 0..COLS {
        let cell = &term.grid()[GridPoint::new(Line(line), Column(col))];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) || cell.c == ' ' || cell.c == '\0' {
            continue;
        }
        let mut text = cell.c.to_string();
        text.extend(cell.zerowidth().into_iter().flatten());
        let w = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        out.push(Seg { col, cells: w, text, fg: fg_of(cell.fg), mixed_fg: false });
    }
    out
}

/// Fitzpatrick skin-tone modifiers (U+1F3FB..=U+1F3FF) bind to the emoji before them.
fn starts_with_skin_modifier(s: &str) -> bool {
    s.chars().next().is_some_and(|c| ('\u{1F3FB}'..='\u{1F3FF}').contains(&c))
}

/// Experimental run builder: a cell whose text ends in ZWJ pulls the next cell into the same segment.
fn run_segs(term: &Term<Quiet>, line: i32) -> Vec<Seg> {
    let mut out: Vec<Seg> = vec![];
    for seg in cell_segs(term, line) {
        match out.last_mut() {
            Some(prev)
                if prev.col + prev.cells == seg.col
                    && (prev.text.ends_with(ZWJ) || starts_with_skin_modifier(&seg.text)) =>
            {
                prev.mixed_fg |= prev.fg != seg.fg;
                prev.text.push_str(&seg.text);
                prev.cells += seg.cells;
            }
            _ => out.push(seg),
        }
    }
    out
}

struct Sample {
    title: &'static str,
    term: Term<Quiet>,
    cursor_col: usize,
}

fn sample(title: &'static str, bytes: &str, cursor_col: usize) -> Sample {
    let mut term = Term::new(Config::default(), &Dim(COLS, 3), Quiet);
    let mut p: Processor = Processor::new();
    p.advance(&mut term, bytes.as_bytes());
    Sample { title, term, cursor_col }
}

fn samples() -> Vec<Sample> {
    let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let tech = "\u{1F469}\u{200D}\u{1F4BB}";
    let tech_skin = "\u{1F469}\u{1F3FD}\u{200D}\u{1F4BB}";
    let rainbow = "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}";
    let thumbs = "\u{1F44D}\u{1F3FD}";
    let flag = "\u{1F1EF}\u{1F1F5}";
    // Different foreground per member: 31;38;2 sequences between code points.
    let mixed = "\u{1b}[38;2;255;80;80m\u{1F468}\u{200D}\u{1b}[38;2;80;255;80m\u{1F469}\u{200D}\u{1b}[38;2;80;80;255m\u{1F467}\u{1b}[0m";
    vec![
        sample("A family B (cursor on B)", &format!("A{fam}B"), 9),
        sample("A family B (cursor mid-run, col 3)", &format!("A{fam}B"), 3),
        sample("A <sp> family <sp> B", &format!("A {fam} B"), 11),
        sample("woman-tech hello", &format!("{tech} hello"), 5),
        sample("hello woman-tech world", &format!("hello {tech} world"), 11),
        sample("woman-skin-tech (modifier before ZWJ)", &format!("x{tech_skin}y"), 6),
        sample("rainbow flag (narrow base + VS16 + ZWJ)", &format!("x{rainbow}y"), 4),
        sample("thumbs+skin (no ZWJ: run builder must NOT join)", &format!("x{thumbs}y"), 5),
        sample("JP flag (Regional Indicators, no ZWJ)", &format!("x{flag}y"), 3),
        sample("mixed fg per member", &format!("x{mixed}y"), 8),
        sample("incomplete ZWJ then ASCII", "x\u{1F469}\u{200D}y", 4),
        sample("underline+strike over family", &format!("\u{1b}[4;9mA{fam}B\u{1b}[0m"), 9),
        sample("family wraps at col 6 (6-col grid not used: see probe)", &format!("{fam}{fam}"), 18),
    ]
}

struct Metrics {
    cw: f32,
    ch: f32,
    size: f32,
}

fn text(content: String, at: Point, color: Color, m: &Metrics) -> Text {
    Text {
        content,
        position: at,
        color,
        size: m.size.into(),
        font: Font::MONOSPACE,
        align_x: Alignment::Default,
        align_y: Vertical::Top,
        line_height: LineHeight::Absolute(m.ch.into()),
        shaping: Shaping::Advanced,
        max_width: f32::INFINITY,
    }
}

struct Board {
    samples: Vec<Sample>,
    m: Metrics,
}

impl<Message> canvas::Program<Message> for Board {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let m = &self.m;
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), Color::from_rgb8(0x1b, 0x1d, 0x23));
        let label = Color::from_rgb8(0x90, 0xa0, 0xb0);
        let mut y = 4.0;
        for s in &self.samples {
            frame.fill_text(Text {
                content: s.title.to_string(),
                position: Point::new(4.0, y),
                color: label,
                size: (m.size * 0.8).into(),
                font: Font::DEFAULT,
                align_x: Alignment::Default,
                align_y: Vertical::Top,
                line_height: LineHeight::Absolute(m.ch.into()),
                shaping: Shaping::Advanced,
                max_width: f32::INFINITY,
            });
            y += m.ch;
            for (tag, segs) in [("cell", cell_segs(&s.term, 0)), ("run", run_segs(&s.term, 0))] {
                let x0 = 56.0;
                // Cell boundaries.
                for c in 0..=COLS {
                    frame.fill_rectangle(
                        Point::new(x0 + c as f32 * m.cw, y),
                        Size::new(1.0, m.ch),
                        Color::from_rgba8(255, 255, 255, 0.10),
                    );
                }
                // Cursor first (then text on top, as the real renderer re-draws the cell char) --
                // in `run` mode this shows what a block cursor does to a mid-run cell.
                frame.fill_rectangle(
                    Point::new(x0 + s.cursor_col as f32 * m.cw, y),
                    Size::new(m.cw, m.ch),
                    Color::from_rgba8(230, 60, 60, 0.55),
                );
                frame.fill_text(text(tag.to_string(), Point::new(4.0, y), label, m));
                for seg in &segs {
                    let at = Point::new(x0 + seg.col as f32 * m.cw, y);
                    // Underline / strikeout the way the cell path does, per segment span.
                    if s.title.starts_with("underline") {
                        let w = seg.cells as f32 * m.cw;
                        frame.fill_rectangle(Point::new(at.x, y + m.ch - 2.0), Size::new(w, 1.0), seg.fg);
                        frame.fill_rectangle(Point::new(at.x, y + m.ch * 0.5), Size::new(w, 1.0), seg.fg);
                    }
                    let _ = seg.mixed_fg;
                    frame.fill_text(text(seg.text.clone(), at, seg.fg, m));
                }
                y += m.ch;
            }
            y += m.ch * 0.4;
        }
        vec![frame.into_geometry()]
    }
}

#[derive(Debug, Clone)]
enum Msg {
    Shoot,
    Shot(iced::window::Screenshot),
}

struct App {
    out: Option<String>,
    scale: f32,
}

impl App {
    fn boot() -> (Self, Task<Msg>) {
        let mut args = std::env::args().skip(1);
        let out = args.next();
        let scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let task = if out.is_some() {
            Task::perform(
                async {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                },
                |_| Msg::Shoot,
            )
        } else {
            Task::none()
        };
        (Self { out, scale }, task)
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Shoot => iced::window::oldest().and_then(iced::window::screenshot).map(Msg::Shot),
            Msg::Shot(shot) => {
                if let Some(path) = &self.out {
                    write_png(path, shot.size.width, shot.size.height, &shot.rgba);
                    println!("saved {path} ({}x{}, scale_factor {})", shot.size.width, shot.size.height, shot.scale_factor);
                }
                iced::exit()
            }
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        let k = self.scale;
        Canvas::new(Board {
            samples: samples(),
            m: Metrics { cw: 8.5 * k, ch: 18.0 * k, size: 13.0 * k },
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

fn main() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .window(iced::window::Settings {
            size: Size::new(480.0, 1300.0),
            ..Default::default()
        })
        .run()
}

/// Minimal PNG writer (stored deflate) so the example needs no image dependency.
fn write_png(path: &str, w: u32, h: u32, rgba: &[u8]) {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(chunks.peek().is_none() as u8);
        z.extend((c.len() as u16).to_le_bytes());
        z.extend((!(c.len() as u16)).to_le_bytes());
        z.extend(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).expect("write png");
}
