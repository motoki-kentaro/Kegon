# Terminal Architecture

Status: accepted for the single terminal session PoC (Issue #5)  
Date: 2026-10-07

## Overview

Kegon integrates a real terminal emulator and PTY session behind a Kegon-owned abstraction boundary. The terminal emulation core is powered by [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal).

## Core Selection

- **Crate**: `alacritty_terminal` `0.26.0` (patched fork; see [Patched alacritty_terminal](#patched-alacritty_terminal-issue-50))
- **Rust Version Requirement**: `1.85.0+` (Kegon uses `1.88+`)
- **License**: `Apache-2.0` (Compatible with Kegon's `MIT` license with attribution retaining required notices)

`alacritty_terminal` was selected because:
1. It is a battle-tested, high-performance terminal emulator core written entirely in Rust.
2. It provides built-in PTY and ConPTY support via `alacritty_terminal::tty` on Windows.
3. It cleanly separates the terminal grid state machine (`Term`) and parser (`vte`) from window creation and rendering.

## Patched alacritty_terminal (Issue #50)

Kegon depends on a patched fork of `alacritty_terminal` instead of the crates.io release. Windows ConPTY leaked OS handles every time a PTY was created, which accumulates across opening and closing terminal tabs (Issue #49).

- **Dependency**: Git dependency on `https://github.com/motoki-kentaro/alacritty`, pinned by full commit SHA in `Cargo.toml` (`rev`) and recorded in `Cargo.lock`.
- **Branch**: `kegon/0.26.0-conpty-fixes`, based on the upstream tag `alacritty_terminal_v0.26.0` (commit `68ca510074ed4000ae5d1b0cba82f5154752e8cf`). The crate version stays `0.26.0` and the only changes are the ConPTY fixes, so the public API and behavior (for example the DA2 reply `2600`) match upstream 0.26.0.
- **Current pin**: `af8509f9858ab4cd54434b7fbcf293fec7532ca8` (Slice 3: adds initialization-failure cleanup on top of `c3a4ffc2`).
- **Upstream reference**: the fix was developed against upstream master `29dc55375da817d53540f4caefbe708703c57b07` (kept as `fix/windows-conpty-handle-leaks` in the fork) and then cherry-picked onto 0.26.0 to avoid pulling unreleased upstream changes.
- **License**: the fork remains Apache-2.0 with the upstream `LICENSE-APACHE` and copyright notices unchanged. Kegon stays MIT. The patch is not submitted upstream.

### Handle leaks found (4 per PTY on the normal path, all fixed)

Measured with a standalone reproduction (`GetProcessHandleCount` over repeated create/drop cycles): about +4 handles per PTY on upstream 0.26.0.

| Handle | Status |
|---|---|
| ConPTY input pipe, PTY-side end | Fixed (Slice 1) |
| ConPTY output pipe, PTY-side end | Fixed (Slice 1) |
| `hThread` from `CreateProcessW` | Fixed (Slice 1) |
| `hProcess` held by `ChildExitWatcher` | Fixed (Slice 2): the watcher owns it (`OwnedHandle`), unregisters the wait with `UnregisterWaitEx` (blocking until a running callback returns), then frees the callback context and closes the handle |

With Slice 1, 10 and 50 iterations (3 runs each) measured +1.00 handle per PTY (only `hProcess` remained). With Slice 2, 10, 50 and 100 iterations measure +0 handles. The `repeated_tab_open_close_does_not_accumulate_resources` test (ignored by default, real shells) fails on growth beyond a fixed slack. If the wait cannot be unregistered, the watcher deliberately leaks its context and handle rather than risk a use-after-free (a warning is logged).

### Initialization failure cleanup (Slice 3)

`conpty::new` now owns the pseudoconsole (`Conpty`) and the pipe reader/writer threads as soon as they exist, so every early return releases the HPCON, both pipes and their drain threads (locals drop in reverse order: the pseudoconsole closes while the output pipe is still drained, which avoids a deadlock in `ClosePseudoConsole`). An `AttributeListGuard` calls `DeleteProcThreadAttributeList` once `InitializeProcThreadAttributeList` succeeded, including on the success path, where the list was previously never deleted.

Verified failure path: `CreateProcessW` failing after the pseudoconsole was created (a shell program that does not exist).

| Measurement (patched pin, release build, `cmd.exe` unless noted) | Result |
|---|---|
| Fork unit tests `failed_process_creation_releases_pseudoconsole` (100 failures) and `successful_creation_and_drop_does_not_leak` (30 cycles) | pass |
| Reproduction program, nonexistent shell, 100 failures x 3 runs | +0 handles each. Upstream 0.26.0 with the same program: +500 (5 per failure) |
| Reproduction program, normal path, 10 / 50 / 100 / 200 iterations x 3 runs | +0 in 9 of 12 runs; +6 in 3 of 12 (two of the 100-iteration runs and one of the 200-iteration runs; none at 10 or 50). The +6 is not proportional to the iteration count (100 and 200 iterations give the same +6) |
| Kegon `repeated_tab_open_close_does_not_accumulate_resources` (real pwsh, 3 + 10 + 50 cycles, then 6 tabs held at once) | handles 153 at baseline, after 10 cycles and after 50 cycles; 201 while 6 extra tabs are open (live resources); 153 after closing them all |

**Unverified failure paths**: failures of `CreatePseudoConsole`, `InitializeProcThreadAttributeList`, `UpdateProcThreadAttribute` and of the exit-wait registration in `ChildExitWatcher::new` could not be induced without fault injection and are covered by code reading only.

**Observation, cause unconfirmed**: the occasional +6 handles seen with the reproduction program consist of IRTimer x2, IoCompletion x1, TpWorkerFactory x1 and WaitCompletionPacket x2 (per-object-type census from `--dump-handles`). These are Windows thread-pool objects, which suggests that the OS thread pool used by the exit-wait callbacks grows once and then stays, but this has not been confirmed. When measuring, count these separately from process, file and thread handles.

### Shells outliving a closed tab (open, not fixed in Slice 3)

Dropping the PTY does not always end the shell. Observations on this machine:

- The reproduction program drops the `Pty` immediately after `tty::new`: about 22-26% of the `cmd.exe` shells were still alive 10 s later (the reproduction program's README records similar residue on upstream 0.26.0; not re-measured in Slice 3). Holding the PTY for 300 ms or 1500 ms before dropping left 0 of 50 alive in both of 2 runs each.
- Kegon's own repeated-tab test shows the same split: with a 150 ms delay before closing, the child count returns to baseline; with the delay set to 0 ms (temporary experiment, not committed) 18 extra processes remained after 50 cycles, while the handle count stayed flat. Handles and surviving shells are independent problems.
- GUI dogfood (shells that had been alive for seconds): closing tabs left no shell behind, and no shell remained after closing the last tab.
- Lifecycle difference: the reproduction program drops the `Pty` synchronously on its main thread; Kegon sends `Msg::Shutdown` and the event-loop thread drops the `Pty` (`ClosePseudoConsole`) asynchronously. In both cases the shell is not terminated explicitly; it relies on the pseudoconsole closing.
- Hypothesis (not confirmed): a pseudoconsole closed before the shell has attached to the console does not terminate it. A user who opens and instantly closes a tab might be able to trigger this in Kegon. This should be tracked as a separate issue.

### Updating

1. Fetch upstream tags and create a new branch from the new `alacritty_terminal_v*` tag.
2. Cherry-pick the fix commits, run Kegon's checks, and push the branch to the fork.
3. Update `rev` in `Cargo.toml` and run `cargo update -p alacritty_terminal`. Review the `Cargo.lock` diff so that only `alacritty_terminal` (and any dependencies it needs) changes. Update the version in `THIRD_PARTY_NOTICES.md`.

### Reverting to upstream

When a released `alacritty_terminal` contains the fixes, replace the dependency in `Cargo.toml` with `alacritty_terminal = "<version>"`, run `cargo update -p alacritty_terminal`, and remove this section and the fork note in `THIRD_PARTY_NOTICES.md`. The fork branches may then be archived.

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

Upstream `alacritty_terminal` 0.26 leaks four handles per pseudoconsole on Windows; the patched fork pinned by Kegon closes all of them (see "Patched alacritty_terminal"), so handle counts return to baseline after tabs are closed.

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
