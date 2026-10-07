//! Confirmation dialog model and semantic types.

use crate::i18n::MessageKey;

/// Semantic purpose of a modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// Questions or confirmations about intended actions.
    Question,
    /// Warnings about potential risk or destructive outcomes.
    Warning,
}

/// Visual emphasis and risk tone of a dialog action button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTone {
    /// Standard neutral or affirmative action tone.
    Normal,
    /// Destructive, irreversible, or high-risk action tone.
    Destructive,
}

/// Active focused action button for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedAction {
    Primary,
    Secondary,
}

impl FocusedAction {
    pub fn toggle(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Primary,
        }
    }
}

/// Logical focus target to restore when the modal dialog closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FocusTarget {
    Terminal,
    Workbench,
}

/// Outcome when a confirmation dialog is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationResult {
    Primary,
    Secondary,
}

/// Model representing a binary confirmation dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationDialog {
    pub kind: DialogKind,
    pub primary_action_tone: ActionTone,
    pub title: MessageKey,
    pub message: MessageKey,
    pub primary_action: MessageKey,
    pub secondary_action: MessageKey,
    pub focused_action: FocusedAction,
    pub restore_focus: FocusTarget,
}

impl ConfirmationDialog {
    pub fn new(
        kind: DialogKind,
        primary_action_tone: ActionTone,
        title: MessageKey,
        message: MessageKey,
        primary_action: MessageKey,
        secondary_action: MessageKey,
        restore_focus: FocusTarget,
    ) -> Self {
        let initial_focus = match primary_action_tone {
            ActionTone::Normal => FocusedAction::Primary,
            ActionTone::Destructive => FocusedAction::Secondary,
        };

        Self {
            kind,
            primary_action_tone,
            title,
            message,
            primary_action,
            secondary_action,
            focused_action: initial_focus,
            restore_focus,
        }
    }

    /// Creates a builder for constructing a [`ConfirmationDialog`].
    pub fn builder(
        kind: DialogKind,
        title: MessageKey,
        message: MessageKey,
    ) -> ConfirmationDialogBuilder {
        ConfirmationDialogBuilder {
            kind,
            primary_action_tone: ActionTone::Normal,
            title,
            message,
            primary_action: MessageKey::DialogActionOk,
            secondary_action: MessageKey::DialogActionCancel,
            restore_focus: FocusTarget::Terminal,
        }
    }

    /// Toggle focus between Secondary and Primary buttons.
    pub fn toggle_focus(&mut self) {
        self.focused_action = self.focused_action.toggle();
    }

    /// Resolves the outcome for the currently focused action button.
    #[allow(dead_code)]
    pub fn resolve_focused_action(&self) -> ConfirmationResult {
        match self.focused_action {
            FocusedAction::Primary => ConfirmationResult::Primary,
            FocusedAction::Secondary => ConfirmationResult::Secondary,
        }
    }
}

/// Fluent builder for [`ConfirmationDialog`].
#[derive(Debug, Clone)]
pub struct ConfirmationDialogBuilder {
    kind: DialogKind,
    primary_action_tone: ActionTone,
    title: MessageKey,
    message: MessageKey,
    primary_action: MessageKey,
    secondary_action: MessageKey,
    restore_focus: FocusTarget,
}

impl ConfirmationDialogBuilder {
    pub fn primary_action(mut self, action: MessageKey, tone: ActionTone) -> Self {
        self.primary_action = action;
        self.primary_action_tone = tone;
        self
    }

    pub fn secondary_action(mut self, action: MessageKey) -> Self {
        self.secondary_action = action;
        self
    }

    pub fn restore_focus(mut self, target: FocusTarget) -> Self {
        self.restore_focus = target;
        self
    }

    pub fn build(self) -> ConfirmationDialog {
        ConfirmationDialog::new(
            self.kind,
            self.primary_action_tone,
            self.title,
            self.message,
            self.primary_action,
            self.secondary_action,
            self.restore_focus,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_focus_reflects_action_tone() {
        let normal_dialog = ConfirmationDialog::new(
            DialogKind::Question,
            ActionTone::Normal,
            MessageKey::DialogSmokeQuestionTitle,
            MessageKey::DialogSmokeQuestionMessage,
            MessageKey::DialogActionContinue,
            MessageKey::DialogActionCancel,
            FocusTarget::Terminal,
        );
        assert_eq!(normal_dialog.focused_action, FocusedAction::Primary);
        assert_eq!(
            normal_dialog.resolve_focused_action(),
            ConfirmationResult::Primary
        );

        let destructive_dialog = ConfirmationDialog::new(
            DialogKind::Warning,
            ActionTone::Destructive,
            MessageKey::DialogSmokeWarningTitle,
            MessageKey::DialogSmokeWarningMessage,
            MessageKey::DialogActionContinue,
            MessageKey::DialogActionCancel,
            FocusTarget::Terminal,
        );
        assert_eq!(destructive_dialog.focused_action, FocusedAction::Secondary);
        assert_eq!(
            destructive_dialog.resolve_focused_action(),
            ConfirmationResult::Secondary
        );
    }

    #[test]
    fn focus_toggles_between_primary_and_secondary() {
        let mut dialog = ConfirmationDialog::new(
            DialogKind::Warning,
            ActionTone::Destructive,
            MessageKey::DialogSmokeWarningTitle,
            MessageKey::DialogSmokeWarningMessage,
            MessageKey::DialogActionContinue,
            MessageKey::DialogActionCancel,
            FocusTarget::Terminal,
        );

        assert_eq!(dialog.focused_action, FocusedAction::Secondary);
        dialog.toggle_focus();
        assert_eq!(dialog.focused_action, FocusedAction::Primary);
        assert_eq!(dialog.resolve_focused_action(), ConfirmationResult::Primary);

        dialog.toggle_focus();
        assert_eq!(dialog.focused_action, FocusedAction::Secondary);
        assert_eq!(
            dialog.resolve_focused_action(),
            ConfirmationResult::Secondary
        );
    }
}
