//! Working-directory reports from the shell.
//!
//! `alacritty_terminal` (via `vte`) ignores OSC 7, so the PTY output is also passed through a
//! small framer ([`CwdScanner`]) that picks out only the OSC sequences below. They are produced by the Shell Integration in [`crate::terminal::shell`]:
//!
//! * `OSC 7 ; file://<host>/<percent-encoded path> ST` — PowerShell (and any shell that speaks
//!   OSC 7, such as one on a remote host).
//! * `OSC 9 ; 9 ; <path> ST` — the ConEmu / Windows Terminal form that `cmd.exe` can emit from
//!   its `PROMPT` string, which cannot percent-encode.
//!
//! A report is only ever the shell's own statement about its current directory. Reports that do
//! not name a local directory (another host, malformed, relative) are dropped, never guessed at.

use std::path::PathBuf;

/// Why a report was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdRejection {
    /// Not a `file:` URI.
    NotFileUri,
    /// The URI names a host other than this machine (for example a remote shell).
    RemoteHost,
    /// Bad percent-encoding, invalid UTF-8, control characters, query or fragment.
    Malformed,
    /// The path is not absolute.
    NotAbsolute,
}

/// Parses the URI payload of an OSC 7 report into a local path.
///
/// `local_host` is this machine's name; a URI host that is empty, `localhost`, or equal to it
/// (case-insensitively) is local. An empty host with a path starting `//` is a UNC path
/// (`file:////server/share/dir`). Any other host is refused rather than reinterpreted as a UNC
/// server, because a remote shell reports its own host.
pub fn parse_osc7_uri(uri: &str, local_host: &str) -> Result<PathBuf, CwdRejection> {
    let rest = strip_scheme(uri).ok_or(CwdRejection::NotFileUri)?;
    let (host, path) = match rest.find('/') {
        Some(slash) => rest.split_at(slash),
        None => return Err(CwdRejection::NotAbsolute),
    };
    if path.contains(['?', '#']) {
        return Err(CwdRejection::Malformed);
    }
    let is_local = host.is_empty()
        || host.eq_ignore_ascii_case("localhost")
        || (!local_host.is_empty() && host.eq_ignore_ascii_case(local_host));
    if !is_local {
        return Err(CwdRejection::RemoteHost);
    }
    let decoded = percent_decode(path).ok_or(CwdRejection::Malformed)?;
    path_from_decoded(&decoded)
}

fn strip_scheme(uri: &str) -> Option<&str> {
    let (scheme, rest) = uri.split_once("://")?;
    scheme.eq_ignore_ascii_case("file").then_some(rest)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Turns a decoded URI path (`/C:/dir`, `//server/share/dir`, `/home/x`) into a native path.
fn path_from_decoded(decoded: &str) -> Result<PathBuf, CwdRejection> {
    if decoded.chars().any(char::is_control) {
        return Err(CwdRejection::Malformed);
    }
    let bytes = decoded.as_bytes();
    let is_drive = bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && bytes.get(3).is_none_or(|b| *b == b'/');
    if is_drive {
        // `/C:` alone means the drive root.
        let mut path = decoded[1..].replace('/', "\\");
        if path.len() == 2 {
            path.push('\\');
        }
        return Ok(PathBuf::from(path));
    }
    if cfg!(windows) {
        if let Some(unc) = decoded.strip_prefix("//")
            && !unc.starts_with('/')
            && !unc.is_empty()
        {
            return Ok(PathBuf::from(format!("\\\\{}", unc.replace('/', "\\"))));
        }
        // A bare `/dir` has no drive and is not a usable absolute path here.
        return Err(CwdRejection::NotAbsolute);
    }
    Ok(PathBuf::from(decoded))
}

/// Parses the payload of `OSC 9 ; 9 ; <path>`: an absolute local path, optionally quoted.
pub fn parse_osc9_9_path(raw: &str) -> Result<PathBuf, CwdRejection> {
    let path = raw.trim();
    let path = path
        .strip_prefix('"')
        .and_then(|p| p.strip_suffix('"'))
        .unwrap_or(path);
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(CwdRejection::Malformed);
    }
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() && !path.starts_with("\\\\.\\") {
        Ok(candidate)
    } else {
        Err(CwdRejection::NotAbsolute)
    }
}

/// Name of this machine as it appears in `file://` URIs from the local shell.
pub fn local_host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

#[allow(dead_code)]
const MAX_PAYLOAD: usize = 8 * 1024;

#[allow(dead_code)]
const ESC: u8 = 0x1b;
#[allow(dead_code)]
const BEL: u8 = 0x07;
#[allow(dead_code)]
const CAN: u8 = 0x18;
#[allow(dead_code)]
const SUB: u8 = 0x1a;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// Outside any escape sequence.
    Ground,
    /// After `ESC`.
    Esc,
    /// Inside `ESC ]` … waiting for the terminator.
    Osc,
    /// Inside an OSC, after an `ESC` that may begin `ST` (`ESC \`).
    OscEsc,
}

/// Streaming extractor of working-directory reports from raw PTY output.
///
/// A deliberately small framer for exactly two OSC sequences, not a terminal parser: the
/// emulator proper stays `alacritty_terminal`'s. `vte`'s own parser was rejected for this job
/// because its OSC buffer is unbounded and it dispatches a sequence that was merely interrupted,
/// which would turn a truncated path into a "report".
///
/// Rules: a report counts only when closed by `BEL` or `ESC \`. `CAN`, `SUB`, or an `ESC`
/// followed by anything else abandons the sequence. The payload is capped at [`MAX_PAYLOAD`]; an
/// overlong one is discarded unread, so memory stays bounded however much output arrives. State
/// carries across `scan` calls, so a sequence split across reads is still recognized.
#[allow(dead_code)]
pub struct CwdScanner {
    local_host: String,
    state: Framing,
    payload: Vec<u8>,
    /// The current OSC exceeded the cap; skip it to its end without dispatching.
    overlong: bool,
}

#[allow(dead_code)]
impl CwdScanner {
    pub fn new(local_host: String) -> Self {
        Self {
            local_host,
            state: Framing::Ground,
            payload: Vec::new(),
            overlong: false,
        }
    }

    /// Feeds PTY output and returns the working directories reported in it, in order.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                Framing::Ground => match bytes[i..].iter().position(|&b| b == ESC) {
                    Some(offset) => {
                        i += offset + 1;
                        self.state = Framing::Esc;
                    }
                    None => i = bytes.len(),
                },
                Framing::Esc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b']' => self.begin_osc(),
                        ESC => {}
                        _ => self.state = Framing::Ground,
                    }
                }
                Framing::Osc => {
                    let run = bytes[i..]
                        .iter()
                        .position(|&b| matches!(b, BEL | ESC | CAN | SUB))
                        .unwrap_or(bytes.len() - i);
                    self.absorb(&bytes[i..i + run]);
                    i += run;
                    if let Some(&terminator) = bytes.get(i) {
                        i += 1;
                        match terminator {
                            BEL => self.finish(&mut found),
                            ESC => self.state = Framing::OscEsc,
                            _ => self.state = Framing::Ground,
                        }
                    }
                }
                Framing::OscEsc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b'\\' => self.finish(&mut found),
                        b']' => self.begin_osc(),
                        ESC => self.state = Framing::Esc,
                        _ => self.state = Framing::Ground,
                    }
                }
            }
        }
        found
    }

    fn begin_osc(&mut self) {
        self.payload.clear();
        self.overlong = false;
        self.state = Framing::Osc;
    }

    fn absorb(&mut self, chunk: &[u8]) {
        if self.overlong {
            return;
        }
        if self.payload.len() + chunk.len() > MAX_PAYLOAD {
            self.overlong = true;
            self.payload.clear();
            self.payload.shrink_to(MAX_PAYLOAD);
        } else {
            self.payload.extend_from_slice(chunk);
        }
    }

    fn finish(&mut self, found: &mut Vec<PathBuf>) {
        self.state = Framing::Ground;
        if self.overlong {
            return;
        }
        if let Some(path) = self.interpret() {
            found.push(path);
        }
    }

    /// Reads the closed payload (`7;<uri>` or `9;9;<path>`) into a local path.
    fn interpret(&self) -> Option<PathBuf> {
        let text = std::str::from_utf8(&self.payload).ok()?;
        let (code, rest) = text.split_once(';')?;
        match code {
            "7" => parse_osc7_uri(rest, &self.local_host).ok(),
            "9" => parse_osc9_9_path(rest.strip_prefix("9;")?).ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "KEGON-PC";

    #[test]
    fn parses_a_drive_path() {
        assert_eq!(
            parse_osc7_uri("file://KEGON-PC/C:/Users/me", HOST),
            Ok(PathBuf::from("C:\\Users\\me"))
        );
        assert_eq!(
            parse_osc7_uri("file:///d:/", HOST),
            Ok(PathBuf::from("d:\\"))
        );
        assert_eq!(
            parse_osc7_uri("file://localhost/D:", HOST),
            Ok(PathBuf::from("D:\\"))
        );
    }

    #[test]
    fn decodes_spaces_and_japanese() {
        assert_eq!(
            parse_osc7_uri(
                "file://kegon-pc/E:/My%20Projects/%E6%97%A5%E6%9C%AC%E8%AA%9E",
                HOST
            ),
            Ok(PathBuf::from("E:\\My Projects\\日本語"))
        );
    }

    #[test]
    fn host_matching_is_case_insensitive_and_remote_hosts_are_refused() {
        assert!(parse_osc7_uri("file://kegon-pc/C:/x", HOST).is_ok());
        assert_eq!(
            parse_osc7_uri("file://build-server/home/me", HOST),
            Err(CwdRejection::RemoteHost)
        );
    }

    #[test]
    fn unc_paths_use_an_empty_host() {
        if cfg!(windows) {
            assert_eq!(
                parse_osc7_uri("file:////server/share/dir", HOST),
                Ok(PathBuf::from("\\\\server\\share\\dir"))
            );
        }
        // A UNC server in the host position is a remote host, not a share.
        assert_eq!(
            parse_osc7_uri("file://server/share/dir", HOST),
            Err(CwdRejection::RemoteHost)
        );
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "C:\\Users",
            "http://localhost/C:/x",
            "file://localhost",
            "file:///C:/x%",
            "file:///C:/x%zz",
            "file:///C:/x%FF",
            "file:///C:/x?y=1",
            "file:///C:/x#frag",
            "file:///C:/x%00y",
            "file:///C:/x%0Ay",
            "file:///relative",
        ] {
            assert!(parse_osc7_uri(bad, HOST).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn osc9_9_accepts_plain_and_quoted_absolute_paths() {
        if cfg!(windows) {
            assert_eq!(
                parse_osc9_9_path("C:\\Users\\日本 語"),
                Ok(PathBuf::from("C:\\Users\\日本 語"))
            );
            assert_eq!(
                parse_osc9_9_path("\"D:\\a b\""),
                Ok(PathBuf::from("D:\\a b"))
            );
            assert!(parse_osc9_9_path("relative\\dir").is_err());
        }
        assert!(parse_osc9_9_path("").is_err());
        assert!(parse_osc9_9_path("C:\\x\u{1b}").is_err());
    }

    fn scan_all(chunks: &[&[u8]]) -> Vec<PathBuf> {
        let mut scanner = CwdScanner::new(String::from(HOST));
        chunks.iter().flat_map(|c| scanner.scan(c)).collect()
    }

    #[test]
    fn scanner_finds_reports_with_either_terminator() {
        let found = scan_all(&[
            b"out\x1b]7;file://KEGON-PC/C:/a\x1b\\more\x1b]7;file://KEGON-PC/C:/b\x07tail",
        ]);
        assert_eq!(found, [PathBuf::from("C:\\a"), PathBuf::from("C:\\b")]);
    }

    #[test]
    fn scanner_handles_a_report_split_across_reads() {
        let found = scan_all(&[b"\x1b]7;file://KEGON-PC/C:/sp", b"lit%20dir\x1b", b"\\"]);
        assert_eq!(found, [PathBuf::from("C:\\split dir")]);
    }

    #[test]
    fn scanner_reads_cmd_style_reports_including_semicolons() {
        if cfg!(windows) {
            let found = scan_all(&[b"\x1b]9;9;C:\\a;b\\c\x1b\\"]);
            assert_eq!(found, [PathBuf::from("C:\\a;b\\c")]);
        }
    }

    #[test]
    fn scanner_ignores_other_sequences_and_bad_reports() {
        let found = scan_all(&[
            b"\x1b[31mred\x1b[0m \x1b]0;title\x07 \x1b]7;file://other/home\x07 \x1b]7;garbage\x07 \x1b]8;;http://x\x07",
        ]);
        assert!(found.is_empty());
    }

    #[test]
    fn interrupted_or_aborted_sequences_are_not_reported() {
        let found = scan_all(&[
            // Cut off by a new escape sequence, by CAN, by SUB, and never terminated.
            b"\x1b]7;file://KEGON-PC/C:/trunc\x1b[0m",
            b"\x1b]7;file://KEGON-PC/C:/can\x18 tail \x07",
            b"\x1b]7;file://KEGON-PC/C:/sub\x1a tail \x07",
            b"\x1b]7;file://KEGON-PC/C:/never",
            b" more output with no terminator",
        ]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_new_osc_after_an_interrupted_one_is_still_read() {
        let found = scan_all(&[b"\x1b]7;file://KEGON-PC/C:/a\x1b]7;file://KEGON-PC/C:/b\x07"]);
        assert_eq!(found, [PathBuf::from("C:\\b")]);
    }

    #[test]
    fn overlong_payloads_are_discarded_with_bounded_memory() {
        let mut scanner = CwdScanner::new(String::from(HOST));
        let mut head = b"\x1b]7;file://KEGON-PC/C:/".to_vec();
        head.extend(std::iter::repeat_n(b'a', 4 * MAX_PAYLOAD));
        assert!(scanner.scan(&head).is_empty());
        for _ in 0..1000 {
            assert!(scanner.scan(&[b'a'; 65_536]).is_empty());
            assert!(scanner.payload.capacity() <= 2 * MAX_PAYLOAD);
        }
        // Closing the overlong sequence reports nothing, and the next one works normally.
        assert!(scanner.scan(b"\x07").is_empty());
        let found = scanner.scan(b"\x1b]7;file://KEGON-PC/C:/ok\x07");
        assert_eq!(found, [PathBuf::from("C:\\ok")]);
    }

    #[test]
    fn nul_control_and_invalid_utf8_payloads_are_rejected() {
        let found = scan_all(&[
            b"\x1b]7;file://KEGON-PC/C:/a\x00b\x07",
            b"\x1b]7;file://KEGON-PC/C:/a\xff\xfeb\x07",
            b"\x1b]7;file://KEGON-PC/C:/a%00b\x07",
            b"\x1b]7;\x07\x1b]7\x07\x1b];\x07\x1b]9;9\x07\x1b]9;9;\x07",
        ]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn splitting_a_stream_anywhere_gives_the_same_reports() {
        let stream: &[u8] = b"x\x1b[1mhi\x1b]0;t\x07\x1b]7;file://KEGON-PC/C:/one%20dir\x1b\\y\x1b]7;file://KEGON-PC/D:/two\x07\x1b]7;bad\x07z";
        let whole = scan_all(&[stream]);
        assert_eq!(whole.len(), 2);
        for cut in 0..=stream.len() {
            assert_eq!(
                scan_all(&[&stream[..cut], &stream[cut..]]),
                whole,
                "cut {cut}"
            );
        }
        let bytewise: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(scan_all(&bytewise), whole);
    }

    #[test]
    fn arbitrary_bytes_never_panic_or_report_garbage() {
        // Deterministic pseudo-random noise, biased towards the bytes that matter.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let alphabet: &[u8] = b"\x1b\x1b]\\\x07\x18\x1a79;:/%0Fa C\xe6\x97\xff\x00";
        let mut scanner = CwdScanner::new(String::from(HOST));
        for _ in 0..200 {
            let chunk: Vec<u8> = (0..1000)
                .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                .collect();
            for path in scanner.scan(&chunk) {
                assert!(path.is_absolute(), "relative path reported: {path:?}");
            }
        }
    }

    /// Throughput of the scanner against the cost of the emulator that parses the same bytes.
    ///
    /// Not a pass/fail check on speed (that is environment dependent); it prints MB/s for each
    /// corpus. Run with: `cargo test --release scanner_throughput -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark; prints numbers, run in release mode"]
    fn scanner_throughput() {
        use alacritty_terminal::event::{Event, EventListener};
        use alacritty_terminal::term::{Config, Term};
        use alacritty_terminal::vte::ansi::Processor;
        use std::time::Instant;

        #[derive(Clone)]
        struct Quiet;
        impl EventListener for Quiet {
            fn send_event(&self, _: Event) {}
        }
        struct Size;
        impl alacritty_terminal::grid::Dimensions for Size {
            fn total_lines(&self) -> usize {
                50
            }
            fn screen_lines(&self) -> usize {
                50
            }
            fn columns(&self) -> usize {
                200
            }
        }

        let repeat = |unit: &[u8], total: usize| unit.repeat(total / unit.len() + 1);
        const TOTAL: usize = 32 * 1024 * 1024;
        let corpora: Vec<(&str, Vec<u8>)> =
            vec![
            (
                "plain ASCII lines",
                repeat(b"The quick brown fox jumps over the lazy dog 0123456789\r\n", TOTAL),
            ),
            (
                "ANSI colors",
                repeat(
                    b"\x1b[1;32mok\x1b[0m  \x1b[38;2;10;20;30mfile_name.rs\x1b[0m  42 lines\r\n",
                    TOTAL,
                ),
            ),
            (
                "CJK text",
                repeat("日本語の全角文字を大量に出力するテストです。\r\n".as_bytes(), TOTAL),
            ),
            (
                "OSC 7 every line",
                repeat(b"\x1b]7;file://PC/C:/Users/me/src%20dir\x1b\\prompt> ls\r\n", TOTAL),
            ),
            (
                "unterminated OSC flood",
                repeat(b"\x1b]7;file://PC/C:/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", TOTAL),
            ),
        ];

        let mb = |bytes: usize, secs: f64| bytes as f64 / 1_048_576.0 / secs;
        println!(
            "{:<26} {:>14} {:>14}",
            "corpus (32 MiB)", "scanner MB/s", "Term MB/s"
        );
        for (name, data) in &corpora {
            let mut scanner = CwdScanner::new(String::from("PC"));
            let started = Instant::now();
            let mut reports = 0usize;
            for chunk in data.chunks(65_536) {
                reports += scanner.scan(chunk).len();
            }
            let scan_secs = started.elapsed().as_secs_f64();

            let mut term = Term::new(Config::default(), &Size, Quiet);
            let mut processor: Processor = Processor::new();
            let started = Instant::now();
            for chunk in data.chunks(65_536) {
                processor.advance(&mut term, chunk);
            }
            let term_secs = started.elapsed().as_secs_f64();
            println!(
                "{name:<26} {:>14.0} {:>14.0}   ({reports} reports)",
                mb(data.len(), scan_secs),
                mb(data.len(), term_secs)
            );
        }
    }
}
