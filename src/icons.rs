//! Codicons SVG icon handles. Icon tints come from the active theme.

use std::sync::LazyLock;

use iced::widget::svg;

use crate::dialog::DialogKind;
use crate::workbench::ActivityItem;

/// Rendered size of an Activity Bar icon, in logical pixels.
pub const ICON_SIZE: f32 = 24.0;

/// Rendered size of a Dialog header icon, in logical pixels.
pub const DIALOG_ICON_SIZE: f32 = 28.0;

static EXPLORER: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/activity-bar/files.svg"
    ))
});

static SEARCH: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/activity-bar/search.svg"
    ))
});

static GIT: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/activity-bar/git-branch.svg"
    ))
});

static SETTINGS_GEAR: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!(
        "../assets/icons/codicons/activity-bar/settings-gear.svg"
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

/// Returns the icon for an Activity Bar entry.
pub fn activity_icon(item: ActivityItem) -> svg::Handle {
    match item {
        ActivityItem::Explorer => EXPLORER.clone(),
        ActivityItem::Search => SEARCH.clone(),
        ActivityItem::Git => GIT.clone(),
        ActivityItem::Settings => SETTINGS_GEAR.clone(),
    }
}

/// Returns the icon for a dialog kind.
pub fn dialog_icon(kind: DialogKind) -> svg::Handle {
    match kind {
        DialogKind::Question => QUESTION.clone(),
        DialogKind::Warning => WARNING.clone(),
    }
}
