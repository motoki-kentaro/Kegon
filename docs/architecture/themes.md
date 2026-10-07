# Themes

Status: accepted (Issue #19)
Date: 2026-10-07

Kegon's colors come from a typed theme. Kegon ships one built-in theme, **Night Dark**.

## Ownership

A `KegonTheme` owns every color that Kegon itself chooses:

- the Activity Bar, Side Bar, tab strip, and workbench background
- buttons, drop-downs, tooltips, and the sash
- the Confirmation Dialog and Font Picker
- the terminal's default colors, cursor, selection, IME composition, and ANSI palette

A theme does not own colors that come from content:

- explicit RGB colors sent by a terminal application (`ESC[38;2;r;g;bm`)
- the generated 256-color cube (indices 16–231) and grayscale ramp (232–255)
- anything inside font data or user files

## Structure

```text
src/theme/
├─ mod.rs         public API
├─ model.rs       ThemeId, KegonTheme, and the token structs
├─ night_dark.rs  the canonical Night Dark definition (the only place its values appear)
├─ registry.rs    resolve_theme(ThemeId) -> &'static KegonTheme
└─ style.rs       widget styles derived from a theme (buttons, modal surface, backdrop)
```

```rust
pub struct KegonTheme {
    pub id: ThemeId,
    pub workbench: WorkbenchColors,
    pub interaction: InteractionColors,
    pub text: TextColors,
    pub icons: IconColors,
    pub semantic: SemanticColors,
    pub terminal: TerminalColors,
}
```

Every token is a named struct field rather than a string-keyed map. A theme that omits a token fails to compile.

## ThemeId

```rust
pub enum ThemeId {
    NightDark,
}
```

| | Night Dark |
|---|---|
| Stable ID (persisted, never localized) | `night-dark` |
| Display name (product name, shown as-is in every locale) | `Night Dark` |

`ThemeId::DEFAULT` is `NightDark`.

## Night Dark tokens

Token names below are the semantic names; the Rust field is in parentheses.

### Workbench and surfaces (`workbench`)

| Token | Color | Use |
|---|---|---|
| `workbench.background` (`background`) | `#1E1E1E` | Main workbench content, the active tab, inset fields |
| `activity_bar.background` (`activity_bar_background`) | `#2C2C2C` | Activity Bar |
| `side_bar.background` (`side_bar_background`) | `#252526` | Side Bar, sash |
| `tab_bar.background` (`tab_bar_background`) | `#252526` | Tab strip |
| `surface.background` (`surface_background`) | `#252526` | Dialog panels |
| `surface.elevated` (`surface_elevated`) | `#2D2D30` | Tooltips, secondary buttons, drop-down menus |
| `surface.backdrop` (`backdrop`) | `#000000` at 65% | Scrim behind a modal |
| `border.default` (`border`) | `#3C3C3C` | Borders and dividers |

### Interaction (`interaction`)

| Token | Color | Use |
|---|---|---|
| `interaction.hover` (`hover`) | `#37373D` | Hovered tabs and buttons, the highlighted Font Picker candidate |
| `interaction.selection` (`selection`) | `#264F78` | Selected text in inputs, the selected drop-down item |
| `accent.primary` (`accent`) | `#007ACC` | Focus rings, the active sash, primary actions |

### Text (`text`)

| Token | Color | Use |
|---|---|---|
| `text.primary` (`primary`) | `#CCCCCC` | Body text, labels, headings |
| `text.muted` (`muted`) | `#858585` | Secondary text, inactive tabs, placeholders |
| `text.disabled` (`disabled`) | `#666666` | Disabled buttons |
| `text.emphasis` (`emphasis`) | `#FFFFFF` | Dialog titles, the active tab, secondary button labels, font names |
| `text.on_accent` (`on_accent`) | `#FFFFFF` | Labels and focus rings on accent or destructive fills |

### Icons (`icons`)

| Token | Color | Use |
|---|---|---|
| `icon.active` (`active`) | `#FFFFFF` | Selected Activity Bar icon and its indicator, Question dialog icon |
| `icon.hovered` (`hovered`) | `#CCCCCC` | Hovered, unselected Activity Bar icon |
| `icon.inactive` (`inactive`) | `#858585` | Unselected Activity Bar icon |

### Semantic status (`semantic`)

| Token | Color | Use |
|---|---|---|
| `semantic.warning` (`warning`) | `#D7BA7D` | Warning dialog icon |
| `semantic.destructive` (`destructive`) | `#D73A49` | Destructive actions, failure messages (process exited) |
| `semantic.success` (`success`) | `#4EC9B0` | Reserved; no UI uses it yet |

### Terminal (`terminal`)

| Token | Color |
|---|---|
| `terminal.background` | `#1E1E1E` |
| `terminal.foreground` | `#CCCCCC` |
| `terminal.cursor` | `#AEAFAD` |
| `terminal.selection` | `#264F78`, drawn at 60% opacity by the renderer |
| `terminal.preedit_background` | `#3A3D41` |
| `terminal.preedit_foreground` | `#FFFFFF` (IME text and underline) |

### ANSI 16 colors (`terminal.ansi`)

| | Normal | Bright |
|---|---|---|
| Black | `#1E1E1E` | `#666666` |
| Red | `#CD3131` | `#F14C4C` |
| Green | `#0DBC79` | `#23D18B` |
| Yellow | `#E5E510` | `#F5F543` |
| Blue | `#2472C8` | `#3B8EE8` |
| Magenta | `#BC3FBC` | `#D670D6` |
| Cyan | `#11A8CD` | `#29B8DB` |
| White | `#E5E5E5` | `#FFFFFF` |

### Tokens beyond the v1 schema

Issue #19 defined the schema above, except for these tokens. Each one covers an existing Kegon color that no schema token could represent:

| Token | Why |
|---|---|
| `surface.backdrop` | The modal scrim is a translucent black overlay. No surface token is translucent, and a surface color at an arbitrary opacity would change the dimming. |
| `text.emphasis` | Titles, the active tab, and font names were already white, distinct from `#CCCCCC` body text. Mapping them to `text.primary` would flatten that hierarchy. Reusing `icon.active` would tie text to an icon role. |
| `text.on_accent` | Labels on blue and red fills need a color that contrasts with the fill. `text.primary` (`#CCCCCC`) on `#007ACC` drops to about 2.8:1. A future light theme will want `emphasis` dark but `on_accent` still light, so the two are separate. |
| `icon.hovered` | The hovered Activity Bar icon (`#CCCCCC`) is an icon state between `active` and `inactive`. Reusing `text.primary` would couple an icon state to body text in future themes. |
| `terminal.preedit_background`, `terminal.preedit_foreground` | IME composition is drawn on its own background (`#3A3D41`), which matches no schema color. Mapping it to `interaction.hover` would visibly change IME rendering. |

### Derived colors

Some colors are computed from tokens instead of being stored:

| Color | Derivation |
|---|---|
| Filled button hover/pressed | the fill × 0.8 (`style::hover_shade`); reproduces the previous `#0062A3` exactly |
| Terminal selection | `terminal.selection` at 60% opacity |
| Process-exited banner background | `semantic.destructive` blended 10% into `workbench.background` |

## Registry and active theme

```text
settings.appearance.theme (ThemePreference)
        ↓ effective_id()
ThemeId
        ↓ resolve_theme()
&'static KegonTheme  (Kegon::active_theme)
        ↓
Workbench views · Dialog / Font Picker · TerminalProgram { colors: &theme.terminal }
```

- `Kegon` holds the active theme as `&'static KegonTheme` and resolves it in one place: `Kegon::new` and `apply_theme_preference`.
- Views receive the theme or the token group they need. No widget reads `settings.toml` or a `ThemeId`.
- The terminal renderer receives `&TerminalColors` once per frame. It never resolves a theme itself, and no lookup happens per cell.
- Changing the theme in Settings saves the setting, resolves the new theme, and re-renders. Neither the window nor the terminal session is recreated.

## Settings integration

```toml
locale = "ja-JP"

[appearance]
theme = "night-dark"
ui_font_family = "Yu Gothic UI"
```

- A missing `theme` means Night Dark. Files written before themes existed load unchanged.
- **Unknown IDs** such as `theme = "future-theme"` are kept as `ThemePreference::Unknown`:
  - Kegon uses Night Dark.
  - It prints `kegon: unknown theme "future-theme" in settings; using night-dark (the setting is kept as is)` to stderr.
  - The rest of the file still applies.
  - The ID is written back unchanged when other settings are saved.
  - Only choosing a theme in Settings replaces it.
- A non-string value, such as `theme = 5`, is a malformed file and follows the Application Settings malformed-file policy.

Settings > Appearance shows a **Theme** drop-down listing `ThemeId::ALL` by display name. The label is localized (`Theme` / `テーマ`). Theme names are product names and are not translated.

## Independence

Theme, UI font, and locale are separate settings:

- Changing the theme does not reset the UI font, reload the font catalog, or change the locale.
- Changing the UI font or locale does not change the theme.
- Each change saves the whole settings model, so all three are always preserved together.

## Relationship to iced::Theme

Kegon still runs on an `iced::Theme`; the application's theme function returns `KegonTheme::iced_theme()`, which is `iced::Theme::Dark` for Night Dark. That theme only affects widgets Kegon leaves at their default style, currently scrollbars.

Everything Kegon draws is styled explicitly from `KegonTheme`. This does not replace iced's theming system.

## Adding a built-in theme

1. Add a variant to `ThemeId`, with its stable ID in `as_str` and its product name in `display_name`. Add it to `ThemeId::ALL`.
2. Define one complete `KegonTheme` in a new file next to `night_dark.rs`. The compiler rejects a theme with a missing token.
3. Return it from `resolve_theme`, and map it to an `iced::Theme` in `KegonTheme::iced_theme`. Both matches are exhaustive.
4. The Settings drop-down lists `ThemeId::ALL`, so the theme appears automatically.
5. Add tests for its stable ID, its tokens, and its ANSI palette.

No widget code changes.

## Known limitations

- Scrollbars use iced's built-in Dark style rather than Kegon tokens.
- Night Dark is the only theme. Runtime switching is exercised by tests and the Settings drop-down, but there is nothing to switch to yet.
- Contrast was checked informally against the existing appearance. No formal WCAG audit has been done.
