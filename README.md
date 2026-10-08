# Kegon

**A native, Git-native terminal workspace, written in Rust.**

> Kegon is not another code editor.
>
> It is a native terminal workspace built around repositories, shells, CLI tools, and Git.

---

## What is Kegon?

Kegon is a desktop application that puts the **terminal** at the center of the workspace, not a code editor.

The main area is designed as a set of terminal tabs, each running its own process: PowerShell, MSYS2, WSL, SSH, Claude Code, Codex CLI, Gemini CLI, or any other command-line tool you use. Around those terminals, Kegon intends to provide just enough GUI to browse the repository, search it, and review changes through Git.

Kegon is **not** an IDE built for AI agents. From Kegon's point of view, an AI coding agent and a plain shell are the same thing: a command running in a terminal.

## Why Kegon?

A growing share of day-to-day development now happens inside terminals. Shells, build tools, and increasingly AI coding agents run there, read and write files, and produce changes that end up in Git.

In that workflow, the human role shifts. Less time is spent typing code into an editor, and more time is spent:

- running and switching between several terminal sessions,
- watching what those tools are doing,
- reviewing which files changed and how,
- deciding what to stage, commit, or discard.

Existing tools tend to approach this from one of two sides. Code editors and IDEs treat the terminal as a secondary panel. Terminal emulators handle sessions well but know nothing about the repository they are running in.

Kegon aims to sit between them: a terminal-first workspace that is aware of the repository and treats Git as a first-class part of the experience.

## Core principles

- **Terminal first.** The main area is a multi-tab terminal. Everything else supports it.
- **Tool agnostic.** Any shell or CLI can be launched as a terminal session. No tool, AI or otherwise, gets special treatment in the core.
- **Git native.** Viewing changed files and diffs should be natural, not an afterthought.
- **Not an editor.** Kegon is a place to operate tools, review their results, and manage them with Git, not a place to write code by hand.
- **Native and lightweight.** Written fully in Rust as a native desktop application. No WebView, Electron, or Tauri.
- **Grow incrementally.** Start small and solid, and add features one step at a time instead of building a large IDE up front.

## Non-goals

To keep the scope focused, Kegon intentionally does **not** aim to be:

- **A code editor.** File viewing and diffs are in scope; a full editing experience is not a core feature.
- **An AI agent IDE.** Kegon does not integrate with or depend on any specific AI tool. Agents are just commands in a terminal.
- **A web-based application.** Kegon is not built on WebView, Electron, or Tauri.
- **A replacement for Git itself.** The goal is to make reviewing and managing changes convenient, not to hide or reimplement Git.
- **A large, all-in-one IDE from day one.** Features are added only once the terminal core works well.

## Current status

Kegon is **experimental and under active development**. It runs day to day, but expect rough edges and breaking changes.

### What works today

- A native desktop window built with [iced](https://github.com/iced-rs/iced), laid out as a workbench: Activity Bar, Side Bar, and a terminal tab strip.
- A real terminal session: the default shell runs in a PTY (ConPTY on Windows), with emulation provided by [alacritty_terminal](https://github.com/alacritty/alacritty). On Windows, PowerShell 7 (`pwsh`) is preferred, falling back to Windows PowerShell and then `cmd.exe`.
- Terminal rendering with font fallback, cursor, text selection, and scrollback.
- Keyboard input, including Japanese IME composition.
- Copy and paste with the system clipboard (bracketed paste aware).
- The terminal grid and the PTY follow window resizes.
- Replies to terminal queries from programs (for example cursor position and device attributes).
- Window and tab titles set by the running program.
- Settings persisted to a TOML file in the OS configuration directory: UI language, UI font, terminal font, and theme (one built-in dark theme so far).
- English (`en-US`) and Japanese (`ja-JP`) UI; by default it follows the OS language.

### Not yet implemented

- Multiple terminal tabs: one terminal session is shown today.
- File Explorer, Search, and Git views: their Side Bar panels are placeholders.
- Terminal profiles and session/workspace management.

### Known limitations

- Windows is the only platform tested so far. The code avoids Windows-only assumptions where it can, but Linux and macOS are **not currently tested**.
- Some terminal interactions are still being hardened, for example selection and copy usability and compatibility with some TUI applications' key handling.

Design notes for the implemented parts live in [docs/architecture](docs/architecture).

## Building and running

Requirements:

- Rust **1.88** or later (the `rust-version` in `Cargo.toml`).
- On Windows with the MSVC toolchain, `rc.exe` from the Windows SDK, which the build uses to embed the application icon (installed with the usual Visual Studio Build Tools setup).

```sh
cargo build --release
cargo run
```

To override the UI language:

```sh
cargo run -- --locale ja-JP
cargo run -- --locale en-US
```

## License

Kegon is licensed under the [MIT License](LICENSE).

Kegon depends on and bundles third-party components that come with their own licenses, including the [Codicons](assets/icons/codicons/README.md) icons (CC BY 4.0). See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

To report a security issue, see [SECURITY.md](SECURITY.md).

## About the name

Kegon is named after [Kegon Falls](https://en.wikipedia.org/wiki/Kegon_Falls) (華厳の滝), a waterfall in Nikko, Japan.

The image is of text pouring down a terminal without pause, like water over a fall.
