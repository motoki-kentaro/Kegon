//! Owns every terminal session, keyed by the workbench tab that shows it.
//!
//! The workbench tab model only knows order, kind, and selection; the sessions live here so that
//! other tab kinds need no terminal state. Ids are never reused by the tab model, so an event that
//! arrives after its tab was removed can never reach a different session.

use std::collections::HashMap;

use crate::terminal::session::{TerminalEvent, TerminalSession};
use crate::terminal::title::normalize_terminal_title;
use crate::workbench::TabId;

/// A terminal session plus the per-terminal state derived from its events.
pub struct TerminalEntry {
    session: TerminalSession,
    /// Normalized title set by the child, if any. `None` shows the default label.
    title: Option<String>,
}

impl TerminalEntry {
    pub fn session(&self) -> &TerminalSession {
        &self.session
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn apply(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::Title(raw) => self.title = normalize_terminal_title(&raw),
            TerminalEvent::ResetTitle => self.title = None,
            // Redraw happens because a message was processed; the last title is kept after exit
            // since the terminal contents stay on screen.
            TerminalEvent::Wakeup | TerminalEvent::ChildExit(_) => {}
        }
    }
}

/// Terminal sessions by [`TabId`]. Removing an entry drops its session, which shuts down the PTY.
#[derive(Default)]
pub struct TerminalRegistry {
    entries: HashMap<TabId, TerminalEntry>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a session for `tab`. An already registered tab is left untouched and the
    /// rejected session is handed back.
    pub fn insert(&mut self, tab: TabId, session: TerminalSession) -> Result<(), TerminalSession> {
        if self.entries.contains_key(&tab) {
            return Err(session);
        }
        self.entries.insert(
            tab,
            TerminalEntry {
                session,
                title: None,
            },
        );
        Ok(())
    }

    pub fn get(&self, tab: TabId) -> Option<&TerminalEntry> {
        self.entries.get(&tab)
    }

    #[allow(dead_code)]
    pub fn get_mut(&mut self, tab: TabId) -> Option<&mut TerminalEntry> {
        self.entries.get_mut(&tab)
    }

    #[allow(dead_code)]
    pub fn contains(&self, tab: TabId) -> bool {
        self.entries.contains_key(&tab)
    }

    /// Removes the entry for `tab`; dropping the returned entry shuts its session down.
    #[allow(dead_code)]
    pub fn remove(&mut self, tab: TabId) -> Option<TerminalEntry> {
        self.entries.remove(&tab)
    }

    /// Every registered session, in no particular order.
    pub fn sessions(&self) -> impl Iterator<Item = (TabId, &TerminalSession)> {
        self.entries
            .iter()
            .map(|(tab, entry)| (*tab, &entry.session))
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Routes an event to the session of `tab`. Returns `false` and changes nothing when the tab
    /// has no session (for example a late event from a removed terminal).
    pub fn handle_event(&mut self, tab: TabId, event: TerminalEvent) -> bool {
        match self.entries.get_mut(&tab) {
            Some(entry) => {
                entry.apply(event);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::{TabKind, WorkbenchTabs};
    use iced::futures::channel::mpsc::unbounded;

    fn session() -> TerminalSession {
        let (tx, _rx) = unbounded();
        TerminalSession::spawn(80, 24, 10, 20, tx).unwrap()
    }

    fn tab_ids(n: usize) -> Vec<TabId> {
        let mut tabs = WorkbenchTabs::new();
        (0..n).map(|_| tabs.add(TabKind::Terminal)).collect()
    }

    fn title(registry: &TerminalRegistry, tab: TabId) -> Option<String> {
        registry
            .get(tab)
            .and_then(|e| e.title())
            .map(str::to_string)
    }

    #[test]
    fn registers_and_looks_up_sessions_by_tab() {
        let ids = tab_ids(3);
        let mut registry = TerminalRegistry::new();
        for id in &ids[..2] {
            assert!(registry.insert(*id, session()).is_ok());
        }

        assert_eq!(registry.len(), 2);
        assert!(registry.contains(ids[0]) && registry.contains(ids[1]));
        assert!(!registry.contains(ids[2]));
        assert!(registry.get(ids[2]).is_none());
        assert!(registry.get_mut(ids[2]).is_none());
        assert_eq!(registry.sessions().count(), 2);
    }

    #[test]
    fn sessions_are_distinguished_by_tab() {
        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.insert(ids[1], session()).ok().unwrap();

        registry
            .get(ids[0])
            .unwrap()
            .session()
            .resize(100, 30, 10, 20);
        let size = |tab| {
            let term = registry.get(tab).unwrap().session().term().lock();
            (term.columns(), term.screen_lines())
        };
        use alacritty_terminal::grid::Dimensions;
        assert_eq!(size(ids[0]), (100, 30));
        assert_eq!(size(ids[1]), (80, 24));
    }

    #[test]
    fn duplicate_registration_is_rejected_without_replacing() {
        let ids = tab_ids(1);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.handle_event(ids[0], TerminalEvent::Title("first".into()));

        assert!(registry.insert(ids[0], session()).is_err());
        assert_eq!(registry.len(), 1);
        assert_eq!(title(&registry, ids[0]).as_deref(), Some("first"));
    }

    #[test]
    fn unknown_tabs_fail_safely() {
        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();

        assert!(registry.remove(ids[1]).is_none());
        assert!(!registry.handle_event(ids[1], TerminalEvent::Title("x".into())));
        assert_eq!(registry.len(), 1);
        assert_eq!(title(&registry, ids[0]), None);
    }

    #[test]
    fn removal_does_not_affect_other_sessions() {
        let ids = tab_ids(3);
        let mut registry = TerminalRegistry::new();
        for id in &ids {
            registry.insert(*id, session()).ok().unwrap();
            registry.handle_event(*id, TerminalEvent::Title(format!("{id:?}")));
        }

        assert!(registry.remove(ids[1]).is_some());
        assert!(!registry.contains(ids[1]));
        assert_eq!(title(&registry, ids[0]), Some(format!("{:?}", ids[0])));
        assert_eq!(title(&registry, ids[2]), Some(format!("{:?}", ids[2])));
    }

    #[test]
    fn events_update_only_the_originating_tab() {
        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.insert(ids[1], session()).ok().unwrap();

        assert!(registry.handle_event(ids[1], TerminalEvent::Title("background".into())));
        assert_eq!(title(&registry, ids[0]), None);
        assert_eq!(title(&registry, ids[1]).as_deref(), Some("background"));

        registry.handle_event(ids[0], TerminalEvent::Title("front".into()));
        registry.handle_event(ids[1], TerminalEvent::ResetTitle);
        assert_eq!(title(&registry, ids[0]).as_deref(), Some("front"));
        assert_eq!(title(&registry, ids[1]), None);

        // Wakeup and exit leave titles alone.
        registry.handle_event(ids[0], TerminalEvent::ChildExit(Some(0)));
        registry.handle_event(ids[0], TerminalEvent::Wakeup);
        assert_eq!(title(&registry, ids[0]).as_deref(), Some("front"));
    }

    #[test]
    fn late_events_after_removal_are_ignored() {
        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.insert(ids[1], session()).ok().unwrap();
        registry.remove(ids[0]);

        assert!(!registry.handle_event(ids[0], TerminalEvent::Title("late".into())));
        assert_eq!(title(&registry, ids[1]), None);
    }

    #[test]
    fn repeated_insert_and_remove_keeps_the_registry_consistent() {
        let mut tabs = WorkbenchTabs::new();
        let mut registry = TerminalRegistry::new();
        let mut live = Vec::new();
        for round in 0..8 {
            let id = tabs.add(TabKind::Terminal);
            registry.insert(id, session()).ok().unwrap();
            live.push(id);
            if round % 2 == 1 {
                let gone = live.remove(round % live.len());
                assert!(registry.remove(gone).is_some());
                assert!(registry.remove(gone).is_none());
            }
            assert_eq!(registry.len(), live.len());
            assert!(live.iter().all(|id| registry.contains(*id)));
        }
    }

    #[test]
    fn exited_sessions_ignore_input_and_resize() {
        let ids = tab_ids(1);
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn(80, 24, 10, 20, tx).unwrap();
        session.write_input(b"exit\r\n".to_vec());
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session).ok().unwrap();

        // Wait for the shell to exit on its own; the entry stays registered.
        let entry = registry.get(ids[0]).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !entry.session().is_exited() {
            assert!(
                std::time::Instant::now() < deadline,
                "shell did not exit on `exit`"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        entry.session().resize(120, 40, 10, 20);
        use alacritty_terminal::grid::Dimensions;
        assert_eq!(entry.session().term().lock().columns(), 80);
        assert!(registry.contains(ids[0]));
    }
}
