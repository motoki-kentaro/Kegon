//! Slice 0 probe for Kegon #74: how ZWJ / emoji sequences land in the alacritty_terminal grid.
//!
//! Headless: feeds each sample through a real `Term`, dumps every occupied cell, and round-trips the
//! row through `bounds_to_string` (the copy path). Run with `cargo run --example zwj_probe`.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

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

fn cps(s: &str) -> String {
    s.chars()
        .map(|c| format!("U+{:04X}", c as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

fn probe(label: &str, text: &str, cols: usize) {
    let mut term = Term::new(Config::default(), &Size(cols, 4), Quiet);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, text.as_bytes());

    println!("=== {label}: {text:?}  (cols={cols})");
    println!("input : {}", cps(text));
    let cursor = term.grid().cursor.point;
    let mut used = 0usize;
    for line in 0..2 {
        for col in 0..cols {
            let cell = &term.grid()[Point::new(Line(line), Column(col))];
            let blank = cell.c == ' ' && cell.zerowidth().is_none() && cell.flags.is_empty();
            if blank {
                continue;
            }
            if line == 0 {
                used = col + 1;
            }
            let zw: Vec<String> = cell
                .zerowidth()
                .map(|z| z.iter().map(|c| format!("U+{:04X}", *c as u32)).collect())
                .unwrap_or_default();
            let mut flags = vec![];
            for (f, n) in [
                (Flags::WIDE_CHAR, "WIDE"),
                (Flags::WIDE_CHAR_SPACER, "SPACER"),
                (Flags::LEADING_WIDE_CHAR_SPACER, "LEADING_SPACER"),
                (Flags::WRAPLINE, "WRAP"),
            ] {
                if cell.flags.contains(f) {
                    flags.push(n);
                }
            }
            println!(
                "  L{line} C{col:<2} c=U+{:04X} zw=[{}] flags={flags:?}",
                cell.c as u32,
                zw.join(",")
            );
        }
    }
    println!(
        "cursor: L{} C{}  occupied cols on L0: {used}",
        cursor.line.0, cursor.column.0
    );
    // Copy through the last used line (a wrapped row needs its continuation; copying row 0 alone
    // trips an upstream `line - 1` lookup when it ends in a LEADING_WIDE_CHAR_SPACER).
    let last_line = if cursor.line.0 > 0 { 1 } else { 0 };
    let copied = term.bounds_to_string(
        Point::new(Line(0), Column(0)),
        Point::new(Line(last_line), Column(cols - 1)),
    );
    let copied = copied.trim_end_matches('\n').to_string();
    println!("copy  : {}", cps(&copied));
    println!(
        "copy == input: {}",
        if copied == text { "YES" } else { "NO" }
    );
    println!();
}

fn main() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let woman_tech = "\u{1F469}\u{200D}\u{1F4BB}";
    let woman_tech_skin = "\u{1F469}\u{1F3FD}\u{200D}\u{1F4BB}";
    let rainbow = "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}";
    let thumbs_skin = "\u{1F44D}\u{1F3FD}";
    let jp = "\u{1F1EF}\u{1F1F5}";
    let e_acute = "e\u{0301}";
    let cases: Vec<(&str, String)> = vec![
        ("family tight", format!("A{family}B")),
        ("family spaced", format!("A {family} B")),
        ("woman tech", format!("{woman_tech} hello")),
        ("woman tech mid", format!("hello {woman_tech} world")),
        ("woman tech skin", woman_tech_skin.to_string()),
        ("rainbow flag", rainbow.to_string()),
        ("thumbs skin", thumbs_skin.to_string()),
        ("JP flag", jp.to_string()),
        ("e + U+0301", e_acute.to_string()),
        ("incomplete ZWJ (trailing)", "\u{1F469}\u{200D}".to_string()),
        (
            "incomplete ZWJ then ASCII",
            "\u{1F469}\u{200D}x".to_string(),
        ),
        ("ZWJ at line start", "\u{200D}\u{1F4BB}".to_string()),
    ];
    for (label, text) in &cases {
        probe(label, text, 20);
    }
    // Wrap at the row end: 4 columns, family emoji starting at column 2 -> wide char straddles.
    probe("wrap: family at col 3 of 6", &format!("abc{family}"), 6);
    probe("wrap: family at col 5 of 6", &format!("abcde{family}"), 6);
}
