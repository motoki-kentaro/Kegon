//! UI-independent workbench state.
//!
//! This module intentionally has no dependency on the GUI toolkit so that the
//! workbench model can be unit tested and later grow into real session and
//! panel management without being tangled with rendering code.

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
}

impl ActivityItem {
    /// All Activity Bar entries, in display order (top to bottom).
    pub const ALL: [Self; 3] = [Self::Explorer, Self::Search, Self::Git];

    /// Human-readable name, used for tooltips and accessibility.
    pub fn label(self) -> &'static str {
        match self {
            Self::Explorer => "File Explorer",
            Self::Search => "Search",
            Self::Git => "Git",
        }
    }

    /// Heading shown at the top of the Side Bar.
    pub fn side_bar_title(self) -> &'static str {
        match self {
            Self::Explorer => "EXPLORER",
            Self::Search => "SEARCH",
            Self::Git => "SOURCE CONTROL",
        }
    }
}

/// Identifies a terminal tab independently of its position in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

/// A tab in the terminal tab strip.
///
/// For now a tab is only a title. It is expected to own a terminal session
/// once PTY support exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalTab {
    pub id: TabId,
    pub title: String,
}

/// The terminal tab strip.
///
/// Invariant: there is always at least one tab, and `active` is a valid index.
#[derive(Debug, Clone)]
pub struct TerminalTabs {
    tabs: Vec<TerminalTab>,
    active: usize,
}

impl TerminalTabs {
    fn new() -> Self {
        Self {
            tabs: vec![TerminalTab {
                id: TabId(0),
                title: String::from("Terminal"),
            }],
            active: 0,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &TerminalTab> {
        self.tabs.iter()
    }

    pub fn active(&self) -> &TerminalTab {
        &self.tabs[self.active]
    }

    pub fn is_active(&self, id: TabId) -> bool {
        self.active().id == id
    }

    /// Activates the tab with the given id. Unknown ids are ignored.
    pub fn select(&mut self, id: TabId) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
            self.active = index;
        }
    }
}

/// The state of the whole workbench window.
#[derive(Debug, Clone)]
pub struct Workbench {
    active_activity: ActivityItem,
    side_bar_width: f32,
    terminal_tabs: TerminalTabs,
}

impl Default for Workbench {
    fn default() -> Self {
        Self {
            active_activity: ActivityItem::Explorer,
            side_bar_width: SIDE_BAR_DEFAULT_WIDTH,
            terminal_tabs: TerminalTabs::new(),
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

    pub fn terminal_tabs(&self) -> &TerminalTabs {
        &self.terminal_tabs
    }

    pub fn select_terminal_tab(&mut self, id: TabId) {
        self.terminal_tabs.select(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_items_are_ordered_explorer_search_git() {
        assert_eq!(
            ActivityItem::ALL,
            [
                ActivityItem::Explorer,
                ActivityItem::Search,
                ActivityItem::Git
            ]
        );
    }

    #[test]
    fn explorer_is_selected_initially() {
        let workbench = Workbench::default();
        assert_eq!(workbench.active_activity(), ActivityItem::Explorer);
        assert_eq!(workbench.active_activity().side_bar_title(), "EXPLORER");
    }

    #[test]
    fn selecting_an_activity_changes_the_side_bar_title() {
        let mut workbench = Workbench::default();

        workbench.select_activity(ActivityItem::Search);
        assert_eq!(workbench.active_activity().side_bar_title(), "SEARCH");

        workbench.select_activity(ActivityItem::Git);
        assert_eq!(
            workbench.active_activity().side_bar_title(),
            "SOURCE CONTROL"
        );
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
        let tabs = workbench.terminal_tabs();

        assert_eq!(tabs.iter().count(), 1);
        assert_eq!(tabs.active().title, "Terminal");
        assert!(tabs.is_active(tabs.active().id));
    }

    #[test]
    fn selecting_an_unknown_tab_is_ignored() {
        let mut workbench = Workbench::default();
        let active = workbench.terminal_tabs().active().id;

        workbench.select_terminal_tab(TabId(42));
        assert_eq!(workbench.terminal_tabs().active().id, active);
    }
}
