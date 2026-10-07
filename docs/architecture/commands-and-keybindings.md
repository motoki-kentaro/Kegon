# Command and keybinding architecture

Status: accepted (Issue #7)  
Date: 2026-10-07

## Overview

Kegon uses a unified command and keybinding pipeline to translate user keyboard events into executable actions across the application:

```text
OS Keyboard Event
      ↓
KeyChord Normalization (from iced)
      ↓
Keybinding Resolver (Context & Platform matching)
      ↓
CommandId (Internal stable identifier)
      ↓
Command Dispatcher
      ↓
Target Execution (Workbench / TerminalSession / SystemClipboard)
```

This architecture decouples OS-specific keyboard layout and platform shortcut conventions from core application logic.

---

## Component design

### 1. `CommandId`

`CommandId` (`src/command/command_id.rs`) is a typo-resistant Rust `enum` representing stable, internal command identifiers:

- `workbench.explorer.focus`
- `workbench.search.focus`
- `workbench.sourceControl.focus`
- `terminal.copy`
- `terminal.copyOrInterrupt`
- `terminal.interrupt`
- `terminal.paste`

`CommandId` is independent of localized UI display labels. It provides round-trip string conversion (`as_str()`, `FromStr`, `Display`) so future configuration files can store keybindings cleanly.

---

### 2. `KeyChord` & Normalization

`KeyChord` (`src/command/key_chord.rs`) is a platform-neutral domain model representing a single key press combined with modifier states:

- **Key**: `Key::Character(String)` (normalized to lower-case string) or `Key::Named(NamedKey)` (e.g. `Enter`, `Tab`, `ArrowUp`, `Escape`).
- **Modifiers**: `ctrl`, `shift`, `alt`, `super_key` (Windows key on Windows/Linux, Command key on macOS).

Low-level GUI toolkit (`iced`) events are normalized into domain `KeyChord` instances, keeping GUI framework types from leaking into the domain layer.

#### Logical Key vs. Physical Key Selection

Kegon uses **Logical Keys** (`iced::keyboard::key::Key`) for command shortcut resolution:
- **Rationale**: Logical key resolution maps to the character or symbol produced according to the user's active keyboard layout (e.g., QWERTY, AZERTY, Dvorak, Japanese JIS). When a user expects `Ctrl+Shift+E`, they intend to press the key that produces the character 'E' in their layout. If physical scancode (`Code::KeyE`) were used, non-QWERTY layouts would trigger commands on unexpected physical positions.
- **PTY Scancodes**: Physical scancodes and raw character sequences are strictly used in the raw terminal input fallthrough layer for terminal escape sequences.

#### AltGr Safety

On Windows and Linux keyboards with an AltGr key, pressing AltGr emits combined `Ctrl + Alt` modifiers.
Kegon's command keymap rules explicitly match modifier states (e.g., `ctrl: true, alt: false`). When AltGr is pressed (`ctrl: true, alt: true`), `KeybindingResolver` returns `None`. Consequently, AltGr character combinations (e.g., `@`, `€`, `~`) are never falsely consumed as Kegon command shortcuts and cleanly fall through to text input.

---

### 3. Platform Abstraction

`Platform` (`src/command/platform.rs`) explicitly represents OS platforms:

- `Platform::Windows`
- `Platform::Linux`
- `Platform::MacOS`

Default keymaps (`src/command/keymap.rs`) are defined explicitly for each platform and can be validated and tested independently on any host OS. At runtime, `Platform::current()` detects the host platform.

#### Default Keymap Matrix

| Platform | Context | Key Chord | Command ID |
|---|---|---|---|
| **Windows / Linux** | `Workbench` | `Ctrl+Shift+E` | `workbench.explorer.focus` |
| **Windows / Linux** | `Workbench` | `Ctrl+Shift+F` | `workbench.search.focus` |
| **Windows / Linux** | `Workbench` | `Ctrl+Shift+G` | `workbench.sourceControl.focus` |
| **Windows / Linux** | `TerminalFocused` | `Ctrl+C` | `terminal.copyOrInterrupt` |
| **Windows / Linux** | `TerminalFocused` | `Ctrl+V` | `terminal.paste` |
| **macOS** | `Workbench` | `Cmd+Shift+E` | `workbench.workbench.explorer.focus` |
| **macOS** | `Workbench` | `Cmd+Shift+F` | `workbench.search.focus` |
| **macOS** | `Workbench` | `Cmd+Shift+G` | `workbench.sourceControl.focus` |
| **macOS** | `TerminalFocused` | `Cmd+C` | `terminal.copy` |
| **macOS** | `macOS` | `Cmd+V` | `terminal.paste` |
| **macOS** | `TerminalFocused` | `Ctrl+C` | `terminal.interrupt` |

---

### 4. Command Context & Fallback Rules

Keybindings are scoped by `CommandContext` (`src/command/context.rs`):

- `CommandContext::TerminalFocused` (active when keyboard focus is inside the terminal)
- `CommandContext::Workbench` (global application level context)

#### Resolution Order

When a key event occurs:

1. **Active Context Check**: `KeybindingResolver` first checks rules defined in the active context (e.g., `TerminalFocused`).
2. **Global Fallback**: If no rule matches and the active context is not `Workbench`, the resolver falls back to checking `CommandContext::Workbench`.
3. **Fallthrough**: If no command matches in any context, the event falls through to raw PTY key handling in `src/terminal/input.rs`.

---

### 5. Terminal Responsibility Separation

The terminal layer handles keyboard input in two distinct stages:

1. **Command Consumption**: Shortcuts recognized by `KeybindingResolver` (such as `workbench.explorer.focus`, `terminal.copyOrInterrupt`, `terminal.paste`) are intercepted and dispatched to application targets. They are **never** sent to the underlying PTY child process.
2. **Raw PTY Fallthrough**: Unmatched key events (normal typing, cursor arrows, function keys, control codes like `Ctrl+A` or `Ctrl+Z`, and Japanese IME commits) pass through to `process_key_event` to produce ANSI/escape sequences sent directly to the PTY.

---

### 6. Keymap Validation & Conflict Detection

`KeybindingResolver::validate` ensures that no keymap contains duplicate or conflicting definitions for the same `KeyChord` within the same `CommandContext`. If a conflict is detected during initialization, a `KeybindingConflictError::DuplicateBinding` error is returned.

---

## Extensibility: Adding New Commands & Keybindings

To add a new command to Kegon:

1. **Add Variant to `CommandId`**: Add a new variant and string identifier in `src/command/command_id.rs`.
2. **Register Action in `CommandDispatcher`**: Add dispatch logic in `src/command/dispatcher.rs` to invoke the target subsystem action.
3. **Register Default Keymaps**: Add binding rules for target platforms in `src/command/keymap.rs`.
4. **Add Unit Tests**: Write tests in `src/command/resolver.rs` verifying resolution across platforms and contexts.
