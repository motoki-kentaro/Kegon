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

## Settings tab

settings-tab-title = Settings
settings-tab-description = Changes are applied immediately and saved automatically.
settings-category-general-title = General
settings-category-general-description = Settings that apply to the whole application.
settings-category-appearance-title = Appearance
settings-category-appearance-description = Theme and the font used by the interface.
settings-category-terminal-title = Terminal
settings-category-terminal-description = How terminal sessions are displayed.
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


