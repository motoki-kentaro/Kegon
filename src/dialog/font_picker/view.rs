//! UI rendering for the Font Picker modal overlay.

use iced::widget::{
    Space, button, checkbox, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Border, Center, Color, Element, Fill, Font, Length};

use crate::dialog::font_picker::{FontPicker, FontPickerFocus, FontPickerMode, FontPickerResult};
use crate::i18n::{Localizer, MessageKey};

mod palette {
    use iced::Color;

    pub const BACKDROP: Color = Color::from_rgba8(0, 0, 0, 0.65);
    pub const SURFACE_BG: Color = Color::from_rgb8(0x25, 0x25, 0x26);
    pub const CANDIDATE_BG: Color = Color::from_rgb8(0x1e, 0x1e, 0x1e);
    pub const SELECTED_BG: Color = Color::from_rgb8(0x37, 0x37, 0x3d);
    pub const PREVIEW_BG: Color = Color::from_rgb8(0x1e, 0x1e, 0x1e);

    pub const BORDER: Color = Color::from_rgb8(0x45, 0x45, 0x45);
    pub const TEXT_TITLE: Color = Color::from_rgb8(0xff, 0xff, 0xff);
    pub const TEXT_BODY: Color = Color::from_rgb8(0xcc, 0xcc, 0xcc);
    pub const TEXT_MUTED: Color = Color::from_rgb8(0x85, 0x85, 0x85);

    pub const BUTTON_BG: Color = Color::from_rgb8(0x33, 0x33, 0x36);
    pub const BUTTON_BG_HOVER: Color = Color::from_rgb8(0x3c, 0x3c, 0x40);
    pub const PRIMARY_BG: Color = Color::from_rgb8(0x00, 0x7a, 0xcc);
    pub const PRIMARY_BG_HOVER: Color = Color::from_rgb8(0x00, 0x62, 0xa3);
    pub const FOCUS_BORDER: Color = Color::from_rgb8(0x00, 0x7a, 0xcc);
    pub const FOCUS_BORDER_BRIGHT: Color = Color::from_rgb8(0xff, 0xff, 0xff);
}

/// Renders the Font Picker modal dialog centered over the base application view.
#[allow(clippy::too_many_arguments)]
pub fn render_font_picker_overlay<'a, Message>(
    base_view: Element<'a, Message>,
    picker: &'a FontPicker,
    localizer: &'a Localizer,
    ui_font: Font,
    on_search_changed: impl Fn(String) -> Message + 'a,
    on_monospace_toggled: impl Fn(bool) -> Message + 'a,
    on_candidate_selected: impl Fn(usize) -> Message + 'a + Copy,
    on_result: impl Fn(FontPickerResult) -> Message + 'a + Clone,
    on_backdrop: Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let title_key = match picker.mode {
        FontPickerMode::Ui => MessageKey::FontPickerTitleUi,
        FontPickerMode::Terminal => MessageKey::FontPickerTitleTerminal,
    };
    let title_text = localizer.text(title_key);
    let search_placeholder = localizer.text(MessageKey::FontPickerSearchPlaceholder);
    let monospace_label = localizer.text(MessageKey::FontPickerMonospaceOnly);
    let preview_header = localizer.text(MessageKey::FontPickerPreviewHeader);
    let no_matching = localizer.text(MessageKey::FontPickerNoMatchingFonts);
    let cancel_label = localizer.text(MessageKey::FontPickerCancel);
    let select_label = localizer.text(MessageKey::FontPickerSelect);

    let is_search_focused = picker.focus == FontPickerFocus::SearchInput;
    let search_input = text_input(&search_placeholder, &picker.search_query)
        .font(ui_font)
        .on_input(on_search_changed)
        .padding(8)
        .size(13.0)
        .style(move |_, _status| {
            let border_color = if is_search_focused {
                palette::FOCUS_BORDER
            } else {
                palette::BORDER
            };
            let border_width = if is_search_focused { 2.0 } else { 1.0 };
            text_input::Style {
                background: palette::CANDIDATE_BG.into(),
                border: Border {
                    color: border_color,
                    width: border_width,
                    radius: 4.0.into(),
                },
                icon: palette::TEXT_MUTED,
                placeholder: palette::TEXT_MUTED,
                value: palette::TEXT_TITLE,
                selection: palette::PRIMARY_BG,
            }
        });

    let filtered = picker.filtered_candidates();

    let mut list_column = column![].spacing(2);
    if filtered.is_empty() {
        list_column = list_column.push(
            container(text(no_matching).size(13.0).color(palette::TEXT_MUTED))
                .padding(12)
                .width(Fill)
                .align_x(Center),
        );
    } else {
        for (idx, candidate) in filtered.iter().enumerate() {
            let is_highlighted = idx == picker.highlighted_index;
            let is_list_focused = picker.focus == FontPickerFocus::CandidateList;

            let font_name = candidate.family_name.clone();
            let label = text(font_name.clone())
                .size(13.0)
                .color(palette::TEXT_TITLE);

            let bg = if is_highlighted {
                palette::SELECTED_BG
            } else {
                palette::CANDIDATE_BG
            };

            let border_color = if is_highlighted && is_list_focused {
                palette::FOCUS_BORDER
            } else {
                Color::TRANSPARENT
            };

            let item_btn = mouse_area(container(label).padding([6, 10]).width(Fill).style(
                move |_| {
                    container::Style::default()
                        .background(bg)
                        .border(Border {
                            color: border_color,
                            width: 1.0,
                            radius: 3.0.into(),
                        })
                        .color(palette::TEXT_TITLE)
                },
            ))
            .on_press(on_candidate_selected(idx));

            list_column = list_column.push(item_btn);
        }
    }

    let list_scrollable = scrollable(list_column).height(160.0);

    let list_container = container(list_scrollable).style(|_| {
        container::Style::default()
            .background(palette::CANDIDATE_BG)
            .border(Border {
                color: palette::BORDER,
                width: 1.0,
                radius: 4.0.into(),
            })
    });

    let highlighted = picker.highlighted_candidate();
    let preview_font_family = highlighted
        .map(|c| c.family_name.clone())
        .unwrap_or_default();

    let preview_sub_header = if preview_font_family.is_empty() {
        preview_header.to_string()
    } else {
        format!("{preview_header} ({preview_font_family})")
    };

    let static_family_name: &'static str = Box::leak(preview_font_family.into_boxed_str());
    let font_arg = Font::with_name(static_family_name);

    let preview_box = container(
        text(picker.preview_text())
            .size(14.0)
            .font(font_arg)
            .color(palette::TEXT_TITLE),
    )
    .padding(12)
    .width(Fill)
    .height(100.0)
    .style(|_| {
        container::Style::default()
            .background(palette::PREVIEW_BG)
            .border(Border {
                color: palette::BORDER,
                width: 1.0,
                radius: 4.0.into(),
            })
    });

    let cancel_on_result = on_result.clone();
    let is_cancel_focused = picker.focus == FontPickerFocus::CancelButton;
    let cancel_btn = button(
        text(cancel_label)
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
        let border_color = if is_cancel_focused {
            palette::FOCUS_BORDER
        } else {
            palette::BORDER
        };
        button::Style {
            background: Some(bg.into()),
            text_color: palette::TEXT_TITLE,
            border: Border {
                color: border_color,
                width: if is_cancel_focused { 2.0 } else { 1.0 },
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    })
    .on_press(cancel_on_result(FontPickerResult::Cancel));

    let select_on_result = on_result;
    let is_select_focused = picker.focus == FontPickerFocus::SelectButton;
    let selected_candidate = highlighted.cloned();

    let mut select_btn = button(
        text(select_label)
            .size(13.0)
            .font(ui_font)
            .color(palette::TEXT_TITLE),
    )
    .padding([6, 16])
    .style(move |_, status| {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => palette::PRIMARY_BG_HOVER,
            _ => palette::PRIMARY_BG,
        };
        let border_color = if is_select_focused {
            palette::FOCUS_BORDER_BRIGHT
        } else {
            palette::BORDER
        };
        button::Style {
            background: Some(bg.into()),
            text_color: palette::TEXT_TITLE,
            border: Border {
                color: border_color,
                width: if is_select_focused { 2.0 } else { 1.0 },
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    });

    if let Some(candidate) = selected_candidate {
        select_btn = select_btn.on_press(select_on_result(FontPickerResult::Select(candidate)));
    }

    let actions = row![cancel_btn, Space::new().width(8.0), select_btn].align_y(Center);

    let mut content = column![
        text(title_text)
            .size(16.0)
            .font(ui_font)
            .color(palette::TEXT_TITLE),
        Space::new().height(12.0),
        search_input,
    ]
    .spacing(0);

    if picker.mode == FontPickerMode::Terminal {
        let is_mono_focused = picker.focus == FontPickerFocus::MonospaceToggle;
        let mono_checkbox = checkbox(picker.monospace_only)
            .label(monospace_label)
            .on_toggle(on_monospace_toggled)
            .size(16)
            .text_size(13.0)
            .style(move |_, _| checkbox::Style {
                background: palette::CANDIDATE_BG.into(),
                icon_color: palette::TEXT_TITLE,
                border: Border {
                    color: if is_mono_focused {
                        palette::FOCUS_BORDER
                    } else {
                        palette::BORDER
                    },
                    width: if is_mono_focused { 2.0 } else { 1.0 },
                    radius: 3.0.into(),
                },
                text_color: Some(palette::TEXT_BODY),
            });

        content = content.push(Space::new().height(8.0)).push(mono_checkbox);
    }

    content = content
        .push(Space::new().height(10.0))
        .push(list_container)
        .push(Space::new().height(12.0))
        .push(
            text(preview_sub_header)
                .size(13.0)
                .color(palette::TEXT_BODY),
        )
        .push(Space::new().height(4.0))
        .push(preview_box)
        .push(Space::new().height(16.0))
        .push(row![Space::new().width(Length::Fill), actions]);

    let dialog_surface = container(content.padding(20)).max_width(540.0).style(|_| {
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
