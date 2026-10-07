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
- Asynchronous events (`Wakeup`, `Title`, `ChildExit`) are posted to an channel and consumed by iced's event subscription loop to trigger UI redraws.

## Renderer

- Rendered via iced's `Canvas` widget (`TerminalProgram` implementing `canvas::Program`).
- Grid cells are measured with dynamic `TerminalCellMetrics` derived from active font OpenType metrics (`units_per_em`, `ascender`, `descender`, `line_gap`, and monospace advance width). Default fallback metrics are `cell_width = 8.5px`, `cell_height = 18.0px`.
- Supports:
  - ASCII and Unicode characters
  - Japanese CJK wide characters (`WIDE_CHAR` occupying 2 column widths)
  - 16 ANSI colors, 256 indexed colors, and TrueColor RGB
  - Cursor styles (block cursor when focused, hollow outline when unfocused)
  - Selection highlights
  - Preedit inline composition rendering for Japanese MS-IME

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

- iced 0.14 native `Event::InputMethod` events are consumed:
  - `ImeEvent::Preedit(text, _)`: Stores transient `preedit_text` in UI state and renders preedit text inline over the cursor position with underline. Preedit text is **never** sent to the PTY. Space pressed during composition drives conversion candidate selection and is not leaked to the PTY.
  - `ImeEvent::Commit(text)`: Clears preedit state and sends the committed UTF-8 string directly to the PTY.
- Prevents double-transmission of composition keystrokes.

## Clipboard Operations

- Integrated using [`arboard`](https://crates.io/crates/arboard) (`3.6.1`).
- All clipboard access is wrapped gracefully in `src/terminal/clipboard.rs` to prevent panics on clipboard access errors.

## Resize and Scrollback

- Terminal area pixel bounds compute column/row count: `(width / cell_width, height / cell_height)`.
- Resizing updates `alacritty_terminal` grid dimensions and sends a `Msg::Resize` command to ConPTY.
- Mouse wheel scrolling shifts the display offset into the scrollback history buffer.

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
