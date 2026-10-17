//! Generic workbench tabs: identity, kind, order, and the active tab.
//!
//! This module knows nothing about terminal sessions, PTYs, or rendering. A
//! tab is only metadata; whatever a tab shows (a terminal session today, other
//! content later) is owned elsewhere and looked up by [`TabId`].

/// Identifies a tab independently of its position in the strip.
///
/// Distinct from any terminal session identifier. Ids are issued by
/// [`WorkbenchTabs`] and never reused within one collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

/// What a tab hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabKind {
    Terminal,
}

/// Metadata common to every tab.
///
/// Holds no display text: the UI labels tabs through the localizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchTab {
    pub id: TabId,
    pub kind: TabKind,
}

/// Ordered tabs plus the active one.
///
/// Invariants: ids are unique, and `active` is `Some` exactly when the
/// collection is non-empty, always pointing at an existing tab.
#[derive(Debug, Clone, Default)]
pub struct WorkbenchTabs {
    tabs: Vec<WorkbenchTab>,
    active: Option<TabId>,
    next_id: u64,
}

impl WorkbenchTabs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a tab and returns its id. The first tab becomes active; later
    /// additions leave the active tab unchanged.
    pub fn add(&mut self, kind: TabKind) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        self.tabs.push(WorkbenchTab { id, kind });
        self.active.get_or_insert(id);
        id
    }

    /// Activates the tab with the given id. Returns whether it existed.
    pub fn activate(&mut self, id: TabId) -> bool {
        let exists = self.get(id).is_some();
        if exists {
            self.active = Some(id);
        }
        exists
    }

    /// Removes a tab and returns it, or `None` for an unknown id.
    ///
    /// Removing the active tab activates its right neighbour, or its left
    /// neighbour when it was last. Removing any other tab keeps the active one.
    pub fn remove(&mut self, id: TabId) -> Option<WorkbenchTab> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let removed = self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|left| self.tabs.get(left)))
                .map(|tab| tab.id);
        }
        Some(removed)
    }

    pub fn get(&self, id: TabId) -> Option<&WorkbenchTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn active(&self) -> Option<&WorkbenchTab> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn is_active(&self, id: TabId) -> bool {
        self.active == Some(id)
    }

    /// Tabs in display order.
    pub fn iter(&self) -> impl Iterator<Item = &WorkbenchTab> {
        self.tabs.iter()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(tabs: &WorkbenchTabs) -> Vec<TabId> {
        tabs.iter().map(|tab| tab.id).collect()
    }

    fn assert_invariants(tabs: &WorkbenchTabs) {
        let all = ids(tabs);
        for (i, id) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(id), "duplicate id {id:?}");
        }
        match tabs.active() {
            Some(active) => assert!(all.contains(&active.id)),
            None => assert!(all.is_empty()),
        }
        assert_eq!(tabs.len(), all.len());
    }

    fn with_tabs(n: usize) -> (WorkbenchTabs, Vec<TabId>) {
        let mut tabs = WorkbenchTabs::new();
        let added = (0..n).map(|_| tabs.add(TabKind::Terminal)).collect();
        (tabs, added)
    }

    #[test]
    fn adds_to_an_empty_state_and_activates_the_first_tab() {
        let mut tabs = WorkbenchTabs::new();
        assert!(tabs.is_empty());
        assert!(tabs.active().is_none());

        let id = tabs.add(TabKind::Terminal);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active().map(|tab| tab.id), Some(id));
        assert_eq!(tabs.get(id).map(|tab| tab.kind), Some(TabKind::Terminal));
        assert_invariants(&tabs);
    }

    #[test]
    fn keeps_tabs_in_insertion_order_without_moving_the_active_tab() {
        let (tabs, added) = with_tabs(3);
        assert_eq!(ids(&tabs), added);
        assert!(tabs.is_active(added[0]));
        assert_invariants(&tabs);
    }

    #[test]
    fn ids_are_unique_even_after_removal() {
        let (mut tabs, added) = with_tabs(3);
        tabs.remove(added[2]);
        let fresh = tabs.add(TabKind::Terminal);
        assert!(!added.contains(&fresh));
        assert_invariants(&tabs);
    }

    #[test]
    fn activates_a_specific_tab() {
        let (mut tabs, added) = with_tabs(3);
        assert!(tabs.activate(added[2]));
        assert_eq!(tabs.active().map(|tab| tab.id), Some(added[2]));
        assert!(tabs.is_active(added[2]));
        assert!(!tabs.is_active(added[0]));
    }

    #[test]
    fn unknown_ids_leave_the_state_untouched() {
        let (mut tabs, added) = with_tabs(2);
        let (_, others) = with_tabs(3);
        // Issued by another collection and never present in `tabs`.
        let unknown = others[2];

        assert!(!tabs.activate(unknown));
        assert!(tabs.remove(unknown).is_none());
        assert!(tabs.get(unknown).is_none());
        assert_eq!(ids(&tabs), added);
        assert!(tabs.is_active(added[0]));
        assert_invariants(&tabs);

        let mut empty = WorkbenchTabs::new();
        assert!(!empty.activate(unknown));
        assert!(empty.remove(unknown).is_none());
        assert_invariants(&empty);
    }

    #[test]
    fn removing_an_inactive_tab_keeps_the_active_tab() {
        let (mut tabs, added) = with_tabs(3);
        tabs.activate(added[1]);

        assert_eq!(tabs.remove(added[0]).map(|tab| tab.id), Some(added[0]));
        assert!(tabs.is_active(added[1]));
        tabs.remove(added[2]);
        assert!(tabs.is_active(added[1]));
        assert_invariants(&tabs);
    }

    #[test]
    fn removing_the_active_tab_selects_the_right_neighbour() {
        let (mut tabs, added) = with_tabs(3);
        tabs.activate(added[1]);
        tabs.remove(added[1]);
        assert!(tabs.is_active(added[2]));

        let (mut tabs, added) = with_tabs(3);
        tabs.remove(added[0]);
        assert!(tabs.is_active(added[1]));
        assert_invariants(&tabs);
    }

    #[test]
    fn removing_the_active_last_tab_selects_the_left_neighbour() {
        let (mut tabs, added) = with_tabs(3);
        tabs.activate(added[2]);
        tabs.remove(added[2]);
        assert!(tabs.is_active(added[1]));
        assert_invariants(&tabs);
    }

    #[test]
    fn removing_the_only_tab_empties_the_state() {
        let (mut tabs, added) = with_tabs(1);
        tabs.remove(added[0]);
        assert!(tabs.is_empty());
        assert!(tabs.active().is_none());
        assert_invariants(&tabs);

        // The collection is usable again afterwards.
        let id = tabs.add(TabKind::Terminal);
        assert!(tabs.is_active(id));
        assert_ne!(id, added[0]);
    }

    #[test]
    fn invariants_hold_across_repeated_adds_and_removals() {
        let mut tabs = WorkbenchTabs::new();
        let mut seen = Vec::new();
        for round in 0..50usize {
            let id = tabs.add(TabKind::Terminal);
            assert!(!seen.contains(&id));
            seen.push(id);
            if round % 3 == 0 {
                tabs.activate(id);
            }
            if round % 2 == 1 {
                let all = ids(&tabs);
                tabs.remove(all[round % all.len()]);
            }
            assert_invariants(&tabs);
        }
        while let Some(id) = ids(&tabs).first().copied() {
            tabs.remove(id);
            assert_invariants(&tabs);
        }
        assert!(tabs.is_empty());
    }
}
