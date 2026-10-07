# Terminal Input Arbitration & Extended Keyboard Architecture

Status: accepted (Issue #9)  
Date: 2026-10-07

## Overview

Kegon organizes terminal keyboard input under a strict **Input Ownership & Arbitration** model. The core design principle is:

> **Kegon commands are opt-in. Terminal input is the default owner.**

Unregistered key combinations (such as `Ctrl+D`, `Ctrl+R`, `Esc`, `Shift+Enter`, `Ctrl+Enter`, `Alt+Enter`, `Ctrl+L`, `Ctrl+U`, `Ctrl+W`) belong to the terminal child process by default and are never intercepted by Kegon.

```text
Keyboard / IME Event
        ↓
Input Arbitration (`InputArbiter`)
        ├─ 1. IME-owned (`InputRoute::Ime`)
        ├─ 2. Kegon Command-owned (`InputRoute::Command(CommandId)`)
        └─ 3. Terminal-owned (`InputRoute::Terminal`)
                 ↓
         TerminalKeyEncoder
           ├─ Legacy VT/xterm mode
           └─ Extended keyboard protocol (Kitty / CSI-u)
                 ↓
                PTY
```

---

## Component Design

### 1. Input Ownership & Arbitration

`InputArbiter` (`src/terminal/arbitration.rs`) evaluates incoming keyboard and IME events in order of precedence:

1. **IME Precedence (`InputRoute::Ime`)**: When an active IME preedit composition is in progress, IME owns keyboard input. Preedit text updates UI state only and is **never** sent to the PTY or checked for application shortcuts.
2. **Kegon Command Precedence (`InputRoute::Command(CommandId)`)**: If the key chord matches a registered Kegon command binding in the active context (e.g. `Ctrl+Shift+E`, or `Ctrl+C` with active text selection), Kegon consumes the event and executes the command. Nothing is sent to the PTY.
3. **Terminal Default Ownership (`InputRoute::Terminal`)**: All unmatched key events pass through to the `TerminalKeyEncoder` and are delivered directly to the child PTY process.

---

### 2. Terminal Key Encoder & Keyboard Protocols

`TerminalKeyEncoder` (`src/terminal/key_encoder.rs`) is a dedicated, decoupled component responsible exclusively for translating terminal-owned input events (`TerminalInputEvent`) into ANSI / CSI escape sequences:

- **Legacy VT/xterm Mode**: Active by default when the child application has not requested extended keyboard protocols.
  - `Enter` → `\r`
  - `Backspace` → `\x7f`
  - `Tab` / `Shift+Tab` → `\t` / `\x1b[Z`
  - `Escape` → `\x1b`
  - `Arrow` / `Home` / `End` / `PageUp` / `PageDown` / `Insert` / `Delete` / `F1`..`F12` → Standard VT escape codes (accounting for `DECCKM` application cursor keys mode).
  - `Ctrl+A`..`Z` → Control codes `0x01`..`0x1a`.
  - `Alt+char` → `\x1b` prefix + UTF-8 character bytes.

- **Extended Keyboard Protocol (Kitty / CSI-u)**: Active when the child application requests progressive enhancement flags via terminal mode escape sequences.
  - `Shift+Enter` → `\x1b[13;2u`
  - `Alt+Enter` → `\x1b[13;3u`
  - `Ctrl+Enter` → `\x1b[13;5u`
  - `Ctrl+Shift+Enter` → `\x1b[13;6u`
  - `Shift+Tab` / `Ctrl+Tab` → `\x1b[9;2u` / `\x1b[9;5u`
  - Extended event types (`:1` press, `:2` repeat, `:3` release) when `REPORT_EVENT_TYPES` is requested by the child process.

---

### 3. Integration with `alacritty_terminal`

Kegon queries terminal keyboard mode flags (`TerminalKeyboardMode` in `src/terminal/mode.rs`) directly from `alacritty_terminal`'s emulator state without exposing `alacritty_terminal` types outside the `terminal` subsystem:

- `DISAMBIGUATE_ESC_CODES` (Kitty flag 1)
- `REPORT_EVENT_TYPES` (Kitty flag 2)
- `REPORT_ALTERNATE_KEYS` (Kitty flag 4)
- `REPORT_ALL_KEYS_AS_ESC` (Kitty flag 8)
- `REPORT_ASSOCIATED_TEXT` (Kitty flag 16)

---

### 4. Key Input Usage (Logical vs. Physical vs. Generated Text)

- **Kegon Command Matching**: Uses **Logical Keys** (`iced::keyboard::key::Key`) so command shortcuts map to expected characters regardless of keyboard layout (QWERTY, AZERTY, Dvorak, JIS).
- **Printable Text Input**: Uses OS/GUI-generated UTF-8 text (`event.text`) so non-US layouts, shifted characters, accented keys, and IME commits are delivered faithfully.
- **Control Codes & Escape Sequences**: Uses logical key codes and physical scancodes (`event.physical_key`) when generating VT/CSI sequences.

---

### 5. AltGr & Layout Safety

On Windows/Linux keyboards with an AltGr key, pressing AltGr emits `Ctrl + Alt` modifiers. Kegon command keybindings explicitly require `alt: false`. When `Ctrl + Alt` is pressed, `InputArbiter` evaluates the binding to `None` and routes the event to `InputRoute::Terminal`. The generated UTF-8 text (e.g. `@`, `€`, `~`) is delivered cleanly to the PTY.

---

### 6. IME Boundary

MS-IME and other IME input methods operate across two phases:

1. **Preedit Phase**: Emits `ImeEvent::Preedit(text, ...)`. `InputArbiter` routes keypresses to `InputRoute::Ime`. Preedit strings are rendered in the UI overlay and are **never** sent to the PTY.
2. **Commit Phase**: Emits `ImeEvent::Commit(text)`. Preedit state clears and the committed UTF-8 text string is written directly to the PTY without passing through command shortcut resolution.

---

### 7. Future Modal Ownership

When modal dialogs or command palettes are added in future issues, a `CommandContext::ModalOpen` (or similar modal arbitration state) will intercept all keyboard, IME, and mouse events, routing them exclusively to the active modal dialog and blocking input to background terminal sessions.

---

## Known Limitations

- **Event Release Reporting**: Event release reporting (`KeyEventKind::Release`) is only emitted when the GUI event loop provides release events and the child application has explicitly requested `REPORT_EVENT_TYPES`. Kegon does not synthesize artificial release events.
