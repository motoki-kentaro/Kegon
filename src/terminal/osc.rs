//! Streaming parser and extractor for OSC (Operating System Command) sequences.
//!
//! Extracts working directory notifications (OSC 7, OSC 9;9) and shell integration markers
//! (OSC 133 A/B/C/D) from raw PTY output streams with bounded memory and chunk reassembly.

use std::path::PathBuf;

use crate::terminal::cwd::{parse_osc7_uri, parse_osc9_9_path};
use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};

/// Events extracted from PTY output by [`OscScanner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscEvent {
    /// Working directory report from OSC 7 or OSC 9;9.
    WorkingDirectory(PathBuf),
    /// Shell integration marker from OSC 133.
    ShellIntegration(Osc133Marker),
}

/// Longest OSC payload kept. Real reports are a few hundred bytes; anything longer is discarded.
const MAX_PAYLOAD: usize = 8 * 1024;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// Streaming extractor of OSC sequences from raw PTY output.
///
/// Rules:
/// - A report counts only when closed by `BEL` (`0x07`) or `ST` (`ESC \`).
/// - `CAN` (`0x18`), `SUB` (`0x1a`), or an `ESC` followed by anything other than `\` or `]` abandons the sequence.
/// - The payload is capped at [`MAX_PAYLOAD`]; an overlong one is discarded unread, ensuring bounded memory.
/// - State carries across `scan` calls, so sequences split across read chunks are properly recognized.
#[derive(Debug)]
pub struct OscScanner {
    local_host: String,
    state: Framing,
    payload: Vec<u8>,
    overlong: bool,
    drop_unmarked_command_markers: bool,
}

impl OscScanner {
    /// Creates a new scanner for the given local machine host name.
    pub fn new(local_host: String) -> Self {
        Self {
            local_host,
            state: Framing::Ground,
            payload: Vec::new(),
            overlong: false,
            drop_unmarked_command_markers: false,
        }
    }

    /// Makes the scanner drop `OSC 133;C` and `OSC 133;D` that carry no `kegon.*` parameter at all.
    ///
    /// For a shell that Kegon has integrated: it identifies each of its own command markers, so a
    /// marker without any identity is the shell's own (fish 4 prints them by itself) and would be
    /// taken for a second, unverified command beside Kegon's. A marker with `kegon.*` parameters,
    /// however malformed, is still reported, so the state machine decides about it.
    pub fn dropping_unmarked_command_markers(mut self) -> Self {
        self.drop_unmarked_command_markers = true;
        self
    }

    /// Feeds PTY output chunks and returns the recognized OSC events in order.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
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

    fn finish(&mut self, found: &mut Vec<OscEvent>) {
        self.state = Framing::Ground;
        if self.overlong {
            return;
        }
        if let Some(event) = self.interpret() {
            found.push(event);
        }
    }

    /// Interprets the closed OSC payload into a typed [`OscEvent`].
    fn interpret(&self) -> Option<OscEvent> {
        let text = std::str::from_utf8(&self.payload).ok()?;
        let (code, rest) = text.split_once(';')?;
        match code {
            "7" => {
                let path = parse_osc7_uri(rest, &self.local_host).ok()?;
                Some(OscEvent::WorkingDirectory(path))
            }
            "9" => {
                let path = parse_osc9_9_path(rest.strip_prefix("9;")?).ok()?;
                Some(OscEvent::WorkingDirectory(path))
            }
            "133" => {
                let marker = parse_osc133(rest)?;
                if self.drop_unmarked_command_markers
                    && matches!(
                        marker,
                        Osc133Marker::CommandStarted | Osc133Marker::CommandFinished(_)
                    )
                    && !carries_kegon_parameter(rest)
                {
                    return None;
                }
                Some(OscEvent::ShellIntegration(marker))
            }
            "1337" => {
                let marker = parse_osc1337(rest)?;
                Some(OscEvent::ShellIntegration(marker))
            }
            _ => None,
        }
    }
}

/// Whether any parameter of an `OSC 133` payload (the text after `133;`) is a `kegon.*` one.
fn carries_kegon_parameter(rest: &str) -> bool {
    rest.split(';')
        .skip(1)
        .any(|param| param.trim().starts_with("kegon."))
}

/// Maximum allowed length in characters for a session ID.
pub const MAX_SID_LEN: usize = 64;

/// Returns true if `s` is a canonical decimal representation:
/// - Non-empty
/// - Contains only ASCII digits '0'..='9'
/// - No leading '+' or '-'
/// - No leading zero unless the value is exactly "0"
pub fn is_canonical_decimal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes.len() > 1 && bytes[0] == b'0' {
        return false;
    }
    bytes.iter().all(|b| b.is_ascii_digit())
}

/// Validates session ID: non-empty, <= 64 characters, ASCII alphanumeric, '-', and '_'.
pub fn is_valid_session_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SID_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Helper parsing `kegon.*` key-value parameters into typed [`Osc133Metadata`].
///
/// Strict parsing rules (F-1):
/// - `kegon.v`: Required protocol version. Must be canonical decimal `1`. Duplicate or invalid -> None.
/// - `kegon.sid`: Required session identifier string (1..=64 chars, [a-zA-Z0-9_-]). Duplicate or invalid -> None.
/// - `kegon.seq`: Required monotonically increasing sequence number (canonical decimal u64). Duplicate or invalid -> None.
/// - `kegon.ps`: Optional shell-reported status (`success` or `failure`). Any invalid value or duplicate -> None.
/// - Any duplicate known key or single invalid value rejects the entire metadata immediately without recovery.
/// - Unknown `kegon.*` keys must have valid `=` format; malformed `kegon.` tokens reject metadata.
///
/// ## N-5: Forward Compatibility Policy for `kegon.ps`
/// In Phase 1 (`kegon.v=1`), `kegon.ps` strictly validates against known values (`success` / `failure`).
/// Under future protocol versions (e.g. `kegon.v=2`), forward compatibility will allow unrecognized status
/// tokens (e.g. `cancelled`, `timeout`) to degrade to `ShellReportedStatus::Unknown` without discarding
/// the envelope or breaking sequence correlation.
///
/// ## F-7: Protocol Swappability & Decoupling
/// The OSC parser layer translates raw byte streams into typed [`Osc133Marker`] events. The command
/// state machine in `integration.rs` depends purely on these typed domain markers and has zero knowledge
/// of escape sequence framing. If Kegon migrates from Option D (structured OSC 133 D) to an alternative
/// protocol (such as OSC 633 or OSC 1337), only the wire parser layer requires adjustment, leaving the
/// core state machine, correlation rules, and result models unmodified.
pub fn parse_kegon_metadata<'a, I>(tokens: I) -> Option<Osc133Metadata>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut version = None;
    let mut session_id = None;
    let mut seq = None;
    let mut shell_status = ShellReportedStatus::Unknown;

    let mut seen_v = false;
    let mut seen_sid = false;
    let mut seen_seq = false;
    let mut seen_ps = false;

    let mut has_kegon = false;

    for token in tokens {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "kegon.v" => {
                    if seen_v {
                        return None;
                    }
                    seen_v = true;
                    has_kegon = true;
                    if !is_canonical_decimal(val) {
                        return None;
                    }
                    let parsed_v = val.parse::<u32>().ok()?;
                    if parsed_v != 1 {
                        return None;
                    }
                    version = Some(parsed_v);
                }
                "kegon.sid" => {
                    if seen_sid {
                        return None;
                    }
                    seen_sid = true;
                    has_kegon = true;
                    if !is_valid_session_id(val) {
                        return None;
                    }
                    session_id = Some(val.to_string());
                }
                "kegon.seq" => {
                    if seen_seq {
                        return None;
                    }
                    seen_seq = true;
                    has_kegon = true;
                    if !is_canonical_decimal(val) {
                        return None;
                    }
                    let parsed_seq = val.parse::<u64>().ok()?;
                    seq = Some(parsed_seq);
                }
                "kegon.ps" => {
                    if seen_ps {
                        return None;
                    }
                    seen_ps = true;
                    has_kegon = true;
                    shell_status = match val {
                        "success" => ShellReportedStatus::Success,
                        "failure" => ShellReportedStatus::Failure,
                        _ => return None,
                    };
                }
                _ => {
                    if key.starts_with("kegon.") {
                        has_kegon = true;
                    }
                }
            }
        } else if trimmed.starts_with("kegon.") {
            // Malformed token starting with kegon. without '='
            return None;
        }
    }

    if !has_kegon {
        return None;
    }

    if version != Some(1) {
        return None;
    }

    Some(Osc133Metadata {
        version: 1,
        session_id: session_id?,
        seq: seq?,
        shell_status,
    })
}

/// Parses the payload after `133;` into an [`Osc133Marker`].
///
/// Handled forms:
/// - `A` (with optional `;...`) -> PromptStarted
/// - `B` (with optional `;...`) -> InputStarted
/// - `C` (with optional `;...`) -> CommandStarted / CommandStartedWithMetadata
/// - `D` -> CommandFinished(None)
/// - `D;<code>` -> CommandFinished(Some(code)) if valid integer, or CommandFinished(None) if invalid
/// - `D;kegon.ps=<status>;kegon.v=1;...` -> CommandFinishedWithMetadata(None, meta)
/// - `D;<code>;kegon.ps=<status>;kegon.v=1;...` -> CommandFinishedWithMetadata(Some(code), meta)
/// - `D;;kegon.v=1;...` -> CommandFinishedWithMetadata(None, meta)
///
/// Numeric slot handling (F-4):
/// Only the first token immediately following `D` is interpreted as the numeric exit code slot,
/// unless it starts with `kegon.` (in which case it is metadata). Subsequent tokens are never
/// scanned to invent an exit code.
pub fn parse_osc133(rest: &str) -> Option<Osc133Marker> {
    let mut parts = rest.split(';');
    let tag = parts.next()?.trim();
    match tag {
        "A" => Some(Osc133Marker::PromptStarted),
        "B" => Some(Osc133Marker::InputStarted),
        "C" => {
            let remaining: Vec<&str> = parts.collect();
            if let Some(meta) = parse_kegon_metadata(remaining) {
                Some(Osc133Marker::CommandStartedWithMetadata(meta))
            } else {
                Some(Osc133Marker::CommandStarted)
            }
        }
        "D" => {
            let mut osc133_exit = None;
            let mut metadata_tokens = Vec::new();

            if let Some(p0) = parts.next() {
                let p0_trimmed = p0.trim();
                if p0_trimmed.starts_with("kegon.") {
                    // Option D candidate syntax: no numeric exit code slot provided.
                    metadata_tokens.push(p0_trimmed);
                } else {
                    // Standard numeric exit slot: only token 1 is evaluated.
                    if !p0_trimmed.is_empty() {
                        osc133_exit = p0_trimmed.parse::<i32>().ok();
                    }
                }
            }

            for raw in parts {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    metadata_tokens.push(trimmed);
                }
            }

            if let Some(meta) = parse_kegon_metadata(metadata_tokens) {
                Some(Osc133Marker::CommandFinishedWithMetadata(osc133_exit, meta))
            } else {
                Some(Osc133Marker::CommandFinished(osc133_exit))
            }
        }
        _ => None,
    }
}

/// Minimal RFC 4648 Base64 decoder.
///
/// Decodes standard Base64 characters (`[A-Za-z0-9+/]`) with optional `=` padding.
/// Rejects any invalid characters, improper padding, or dangling non-zero bits.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let clean = input.trim();
    if clean.is_empty() {
        return Some(Vec::new());
    }
    if !clean.len().is_multiple_of(4) {
        return None;
    }
    let bytes = clean.as_bytes();
    let mut out = Vec::with_capacity((bytes.len() * 3) / 4);

    let decode_char = |b: u8| -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };

    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    let mut padding_count = 0;
    let mut data_count = 0;

    for &b in bytes {
        if b == b'=' {
            if data_count < 2 {
                // Padding cannot appear before at least 2 data characters
                return None;
            }
            padding_count += 1;
            if padding_count > 2 {
                // At most 2 padding characters in standard Base64
                return None;
            }
            continue;
        }
        if padding_count > 0 {
            // Data character after padding '=' is forbidden
            return None;
        }
        let val = decode_char(b)?;
        data_count += 1;
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    if padding_count > 0 && buf != 0 {
        // Non-zero unused bits in padding
        return None;
    }

    Some(out)
}

/// Parses the payload after `1337;` into an [`Osc133Marker`].
///
/// Only `SetUserVar=kegon_status=<Base64>` is recognized. All other user variables
/// and other OSC 1337 subcommands are safely ignored and return `None`.
///
/// ## Parsing Failure & Invalid Metadata Policy (M-3)
/// If the target user variable is `kegon_status`, any parse failure (such as invalid Base64,
/// invalid UTF-8, malformed token framing, or invalid metadata fields) emits [`Osc133Marker::InvalidMetadata`].
/// This allows the state machine to explicitly mark the active command as corrupted and degrade its
/// shell status to [`ShellReportedStatus::Unknown`] rather than silently ignoring the corruption.
///
/// ## Payload Size Limit & Framing (M-3)
/// Note that `OscScanner` caps all OSC payloads at [`MAX_PAYLOAD`] (8KB). Any OSC sequence
/// exceeding 8KB is discarded at the scanner framing layer before reaching this parser.
pub fn parse_osc1337(rest: &str) -> Option<Osc133Marker> {
    let trimmed = rest.trim();
    let param = trimmed.strip_prefix("SetUserVar=")?;
    let (var_name, b64_val) = match param.split_once('=') {
        Some((name, val)) => (name.trim(), val.trim()),
        None => {
            if param.trim() == "kegon_status" {
                return Some(Osc133Marker::InvalidMetadata);
            }
            return None;
        }
    };
    if var_name != "kegon_status" {
        return None;
    }
    let decoded_bytes = match base64_decode(b64_val) {
        Some(b) => b,
        None => return Some(Osc133Marker::InvalidMetadata),
    };
    let decoded_str = match std::str::from_utf8(&decoded_bytes) {
        Ok(s) => s,
        Err(_) => return Some(Osc133Marker::InvalidMetadata),
    };
    let tokens = decoded_str.split(';');
    match parse_kegon_metadata(tokens) {
        Some(meta) => Some(Osc133Marker::IndependentMetadata(meta)),
        None => Some(Osc133Marker::InvalidMetadata),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "KEGON-TEST-PC";

    #[test]
    fn parses_osc133_markers_bel_terminated() {
        let mut scanner = OscScanner::new(HOST.into());

        assert_eq!(
            scanner.scan(b"\x1b]133;A\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::PromptStarted)]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;B\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::InputStarted)]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;C\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandStarted)]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;D\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;D;0\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;D;1\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(1)
            ))]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;D;127\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(127)
            ))]
        );
    }

    #[test]
    fn parses_osc133_st_terminated() {
        let mut scanner = OscScanner::new(HOST.into());

        assert_eq!(
            scanner.scan(b"\x1b]133;A\x1b\\"),
            vec![OscEvent::ShellIntegration(Osc133Marker::PromptStarted)]
        );
        assert_eq!(
            scanner.scan(b"\x1b]133;D;0\x1b\\"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );
    }

    #[test]
    fn parses_osc133_with_extra_arguments() {
        let mut scanner = OscScanner::new(HOST.into());

        // A with key-value parameters
        assert_eq!(
            scanner.scan(b"\x1b]133;A;aid=1234\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::PromptStarted)]
        );
        // D with exit code and key-value parameters
        assert_eq!(
            scanner.scan(b"\x1b]133;D;42;aid=1234\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(42)
            ))]
        );
    }

    #[test]
    fn handles_invalid_and_overflow_exit_codes_safely_as_none() {
        let mut scanner = OscScanner::new(HOST.into());

        // Invalid non-integer string: becomes None, does not panic
        assert_eq!(
            scanner.scan(b"\x1b]133;D;invalid\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
        // Empty exit code
        assert_eq!(
            scanner.scan(b"\x1b]133;D;\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
        // Integer overflow: becomes None, does not panic
        assert_eq!(
            scanner.scan(b"\x1b]133;D;99999999999999999999999999999999\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
        // Negative exit code
        assert_eq!(
            scanner.scan(b"\x1b]133;D;-1\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(-1)
            ))]
        );
    }

    #[test]
    fn handles_chunk_fragmentation() {
        let mut scanner = OscScanner::new(HOST.into());

        // Split inside the sequence header
        assert_eq!(scanner.scan(b"\x1b]13"), vec![]);
        assert_eq!(
            scanner.scan(b"3;A\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::PromptStarted)]
        );

        // Split across ESC and backslash of ST
        assert_eq!(scanner.scan(b"\x1b]133;D;0\x1b"), vec![]);
        assert_eq!(
            scanner.scan(b"\\"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );
    }

    #[test]
    fn handles_interrupted_sequences() {
        let mut scanner = OscScanner::new(HOST.into());

        // Interrupted by CAN (0x18)
        assert_eq!(scanner.scan(b"\x1b]133;A\x18"), vec![]);
        // Next valid sequence works normally
        assert_eq!(
            scanner.scan(b"\x1b]133;B\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::InputStarted)]
        );

        // Interrupted by SUB (0x1a)
        assert_eq!(scanner.scan(b"\x1b]133;C\x1a"), vec![]);
        assert_eq!(
            scanner.scan(b"\x1b]133;D;0\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );
    }

    #[test]
    fn bounds_memory_for_overlong_payloads() {
        let mut scanner = OscScanner::new(HOST.into());

        let overlong_chunk = vec![b'x'; MAX_PAYLOAD + 100];
        assert_eq!(scanner.scan(b"\x1b]133;"), vec![]);
        assert_eq!(scanner.scan(&overlong_chunk), vec![]);
        assert_eq!(
            scanner.scan(b"\x07"),
            vec![],
            "Overlong sequence must be discarded without dispatch"
        );

        // Scanner recovers on the next valid sequence
        assert_eq!(
            scanner.scan(b"\x1b]133;A\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::PromptStarted)]
        );
    }

    #[test]
    fn intermixed_osc7_osc9_and_osc133() {
        let mut scanner = OscScanner::new(HOST.into());

        let stream = format!(
            "\x1b]133;A\x07\x1b]7;file://{HOST}/C:/works/kegon\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07"
        );

        let events = scanner.scan(stream.as_bytes());
        assert_eq!(
            events,
            vec![
                OscEvent::ShellIntegration(Osc133Marker::PromptStarted),
                OscEvent::WorkingDirectory(PathBuf::from(if cfg!(windows) {
                    r"C:\works\kegon"
                } else {
                    "/C:/works/kegon"
                })),
                OscEvent::ShellIntegration(Osc133Marker::InputStarted),
                OscEvent::ShellIntegration(Osc133Marker::CommandStarted),
                OscEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
            ]
        );
    }

    #[test]
    fn parses_osc133_with_kegon_metadata_option_d_candidate() {
        let mut scanner = OscScanner::new(HOST.into());

        let stream = b"\x1b]133;C;kegon.v=1;kegon.sid=PS_S1;kegon.seq=100\x07\x1b]133;D;kegon.ps=success;kegon.v=1;kegon.sid=PS_S1;kegon.seq=100\x07";
        let events = scanner.scan(stream);

        assert_eq!(
            events,
            vec![
                OscEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(
                    Osc133Metadata {
                        version: 1,
                        session_id: "PS_S1".into(),
                        seq: 100,
                        shell_status: ShellReportedStatus::Unknown,
                    }
                )),
                OscEvent::ShellIntegration(Osc133Marker::CommandFinishedWithMetadata(
                    None,
                    Osc133Metadata {
                        version: 1,
                        session_id: "PS_S1".into(),
                        seq: 100,
                        shell_status: ShellReportedStatus::Success,
                    }
                )),
            ]
        );
    }

    #[test]
    fn parses_osc133_with_kegon_metadata_and_numeric_exit_code() {
        let mut scanner = OscScanner::new(HOST.into());

        // Numeric exit 42 with failure status
        let stream = b"\x1b]133;D;42;kegon.ps=failure;kegon.v=1;kegon.sid=PS_S1;kegon.seq=101\x07";
        let events = scanner.scan(stream);

        assert_eq!(
            events,
            vec![OscEvent::ShellIntegration(
                Osc133Marker::CommandFinishedWithMetadata(
                    Some(42),
                    Osc133Metadata {
                        version: 1,
                        session_id: "PS_S1".into(),
                        seq: 101,
                        shell_status: ShellReportedStatus::Failure,
                    }
                )
            )]
        );
    }

    #[test]
    fn parses_osc133_with_kegon_metadata_option_a_empty_slot() {
        let mut scanner = OscScanner::new(HOST.into());

        // Option A format with empty slot ;;
        let stream = b"\x1b]133;D;;kegon.v=1;kegon.sid=PS_S1;kegon.seq=102;kegon.ps=success\x07";
        let events = scanner.scan(stream);

        assert_eq!(
            events,
            vec![OscEvent::ShellIntegration(
                Osc133Marker::CommandFinishedWithMetadata(
                    None,
                    Osc133Metadata {
                        version: 1,
                        session_id: "PS_S1".into(),
                        seq: 102,
                        shell_status: ShellReportedStatus::Success,
                    }
                )
            )]
        );
    }

    #[test]
    fn rejects_unknown_protocol_version_as_standard_d() {
        let mut scanner = OscScanner::new(HOST.into());

        // Unknown version 99 degrades metadata, keeps numeric exit code if present
        let stream = b"\x1b]133;D;0;kegon.v=99;kegon.sid=S1;kegon.seq=1;kegon.ps=success\x07";
        let events = scanner.scan(stream);

        assert_eq!(
            events,
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );

        // Unknown version without exit code degrades to CommandFinished(None)
        let stream_no_code = b"\x1b]133;D;kegon.v=99;kegon.sid=S1;kegon.seq=1;kegon.ps=success\x07";
        let events_no_code = scanner.scan(stream_no_code);

        assert_eq!(
            events_no_code,
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
    }

    #[test]
    fn rejects_malformed_metadata_tokens() {
        let mut scanner = OscScanner::new(HOST.into());

        // Missing sid and seq -> degrades
        let stream = b"\x1b]133;D;kegon.ps=success\x07";
        let events = scanner.scan(stream);

        assert_eq!(
            events,
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
    }

    #[test]
    fn metadata_with_st_termination_and_fragmentation() {
        let mut scanner = OscScanner::new(HOST.into());

        let msg = b"\x1b]133;D;kegon.ps=failure;kegon.v=1;kegon.sid=ST_SID;kegon.seq=1\x1b\\";

        // Feed byte by byte
        let mut collected = Vec::new();
        for &b in msg {
            collected.extend(scanner.scan(&[b]));
        }

        assert_eq!(
            collected,
            vec![OscEvent::ShellIntegration(
                Osc133Marker::CommandFinishedWithMetadata(
                    None,
                    Osc133Metadata {
                        version: 1,
                        session_id: "ST_SID".into(),
                        seq: 1,
                        shell_status: ShellReportedStatus::Failure,
                    }
                )
            )]
        );
    }

    #[test]
    fn rejects_duplicate_metadata_keys() {
        let mut scanner = OscScanner::new(HOST.into());

        // Duplicate kegon.v
        let stream1 =
            b"\x1b]133;D;0;kegon.v=1;kegon.v=1;kegon.sid=S1;kegon.seq=1;kegon.ps=success\x07";
        assert_eq!(
            scanner.scan(stream1),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );

        // Duplicate kegon.sid
        let stream2 =
            b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.sid=S2;kegon.seq=1;kegon.ps=success\x07";
        assert_eq!(
            scanner.scan(stream2),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Duplicate kegon.seq
        let stream3 =
            b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=1;kegon.seq=2;kegon.ps=success\x07";
        assert_eq!(
            scanner.scan(stream3),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Duplicate kegon.ps
        let stream4 =
            b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=1;kegon.ps=success;kegon.ps=failure\x07";
        assert_eq!(
            scanner.scan(stream4),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
    }

    #[test]
    fn rejects_invalid_values_without_recovery() {
        let mut scanner = OscScanner::new(HOST.into());

        // Invalid key value followed by valid key value must NOT recover
        let stream_v = b"\x1b]133;D;kegon.v=bad;kegon.v=1;kegon.sid=S1;kegon.seq=1\x07";
        assert_eq!(
            scanner.scan(stream_v),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        let stream_seq = b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=bad;kegon.seq=1\x07";
        assert_eq!(
            scanner.scan(stream_seq),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        let stream_ps =
            b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=1;kegon.ps=bad;kegon.ps=success\x07";
        assert_eq!(
            scanner.scan(stream_ps),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Non-canonical decimals: +5, 01, overflow
        let stream_plus = b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=+5\x07";
        assert_eq!(
            scanner.scan(stream_plus),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        let stream_leading_zero = b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=01\x07";
        assert_eq!(
            scanner.scan(stream_leading_zero),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        let stream_v_zero = b"\x1b]133;D;kegon.v=01;kegon.sid=S1;kegon.seq=1\x07";
        assert_eq!(
            scanner.scan(stream_v_zero),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        let stream_overflow =
            b"\x1b]133;D;kegon.v=1;kegon.sid=S1;kegon.seq=99999999999999999999999999999999\x07";
        assert_eq!(
            scanner.scan(stream_overflow),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );
    }

    #[test]
    fn rejects_invalid_session_ids() {
        let mut scanner = OscScanner::new(HOST.into());

        // Empty sid
        let stream_empty = b"\x1b]133;D;kegon.v=1;kegon.sid=;kegon.seq=1\x07";
        assert_eq!(
            scanner.scan(stream_empty),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Space in sid
        let stream_space = b"\x1b]133;D;kegon.v=1;kegon.sid=session id with space;kegon.seq=1\x07";
        assert_eq!(
            scanner.scan(stream_space),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Disallowed punctuation
        let stream_punct = b"\x1b]133;D;kegon.v=1;kegon.sid=session/slash;kegon.seq=1\x07";
        assert_eq!(
            scanner.scan(stream_punct),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Overlong sid (> 64 chars)
        let long_sid = "a".repeat(65);
        let stream_long = format!("\x1b]133;D;kegon.v=1;kegon.sid={long_sid};kegon.seq=1\x07");
        assert_eq!(
            scanner.scan(stream_long.as_bytes()),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // Exactly 64 chars is valid
        let max_sid = "b".repeat(64);
        let stream_max =
            format!("\x1b]133;D;kegon.v=1;kegon.sid={max_sid};kegon.seq=1;kegon.ps=success\x07");
        assert_eq!(
            scanner.scan(stream_max.as_bytes()),
            vec![OscEvent::ShellIntegration(
                Osc133Marker::CommandFinishedWithMetadata(
                    None,
                    Osc133Metadata {
                        version: 1,
                        session_id: max_sid,
                        seq: 1,
                        shell_status: ShellReportedStatus::Success,
                    }
                )
            )]
        );
    }

    #[test]
    fn preserves_exact_numeric_exit_code_slot() {
        let mut scanner = OscScanner::new(HOST.into());

        // D;0 -> Some(0)
        assert_eq!(
            scanner.scan(b"\x1b]133;D;0\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(0)
            ))]
        );

        // D;42 -> Some(42)
        assert_eq!(
            scanner.scan(b"\x1b]133;D;42\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(42)
            ))]
        );

        // D;abc;5 -> None (abc in slot 1 is invalid; does not search slot 2 for 5)
        assert_eq!(
            scanner.scan(b"\x1b]133;D;abc;5\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // D;;7 -> None (slot 1 is empty; does not search slot 2 for 7)
        assert_eq!(
            scanner.scan(b"\x1b]133;D;;7\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                None
            ))]
        );

        // D;1;2 -> Some(1) (only first slot is the exit code)
        assert_eq!(
            scanner.scan(b"\x1b]133;D;1;2\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(1)
            ))]
        );

        // D;42;kegon.v=99 -> Some(42) (numeric code preserved even when metadata fails)
        assert_eq!(
            scanner.scan(b"\x1b]133;D;42;kegon.v=99;kegon.sid=S1;kegon.seq=1\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::CommandFinished(
                Some(42)
            ))]
        );
    }

    #[test]
    fn parses_valid_osc1337_setuservar_kegon_status() {
        let mut scanner = OscScanner::new(HOST.into());

        // Success payload: kegon.v=1;kegon.sid=S_1337;kegon.seq=5;kegon.ps=success
        // Base64: a2Vnb24udj0xO2tlZ29uLnNpZD1TXzEzMzc7a2Vnb24uc2VxPTU7a2Vnb24ucHM9c3VjY2Vzcw==
        let seq_bel = b"\x1b]1337;SetUserVar=kegon_status=a2Vnb24udj0xO2tlZ29uLnNpZD1TXzEzMzc7a2Vnb24uc2VxPTU7a2Vnb24ucHM9c3VjY2Vzcw==\x07";
        assert_eq!(
            scanner.scan(seq_bel),
            vec![OscEvent::ShellIntegration(
                Osc133Marker::IndependentMetadata(Osc133Metadata {
                    version: 1,
                    session_id: "S_1337".into(),
                    seq: 5,
                    shell_status: ShellReportedStatus::Success,
                })
            )]
        );

        // Failure payload with ST terminator: kegon.v=1;kegon.sid=S_1337;kegon.seq=6;kegon.ps=failure
        // Base64: a2Vnb24udj0xO2tlZ29uLnNpZD1TXzEzMzc7a2Vnb24uc2VxPTY7a2Vnb24ucHM9ZmFpbHVyZQ==
        let seq_st = b"\x1b]1337;SetUserVar=kegon_status=a2Vnb24udj0xO2tlZ29uLnNpZD1TXzEzMzc7a2Vnb24uc2VxPTY7a2Vnb24ucHM9ZmFpbHVyZQ==\x1b\\";
        assert_eq!(
            scanner.scan(seq_st),
            vec![OscEvent::ShellIntegration(
                Osc133Marker::IndependentMetadata(Osc133Metadata {
                    version: 1,
                    session_id: "S_1337".into(),
                    seq: 6,
                    shell_status: ShellReportedStatus::Failure,
                })
            )]
        );
    }

    #[test]
    fn rejects_invalid_osc1337_variants() {
        let mut scanner = OscScanner::new(HOST.into());

        // L-1: Unrecognized variable name -> ignored silently (returns None / empty scan)
        assert_eq!(
            scanner.scan(b"\x1b]1337;SetUserVar=other_var=a2Vnb24udj0x\x07"),
            vec![]
        );
        assert_eq!(parse_osc1337("SetUserVar=other_var=a2Vnb24udj0x"), None);

        // Missing SetUserVar prefix -> ignored silently
        assert_eq!(scanner.scan(b"\x1b]1337;File=name=dGVzdA==\x07"), vec![]);
        assert_eq!(parse_osc1337("File=name=dGVzdA=="), None);

        // M-3: Target kegon_status parse failures emit InvalidMetadata:
        // 1. Corrupted base64 characters
        assert_eq!(
            scanner.scan(b"\x1b]1337;SetUserVar=kegon_status=not!valid!base64!!\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::InvalidMetadata)]
        );

        // 2. Invalid base64 padding
        assert_eq!(
            scanner.scan(b"\x1b]1337;SetUserVar=kegon_status=a2Vnb24udj0x===\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::InvalidMetadata)]
        );

        // 3. Valid base64 but malformed payload (missing version, sid, seq, or ps)
        // Payload: "hello=world" -> aGVsbG89d29ybGQ=
        assert_eq!(
            scanner.scan(b"\x1b]1337;SetUserVar=kegon_status=aGVsbG89d29ybGQ=\x07"),
            vec![OscEvent::ShellIntegration(Osc133Marker::InvalidMetadata)]
        );

        // 4. Valid base64 but duplicate keys in payload
        // Payload: "kegon.v=1;kegon.v=1;kegon.sid=S1;kegon.seq=1;kegon.ps=success"
        let dup_b64 =
            "a2Vnb24udj0xO2tlZ29uLnY9MTtrZWdvbi5zaWQ9UzE7a2Vnb24uc2VxPTE7a2Vnb24ucHM9c3VjY2Vzcw==";
        let dup_seq = format!("\x1b]1337;SetUserVar=kegon_status={dup_b64}\x07");
        assert_eq!(
            scanner.scan(dup_seq.as_bytes()),
            vec![OscEvent::ShellIntegration(Osc133Marker::InvalidMetadata)]
        );

        // 5. Malformed without value separator
        assert_eq!(
            parse_osc1337("SetUserVar=kegon_status"),
            Some(Osc133Marker::InvalidMetadata)
        );
    }

    #[test]
    fn base64_decode_rfc4648_vectors() {
        // Standard RFC 4648 vectors
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");

        // L-1: Rejection of non-zero unused bits in padding
        // "QR==": 'Q' is 0b010000, 'R' is 0b010001 -> 12 bits = 0b010000010001.
        // 8 data bits = 0b01000001 ('A'), remaining 4 padding bits = 0b0001 != 0.
        // RFC 4648 Section 3.5 mandates rejecting non-zero padding bits.
        assert!(base64_decode("QR==").is_none());

        // Length not multiple of 4
        assert!(base64_decode("Z").is_none());
        assert!(base64_decode("Zg").is_none());
        assert!(base64_decode("Zg=").is_none());

        // Invalid characters
        assert!(base64_decode("Zg-=").is_none());
        assert!(base64_decode("Zg_0").is_none());
        assert!(base64_decode("!!!!").is_none());

        // Incorrect padding positions
        assert!(base64_decode("=m9v").is_none());
        assert!(base64_decode("Z=9v").is_none());
        assert!(base64_decode("Zm=v").is_none());
    }

    // --- native OSC 133 of an integrated fish (T-2, T-3) -----------------------------------

    use crate::terminal::integration::{
        CommandStateMachine, CompletionReason, MetadataCorrelation, VerifiedCommandEvent,
    };

    const SID: &str = "kegon-1-abc";

    fn markers(scanner: &mut OscScanner, bytes: &[u8]) -> Vec<Osc133Marker> {
        scanner
            .scan(bytes)
            .into_iter()
            .filter_map(|event| match event {
                OscEvent::ShellIntegration(marker) => Some(marker),
                _ => None,
            })
            .collect()
    }

    fn fish_scanner() -> OscScanner {
        OscScanner::new(HOST.into()).dropping_unmarked_command_markers()
    }

    /// What fish 4.0.2 and the Kegon hooks print for one command line, in the observed order.
    fn fish_command_line(exit: i32, status: &str) -> Vec<u8> {
        format!(
            "\x1b]133;C;cmdline_url=true\x07\
             \x1b]133;C;kegon.v=1;kegon.sid={SID};kegon.seq=1\x07\
             \x1b]133;D;{exit}\x07\
             \x1b]133;D;{exit};kegon.v=1;kegon.sid={SID};kegon.seq=1;kegon.ps={status}\x07\
             \x1b]7;file://{HOST}/tmp\x07\x1b]133;A\x07\x1b]133;A;special_key=1\x07"
        )
        .into_bytes()
    }

    fn finished_results(
        stream: &[u8],
        scanner: &mut OscScanner,
    ) -> Vec<crate::terminal::integration::CommandExecutionResult> {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some(SID.to_string()));
        markers(scanner, stream)
            .into_iter()
            .filter_map(|marker| match sm.transition(marker) {
                Some(VerifiedCommandEvent::CommandFinished {
                    execution_result, ..
                }) => Some(execution_result),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn integrated_fish_native_command_markers_are_dropped_and_kegons_kept() {
        let mut scanner = fish_scanner();
        let found = markers(&mut scanner, &fish_command_line(0, "success"));
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(matches!(
            found[0],
            Osc133Marker::CommandStartedWithMetadata(_)
        ));
        assert!(matches!(
            found[1],
            Osc133Marker::CommandFinishedWithMetadata(Some(0), _)
        ));
        // Both prompt marks pass: the native one is harmless (see the state machine test below).
        assert_eq!(found[2], Osc133Marker::PromptStarted);
        assert_eq!(found[3], Osc133Marker::PromptStarted);
    }

    #[test]
    fn integrated_fish_command_line_is_one_matched_result() {
        for (exit, status, shell_status) in [
            (0, "success", ShellReportedStatus::Success),
            (1, "failure", ShellReportedStatus::Failure),
            (127, "failure", ShellReportedStatus::Failure),
        ] {
            let results = finished_results(&fish_command_line(exit, status), &mut fish_scanner());
            assert_eq!(results.len(), 1, "{results:?}");
            assert_eq!(results[0].shell_status, shell_status);
            assert_eq!(results[0].osc133_exit, Some(exit));
            assert_eq!(results[0].completion, CompletionReason::DObserved);
            assert_eq!(results[0].correlation, MetadataCorrelation::Matched);
        }
    }

    #[test]
    fn unfiltered_scanner_keeps_the_native_markers_and_the_mismatch() {
        // The filter is what separates the two: without it the native C makes Kegon's a duplicate.
        let results = finished_results(
            &fish_command_line(0, "success"),
            &mut OscScanner::new(HOST.into()),
        );
        assert_eq!(results[0].correlation, MetadataCorrelation::Mismatched);
        assert_eq!(results[0].shell_status, ShellReportedStatus::Unknown);
    }

    #[test]
    fn native_prompt_marks_do_not_repeat_a_verified_command_or_dopamine_event() {
        // After a finished command the native A arrives in the Prompt state: a second
        // PromptStarted, no command event at all.
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some(SID.to_string()));
        let mut scanner = fish_scanner();
        let events: Vec<_> = markers(&mut scanner, &fish_command_line(0, "success"))
            .into_iter()
            .filter_map(|marker| sm.transition(marker))
            .collect();
        let commands = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    VerifiedCommandEvent::CommandStarted { .. }
                        | VerifiedCommandEvent::CommandFinished { .. }
                )
            })
            .count();
        assert_eq!(commands, 2, "one start and one finish: {events:?}");
        assert_eq!(
            events.last(),
            Some(&VerifiedCommandEvent::PromptStarted),
            "{events:?}"
        );
    }

    #[test]
    fn filter_keeps_markers_with_foreign_or_broken_kegon_parameters() {
        let mut scanner = fish_scanner();
        // Another tab's identity: complete metadata, so the state machine rejects it.
        let foreign = markers(
            &mut scanner,
            b"\x1b]133;C;kegon.v=1;kegon.sid=other-tab;kegon.seq=1\x07",
        );
        assert!(matches!(
            foreign[..],
            [Osc133Marker::CommandStartedWithMetadata(_)]
        ));
        // Broken or partial metadata is no complete envelope but is still Kegon's business.
        for bad in [
            &b"\x1b]133;C;kegon.v=1\x07"[..],
            b"\x1b]133;C;kegon.v=1;kegon.sid=bad sid;kegon.seq=1\x07",
            b"\x1b]133;C;kegon.v=9;kegon.sid=a;kegon.seq=1\x07",
            b"\x1b]133;C;kegon.v=1;kegon.v=1;kegon.sid=a;kegon.seq=1\x07",
            b"\x1b]133;D;0;kegon.v=1\x07",
            b"\x1b]133;D;kegon.v=1;kegon.sid=a\x07",
        ] {
            assert_eq!(
                markers(&mut scanner, bad).len(),
                1,
                "dropped {:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // A look-alike is not a parameter: only the structured `kegon.` key counts.
        assert!(markers(&mut scanner, b"\x1b]133;C;cmdline_url=kegon.sid\x07").is_empty());
        assert!(markers(&mut scanner, b"\x1b]133;D;0;mykegon.v=1\x07").is_empty());
    }

    #[test]
    fn filter_never_touches_other_osc() {
        let mut scanner = fish_scanner();
        assert_eq!(
            scanner.scan(b"\x1b]133;A;special_key=1\x07\x1b]133;B\x07"),
            vec![
                OscEvent::ShellIntegration(Osc133Marker::PromptStarted),
                OscEvent::ShellIntegration(Osc133Marker::InputStarted),
            ]
        );
        // The URI path of an absolute directory: Windows needs a drive letter (`/tmp` is not an
        // absolute path there, so the report is rightly rejected); elsewhere `/tmp` is fine.
        let dir = if cfg!(windows) { "/C:/tmp" } else { "/tmp" };
        assert_eq!(
            scanner
                .scan(format!("\x1b]7;file://{HOST}{dir}\x07").as_bytes())
                .len(),
            1
        );
        let b64 = "a2Vnb24udj0xO2tlZ29uLnNpZD1hO2tlZ29uLnNlcT0x";
        let status = format!("\x1b]1337;SetUserVar=kegon_status={b64}\x07");
        assert_eq!(scanner.scan(status.as_bytes()).len(), 1);
    }

    #[test]
    fn default_scanner_reports_unmarked_command_markers() {
        // Bash, zsh, PowerShell, a disabled or unsupported shell: nothing is filtered.
        let mut scanner = OscScanner::new(HOST.into());
        assert_eq!(
            markers(
                &mut scanner,
                b"\x1b]133;C;cmdline_url=true\x07\x1b]133;D;0\x07"
            ),
            vec![
                Osc133Marker::CommandStarted,
                Osc133Marker::CommandFinished(Some(0))
            ]
        );
    }
}
