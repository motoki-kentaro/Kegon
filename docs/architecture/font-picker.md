# System Font Picker Architecture

## 1. Responsibilities

The System Font Picker component provides a reusable native iced modal UI for discovering, filtering, previewing, and selecting installed font families on the host operating system.

It is designed to serve both UI font selection and Terminal font selection through a single unified component driven by `FontPickerMode`:

- `FontPickerMode::Ui`: Lists all reasonable installed font families and renders mixed Latin, numeric, and CJK text.
- `FontPickerMode::Terminal`: Enables monospaced font filtering by default and renders terminal-oriented preview text (including ASCII, CJK, box-drawing characters, and emojis).

The Font Picker does **not** persist selections to `settings.toml` or apply fonts to active terminal sessions or UI elements directly. It returns a `FontPickerResult` (`Select(FontCandidate)` or `Cancel`) to the caller.

---

## 2. System Font Enumeration & Dependencies

Font discovery and font metadata inspection are decoupled from the view layer:

- **`fontdb` (v0.23)**: Scans platform system font directories (`load_system_fonts()`) on Windows, macOS, and Linux without external C/C++ build dependencies.
- **`ttf-parser` (v0.25)**: Parses underlying font file headers (specifically the OpenType `post` table `isFixedPitch` metadata flag) to perform accurate monospace classification.

### Dependency Licenses

- `fontdb`: MIT (compatible with Kegon MIT)
- `ttf-parser`: MIT / Apache-2.0 (compatible with Kegon MIT)

---

## 3. Family Normalization & Representative Face Policy

OS font enumeration returns individual font faces (e.g. `YuGothM.ttc`, `YuGothB.ttc`, `CascadiaMono-Bold.ttf`). The catalog normalizes and collapses these faces at the **family level**:

1. All faces sharing a primary `family_name` are grouped into a single `FontCandidate`.
2. A deterministic **representative face** is selected for each family:
   - Prefer faces with `Style::Normal` and `Weight::NORMAL` (weight 400).
   - If no Regular face exists, select the face closest to normal style and weight (avoiding bold or italic faces as primary family representatives).
3. Family candidates are sorted **alphabetically and case-insensitively** by `family_name`, with stable tie-breaking to ensure deterministic ordering across OS enumeration order differences.

---

## 4. Monospace Classification

Monospace classification is determined using defensible OpenType metadata:

- Primary detector: `ttf_parser::Face::parse(&font_bytes, index).map(|face| face.is_monospaced())` checking the standard `post` table `isFixedPitch` flag.
- Secondary detector: `fontdb::FaceInfo::monospaced` metadata.

Classification does **not** rely on family names containing substring matches like `"Mono"` or `"Console"`.

---

## 5. Mode Behavior, Search & Filtering

- **UI Mode (`FontPickerMode::Ui`)**:
  - Shows all installed font families.
  - Substring search query filter (case-insensitive, works for Latin and CJK font family names).
- **Terminal Mode (`FontPickerMode::Terminal`)**:
  - Includes a `[✓] Monospaced fonts only` checkbox toggle (default: `ON`).
  - Search query AND monospace filter are combined deterministically (`query AND (if monospace_only { is_monospace } else { true })`).

---

## 6. Runtime Font Loading & Caching

Font file bytes are loaded lazily on demand when a candidate is highlighted:

- **`FontCache`**: Reads font file bytes once from disk and caches them in memory (`Arc<Vec<u8>>`).
- Corrupt or unreadable font files fail gracefully, log a warning, and avoid repeated disk retries.
- Preview text renders using `iced::Font::with_name(family_name)`.

---

## 7. Modal Presentation & Input Ownership

The Font Picker runs as a modal overlay in the application foreground:

- Background input to Workbench and Terminal sessions is blocked (`InputArbiter` routes all keyboard events to `InputRoute::Modal`).
- Search field supports MS-IME typing (preedit, commit) without double-send or leakage to the background process.
- Keyboard navigation:
  - `Up` / `Down`: Move candidate highlight
  - `Enter`: Confirm selection or trigger focused button
  - `Escape`: Cancel and close picker
  - `Tab` / `Shift+Tab`: Focus traversal (`SearchInput` <-> `MonospaceToggle` <-> `CandidateList` <-> `CancelButton` <-> `SelectButton`)

---

## 8. CLI Smoke Test Entry Points

For dogfooding and visual verification, temporary CLI flags are supported in release builds:

- `kegon.exe --smoke-font-picker=ui`
- `kegon.exe --smoke-font-picker=terminal`

Invalid values (e.g. `--smoke-font-picker=invalid`) print a warning and exit safely without panicking.

---

## 9. Known Limitations & Future Work

- **Weight / Style Picker**: Selecting specific font weights (e.g. Light, Medium, Bold) or italic variants is out of scope for this Issue and will be added when theme font customization is implemented.
- **Terminal Cell Geometry**: Cell width/height calculation for alacritty_terminal remains fixed at default cell metrics until multi-font terminal grid resizing is added in a future Issue.
