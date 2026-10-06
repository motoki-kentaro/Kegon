# Kegon

**A native, Git-native terminal workspace, written in Rust.**

> Kegon is not another code editor.
>
> It is a native terminal workspace built around repositories, shells, CLI tools, and Git.

---

## What is Kegon?

Kegon is a desktop application that puts the **terminal** at the center of the workspace, not a code editor.

The main area is a set of terminal tabs. Each tab runs its own process: PowerShell, MSYS2, WSL, SSH, Claude Code, Codex CLI, Gemini CLI, or any other command-line tool you use. Around those terminals, Kegon intends to provide just enough GUI to browse the repository, search it, and review changes through Git.

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

## Planned initial features

None of the following is implemented yet. This section describes the intended direction.

### First milestones

The first technical goals are deliberately narrow:

1. Open a native Rust window.
2. Display a single terminal.
3. Operate PowerShell (and similar shells) correctly through a PTY.
4. Create and switch between multiple terminal tabs.
5. Run TUI applications such as Claude Code correctly.

### Planned layout

- **Main area:** multi-tab terminal, with each tab backed by an independent PTY / process session.
- **Activity Bar** (left, VS Code-style), with the initial items, from top to bottom:
  1. File Explorer
  2. Search
  3. Git
- **Side Bar:** the panel for the selected Activity Bar item.

### Later

After the terminal foundation is stable, the plan is to add, step by step:

- File Explorer
- Search across the workspace
- Git status and diff views
- Terminal profiles (configurable shells and CLI tools to launch)
- Session and workspace management

## Non-goals

To keep the scope focused, Kegon intentionally does **not** aim to be:

- **A code editor.** File viewing and diffs are in scope; a full editing experience is not a core feature.
- **An AI agent IDE.** Kegon does not integrate with or depend on any specific AI tool. Agents are just commands in a terminal.
- **A web-based application.** Kegon is not built on WebView, Electron, or Tauri.
- **A replacement for Git itself.** The goal is to make reviewing and managing changes convenient, not to hide or reimplement Git.
- **A large, all-in-one IDE from day one.** Features are added only once the terminal core works well.

## Current status

Kegon is at the **very beginning** of development.

- A proof-of-concept window exists. It shows the workbench layout (Activity Bar, Side Bar, terminal tab strip), but only as placeholders. No terminal, File Explorer, Search, or Git functionality is implemented yet.
- The GUI toolkit has been chosen for the proof of concept: [iced](https://github.com/iced-rs/iced). See [docs/architecture/gui-stack.md](docs/architecture/gui-stack.md) for the reasoning.
- The terminal emulation, PTY, and Git libraries are **not yet decided**.
- The design described in this README may change as development progresses.

This README serves as a statement of intent for the project.

### Running the proof of concept

Requires a recent stable Rust toolchain.

```sh
cargo run
```

## About the name

Kegon is named after [Kegon Falls](https://en.wikipedia.org/wiki/Kegon_Falls) (華厳の滝), a waterfall in Nikko, Japan.

The image is of text pouring down a terminal without pause, like water over a fall.
