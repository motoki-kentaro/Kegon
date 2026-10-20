//! Codicons SVG icon handles. Icon tints come from the active theme.

use std::sync::LazyLock;

use iced::widget::svg;

use crate::dialog::DialogKind;

/// Rendered size of the Settings gear icon in the tab bar, in logical pixels.
pub const ICON_SIZE: f32 = 16.0;

/// Rendered size of a Dialog header icon, in logical pixels.
pub const DIALOG_ICON_SIZE: f32 = 28.0;

/// Rendered size of the tab close icon, in logical pixels.
pub const TAB_CLOSE_ICON_SIZE: f32 = 14.0;

/// Rendered size of the tab add icon, in logical pixels.
pub const TAB_ADD_ICON_SIZE: f32 = 16.0;

/// Rendered size of the tab scroll navigation triangle icons, in logical pixels.
pub const TAB_SCROLL_ICON_SIZE: f32 = 14.0;

static SETTINGS_GEAR: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/general/settings-gear.svg"
    ))
});

static QUESTION: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/dialog/question.svg"
    ))
});

static WARNING: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/dialog/warning.svg"
    ))
});

static ADD: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../assets/icons/codicons/general/add.svg"))
});

static CLOSE: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../assets/icons/codicons/general/close.svg"))
});

static TRIANGLE_LEFT: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/general/triangle-left.svg"
    ))
});

static TRIANGLE_RIGHT: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/general/triangle-right.svg"
    ))
});

/// Returns the icon for the Settings action at the right end of the tab bar.
pub fn settings_icon() -> svg::Handle {
    SETTINGS_GEAR.clone()
}

/// Returns the icon for a dialog kind.
pub fn dialog_icon(kind: DialogKind) -> svg::Handle {
    match kind {
        DialogKind::Question => QUESTION.clone(),
        DialogKind::Warning => WARNING.clone(),
    }
}

/// Returns the icon for adding a tab.
pub fn tab_add_icon() -> svg::Handle {
    ADD.clone()
}

/// Returns the icon for closing a tab.
pub fn tab_close_icon() -> svg::Handle {
    CLOSE.clone()
}

/// Returns the icon for scrolling tabs to the left.
pub fn tab_scroll_left_icon() -> svg::Handle {
    TRIANGLE_LEFT.clone()
}

/// Returns the icon for scrolling tabs to the right.
pub fn tab_scroll_right_icon() -> svg::Handle {
    TRIANGLE_RIGHT.clone()
}
