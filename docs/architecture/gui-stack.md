# GUI stack selection

Status: accepted for the PoC (Issue #1)
Date: 2026-10-07

## Decision

Kegon uses **[iced](https://github.com/iced-rs/iced) 0.14** as its GUI toolkit.

This decision covers the GUI layer only. Terminal emulation, PTY, and Git crates
are intentionally left open for later issues.

## Requirements

What Kegon needs from a GUI stack, roughly in order of importance:

1. Application code fully in Rust, with no WebView, Electron, or Tauri.
2. Stable as a Windows desktop application. Not tied to Windows.
3. A credible path to a terminal renderer: custom painting of a large grid of
   glyphs that redraws often.
4. Correct Unicode text, including Japanese, with system font fallback, and IME
   input.
5. A VS Code-style workbench: Activity Bar, Side Bar, resizable panels, tabs.
6. Keyboard input, focus management, and mouse interaction.
7. DPI scaling.
8. Long-term maintainability: an architecture that keeps state apart from
   rendering, and a project that is likely to stay maintained.
9. Reasonable startup time and memory footprint.

## Candidates

Versions and release dates are from crates.io as of 2026-10-07.

| | iced | egui / eframe | GPUI | Slint |
|---|---|---|---|---|
| Latest release | 0.14.0 (2025-12) | 0.36.2 (2026-09) | 0.2.2 (2025-10) | 1.18.1 (2026-09) |
| Model | Retained, Elm architecture | Immediate mode | Retained, hybrid; built for Zed | Declarative `.slint` markup plus Rust |
| Custom painting | `canvas`, custom widgets, custom `shader` | `Painter`, very direct | Full control; Zed renders its terminal with it | Limited: `Path` elements, or render into an image |
| Text / CJK | cosmic-text with system font fallback; Japanese works out of the box | Bundled fonts do not cover Japanese; a CJK font has to be loaded by the app | Platform text stacks (DirectWrite on Windows) | Good; system fonts |
| IME | Supported since 0.14 | Supported | Supported (Zed) | Supported |
| Resizable panels / tabs | `pane_grid`, or a few lines of custom code | Built-in resizable panels, plus `egui_dock` / `egui_tiles` | Built by hand, with Zed as the reference | Built by hand |
| Terminal precedent | `iced_term` (alacritty_terminal); COSMIC Terminal on the libcosmic fork of iced | `egui_term` 0.1 | Zed's integrated terminal | None known |
| Main risk | Slow release cadence; pre-1.0 API churn | Text quality and CJK handling; immediate mode at workbench scale | Developed inside the Zed monorepo; crates.io releases lag; thin docs | Weak custom painting for a terminal grid; license choice |

Also considered and rejected early:

- **Floem** (last release 2024-11) and **Xilem** (0.4, small user base) are
  promising but too early to build on.
- **gtk4-rs / relm4** depend on the GTK C libraries, which works against the
  "fully Rust" goal and makes Windows distribution harder.
- **Dioxus, Tauri, and other WebView-based options** are excluded by
  requirement.
- **Raw winit + wgpu** would give the most control, but text layout,
  accessibility, widgets, and input handling would all have to be built from
  scratch. That is too much to take on before Kegon has a working terminal.

## Rationale

**iced** fits Kegon best overall:

- **State is separated from the view by design.** The Elm architecture
  (`State`, `Message`, `update`, `view`) is what Issue #1 asks for: the
  workbench model can be unit tested without a window, and terminal sessions
  can later be added as state driven by messages. Kegon keeps this model in
  `src/workbench.rs`, which does not depend on iced.
- **The text stack is good for Kegon's users.** cosmic-text provides shaping
  and system font fallback, so Japanese renders without bundling fonts. With
  egui this would be the app's job.
- **A terminal on iced has been done.** `iced_term` and COSMIC Terminal both
  pair the same text stack with `alacritty_terminal`, so the path to a terminal
  renderer is known.
- **Custom painting has room to grow.** It ranges from `canvas` to custom
  widgets and raw `shader` primitives, if a terminal grid needs a dedicated
  GPU pipeline. An early version of this PoC drew the Activity Bar icons with
  `canvas` paths. They are now monochrome SVGs, which the `svg` widget recolors
  per state, so `canvas` is not currently a dependency.
- **Rendering is cross-platform**, using wgpu, with a pure-CPU tiny-skia
  fallback. No Windows-specific code was needed for this PoC.

Why the others were not chosen:

- **egui** is the most mature and most actively released option, and it was a
  close second. It was not chosen because of Japanese text: no system font
  fallback by default, and simpler text layout. That matters for a tool whose
  main surface is text. Immediate mode also tends to mix state and drawing
  code, which works against the separation this project wants.
- **GPUI** is technically the closest match, since Zed is a workbench with a
  terminal. However, its development is driven by Zed's own needs, crates.io
  releases lag the monorepo, and documentation is limited. Building on it
  would be a bet on a moving target.
- **Slint** is polished and well maintained. However, its custom painting
  model is a poor fit for a high-throughput terminal grid, and its UI lives in
  a separate markup language.

## PoC findings

Measured on Windows 11 with an i7-13700F and an RTX 3060, using a release
build, at 100% display scaling.

| Renderer | Time to window handle | Working set | Private bytes |
|---|---|---|---|
| wgpu (default adapter selection) | ~30 ms | ~180 MB | ~217 MB |
| wgpu with `WGPU_BACKEND=dx12` | ~20–90 ms | ~100–112 MB | ~122 MB |
| tiny-skia (`ICED_BACKEND=tiny-skia`) | ~30 ms | ~25 MB | ~10 MB |

- Startup is fast with every backend.
- Most of the memory used with wgpu belongs to the GPU driver, not to Kegon.
  The tiny-skia numbers show the application itself is small. Choosing the
  default backend is left for when the terminal renderer exists and its
  performance can be measured.
- Verified to work: the window launches and resizes, and dragging its border
  stops at the configured 640×400 minimum (a logical client size, applied by
  winit); Activity Bar items can be
  selected by mouse and by Ctrl+Shift+E/F/G; the SVG icons switch between
  their normal, hovered, and selected colors; the Side Bar placeholder follows
  the selection; the Side Bar can be resized by dragging; Japanese text
  renders. These checks used synthetic input. In one of four automated runs a
  sash drag did not register. This was not reproduced and the cause is not
  known.
- Release binary size is about 13 MB with SVG support (about 11 MB without).

## Open risks and follow-ups

- **IME** has not been exercised yet, because the PoC has no text input. It
  has to be verified with the first terminal session.
- **Focus management.** iced has widget operations for focus, but no
  workbench-level focus model. Kegon will need its own concept of which pane
  has keyboard focus (terminal, Side Bar, tab strip), especially to decide
  which key presses go to the terminal.
- **Font selection.** Japanese text falls back per glyph, so kanji and kana
  may come from different fonts. Kegon should choose its UI and terminal fonts
  explicitly.
- **DPI scaling** above 100% has not been verified yet.
- **Release cadence.** iced 0.14 is the latest release, and there has been
  none on crates.io for about ten months. Keeping the workbench model
  independent of iced limits the cost of switching or forking later.
