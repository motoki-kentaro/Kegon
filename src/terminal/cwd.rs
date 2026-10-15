//! Working-directory reports from the shell.
//!
//! `alacritty_terminal` (via `vte`) ignores OSC 7, so the PTY output is also fed to a second,
//! tiny `vte` parser that only looks at the OSC sequences below and ignores everything else. The
//! sequences are produced by the Shell Integration in [`crate::terminal::shell`]:
//!
//! * `OSC 7 ; file://<host>/<percent-encoded path> ST` — PowerShell (and any shell that speaks
//!   OSC 7, such as one on a remote host).
//! * `OSC 9 ; 9 ; <path> ST` — the ConEmu / Windows Terminal form that `cmd.exe` can emit from
//!   its `PROMPT` string, which cannot percent-encode.
//!
//! A report is only ever the shell's own statement about its current directory. Reports that do
//! not name a local directory (another host, malformed, relative) are dropped, never guessed at.

use std::path::PathBuf;

use vte::{Parser, Perform};

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

/// Streaming extractor of working-directory reports from raw PTY output.
///
/// Holds `vte` state, so a sequence split across reads is still recognized.
pub struct CwdScanner {
    parser: Parser,
    collector: Collector,
}

struct Collector {
    local_host: String,
    found: Vec<PathBuf>,
}

impl CwdScanner {
    pub fn new(local_host: String) -> Self {
        Self {
            parser: Parser::new(),
            collector: Collector {
                local_host,
                found: Vec::new(),
            },
        }
    }

    /// Feeds PTY output and returns the working directories reported in it, in order.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<PathBuf> {
        self.parser.advance(&mut self.collector, bytes);
        std::mem::take(&mut self.collector.found)
    }
}

impl Perform for Collector {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let text = |index: usize| params.get(index).and_then(|p| std::str::from_utf8(p).ok());
        let parsed = match params.first().copied() {
            Some(b"7") => text(1).map(|uri| parse_osc7_uri(uri, &self.local_host)),
            // The path may itself contain `;`, which `vte` splits on.
            Some(b"9") if params.get(1).copied() == Some(b"9") => {
                let joined: Option<Vec<&str>> = params[2..]
                    .iter()
                    .map(|p| std::str::from_utf8(p).ok())
                    .collect();
                joined.map(|parts| parse_osc9_9_path(&parts.join(";")))
            }
            _ => None,
        };
        if let Some(Ok(path)) = parsed {
            self.found.push(path);
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
}
