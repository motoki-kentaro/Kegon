# Terminal Architecture

Status: accepted for the single terminal session PoC (Issue #5)  
Date: 2026-10-07

## Overview

Kegon integrates a real terminal emulator and PTY session behind a Kegon-owned abstraction boundary. The terminal emulation core is powered by [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal).

## Core Selection

- **Crate**: `alacritty_terminal` `0.26.0`
- **Rust Version Requirement**: `1.85.0+` (Kegon uses `1.88+`)
- **License**: `Apache-2.0` (Compatible with Kegon's `MIT` license with attribution retaining required notices)

`alacritty_terminal` was selected because:
1. It is a battle-tested, high-performance terminal emulator core written entirely in Rust.
2. It provides built-in PTY and ConPTY support via `alacritty_terminal::tty` on Windows.
3. It cleanly separates the terminal grid state machine (`Term`) and parser (`vte`) from window creation and rendering.

## Abstraction Boundary

To ensure `alacritty_terminal` details do not leak throughout the codebase, Kegon wraps all terminal emulation, PTY process lifecycle, and input dispatch inside `src/terminal/`:

```text
Kegon UI / iced (src/app.rs)
      ↓
Kegon-owned Terminal Abstraction (src/terminal/session.rs, renderer.rs, input.rs)
      ↓
alacritty_terminal (Term, vte)
      ↓
PTY / ConPTY (alacritty_terminal::tty)
      ↓
pwsh.exe / powershell.exe / cmd.exe
```

- `src/workbench.rs` holds no GUI toolkit or `alacritty_terminal` dependencies.
- `src/app.rs` manages high-level lifecycle through `TerminalSession`, `TerminalProgram`, and `process_key_event`.

## Shell Resolution

On Windows, initial shell discovery prioritizes:
1. `pwsh.exe` (PowerShell Core, checked via PATH and `C:\Program Files\PowerShell\7\pwsh.exe`).
2. `powershell.exe` (Windows PowerShell fallback).
3. `cmd.exe` (Command Prompt ultimate fallback).

## PTY / ConPTY Integration

- On Windows, `alacritty_terminal::tty::new` initializes Windows ConPTY natively.
- Background process I/O is managed by `alacritty_terminal::event_loop::EventLoop`, which polls the PTY reader/writer pipes and updates `Term` grid state in a dedicated thread.
- Asynchronous events (`Wakeup`, `Title`, `ResetTitle`, `ChildExit`) are posted to an channel and consumed by iced's event subscription loop to trigger UI redraws.

### Terminal Title

- `Title` (OSC 0 / OSC 2) and `ResetTitle` are forwarded raw to the app. `terminal::title::normalize_terminal_title` then drops control characters, trims, and caps the title at 120 chars with `…`. A title that ends up empty counts as no title.
- `Kegon::terminal_title: Option<String>` is the only copy of the title. The active tab shows it (falling back to the translated default label), and the window title is `<title> - Kegon` (or `Kegon`). iced re-reads `title()` after each update.
- On Windows, conhost reports the child's executable path as the initial title. `CSI 22 t` / `CSI 23 t` stay inside conhost, so `ResetTitle` is not observed there.

### Terminal-Generated Query Responses

- Replies that `alacritty_terminal` generates for child queries (`Event::PtyWrite`: DA, DSR, DECRQM, `CSI 18 t`; `Event::TextAreaSizeRequest`: `CSI 14 t`) are written by `EventProxy` straight back to the PTY through the session's `EventLoopSender` (`Msg::Input`), mirroring upstream Alacritty's `Notifier`.
- They deliberately bypass the iced subscription so a child blocked on a query never waits for a UI frame.
- `EventProxy::send_event` runs on the PTY thread with the `Term` lock held, so it must never lock `Term`. `TextAreaSizeRequest` reads the `WindowSize` that `TerminalSession::resize` records instead.
- The sender is injected through a `OnceLock` after `EventLoop::new` and before `spawn`, so no reply can be generated without a sink.
- On Windows, the inbox conhost answers most queries itself (DA1/DA2/DSR/DECRQM/`CSI 18 t`) and does not forward them; `CSI 14 t` is forwarded and answered by Kegon. No duplicate replies were observed.
- `ColorRequest`, `ClipboardStore`, and `ClipboardLoad` remain unhandled.

## Renderer

- Rendered via iced's `Canvas` widget (`TerminalProgram` implementing `canvas::Program`).
- Grid cells are measured with dynamic `TerminalCellMetrics` derived from active font OpenType metrics (`units_per_em`, `ascender`, `descender`, `line_gap`, and monospace advance width). Default fallback metrics are `cell_width = 8.5px`, `cell_height = 18.0px`.
- **Text Alignment & Shaping**:
  - `LineHeight::Absolute(metrics.cell_height)` matches cell height to preserve vertical baseline placement across fonts.
  - `Shaping::Advanced` enables `cosmic-text` system font fallback for missing glyphs (Braille `U+2800..U+28FF`, Box Drawing `U+2500..U+257F`, Block Elements `U+2580..U+259F`, symbols `⚠`, `✓`, `✗`, `•`, `→`, `←`, and CJK).
  - Individual per-cell text rendering preserves logical grid isolation and prevents multi-cell ligature merges.
- **Attributes & Features**:
  - ASCII, Unicode, combining zero-width characters (`zerowidth()`), and Japanese CJK wide characters (`WIDE_CHAR`).
  - Terminal cell attributes: `BOLD` (weight mapping), `ITALIC` (style mapping), `DIM` (foreground alpha scaling), `INVERSE` (fg/bg swap), `HIDDEN` (background preserved, text hidden).
  - Metric-driven text line attributes: `UNDERLINE`, `DOUBLE_UNDERLINE`, and `STRIKEOUT`.
  - 16 ANSI colors, 256 indexed colors, and TrueColor RGB.
  - Cursor styles: block cursor when focused with contrasting character text rendered over solid block, hollow outline when unfocused.
  - Translucent text selection highlights (`SELECTION_ALPHA = 0.6`).
  - Preedit inline composition rendering for Japanese MS-IME.

## Keyboard Input Routing & Escapes

Logical key events are mapped to terminal escape sequences and characters:
- **Printable Text**: Input text is passed directly to the PTY. Physical key codes are not forced to US layout, preserving non-US (e.g. Japanese JIS) and AltGr layouts.
- **Space Key**: `Named::Space` and `Character(" ")` encode to `0x20` (Space) in legacy mode, `\x00` for `Ctrl+Space`, `\x1b ` for `Alt+Space`, and `\x1b[32;...u` under Kitty/CSI-u extended mode.
- **Enter**: `\r` (or `\x1b[13;...u` extended)
- **Backspace**: `\x7f` (or `\x1b[127;...u` extended)
- **Tab / Shift+Tab**: `\t` / `\x1b[Z` (or `\x1b[9;...u` extended)
- **Arrows**: ANSI cursor sequences (`\x1b[A`, `\x1b[B`, etc.) or application cursor mode (`\x1bOA`, etc.)
- **Home / End / PageUp / PageDown / Delete / Insert**: VT escape sequences (`\x1b[H`, `\x1b[5~`, etc.)
- **Function Keys**: F1..F12 (`\x1bOP`..`\x1b[24~`)
- **Ctrl+A..Z**: Control characters `\x01`..`\x1a`
- **Alt+Key**: ESC prefix `\x1b<char>`
- **Workbench Shortcuts**: `Ctrl+Shift+E/F/G` are intercepted and routed to the Workbench Activity Bar, bypassing the terminal PTY.

## Text Selection & Scrollback Mapping

- Mouse left-button drag creates and extends local simple text selections.
- Selection cell coordinates use active `TerminalCellMetrics` (`cell_width`, `cell_height`) and account for `display_offset` in `alacritty_terminal` grid line indexing, supporting scrollback selection.
- Mouse coordinates outside the terminal viewport bounds are safely clamped to the visible grid boundaries `[0..cols-1, 0..rows-1]`.
- Selection highlights are rendered using active theme tokens (`colors.selection`) with `SELECTION_ALPHA` opacity.

## Ctrl+C / Ctrl+V & Clipboard Policy

- **Windows / Linux**:
  - **Ctrl+C**:
    - If a text selection exists: Copies the selected text to the OS clipboard (`arboard`) and clears the selection. Does **not** send `\x03` to the child process.
    - If no selection exists: Sends `\x03` (SIGINT) to the child process via PTY.
  - **Ctrl+V**:
    - Reads text from the OS system clipboard (`arboard`).
    - If the child application has enabled bracketed paste mode (`BRACKETED_PASTE`), wraps the text with `\x1b[200~` ... `\x1b[201~`.
    - Normalizes newlines to `\r` for normal terminal paste.
- **macOS**:
  - **Cmd+C**: Copy active selection.
  - **Cmd+V**: Paste from clipboard.
  - **Ctrl+C**: Send `\x03` (SIGINT) interrupt signal.

## Japanese MS-IME Integration

- **Input Method Activation & Candidate Positioning**:
  - Managed by custom widget `TerminalSurface` (`src/terminal/surface.rs`) wrapping the terminal canvas.
  - Calls iced 0.14 `shell.request_input_method(&InputMethod::Enabled { cursor, purpose: Purpose::Normal, preedit: None })` when the terminal has keyboard focus (`terminal_focused == true` and no modal is active).
  - Calculates `cursor` rectangle from active grid cursor position `(cursor_col, cursor_line)` and dynamic `TerminalCellMetrics` (`cell_width`, `cell_height`) mapped to absolute window coordinates: `Rectangle { x: bounds.x + col * cell_width, y: bounds.y + line * cell_height, width: cell_width, height: cell_height }`.
  - When focus leaves the terminal (e.g. modal confirmation or font picker opens), calls `shell.request_input_method(&InputMethod::Disabled)` to disable native IME ownership.
- **IME Event Handling**:
  - iced 0.14 native `Event::InputMethod` events are consumed:
    - `ImeEvent::Preedit(text, _)`: Stores transient `preedit_text` in UI state and renders preedit text inline over the cursor position with underline. Preedit text is **never** sent to the PTY. Space pressed during composition drives conversion candidate selection and is not leaked to the PTY.
    - `ImeEvent::Commit(text)`: Clears preedit state and sends the committed UTF-8 string directly to the PTY.
    - `ImeEvent::Closed`: Clears preedit state.
- Prevents double-transmission of composition keystrokes.

## Clipboard Operations

- Integrated using [`arboard`](https://crates.io/crates/arboard) (`3.6.1`).
- All clipboard access is wrapped gracefully in `src/terminal/clipboard.rs` to prevent panics on clipboard access errors.

## Resize and Scrollback

- Terminal area pixel bounds compute column/row count: `(width / cell_width, height / cell_height)`.
- `TerminalSession::resize` applies one clamped (`>= 1`) cols/rows pair to both sides: it sends `Msg::Resize` to the PTY and calls `Term::resize` on the grid. `Msg::Resize` alone never resizes the grid, because the event loop only forwards it to the PTY.
- Both updates (and the shared `WindowSize` used for `CSI 14 t`) happen under one `Term` lock, as in upstream Alacritty. As a result, the PTY thread cannot parse the child's redraw for the new size into the old grid.
- Reflow, cursor clamping, and selection invalidation follow `alacritty_terminal`'s own `Term::resize` semantics.
- Mouse wheel scrolling shifts the display offset into the scrollback history buffer.

## Multiple Terminal Tabs (Issue #49)

Responsibilities are split so that other tab kinds can be added without touching terminal code:

| Piece | Owns | Does not know about |
| --- | --- | --- |
| `workbench::WorkbenchTabs` | Tab order, `TabKind`, the active tab, never-reused `TabId`s. `remove` implements the "right neighbour, else left" rule; the UI never repeats it. | Sessions, PTYs, rendering |
| `terminal::TerminalRegistry` | Every `TerminalSession`, keyed by `TabId`, with per-terminal state derived from its events: title and last reported working directory. Rejects duplicate registration; unknown ids fail safely. | Tab order, selection |
| `app::Kegon` | The pairing of the two: opens and closes tabs and sessions together, resolves the active session, and keeps window-level state (focus, IME preedit, drag selection). | PTY details |

- **Opening a tab** adds the tab, spawns the session, registers it, then activates it and focuses the terminal. If the session cannot be started the tab is removed again, so a tab never points at a missing session.
- **Closing a tab** removes the registry entry (dropping the session) and then the tab. Closing the last tab exits the application.
- **Event routing.** Every session reports through an `EventSink` that tags each event with its `TabId`; the registry applies it to that tab only. Ids are never reused, so a late event from a closed tab can only be dropped, never misdelivered. Background terminals keep receiving output and titles.
- **Input and drawing** go to the active tab's session only. Switching tabs clears the IME preedit and any drag selection and focuses the terminal.
- **Resize.** All sessions share one viewport, so every running session is resized together; switching tabs needs no resize. Exited sessions are never written to or resized.
- **Lifecycle.** A shell that exits on its own keeps its tab and shows the exit banner until the tab is closed. `TerminalSession::drop` sends `Msg::Shutdown`; the event loop thread then drops the PTY, which closes the pseudoconsole and ends the child. The UI thread never waits for this.

Known cost: `alacritty_terminal` 0.26 on Windows leaks four handles per pseudoconsole (the child's process and thread handles and the two pipe ends given to `CreatePseudoConsole` are never closed). Child processes do end; only the handles accumulate, about 4 per tab ever opened. Fixing it needs a patched or newer `alacritty_terminal`.

## Shell Integration and Working Directory (Issue #49)

`alacritty_terminal` (through `vte`) ignores OSC 7 and exposes no working-directory API, so Kegon reads the reports itself:

- `TappedPty` wraps the real PTY handed to `EventLoop`. Every read is passed through unchanged and also scanned by `cwd::CwdScanner`, a small framer that recognizes only `OSC 7` and `OSC 9;9`. Complete reports become `TerminalEvent::WorkingDirectory`, tagged with the tab like any other event; only the last report of a read is forwarded.
- `vte`'s parser was not used for this: with the `std` feature (which `alacritty_terminal` turns on) its OSC buffer is unbounded, and it dispatches a sequence that was merely interrupted, so a truncated path would count as a report. The framer caps the payload at 8 KiB (an overlong one is discarded unread), accepts a report only when closed by `BEL` or `ESC \`, and abandons it on `CAN`, `SUB`, or any other escape. State carries across reads.
- `OSC 7` carries `file://<host>/<percent-encoded path>`. A host that is empty, `localhost`, or this machine is local; any other host (for example a remote shell over SSH) is rejected, never read as a UNC server. UNC paths use an empty host and a `//server/share` path. Malformed URIs, queries, fragments, control characters and relative paths are rejected.
- `OSC 9;9;<path>` is the form `cmd.exe` can print from `PROMPT` (it cannot percent-encode).
- The shell is the only source of truth. `TerminalEntry::reported_cwd` starts as `None` and is set only by a report; the directory a session was started in is never recorded as its current directory. A new tab inherits the active tab's reported directory only if it still exists as a directory, and otherwise starts in the default directory (as it also does if the shell cannot start there).

How shells are made to report (`shell::Launch`), without touching profiles, execution policy or any persistent setting:

| Shell | Method |
| --- | --- |
| PowerShell 7 / Windows PowerShell 5.1 | `-NoExit -EncodedCommand`: after the user's profile, wraps the existing `prompt` function and prepends an invisible OSC 7. Runs inside `try`/`catch`, so a failure leaves the shell as the profile made it. |
| `cmd.exe` | Child-only `PROMPT` environment variable prefixed with an `OSC 9;9` sequence; an existing `PROMPT` is kept after it. |
| Anything else, or a shell started with custom arguments | Started exactly as before; no directory reports, so new tabs use the default directory. |

Set `KEGON_SHELL_INTEGRATION=0` to disable it.

**Known limit: stale directories.** A report is only as fresh as the shell's last prompt. If the user replaces the prompt (`function prompt { … }` in PowerShell, `prompt $G` in cmd) or moves into a shell without integration (for example starting `pwsh` or `bash` from inside it), reports stop and `reported_cwd` keeps the last value, so a new tab can start in a directory the user has since left. Both prompt cases are pinned by ignored tests. Detecting this reliably needs prompt boundaries (for example OSC 133 marks) so that "a prompt appeared without a report" can reset the directory to unknown; that is a separate design.

## Testing the Terminal Stack

- `cargo test` runs the unit tests: tab and registry state, event routing, CWD parsing and the OSC framer (including malformed, split, overlong and random input), shell launch planning, and randomized tab operation sequences.
- `cargo test -- --ignored` also runs the tests that need real shells and ConPTY (Windows with PowerShell 7, Windows PowerShell 5.1 and `cmd.exe`): directory reports for each shell, child processes ending when a session is dropped, prompt-staleness characterization, and repeated tab open/close with process and handle counts. They fail rather than skip when a shell is missing or a timeout passes.
- Measurements (they print numbers and assert nothing about speed): `cargo test --release scanner_throughput -- --ignored --nocapture` and `cargo test --release end_to_end_output -- --ignored --nocapture`.

## Non-Goals (Out of Scope for Issue #27)

- Right-click paste and terminal context menus.
- Double-click word selection and triple-click line selection.
- Child application mouse event reporting (X10, VT200, SGR mouse capture).
- Advanced graphics protocols (Sixel, Kitty, iTerm2 inline images).

## License & Attribution

- Kegon is licensed under the **MIT License**.
- `alacritty_terminal` is licensed under **Apache-2.0**.
- `arboard` is dual-licensed under **MIT OR Apache-2.0**.
- Third-party notices and licenses are documented in `THIRD_PARTY_NOTICES.md`.
