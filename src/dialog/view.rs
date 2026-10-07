//! Modal shell and confirmation dialog UI rendering.

use iced::widget::{Space, button, column, container, mouse_area, row, space, svg, text};
use iced::{Border, Center, Element, Fill, Font, Length};

use crate::dialog::confirmation::{
    ActionTone, ConfirmationDialog, ConfirmationResult, FocusedAction,
};
use crate::i18n::Localizer;
use crate::icons::{DIALOG_ICON_SIZE, dialog_icon};

mod palette {
    use iced::Color;

    pub const BACKDROP: Color = Color::from_rgba8(0, 0, 0, 0.65);
    pub const SURFACE_BG: Color = Color::from_rgb8(0x25, 0x25, 0x26);
    pub const BORDER: Color = Color::from_rgb8(0x45, 0x45, 0x45);
    pub const TEXT_TITLE: Color = Color::from_rgb8(0xff, 0xff, 0xff);
    pub const TEXT_BODY: Color = Color::from_rgb8(0xcc, 0xcc, 0xcc);

    pub const BUTTON_BG: Color = Color::from_rgb8(0x33, 0x33, 0x36);
    pub const BUTTON_BG_HOVER: Color = Color::from_rgb8(0x3c, 0x3c, 0x40);
    pub const PRIMARY_BG: Color = Color::from_rgb8(0x00, 0x7a, 0xcc);
    pub const PRIMARY_BG_HOVER: Color = Color::from_rgb8(0x00, 0x62, 0xa3);
    pub const DESTRUCTIVE_BG: Color = Color::from_rgb8(0xd7, 0x3a, 0x49);
    pub const DESTRUCTIVE_BG_HOVER: Color = Color::from_rgb8(0xb3, 0x2d, 0x3a);
    pub const FOCUS_BORDER: Color = Color::from_rgb8(0x00, 0x7a, 0xcc);
    pub const FOCUS_BORDER_BRIGHT: Color = Color::from_rgb8(0xff, 0xff, 0xff);
}

/// Renders a modal confirmation dialog overlay centered over the main UI.
pub fn render_modal_overlay<'a, Message>(
    base_view: Element<'a, Message>,
    dialog: &'a ConfirmationDialog,
    localizer: &'a Localizer,
    ui_font: Font,
    on_action: impl Fn(ConfirmationResult) -> Message + 'a + Clone,
    on_backdrop: Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let (icon_handle, icon_color) = dialog_icon(dialog.kind);
    let title_text = localizer.text(dialog.title);
    let message_text = localizer.text(dialog.message);
    let primary_label = localizer.text(dialog.primary_action);
    let secondary_label = localizer.text(dialog.secondary_action);

    let header = row![
        svg(icon_handle)
            .width(DIALOG_ICON_SIZE)
            .height(DIALOG_ICON_SIZE)
            .style(move |_, _| svg::Style {
                color: Some(icon_color),
            }),
        Space::new().width(12.0),
        text(title_text)
            .size(16.0)
            .font(ui_font)
            .color(palette::TEXT_TITLE)
    ]
    .align_y(Center);

    let body = text(message_text)
        .size(13.0)
        .font(ui_font)
        .color(palette::TEXT_BODY);

    let is_sec_focused = dialog.focused_action == FocusedAction::Secondary;
    let sec_on_action = on_action.clone();
    let secondary_btn = button(
        text(secondary_label)
            .size(13.0)
            .font(ui_font)
            .color(palette::TEXT_TITLE),
    )
    .padding([6, 16])
    .style(move |_, status| {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => palette::BUTTON_BG_HOVER,
            _ => palette::BUTTON_BG,
        };
        let border_color = if is_sec_focused {
            palette::FOCUS_BORDER
        } else {
            palette::BORDER
        };
        let border_width = if is_sec_focused { 2.0 } else { 1.0 };
        button::Style {
            background: Some(bg.into()),
            text_color: palette::TEXT_TITLE,
            border: Border {
                color: border_color,
                width: border_width,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    })
    .on_press(sec_on_action(ConfirmationResult::Secondary));

    let is_pri_focused = dialog.focused_action == FocusedAction::Primary;
    let tone = dialog.primary_action_tone;
    let pri_on_action = on_action;
    let primary_btn = button(
        text(primary_label)
            .size(13.0)
            .font(ui_font)
            .color(palette::TEXT_TITLE),
    )
    .padding([6, 16])
    .style(move |_, status| {
        let (bg, hover_bg) = match tone {
            ActionTone::Destructive => (palette::DESTRUCTIVE_BG, palette::DESTRUCTIVE_BG_HOVER),
            ActionTone::Normal => (palette::PRIMARY_BG, palette::PRIMARY_BG_HOVER),
        };
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => hover_bg,
            _ => bg,
        };
        let border_color = if is_pri_focused {
            palette::FOCUS_BORDER_BRIGHT
        } else {
            palette::BORDER
        };
        let border_width = if is_pri_focused { 2.0 } else { 1.0 };
        button::Style {
            background: Some(background.into()),
            text_color: palette::TEXT_TITLE,
            border: Border {
                color: border_color,
                width: border_width,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    })
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

    let dialog_surface = container(content).max_width(480.0).style(|_| {
        container::Style::default()
            .background(palette::SURFACE_BG)
            .border(Border {
                color: palette::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            })
    });

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
        .style(|_| container::Style::default().background(palette::BACKDROP)),
    )
    .on_press(on_backdrop);

    iced::widget::stack![base_view, backdrop].into()
}
