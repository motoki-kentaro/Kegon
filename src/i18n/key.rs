//! Typed message keys.
//!
//! UI code refers to messages through [`MessageKey`] rather than string IDs,
//! so a misspelled key is a compile error. Tests check that every key exists
//! in every locale and that no message in the resources is left unused.

macro_rules! message_keys {
    ($($variant:ident => $id:literal,)*) => {
        /// A Kegon UI message. See `assets/i18n/en-US.ftl` for the text.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[allow(dead_code)]
        pub enum MessageKey {
            $($variant,)*
        }

        impl MessageKey {
            /// Every key, in declaration order.
            #[cfg(test)]
            pub const ALL: &[Self] = &[$(Self::$variant,)*];

            /// The Fluent message ID.
            pub fn id(self) -> &'static str {
                match self {
                    $(Self::$variant => $id,)*
                }
            }
        }
    };
}

message_keys! {
    ActivityExplorer => "activity-explorer",
    ActivitySearch => "activity-search",
    ActivityGit => "activity-git",
    ActivitySettings => "activity-settings",

    SideBarExplorerTitle => "side-bar-explorer-title",
    SideBarExplorerPlaceholder => "side-bar-explorer-placeholder",
    SideBarSearchTitle => "side-bar-search-title",
    SideBarSearchPlaceholder => "side-bar-search-placeholder",
    SideBarGitTitle => "side-bar-git-title",
    SideBarGitPlaceholder => "side-bar-git-placeholder",
    SettingsLanguageLabel => "settings-language-label",
    SettingsLanguageSystem => "settings-language-system",
    SettingsCliOverrideNote => "settings-cli-override-note",
    SettingsThemeLabel => "settings-theme-label",
    SettingsUiFontLabel => "settings-ui-font-label",
    SettingsUiFontSystem => "settings-ui-font-system",
    SettingsUiFontChoose => "settings-ui-font-choose",
    SettingsUiFontReset => "settings-ui-font-reset",
    SettingsUiFontNotInstalled => "settings-ui-font-not-installed",
    SettingsTerminalFontLabel => "settings-terminal-font-label",
    SettingsTerminalFontDefault => "settings-terminal-font-default",
    SettingsTerminalFontChoose => "settings-terminal-font-choose",
    SettingsTerminalFontReset => "settings-terminal-font-reset",
    SettingsTerminalFontNotInstalled => "settings-terminal-font-not-installed",

    TerminalTabDefaultTitle => "terminal-tab-default-title",
    TerminalNewTabTooltip => "terminal-new-tab-tooltip",
    TerminalCloseTabTooltip => "terminal-close-tab-tooltip",
    TerminalProcessExited => "terminal-process-exited",
    TerminalTooltipShellIntegration => "terminal-tooltip-shell-integration",
    TerminalTooltipStatusActive => "terminal-tooltip-status-active",
    TerminalTooltipStatusActivePromptOnly => "terminal-tooltip-status-active-prompt-only",
    TerminalTooltipStatusWaiting => "terminal-tooltip-status-waiting",
    TerminalTooltipStatusDisabled => "terminal-tooltip-status-disabled",
    TerminalTooltipStatusUnsupported => "terminal-tooltip-status-unsupported",
    TerminalTooltipStatusUnavailable => "terminal-tooltip-status-unavailable",
    TerminalTooltipCommandState => "terminal-tooltip-command-state",
    TerminalTooltipCommandStateUnknown => "terminal-tooltip-command-state-unknown",
    TerminalTooltipCommandStatePrompt => "terminal-tooltip-command-state-prompt",
    TerminalTooltipCommandStateEditing => "terminal-tooltip-command-state-editing",
    TerminalTooltipCommandStateExecuting => "terminal-tooltip-command-state-executing",
    TerminalTooltipCommandStateFinished => "terminal-tooltip-command-state-finished",
    TerminalTooltipLastExitCode => "terminal-tooltip-last-exit-code",
    TerminalTooltipExitCodeUnknown => "terminal-tooltip-exit-code-unknown",
    TerminalTooltipExitCodeNone => "terminal-tooltip-exit-code-none",

    SettingsTabTitle => "settings-tab-title",
    SettingsTabDescription => "settings-tab-description",
    SettingsCategoryGeneralTitle => "settings-category-general-title",
    SettingsCategoryGeneralDescription => "settings-category-general-description",
    SettingsCategoryAppearanceTitle => "settings-category-appearance-title",
    SettingsCategoryAppearanceDescription => "settings-category-appearance-description",
    SettingsCategoryTerminalTitle => "settings-category-terminal-title",
    SettingsCategoryTerminalDescription => "settings-category-terminal-description",
    SettingsFontSizeLabel => "settings-font-size-label",
    SettingsFontSizeDescription => "settings-font-size-description",
    SettingsFontSizeDefault => "settings-font-size-default",
    SettingsCellHeightLabel => "settings-cell-height-label",
    SettingsCellHeightDescription => "settings-cell-height-description",
    SettingsCellHeightDefault => "settings-cell-height-default",
    SettingsCellWidthLabel => "settings-cell-width-label",
    SettingsCellWidthDescription => "settings-cell-width-description",
    SettingsCellWidthDefault => "settings-cell-width-default",
    SettingsMetricCurrent => "settings-metric-current",
    SettingsMetricDefaultValue => "settings-metric-default-value",
    SettingsMetricPending => "settings-metric-pending",
    SettingsMetricApply => "settings-metric-apply",
    SettingsMetricReset => "settings-metric-reset",
    SettingsMetricRangeError => "settings-metric-range-error",
    SettingsMetricStoredInvalid => "settings-metric-stored-invalid",
    SettingsBuiltinGlyphsLabel => "settings-builtin-glyphs-label",
    SettingsBuiltinGlyphsDescription => "settings-builtin-glyphs-description",
    SettingsBuiltinGlyphsStoredInvalid => "settings-builtin-glyphs-stored-invalid",
    SettingsCategoryDopamineTitle => "settings-category-dopamine-title",
    SettingsCategoryDopamineDescription => "settings-category-dopamine-description",
    SettingsDopamineEnabledLabel => "settings-dopamine-enabled-label",
    SettingsDopamineEnabledDescription => "settings-dopamine-enabled-description",
    SettingsDopamineEnabledStoredInvalid => "settings-dopamine-enabled-stored-invalid",
    SettingsDopamineSoundLabel => "settings-dopamine-sound-label",
    SettingsDopamineSoundZap => "settings-dopamine-sound-zap",
    SettingsDopamineSoundCrack => "settings-dopamine-sound-crack",
    SettingsDopamineSoundBurst => "settings-dopamine-sound-burst",
    SettingsDopamineSoundPreview => "settings-dopamine-sound-preview",
    SettingsShellModeLabel => "settings-shell-mode-label",
    SettingsShellModeDescription => "settings-shell-mode-description",
    SettingsShellModeAuto => "settings-shell-mode-auto",
    SettingsShellModeCustom => "settings-shell-mode-custom",
    SettingsShellPathLabel => "settings-shell-path-label",
    SettingsShellPathPlaceholder => "settings-shell-path-placeholder",
    SettingsShellIntegrationLabel => "settings-shell-integration-label",
    SettingsShellIntegrationDescription => "settings-shell-integration-description",
    SettingsShellIntegrationAuto => "settings-shell-integration-auto",
    SettingsShellIntegrationDisabled => "settings-shell-integration-disabled",
    SettingsShellNewTabNote => "settings-shell-new-tab-note",
    SettingsShellPathEmpty => "settings-shell-path-empty",
    SettingsShellPathNotAbsolute => "settings-shell-path-not-absolute",
    SettingsShellPathNotFound => "settings-shell-path-not-found",
    SettingsShellPathIsDirectory => "settings-shell-path-is-directory",
    SettingsShellPathNotExecutable => "settings-shell-path-not-executable",
    SettingsShellPathHasArguments => "settings-shell-path-has-arguments",
    SettingsTabCloseTooltip => "settings-tab-close-tooltip",

    MainPlaceholderTitle => "main-placeholder-title",
    MainPlaceholderBody => "main-placeholder-body",

    DialogSmokeQuestionTitle => "dialog-smoke-question-title",
    DialogSmokeQuestionMessage => "dialog-smoke-question-message",
    DialogSmokeWarningTitle => "dialog-smoke-warning-title",
    DialogSmokeWarningMessage => "dialog-smoke-warning-message",

    DialogActionCancel => "dialog-action-cancel",
    DialogActionContinue => "dialog-action-continue",
    DialogActionOk => "dialog-action-ok",
    DialogActionClose => "dialog-action-close",
    DialogActionDelete => "dialog-action-delete",
    DialogActionExit => "dialog-action-exit",

    FontPickerTitleUi => "font-picker-title-ui",
    FontPickerTitleTerminal => "font-picker-title-terminal",
    FontPickerSearchPlaceholder => "font-picker-search-placeholder",
    FontPickerMonospaceOnly => "font-picker-monospace-only",
    FontPickerPreviewHeader => "font-picker-preview-header",
    FontPickerNoMatchingFonts => "font-picker-no-matching-fonts",
    FontPickerCancel => "font-picker-cancel",
    FontPickerSelect => "font-picker-select",
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = MessageKey::ALL.iter().map(|key| key.id()).collect();
        assert_eq!(ids.len(), MessageKey::ALL.len());
    }

    #[test]
    fn ids_follow_the_naming_convention() {
        for key in MessageKey::ALL {
            let id = key.id();
            assert!(
                id.split('-').all(|part| {
                    !part.is_empty()
                        && part
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                }),
                "{id:?} is not lowercase kebab-case"
            );
        }
    }
}
