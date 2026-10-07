//! Modal shell and confirmation dialog UI rendering.

use iced::widget::{Space, button, column, container, mouse_area, row, space, svg, text};
use iced::{Center, Color, Element, Fill, Font, Length};

use crate::dialog::confirmation::{
    ActionTone, ConfirmationDialog, ConfirmationResult, DialogKind, FocusedAction,
};
use crate::i18n::Localizer;
use crate::icons::{DIALOG_ICON_SIZE, dialog_icon};
use crate::theme::{KegonTheme, style};

/// The tint of a dialog's header icon.
pub fn dialog_icon_color(theme: &KegonTheme, kind: DialogKind) -> Color {
    match kind {
        DialogKind::Question => theme.icons.active,
        DialogKind::Warning => theme.semantic.warning,
    }
}

/// The fill of a dialog's primary action. Depends only on the action's tone,
/// never on the dialog kind.
pub fn primary_action_fill(theme: &KegonTheme, tone: ActionTone) -> Color {
    match tone {
        ActionTone::Normal => theme.interaction.accent,
        ActionTone::Destructive => theme.semantic.destructive,
    }
}

/// Renders a modal confirmation dialog overlay centered over the main UI.
pub fn render_modal_overlay<'a, Message>(
    base_view: Element<'a, Message>,
    dialog: &'a ConfirmationDialog,
    localizer: &'a Localizer,
    ui_font: Font,
    theme: &'a KegonTheme,
    on_action: impl Fn(ConfirmationResult) -> Message + 'a + Clone,
    on_backdrop: Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let icon_color = dialog_icon_color(theme, dialog.kind);
    let title_text = localizer.text(dialog.title);
    let message_text = localizer.text(dialog.message);
    let primary_label = localizer.text(dialog.primary_action);
    let secondary_label = localizer.text(dialog.secondary_action);

    let header = row![
        svg(dialog_icon(dialog.kind))
            .width(DIALOG_ICON_SIZE)
            .height(DIALOG_ICON_SIZE)
            .style(move |_, _| svg::Style {
                color: Some(icon_color),
            }),
        Space::new().width(12.0),
        text(title_text)
            .size(16.0)
            .font(ui_font)
            .color(theme.text.emphasis)
    ]
    .align_y(Center);

    let body = text(message_text)
        .size(13.0)
        .font(ui_font)
        .color(theme.text.primary);

    let is_sec_focused = dialog.focused_action == FocusedAction::Secondary;
    let sec_on_action = on_action.clone();
    let secondary_btn = button(text(secondary_label).size(13.0).font(ui_font))
        .padding([6, 16])
        .style(move |_, status| style::secondary_button(theme, status, is_sec_focused))
        .on_press(sec_on_action(ConfirmationResult::Secondary));

    let is_pri_focused = dialog.focused_action == FocusedAction::Primary;
    let primary_fill = primary_action_fill(theme, dialog.primary_action_tone);
    let pri_on_action = on_action;
    let primary_btn = button(text(primary_label).size(13.0).font(ui_font))
        .padding([6, 16])
        .style(move |_, status| style::filled_button(theme, primary_fill, status, is_pri_focused))
        .on_press(pri_on_action(ConfirmationResult::Primary));

    let actions = row![secondary_btn, Space::new().width(8.0), primary_btn].align_y(Center);

    let content = column![
        header,
        Space::new().height(12.0),
        body,
        Space::new().height(20.0),
        row![space::horizontal(), actions]
    ]
    .padding(20)
    .width(Length::Shrink);

    let dialog_surface = container(content)
        .max_width(480.0)
        .style(move |_| style::modal_surface(theme));

    let backdrop = mouse_area(
        container(
            container(dialog_surface)
                .width(Fill)
                .height(Fill)
                .align_x(Center)
                .align_y(Center),
        )
        .width(Fill)
        .height(Fill)
        .style(move |_| style::modal_backdrop(theme)),
    )
    .on_press(on_backdrop);

    iced::widget::stack![base_view, backdrop].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::NIGHT_DARK;

    #[test]
    fn icon_color_follows_the_dialog_kind() {
        assert_eq!(
            dialog_icon_color(&NIGHT_DARK, DialogKind::Question),
            NIGHT_DARK.icons.active
        );
        assert_eq!(
            dialog_icon_color(&NIGHT_DARK, DialogKind::Warning),
            NIGHT_DARK.semantic.warning
        );
    }

    #[test]
    fn primary_fill_follows_the_action_tone() {
        assert_eq!(
            primary_action_fill(&NIGHT_DARK, ActionTone::Normal),
            NIGHT_DARK.interaction.accent
        );
        assert_eq!(
            primary_action_fill(&NIGHT_DARK, ActionTone::Destructive),
            NIGHT_DARK.semantic.destructive
        );
    }

    #[test]
    fn kind_and_tone_are_independent() {
        // A Warning with a normal action is not destructive, and a Question
        // with a destructive action is still a Question.
        let mut theme = NIGHT_DARK;
        theme.semantic.warning = Color::from_rgb8(1, 1, 1);
        theme.semantic.destructive = Color::from_rgb8(2, 2, 2);
        theme.interaction.accent = Color::from_rgb8(3, 3, 3);

        assert_eq!(
            dialog_icon_color(&theme, DialogKind::Warning),
            Color::from_rgb8(1, 1, 1)
        );
        assert_eq!(
            primary_action_fill(&theme, ActionTone::Normal),
            Color::from_rgb8(3, 3, 3)
        );
        assert_eq!(
            dialog_icon_color(&theme, DialogKind::Question),
            theme.icons.active
        );
        assert_eq!(
            primary_action_fill(&theme, ActionTone::Destructive),
            Color::from_rgb8(2, 2, 2)
        );
    }
}
