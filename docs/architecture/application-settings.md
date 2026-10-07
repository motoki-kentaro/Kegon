# Application Settings Foundation

Status: accepted (Issue #13)
Date: 2026-10-07

Kegon provides a typed, persistent application settings foundation with runtime localization switching and an Activity Bar Settings entry point.

## Scope

The initial scope of `ApplicationSettings` is deliberately limited to **Application Language**.
Future settings (Themes, Fonts, Keybindings) will extend this architecture without breaking changes or migration friction.

Non-goals for this issue:
- Theme settings and Theme pickers (light/dark/system)
- UI Font setting and Terminal Font setting
- Font Picker, installed font enumeration, font preview, or font file bundling
- Terminal geometry change settings
- Keybinding editor or overrides
- Project-level settings
- Settings search or category tree

## Data Model & Types

`ApplicationSettings` is a strongly-typed Rust struct owned by the application layer. Dynamic string-based key-value maps are explicitly avoided.

```rust
pub enum LocalePreference {
    System, // "system" (default)
    EnUs,   // "en-US"
    JaJp,   // "ja-JP"
}

pub struct ApplicationSettings {
    pub locale: LocalePreference,
}
```

- **Default Preference**: `LocalePreference::System`. On missing configuration, Kegon resolves locale from the OS preferred UI languages, falling back to `en-US`.
- **TOML Identifiers**: Saved in TOML using stable lower/kebab-case strings (`"system"`, `"en-US"`, `"ja-JP"`). Localized display text is never saved to `settings.toml`.

## Configuration Path Policy

Application settings are stored per-user in the operating system's standard user configuration directory:

```text
<config_dir>/Kegon/settings.toml
```

Resolved paths per platform:
- **Windows**: `%APPDATA%\Kegon\settings.toml` (e.g. `C:\Users\<user>\AppData\Roaming\Kegon\settings.toml`)
- **macOS**: `$HOME/Library/Application Support/Kegon/settings.toml`
- **Linux / POSIX**: `$XDG_CONFIG_HOME/Kegon/settings.toml` or `$HOME/.config/Kegon/settings.toml`

The path resolution is abstracted via the `dirs` crate. Hard-coded Windows-only or OS-specific paths are strictly forbidden in application code.

## File Loading & Diagnostic Policy

Startup configuration loading follows defensive, zero-panic principles:

1. **Missing File**: Returns default `ApplicationSettings` (`System`). No empty file is automatically created at startup.
2. **Valid File**: Parses TOML cleanly into `ApplicationSettings`.
3. **Malformed / Invalid File**: Returns default `ApplicationSettings` (`System`), logs a diagnostic warning message to `stderr`, and **never** overwrites or deletes the user's broken source file on startup.

## Persistence & Save Policy

Settings persistence is managed centrally by `save_settings`:

- **Atomic Replacement**: Serializes settings to TOML, writes to `settings.toml.tmp` in the target directory, and replaces `settings.toml` using atomic filesystem rename.
- **Sequence**: Settings are saved to disk first. Upon successful persistence (or clean handling), the runtime state is updated.
- **Error Handling**: Save failures log an error to `stderr` and do not panic or crash the application.

## Locale Resolution Precedence

Application UI locale resolution follows strict, unambiguous precedence:

```text
Command Line (--locale)
    ├─ Specified → Process-level CLI Override (Wins)
    └─ Unspecified
          ↓
   ApplicationSettings (settings.toml)
    ├─ en-US → en-US
    ├─ ja-JP → ja-JP
    └─ system
          ↓
   OS Preferred UI Language
          ↓
        en-US (Fallback)
```

1. **CLI Override (`--locale`)**: Takes top precedence for the current running process. It never modifies `settings.toml` or persisted preferences.
2. **Persisted Preference**: Used when no `--locale` CLI flag is present.
3. **OS Preferred Language**: Used when preference is `system`.
4. **`en-US` Fallback**: Canonical fallback.

## CLI Override in Settings UI

When Kegon is started with `--locale`:
- The process operates under process-level override.
- In the Settings UI, the Application Language picker is **disabled** to prevent ambiguous UI states.
- A localized notice is displayed in the Settings panel:
  - `en-US`: *"Application language is overridden by --locale for this session."*
  - `ja-JP`: *"このセッションでは --locale により言語が上書きされています。"*

## Runtime Language Switching

When a user selects a new language in the Settings UI (without `--locale` override):
1. `save_settings` persists the new `LocalePreference` to `settings.toml`.
2. `Kegon` re-resolves the effective locale and replaces `self.localizer = Localizer::new(new_locale)`.
3. The UI re-renders immediately with localized strings in the newly selected language.
4. **Terminal Session Preservation**: The `TerminalSession` child process, shell state, PTY, and scrollback history are preserved intact and are never restarted or recreated.

## UI Surface & Activity Bar Layout

The Activity Bar layout separates primary activities from global bottom actions:

```text
Activity Bar
├─ File Explorer
├─ Search
├─ Source Control
│
│  [flexible vertical space]
│
└─ ⚙ Settings (bottom action)
```

- **Asset**: Uses `assets/icons/codicons/activity-bar/settings-gear.svg`.
- **Command**: Activity Bar gear click dispatches stable `CommandId::WorkbenchSettingsOpen` (`workbench.settings.open`).
- **Minimum Window Layout**: At the minimum logical client size of 640x400, top activity icons and bottom gear remain separated by flexible space without overlap.
- **Settings Surface**: Clicking Settings displays the Settings panel in the Side Bar area while preserving the main Terminal view. Clicking primary activities (Explorer, Search, Source Control) returns to standard Side Bar views.

## Future Extensions

The `ApplicationSettings` structure and UI surface are designed to accommodate future configuration domains:
- **Theme Settings**: `theme` field (`System`, `Dark`, `Light`, custom themes).
- **UI Font & Terminal Font Settings**: Font family, font size, line height, and font weight configuration.
- **Font Picker**: Installed font enumeration, preview, and selection UI.
- **Keybinding Overrides**: User-customizable keybinding shortcuts in `settings.toml`.
