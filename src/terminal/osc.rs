//! Streaming parser and extractor for OSC (Operating System Command) sequences.
//!
//! Extracts working directory notifications (OSC 7, OSC 9;9) and shell integration markers
//! (OSC 133 A/B/C/D) from raw PTY output streams with bounded memory and chunk reassembly.

use std::path::PathBuf;

use crate::terminal::cwd::{parse_osc7_uri, parse_osc9_9_path};
use crate::terminal::integration::Osc133Marker;

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
}

impl OscScanner {
    /// Creates a new scanner for the given local machine host name.
    pub fn new(local_host: String) -> Self {
        Self {
            local_host,
            state: Framing::Ground,
            payload: Vec::new(),
            overlong: false,
        }
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
                Some(OscEvent::ShellIntegration(marker))
            }
            _ => None,
        }
    }
}

/// Parses the payload after `133;` into an [`Osc133Marker`].
///
/// Handled forms:
/// - `A` (with optional `;...`) -> PromptStarted
/// - `B` (with optional `;...`) -> InputStarted
/// - `C` (with optional `;...`) -> CommandStarted
/// - `D` -> CommandFinished(None)
/// - `D;<code>` -> CommandFinished(Some(code)) if valid integer, or CommandFinished(None) if invalid
pub fn parse_osc133(rest: &str) -> Option<Osc133Marker> {
    let mut parts = rest.split(';');
    let tag = parts.next()?.trim();
    match tag {
        "A" => Some(Osc133Marker::PromptStarted),
        "B" => Some(Osc133Marker::InputStarted),
        "C" => Some(Osc133Marker::CommandStarted),
        "D" => {
            let exit_code = match parts.next() {
                Some(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        trimmed.parse::<i32>().ok()
                    }
                }
                None => None,
            };
            Some(Osc133Marker::CommandFinished(exit_code))
        }
        _ => None,
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
                OscEvent::WorkingDirectory(PathBuf::from(r"C:\works\kegon")),
                OscEvent::ShellIntegration(Osc133Marker::InputStarted),
                OscEvent::ShellIntegration(Osc133Marker::CommandStarted),
                OscEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
            ]
        );
    }
}
