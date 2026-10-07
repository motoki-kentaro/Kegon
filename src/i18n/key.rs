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

    SideBarExplorerTitle => "side-bar-explorer-title",
    SideBarExplorerPlaceholder => "side-bar-explorer-placeholder",
    SideBarSearchTitle => "side-bar-search-title",
    SideBarSearchPlaceholder => "side-bar-search-placeholder",
    SideBarGitTitle => "side-bar-git-title",
    SideBarGitPlaceholder => "side-bar-git-placeholder",

    TerminalTabDefaultTitle => "terminal-tab-default-title",
    TerminalNewTabTooltip => "terminal-new-tab-tooltip",
    TerminalProcessExited => "terminal-process-exited",

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
