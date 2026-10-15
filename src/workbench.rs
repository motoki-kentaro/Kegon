//! UI-independent workbench state.
//!
//! This module intentionally has no dependency on the GUI toolkit so that the
//! workbench model can be unit tested and later grow into real session and
//! panel management without being tangled with rendering code.

mod tabs;

pub use tabs::{TabId, TabKind, WorkbenchTabs};

/// Default width of the Side Bar, in logical pixels.
pub const SIDE_BAR_DEFAULT_WIDTH: f32 = 260.0;
/// Narrowest the Side Bar can be resized to, in logical pixels.
pub const SIDE_BAR_MIN_WIDTH: f32 = 170.0;
/// Widest the Side Bar can be resized to, in logical pixels.
pub const SIDE_BAR_MAX_WIDTH: f32 = 400.0;

/// An entry in the Activity Bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivityItem {
    Explorer,
    Search,
    Git,
    Settings,
}

impl ActivityItem {
    /// Top primary Activity Bar entries, in display order.
    pub const TOP_ITEMS: [Self; 3] = [Self::Explorer, Self::Search, Self::Git];

    /// All Activity Bar entries, including bottom global actions.
    #[allow(dead_code)]
    pub const ALL: [Self; 4] = [Self::Explorer, Self::Search, Self::Git, Self::Settings];
}

/// The state of the whole workbench window.
#[derive(Debug, Clone)]
pub struct Workbench {
    active_activity: ActivityItem,
    side_bar_width: f32,
    tabs: WorkbenchTabs,
}

impl Workbench {
    /// A fresh workbench starts with one active Terminal tab.
    fn initial_tabs() -> WorkbenchTabs {
        let mut tabs = WorkbenchTabs::new();
        tabs.add(TabKind::Terminal);
        tabs
    }
}

impl Default for Workbench {
    fn default() -> Self {
        Self {
            active_activity: ActivityItem::Explorer,
            side_bar_width: SIDE_BAR_DEFAULT_WIDTH,
            tabs: Self::initial_tabs(),
        }
    }
}

impl Workbench {
    pub fn active_activity(&self) -> ActivityItem {
        self.active_activity
    }

    pub fn select_activity(&mut self, item: ActivityItem) {
        self.active_activity = item;
    }

    pub fn side_bar_width(&self) -> f32 {
        self.side_bar_width
    }

    /// Sets the Side Bar width, clamped to the allowed range.
    pub fn resize_side_bar(&mut self, width: f32) {
        if width.is_finite() {
            self.side_bar_width = width.clamp(SIDE_BAR_MIN_WIDTH, SIDE_BAR_MAX_WIDTH);
        }
    }

    pub fn tabs(&self) -> &WorkbenchTabs {
        &self.tabs
    }

    pub fn select_tab(&mut self, id: TabId) {
        self.tabs.activate(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_items_are_ordered_explorer_search_git() {
        assert_eq!(
            ActivityItem::TOP_ITEMS,
            [
                ActivityItem::Explorer,
                ActivityItem::Search,
                ActivityItem::Git
            ]
        );
        assert_eq!(
            ActivityItem::ALL,
            [
                ActivityItem::Explorer,
                ActivityItem::Search,
                ActivityItem::Git,
                ActivityItem::Settings,
            ]
        );
    }

    #[test]
    fn explorer_is_selected_initially() {
        let workbench = Workbench::default();
        assert_eq!(workbench.active_activity(), ActivityItem::Explorer);
    }

    #[test]
    fn selecting_an_activity_changes_the_active_activity() {
        let mut workbench = Workbench::default();

        workbench.select_activity(ActivityItem::Search);
        assert_eq!(workbench.active_activity(), ActivityItem::Search);

        workbench.select_activity(ActivityItem::Git);
        assert_eq!(workbench.active_activity(), ActivityItem::Git);
    }

    /// The model must stay localization-neutral: display text belongs to the
    /// UI layer and is resolved through the localizer. This guards against
    /// string literals or i18n imports creeping into the model.
    #[test]
    fn model_holds_no_display_text() {
        let source = include_str!("workbench.rs");
        let model = &source[..source.find("#[cfg(test)]").expect("test module")];
        let model_lines = model
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"));

        for line in model_lines {
            assert!(!line.contains('"'), "string literal in model: {line}");
            assert!(!line.contains("i18n"), "i18n dependency in model: {line}");
        }
    }

    #[test]
    fn side_bar_width_is_clamped() {
        let mut workbench = Workbench::default();
        assert_eq!(workbench.side_bar_width(), SIDE_BAR_DEFAULT_WIDTH);

        workbench.resize_side_bar(10.0);
        assert_eq!(workbench.side_bar_width(), SIDE_BAR_MIN_WIDTH);

        workbench.resize_side_bar(10_000.0);
        assert_eq!(workbench.side_bar_width(), SIDE_BAR_MAX_WIDTH);

        workbench.resize_side_bar(300.0);
        assert_eq!(workbench.side_bar_width(), 300.0);

        workbench.resize_side_bar(f32::NAN);
        assert_eq!(workbench.side_bar_width(), 300.0);
    }

    #[test]
    fn starts_with_a_single_active_terminal_tab() {
        let workbench = Workbench::default();
        let tabs = workbench.tabs();

        assert_eq!(tabs.len(), 1);
        let active = tabs.active().expect("active tab");
        assert_eq!(active.kind, TabKind::Terminal);
        assert!(tabs.is_active(active.id));
    }

    #[test]
    fn selecting_an_unknown_tab_is_ignored() {
        let mut workbench = Workbench::default();
        let active = workbench.tabs().active().map(|tab| tab.id);
        let mut other = WorkbenchTabs::new();
        other.add(TabKind::Terminal);
        let unknown = other.add(TabKind::Terminal);

        workbench.select_tab(unknown);
        assert_eq!(workbench.tabs().active().map(|tab| tab.id), active);
    }
}
