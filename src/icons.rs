//! Activity Bar icons.
//!
//! The icons are monochrome SVGs from Codicons (see
//! `assets/icons/codicons/`). They are embedded in the binary, and their
//! color is applied by the UI at render time, so the files themselves carry
//! no state colors.

use std::sync::LazyLock;

use iced::widget::svg;

use crate::workbench::ActivityItem;

/// Rendered size of an Activity Bar icon, in logical pixels.
pub const ICON_SIZE: f32 = 24.0;

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

/// Returns the icon for an Activity Bar entry.
pub fn activity_icon(item: ActivityItem) -> svg::Handle {
    match item {
        ActivityItem::Explorer => EXPLORER.clone(),
        ActivityItem::Search => SEARCH.clone(),
        ActivityItem::Git => GIT.clone(),
    }
}
