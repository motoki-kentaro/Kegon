//! Widget styles derived from a [`KegonTheme`], shared by the workbench and
//! dialogs.

use iced::widget::{button, container};
use iced::{Border, Color};

use super::model::KegonTheme;

/// Darkens a filled button's color for its hovered and pressed states.
pub fn hover_shade(color: Color) -> Color {
    const FACTOR: f32 = 0.8;
    Color {
        r: color.r * FACTOR,
        g: color.g * FACTOR,
        b: color.b * FACTOR,
        a: color.a,
    }
}

/// Mixes `over` into `base` by `amount` (0.0 = `base`, 1.0 = `over`).
pub fn blend(base: Color, over: Color, amount: f32) -> Color {
    let mix = |a: f32, b: f32| a + (b - a) * amount;
    Color {
        r: mix(base.r, over.r),
        g: mix(base.g, over.g),
        b: mix(base.b, over.b),
        a: mix(base.a, over.a),
    }
}

/// A neutral button on an elevated surface, such as Cancel.
pub fn secondary_button(
    theme: &KegonTheme,
    status: button::Status,
    focused: bool,
) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered | button::Status::Pressed => {
            (theme.interaction.hover, theme.text.emphasis)
        }
        button::Status::Disabled => (theme.workbench.surface_elevated, theme.text.disabled),
        button::Status::Active => (theme.workbench.surface_elevated, theme.text.emphasis),
    };

    button::Style {
        background: Some(background.into()),
        text_color,
        border: focus_border(theme.interaction.accent, theme.workbench.border, focused),
        ..button::Style::default()
    }
}

/// A button filled with `fill`, such as a primary or destructive action.
///
/// The focus ring uses `text.on_accent`, because an accent ring would vanish
/// against an accent fill.
pub fn filled_button(
    theme: &KegonTheme,
    fill: Color,
    status: button::Status,
    focused: bool,
) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => hover_shade(fill),
        button::Status::Active | button::Status::Disabled => fill,
    };

    button::Style {
        background: Some(background.into()),
        text_color: theme.text.on_accent,
        border: focus_border(theme.text.on_accent, theme.workbench.border, focused),
        ..button::Style::default()
    }
}

/// The panel of a modal dialog.
pub fn modal_surface(theme: &KegonTheme) -> container::Style {
    container::Style::default()
        .background(theme.workbench.surface_background)
        .border(Border {
            color: theme.workbench.border,
            width: 1.0,
            radius: 6.0.into(),
        })
}

/// The scrim behind a modal dialog.
pub fn modal_backdrop(theme: &KegonTheme) -> container::Style {
    container::Style::default().background(theme.workbench.backdrop)
}

fn focus_border(focus: Color, normal: Color, focused: bool) -> Border {
    Border {
        color: if focused { focus } else { normal },
        width: if focused { 2.0 } else { 1.0 },
        radius: 4.0.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::NIGHT_DARK;

    fn background(style: &button::Style) -> Color {
        match style.background {
            Some(iced::Background::Color(color)) => color,
            _ => panic!("expected a solid background"),
        }
    }

    #[test]
    fn hover_shade_preserves_the_previous_primary_hover_color() {
        let shaded = hover_shade(NIGHT_DARK.interaction.accent);
        assert_eq!(shaded.into_rgba8(), [0x00, 0x62, 0xA3, 0xFF]);
    }

    #[test]
    fn filled_button_uses_the_fill_and_on_accent_text() {
        let fill = NIGHT_DARK.semantic.destructive;
        let style = filled_button(&NIGHT_DARK, fill, button::Status::Active, false);
        assert_eq!(background(&style), fill);
        assert_eq!(style.text_color, NIGHT_DARK.text.on_accent);

        let hovered = filled_button(&NIGHT_DARK, fill, button::Status::Hovered, false);
        assert_eq!(background(&hovered), hover_shade(fill));

        let focused = filled_button(&NIGHT_DARK, fill, button::Status::Active, true);
        assert_eq!(focused.border.color, NIGHT_DARK.text.on_accent);
    }

    #[test]
    fn secondary_button_states() {
        let active = secondary_button(&NIGHT_DARK, button::Status::Active, false);
        assert_eq!(background(&active), NIGHT_DARK.workbench.surface_elevated);
        assert_eq!(active.border.color, NIGHT_DARK.workbench.border);

        let hovered = secondary_button(&NIGHT_DARK, button::Status::Hovered, false);
        assert_eq!(background(&hovered), NIGHT_DARK.interaction.hover);

        let disabled = secondary_button(&NIGHT_DARK, button::Status::Disabled, false);
        assert_eq!(disabled.text_color, NIGHT_DARK.text.disabled);

        let focused = secondary_button(&NIGHT_DARK, button::Status::Active, true);
        assert_eq!(focused.border.color, NIGHT_DARK.interaction.accent);
    }
}
