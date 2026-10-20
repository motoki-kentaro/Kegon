//! UI-independent workbench state.
//!
//! This module intentionally has no dependency on the GUI toolkit so that the
//! workbench model can be unit tested and later grow into real session and
//! panel management without being tangled with rendering code.

mod tabs;

pub use tabs::{TabId, TabKind, WorkbenchTabs};

/// The state of the whole workbench window.
#[derive(Debug, Clone)]
pub struct Workbench {
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
            tabs: Self::initial_tabs(),
        }
    }
}

impl Workbench {
    pub fn tabs(&self) -> &WorkbenchTabs {
        &self.tabs
    }

    /// Appends a tab without changing which one is active (the first tab of an empty strip
    /// becomes active).
    pub fn add_tab(&mut self, kind: TabKind) -> TabId {
        self.tabs.add(kind)
    }

    /// Opens a tab of a non-terminal kind and activates it. A singleton kind that is already open
    /// is activated instead of being added again. Returns the tab and whether it is new.
    pub fn open_special_tab(&mut self, kind: TabKind) -> (TabId, bool) {
        if kind.is_singleton()
            && let Some(existing) = self.tabs.find_by_kind(kind)
        {
            self.tabs.activate(existing);
            return (existing, false);
        }
        let id = self.tabs.add(kind);
        self.tabs.activate(id);
        (id, true)
    }

    /// Removes a tab; the active tab follows the rules of [`WorkbenchTabs::remove`].
    pub fn remove_tab(&mut self, id: TabId) -> bool {
        self.tabs.remove(id).is_some()
    }

    pub fn select_tab(&mut self, id: TabId) {
        self.tabs.activate(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn starts_with_a_single_active_terminal_tab() {
        let workbench = Workbench::default();
        let tabs = workbench.tabs();

        assert_eq!(tabs.len(), 1);
        let active = tabs.active().expect("active tab");
        assert_eq!(active.kind, TabKind::Terminal);
        assert!(tabs.is_active(active.id));
    }

    fn kinds(workbench: &Workbench) -> Vec<TabKind> {
        workbench.tabs().iter().map(|tab| tab.kind).collect()
    }

    #[test]
    fn settings_coexists_with_terminals_and_opens_once() {
        let mut workbench = Workbench::default();
        let second = workbench.add_tab(TabKind::Terminal);

        let (settings, is_new) = workbench.open_special_tab(TabKind::Settings);
        assert!(is_new);
        assert_eq!(
            kinds(&workbench),
            [TabKind::Terminal, TabKind::Terminal, TabKind::Settings]
        );
        assert!(workbench.tabs().is_active(settings));

        // Terminal -> Settings -> Terminal switching keeps order and ids.
        workbench.select_tab(second);
        assert!(workbench.tabs().is_active(second));

        // Opening again re-selects the existing tab instead of adding a second one.
        let (again, is_new) = workbench.open_special_tab(TabKind::Settings);
        assert!(!is_new);
        assert_eq!(again, settings);
        assert_eq!(workbench.tabs().len(), 3);
        assert!(workbench.tabs().is_active(settings));
    }

    #[test]
    fn closing_settings_activates_a_neighbour_and_keeps_terminals() {
        let mut workbench = Workbench::default();
        let second = workbench.add_tab(TabKind::Terminal);
        let (settings, _) = workbench.open_special_tab(TabKind::Settings);

        assert!(workbench.remove_tab(settings));
        assert_eq!(kinds(&workbench), [TabKind::Terminal, TabKind::Terminal]);
        assert!(
            workbench.tabs().is_active(second),
            "left neighbour when last"
        );
    }

    #[test]
    fn settings_can_remain_alone_and_reopen_after_close() {
        let mut workbench = Workbench::default();
        let terminal = workbench.tabs().active().map(|tab| tab.id).unwrap();
        let (settings, _) = workbench.open_special_tab(TabKind::Settings);

        assert!(workbench.remove_tab(terminal));
        assert_eq!(kinds(&workbench), [TabKind::Settings]);
        assert!(workbench.tabs().is_active(settings));

        assert!(workbench.remove_tab(settings));
        assert!(workbench.tabs().is_empty());

        let (reopened, is_new) = workbench.open_special_tab(TabKind::Settings);
        assert!(is_new);
        assert_ne!(reopened, settings, "ids are never reused");
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
