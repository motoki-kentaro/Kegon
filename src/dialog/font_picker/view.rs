//! UI rendering for the Font Picker modal overlay.

use iced::widget::{
    Space, button, checkbox, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Border, Center, Color, Element, Fill, Font, Length};

use crate::dialog::font_picker::{FontPicker, FontPickerFocus, FontPickerMode, FontPickerResult};
use crate::i18n::{Localizer, MessageKey};
use crate::theme::{KegonTheme, style};

/// Renders the Font Picker modal dialog centered over the base application view.
#[allow(clippy::too_many_arguments)]
pub fn render_font_picker_overlay<'a, Message>(
    base_view: Element<'a, Message>,
    picker: &'a FontPicker,
    localizer: &'a Localizer,
    ui_font: Font,
    theme: &'a KegonTheme,
    on_search_changed: impl Fn(String) -> Message + 'a,
    on_monospace_toggled: impl Fn(bool) -> Message + 'a,
    on_candidate_selected: impl Fn(usize) -> Message + 'a + Copy,
    on_result: impl Fn(FontPickerResult) -> Message + 'a + Clone,
    on_backdrop: Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    // Inset areas (search field, candidate list, preview) sit on the
    // workbench base color, below the dialog surface.
    let inset_background = theme.workbench.background;

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
        .style(move |_, _status| text_input::Style {
            background: inset_background.into(),
            border: focus_border(theme, is_search_focused, 4.0),
            icon: theme.text.muted,
            placeholder: theme.text.muted,
            value: theme.text.emphasis,
            selection: theme.interaction.selection,
        });

    let filtered = picker.filtered_candidates();

    let mut list_column = column![].spacing(2);
    if filtered.is_empty() {
        list_column = list_column.push(
            container(text(no_matching).size(13.0).color(theme.text.muted))
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
                .color(theme.text.emphasis);

            let bg = if is_highlighted {
                theme.interaction.hover
            } else {
                inset_background
            };

            let border_color = if is_highlighted && is_list_focused {
                theme.interaction.accent
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
                        .color(theme.text.emphasis)
                },
            ))
            .on_press(on_candidate_selected(idx));

            list_column = list_column.push(item_btn);
        }
    }

    let list_scrollable = scrollable(list_column).height(160.0);

    let list_container =
        container(list_scrollable).style(move |_| inset_style(theme, inset_background));

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
            .color(theme.text.emphasis),
    )
    .padding(12)
    .width(Fill)
    .height(100.0)
    .style(move |_| inset_style(theme, inset_background));

    let cancel_on_result = on_result.clone();
    let is_cancel_focused = picker.focus == FontPickerFocus::CancelButton;
    let cancel_btn = button(text(cancel_label).size(13.0).font(ui_font))
        .padding([6, 16])
        .style(move |_, status| style::secondary_button(theme, status, is_cancel_focused))
        .on_press(cancel_on_result(FontPickerResult::Cancel));

    let select_on_result = on_result;
    let is_select_focused = picker.focus == FontPickerFocus::SelectButton;
    let selected_candidate = highlighted.cloned();

    let mut select_btn = button(text(select_label).size(13.0).font(ui_font))
        .padding([6, 16])
        .style(move |_, status| {
            style::filled_button(theme, theme.interaction.accent, status, is_select_focused)
        });

    if let Some(candidate) = selected_candidate {
        select_btn = select_btn.on_press(select_on_result(FontPickerResult::Select(candidate)));
    }

    let actions = row![cancel_btn, Space::new().width(8.0), select_btn].align_y(Center);

    let mut content = column![
        text(title_text)
            .size(16.0)
            .font(ui_font)
            .color(theme.text.emphasis),
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
                background: inset_background.into(),
                icon_color: theme.text.emphasis,
                border: focus_border(theme, is_mono_focused, 3.0),
                text_color: Some(theme.text.primary),
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
                .color(theme.text.primary),
        )
        .push(Space::new().height(4.0))
        .push(preview_box)
        .push(Space::new().height(16.0))
        .push(row![Space::new().width(Length::Fill), actions]);

    let dialog_surface = container(content.padding(20))
        .max_width(540.0)
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

fn inset_style(theme: &KegonTheme, background: Color) -> container::Style {
    container::Style::default()
        .background(background)
        .border(Border {
            color: theme.workbench.border,
            width: 1.0,
            radius: 4.0.into(),
        })
}

fn focus_border(theme: &KegonTheme, focused: bool, radius: f32) -> Border {
    Border {
        color: if focused {
            theme.interaction.accent
        } else {
            theme.workbench.border
        },
        width: if focused { 2.0 } else { 1.0 },
        radius: radius.into(),
    }
}
