# Kegon UI strings: en-US (canonical and fallback locale).
#
# Every message used by the UI must exist here. Message IDs are referenced
# from Rust through `MessageKey` in src/i18n/key.rs; see
# docs/architecture/i18n.md for conventions.

## Activity Bar (tooltips)

activity-explorer = File Explorer
activity-search = Search
activity-git = Git
activity-settings = Settings

## Side Bar

side-bar-explorer-title = EXPLORER
side-bar-explorer-placeholder = File Explorer is not implemented yet.
side-bar-search-title = SEARCH
side-bar-search-placeholder = Project-wide search is not implemented yet.
side-bar-git-title = SOURCE CONTROL
side-bar-git-placeholder = Git integration is not implemented yet.
settings-language-label = Application Language
settings-language-system = System default
settings-cli-override-note = Application language is overridden by --locale for this session.
settings-theme-label = Theme
settings-ui-font-label = UI Font
settings-ui-font-system = System default
settings-ui-font-choose = Choose...
settings-ui-font-reset = Reset
settings-ui-font-not-installed = { $family } (not installed)
settings-terminal-font-label = Terminal Font
settings-terminal-font-default = Default
settings-terminal-font-choose = Choose...
settings-terminal-font-reset = Reset
settings-terminal-font-not-installed = { $family } (not installed)


## Terminal tab strip

terminal-tab-default-title = Terminal
terminal-new-tab-tooltip = New Terminal
terminal-close-tab-tooltip = Close Terminal
terminal-process-exited = Process exited with code { $code }.
terminal-tooltip-shell-integration = Shell Integration
terminal-tooltip-status-active = Active
terminal-tooltip-status-active-prompt-only = Active (Prompt only)
terminal-tooltip-status-waiting = Waiting
terminal-tooltip-status-disabled = Disabled
terminal-tooltip-status-unsupported = Unsupported
terminal-tooltip-status-unavailable = Unavailable
terminal-tooltip-command-state = Command State
terminal-tooltip-command-state-unknown = Unknown
terminal-tooltip-command-state-prompt = Prompt
terminal-tooltip-command-state-editing = Editing
terminal-tooltip-command-state-executing = Executing
terminal-tooltip-command-state-finished = Finished
terminal-tooltip-shell-reported-status = Shell-reported status
terminal-tooltip-status-success = Success
terminal-tooltip-status-failure = Failure
terminal-tooltip-status-unknown = Unknown
terminal-tooltip-osc-exit-code = OSC-reported exit code
terminal-tooltip-exit-code-none = —

## Settings tab

settings-tab-title = Settings
settings-tab-description = Changes are applied immediately and saved automatically.
settings-category-general-title = General
settings-category-general-description = Settings that apply to the whole application.
settings-category-appearance-title = Appearance
settings-category-appearance-description = Theme and the font used by the interface.
settings-category-terminal-title = Terminal
settings-category-terminal-description = How terminal sessions are displayed.
settings-font-size-label = Font size
settings-font-size-description = Size of the terminal text, in points. Cell width and height follow it unless set below.
settings-font-size-default = 9.75 pt (13 px)
settings-cell-height-label = Line height
settings-cell-height-description = Cell height as a multiple of the font size (0.5 to 3.0). Unset follows the font.
settings-cell-height-default = follows the font
settings-cell-width-label = Cell width
settings-cell-width-description = Cell width as a multiple of the font size (0.2 to 2.0). Unset follows the font.
settings-cell-width-default = follows the font
settings-metric-current = Current: { $value }
settings-metric-default-value = Default: { $value }
settings-metric-pending = Not applied
settings-metric-apply = Apply
settings-metric-reset = Reset
settings-metric-range-error = Enter a number from { $min } to { $max }.
settings-metric-stored-invalid = The value in settings.toml ({ $value }) is not valid and is ignored. Applying a new value replaces it.
settings-builtin-glyphs-label = Built-in Glyphs
settings-builtin-glyphs-description = When on, Unicode box-drawing and block characters are drawn as dedicated shapes instead of with the font, so they join the cell grid without gaps. Applies at once.
settings-builtin-glyphs-stored-invalid = The value in settings.toml ({ $value }) is not true or false and is treated as on. Changing the setting replaces it.
settings-shell-mode-label = Shell
settings-shell-mode-description = Choose which shell to launch in terminal sessions.
settings-shell-mode-auto = Auto-detect
settings-shell-mode-custom = Specify executable
settings-shell-path-label = Executable path
settings-shell-path-placeholder = C:\Program Files\PowerShell\7\pwsh.exe
settings-shell-integration-label = Shell Integration
settings-shell-integration-description = When set to Auto, detects command execution in supported shells (PowerShell). When disabled, shell integration is turned off.
settings-shell-integration-auto = Auto
settings-shell-integration-disabled = Disabled
settings-shell-new-tab-note = Changes apply to newly opened terminal tabs.
settings-shell-path-empty = Path cannot be empty.
settings-shell-path-not-absolute = Path must be an absolute path (e.g. C:\...).
settings-shell-path-not-found = The specified file does not exist.
settings-shell-path-is-directory = A directory was specified instead of an executable file.
settings-shell-path-not-executable = The file must be an executable (.exe).
settings-shell-path-has-arguments = Arguments cannot be included; specify the executable path only.
settings-tab-close-tooltip = Close Settings

## Main area

main-placeholder-title = Terminal (placeholder)
main-placeholder-body = Terminal sessions are not implemented yet.

## Confirmation Dialog

dialog-smoke-question-title = Smoke Test Question
dialog-smoke-question-message = Do you want to proceed with this test operation?
dialog-smoke-warning-title = Smoke Test Warning
dialog-smoke-warning-message = Warning: This test operation will perform a simulated destructive action.

dialog-action-cancel = Cancel
dialog-action-continue = Continue
dialog-action-ok = OK
dialog-action-close = Close
dialog-action-delete = Delete
dialog-action-exit = Exit

## Font Picker

font-picker-title-ui = UI Font
font-picker-title-terminal = Terminal Font
font-picker-search-placeholder = Search fonts...
font-picker-monospace-only = Monospaced fonts only
font-picker-preview-header = Preview
font-picker-no-matching-fonts = No matching fonts
font-picker-cancel = Cancel
font-picker-select = Select


