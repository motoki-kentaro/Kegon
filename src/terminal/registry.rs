//! Owns every terminal session, keyed by the workbench tab that shows it.
//!
//! The workbench tab model only knows order, kind, and selection; the sessions live here so that
//! other tab kinds need no terminal state. Ids are never reused by the tab model, so an event that
//! arrives after its tab was removed can never reach a different session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::terminal::integration::{
    CommandExecutionResult, CommandState, CommandStateMachine, ShellIntegrationStatus,
    VerifiedCommandEvent,
};
use crate::terminal::session::{TerminalEvent, TerminalSession};
use crate::terminal::shell::ShellIntegrationLaunch;
use crate::terminal::title::normalize_terminal_title;
use crate::workbench::TabId;

/// A terminal session plus the per-terminal state derived from its events.
pub struct TerminalEntry {
    session: TerminalSession,
    /// Normalized title set by the child, if any. `None` shows the default label.
    title: Option<String>,
    /// The directory the shell last reported through Shell Integration. `None` means unknown:
    /// never filled from a guess or from the directory the session was started in, since the
    /// shell may have changed directory since without being able to tell us.
    reported_cwd: Option<PathBuf>,
    /// Command execution state machine tracking OSC 133 shell integration.
    command_state: CommandStateMachine,
}

impl TerminalEntry {
    pub fn session(&self) -> &TerminalSession {
        &self.session
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The last directory the shell reported, if it has reported any.
    pub fn reported_cwd(&self) -> Option<&Path> {
        self.reported_cwd.as_deref()
    }

    /// Returns the current command execution state for this terminal.
    pub fn command_state(&self) -> &CommandState {
        self.command_state.state()
    }

    /// Returns a reference to the command state machine for this terminal.
    pub fn command_state_machine(&self) -> &CommandStateMachine {
        &self.command_state
    }

    /// Returns a mutable reference to the command state machine for this terminal.
    #[allow(dead_code)]
    pub fn command_state_machine_mut(&mut self) -> &mut CommandStateMachine {
        &mut self.command_state
    }

    /// Returns the executable path or name of the shell running in this terminal.
    pub fn shell_program(&self) -> &str {
        self.session.shell_program()
    }

    /// Returns the session ID generated for this terminal's PTY session, if any.
    pub fn session_id(&self) -> Option<&str> {
        self.session.session_id()
    }

    /// Derives the current Shell Integration status based on launch configuration,
    /// received OSC 133 markers, and process exit status.
    pub fn shell_integration_status(&self) -> ShellIntegrationStatus {
        match self.session.integration_launch() {
            ShellIntegrationLaunch::Disabled => ShellIntegrationStatus::Disabled,
            ShellIntegrationLaunch::Unsupported => ShellIntegrationStatus::Unsupported,
            ShellIntegrationLaunch::Injected => {
                if self.command_state.has_verified_command_execution() {
                    ShellIntegrationStatus::Active
                } else if self.command_state.has_verified_prompt() {
                    ShellIntegrationStatus::ActivePromptOnly
                } else if self.session.is_exited() {
                    ShellIntegrationStatus::Unavailable
                } else {
                    ShellIntegrationStatus::Waiting
                }
            }
        }
    }

    /// Returns the comprehensive execution result of the last finished command, if any.
    #[allow(dead_code)]
    pub fn last_execution_result(&self) -> Option<CommandExecutionResult> {
        self.command_state.last_execution_result()
    }

    pub fn apply(&mut self, event: TerminalEvent) -> Option<VerifiedCommandEvent> {
        match event {
            TerminalEvent::Title(raw) => {
                self.title = normalize_terminal_title(&raw);
                None
            }
            TerminalEvent::ResetTitle => {
                self.title = None;
                None
            }
            TerminalEvent::WorkingDirectory(path) => {
                self.reported_cwd = Some(path);
                None
            }
            TerminalEvent::ShellIntegration(marker) => self.command_state.transition(marker),
            TerminalEvent::ChildExit(_) => self.command_state.on_shell_exit(),
            // Redraw happens because a message was processed; the last title is kept after exit
            // since the terminal contents stay on screen.
            TerminalEvent::Wakeup => None,
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
    #[allow(clippy::result_large_err)]
    pub fn insert(&mut self, tab: TabId, session: TerminalSession) -> Result<(), TerminalSession> {
        if self.entries.contains_key(&tab) {
            return Err(session);
        }
        let mut command_state = CommandStateMachine::new();
        if let Some(sid) = session.session_id() {
            command_state.set_expected_session_id(Some(sid.to_string()));
        }
        self.entries.insert(
            tab,
            TerminalEntry {
                session,
                title: None,
                reported_cwd: None,
                command_state,
            },
        );
        Ok(())
    }

    pub fn get(&self, tab: TabId) -> Option<&TerminalEntry> {
        self.entries.get(&tab)
    }

    #[allow(dead_code)] // Part of the session-management API; no caller needs it yet.
    pub fn get_mut(&mut self, tab: TabId) -> Option<&mut TerminalEntry> {
        self.entries.get_mut(&tab)
    }

    /// Returns the command state of `tab` if registered.
    #[allow(dead_code)]
    pub fn command_state(&self, tab: TabId) -> Option<&CommandState> {
        self.entries.get(&tab).map(TerminalEntry::command_state)
    }

    /// Returns a reference to the command state machine of `tab` if registered.
    #[allow(dead_code)]
    pub fn command_state_machine(&self, tab: TabId) -> Option<&CommandStateMachine> {
        self.entries
            .get(&tab)
            .map(TerminalEntry::command_state_machine)
    }

    /// Returns a mutable reference to the command state machine of `tab` if registered.
    #[allow(dead_code)]
    pub fn command_state_machine_mut(&mut self, tab: TabId) -> Option<&mut CommandStateMachine> {
        self.entries
            .get_mut(&tab)
            .map(TerminalEntry::command_state_machine_mut)
    }

    /// Returns the derived Shell Integration status for `tab` if registered.
    #[allow(dead_code)]
    pub fn shell_integration_status(&self, tab: TabId) -> Option<ShellIntegrationStatus> {
        self.entries
            .get(&tab)
            .map(TerminalEntry::shell_integration_status)
    }

    /// Returns the shell program name for `tab` if registered.
    #[allow(dead_code)]
    pub fn shell_program(&self, tab: TabId) -> Option<&str> {
        self.entries.get(&tab).map(TerminalEntry::shell_program)
    }

    /// Returns the session ID for `tab` if registered and active.
    #[allow(dead_code)]
    pub fn session_id(&self, tab: TabId) -> Option<&str> {
        self.entries.get(&tab).and_then(TerminalEntry::session_id)
    }

    #[cfg(test)]
    pub fn contains(&self, tab: TabId) -> bool {
        self.entries.contains_key(&tab)
    }

    /// Removes the entry for `tab`; dropping the returned entry shuts its session down.
    pub fn remove(&mut self, tab: TabId) -> Option<TerminalEntry> {
        self.entries.remove(&tab)
    }

    /// Every registered session, in no particular order.
    pub fn sessions(&self) -> impl Iterator<Item = (TabId, &TerminalSession)> {
        self.entries
            .iter()
            .map(|(tab, entry)| (*tab, &entry.session))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the comprehensive execution result of the last finished command for `tab`, if any.
    #[allow(dead_code)]
    pub fn last_execution_result(&self, tab: TabId) -> Option<CommandExecutionResult> {
        self.entries
            .get(&tab)
            .and_then(TerminalEntry::last_execution_result)
    }

    /// Handles an incoming terminal event, returning whether the tab was found and
    /// any verified command lifecycle event generated by the transition.
    pub fn handle_event_with_command_event(
        &mut self,
        tab: TabId,
        event: TerminalEvent,
    ) -> (bool, Option<VerifiedCommandEvent>) {
        match self.entries.get_mut(&tab) {
            Some(entry) => {
                let cmd_event = entry.apply(event);
                (true, cmd_event)
            }
            None => (false, None),
        }
    }

    /// Routes an event to the session of `tab`. Returns `false` and changes nothing when the tab
    /// has no session (for example a late event from a removed terminal).
    #[allow(dead_code)]
    pub fn handle_event(&mut self, tab: TabId, event: TerminalEvent) -> bool {
        self.handle_event_with_command_event(tab, event).0
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

    #[test]
    fn cwd_is_unknown_until_reported_and_tracked_per_tab() {
        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.insert(ids[1], session()).ok().unwrap();
        assert!(registry.get(ids[0]).unwrap().reported_cwd().is_none());

        let dir = PathBuf::from(r"C:\work dir\日本語");
        assert!(registry.handle_event(ids[1], TerminalEvent::WorkingDirectory(dir.clone())));
        assert!(registry.get(ids[0]).unwrap().reported_cwd().is_none());
        assert_eq!(
            registry.get(ids[1]).unwrap().reported_cwd(),
            Some(dir.as_path())
        );

        registry.handle_event(
            ids[1],
            TerminalEvent::WorkingDirectory(PathBuf::from(r"D:\x")),
        );
        assert_eq!(
            registry.get(ids[1]).unwrap().reported_cwd(),
            Some(Path::new(r"D:\x"))
        );
        // Title and exit events never touch it.
        registry.handle_event(ids[1], TerminalEvent::ResetTitle);
        registry.handle_event(ids[1], TerminalEvent::ChildExit(Some(0)));
        assert_eq!(
            registry.get(ids[1]).unwrap().reported_cwd(),
            Some(Path::new(r"D:\x"))
        );
    }

    #[test]
    fn cwd_reports_for_removed_tabs_are_ignored() {
        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.remove(ids[0]);
        assert!(!registry.handle_event(
            ids[0],
            TerminalEvent::WorkingDirectory(PathBuf::from(r"C:\x"))
        ));
    }

    #[test]
    fn shell_integration_state_machine_tab_isolation_and_lifecycle() {
        use crate::terminal::integration::{
            CommandExecutionResult, CommandResult, CommandState, CompletionReason,
            MetadataCorrelation, Osc133Marker, ShellReportedStatus,
        };

        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.insert(ids[1], session()).ok().unwrap();

        assert_eq!(registry.command_state(ids[0]), Some(&CommandState::Unknown));
        assert_eq!(registry.command_state(ids[1]), Some(&CommandState::Unknown));

        // Advance tab 0: A -> B -> C
        registry.handle_event(
            ids[0],
            TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
        );
        assert_eq!(registry.command_state(ids[0]), Some(&CommandState::Prompt));
        // Tab 1 remains completely isolated
        assert_eq!(registry.command_state(ids[1]), Some(&CommandState::Unknown));

        registry.handle_event(
            ids[0],
            TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
        );
        assert_eq!(registry.command_state(ids[0]), Some(&CommandState::Editing));

        registry.handle_event(
            ids[0],
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
        );
        assert_eq!(
            registry.command_state(ids[0]),
            Some(&CommandState::Executing { command_id: 1 })
        );
        assert_eq!(registry.command_state(ids[1]), Some(&CommandState::Unknown));

        // Finish tab 0 command with exit code 0
        registry.handle_event(
            ids[0],
            TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
        );
        assert_eq!(
            registry.command_state(ids[0]),
            Some(&CommandState::Finished {
                command_id: 1,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    Some(0),
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Unprovided,
                ),
            })
        );

        // Advance tab 1 into Executing
        registry.handle_event(
            ids[1],
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
        );
        assert_eq!(
            registry.command_state(ids[1]),
            Some(&CommandState::Executing { command_id: 1 })
        );

        // Child exit on tab 1 interrupts active command as Unknown (does NOT borrow shell exit code)
        registry.handle_event(ids[1], TerminalEvent::ChildExit(Some(42)));
        assert_eq!(
            registry.command_state(ids[1]),
            Some(&CommandState::Finished {
                command_id: 1,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    None,
                    None,
                    CompletionReason::ShellExited,
                    MetadataCorrelation::Unprovided,
                ),
            })
        );

        // Removing a tab drops its entry and state
        registry.remove(ids[0]);
        assert_eq!(registry.command_state(ids[0]), None);
    }

    #[test]
    fn shell_integration_status_transitions_and_isolation() {
        use crate::terminal::integration::{
            CommandResult, CommandState, Osc133Marker, ShellIntegrationStatus,
        };

        let ids = tab_ids(2);
        let mut registry = TerminalRegistry::new();
        registry.insert(ids[0], session()).ok().unwrap();
        registry.insert(ids[1], session()).ok().unwrap();

        let status0 = registry.shell_integration_status(ids[0]).unwrap();
        if status0 == ShellIntegrationStatus::Waiting {
            // A marker -> ActivePromptOnly
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::ActivePromptOnly)
            );
            // Tab 1 remains Waiting
            assert_eq!(
                registry.shell_integration_status(ids[1]),
                Some(ShellIntegrationStatus::Waiting)
            );

            // B marker -> Still ActivePromptOnly
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::InputStarted),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::ActivePromptOnly)
            );

            // C marker -> Full Active
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::Active)
            );

            // D marker -> Still Active, last_result recorded
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::Active)
            );
            assert_eq!(
                registry
                    .get(ids[0])
                    .unwrap()
                    .command_state_machine()
                    .last_result(),
                Some(CommandResult::Success)
            );

            // Tab 1 was untouched
            assert_eq!(
                registry.shell_integration_status(ids[1]),
                Some(ShellIntegrationStatus::Waiting)
            );
        }
    }

    #[test]
    fn shell_integration_status_disabled_session() {
        use crate::settings::{ShellIntegrationMode, TerminalSettings};
        use crate::terminal::integration::ShellIntegrationStatus;

        let (tx, _rx) = unbounded();
        let settings = TerminalSettings {
            shell_integration: ShellIntegrationMode::Disabled,
            ..TerminalSettings::default()
        };
        let session =
            TerminalSession::spawn_with_settings(&settings, 80, 24, 10, 20, tx, None).unwrap();
        let mut registry = TerminalRegistry::new();
        let ids = tab_ids(1);
        registry.insert(ids[0], session).ok().unwrap();

        assert_eq!(
            registry.shell_integration_status(ids[0]),
            Some(ShellIntegrationStatus::Disabled)
        );
    }

    #[cfg(windows)]
    #[test]
    fn orphaned_d_does_not_promote_shell_integration_to_active() {
        use crate::terminal::integration::{Osc133Marker, ShellIntegrationStatus};
        use crate::terminal::shell::{Launch, ShellConfig, installed};

        let ids = tab_ids(1);
        let mut registry = TerminalRegistry::new();
        // Explicit launch: neither the user's integration environment variable nor shell
        // auto-detection may silently skip this regression. Its real events are not applied.
        let launch = Launch::new(
            ShellConfig {
                program: installed::windows_powershell().expect("Windows PowerShell is required"),
                args: vec![],
            },
            true,
            &crate::terminal::cwd::local_host_name(),
            None,
        );
        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn_launch(launch, 80, 24, 10, 20, tx).unwrap();
        registry.insert(ids[0], session).ok().unwrap();
        assert_eq!(
            registry.shell_integration_status(ids[0]),
            Some(ShellIntegrationStatus::Waiting)
        );
        {
            // 1. Orphaned D received without C: must NOT become Active; stays Waiting
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::Waiting),
                "Orphaned D alone must not promote to Active"
            );

            // 2. Prompt A received: becomes ActivePromptOnly
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::PromptStarted),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::ActivePromptOnly)
            );

            // 3. Another orphaned D received: still ActivePromptOnly, NOT Active
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::ActivePromptOnly),
                "Orphaned D after prompt must not promote to Active"
            );

            // 4. Legitimate C received: now promoted to Active
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::CommandStarted),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::Active)
            );

            // 5. Normal D finishes command: remains Active
            registry.handle_event(
                ids[0],
                TerminalEvent::ShellIntegration(Osc133Marker::CommandFinished(Some(0))),
            );
            assert_eq!(
                registry.shell_integration_status(ids[0]),
                Some(ShellIntegrationStatus::Active)
            );
        }
    }

    #[test]
    fn multi_tab_sid_wiring_isolation_and_lifecycle() {
        use crate::terminal::integration::{Osc133Marker, Osc133Metadata, ShellReportedStatus};
        use crate::terminal::shell::mock::injected_launch;

        let mut registry = TerminalRegistry::new();
        let tabs = tab_ids(2);

        // Spawn Tab 0 with an injected launch (PowerShell on Windows)
        let launch0 = injected_launch("PC");
        let sid0 = launch0
            .session_id()
            .expect("launch0 must have session_id")
            .to_string();
        let (tx0, _rx0) = unbounded();
        let session0 = TerminalSession::spawn_launch(launch0, 80, 24, 10, 20, tx0).unwrap();
        registry.insert(tabs[0], session0).ok().unwrap();

        // Spawn Tab 1 with an injected launch (PowerShell on Windows)
        let launch1 = injected_launch("PC");
        let sid1 = launch1
            .session_id()
            .expect("launch1 must have session_id")
            .to_string();
        let (tx1, _rx1) = unbounded();
        let session1 = TerminalSession::spawn_launch(launch1, 80, 24, 10, 20, tx1).unwrap();
        registry.insert(tabs[1], session1).ok().unwrap();

        // 1. Verify SIDs are distinct and non-empty
        assert_ne!(sid0, sid1);
        assert_eq!(registry.session_id(tabs[0]), Some(sid0.as_str()));
        assert_eq!(registry.session_id(tabs[1]), Some(sid1.as_str()));

        // 2. Verify state machines received the expected session IDs
        assert_eq!(
            registry
                .command_state_machine(tabs[0])
                .unwrap()
                .expected_session_id(),
            Some(sid0.as_str())
        );
        assert_eq!(
            registry
                .command_state_machine(tabs[1])
                .unwrap()
                .expected_session_id(),
            Some(sid1.as_str())
        );

        // 3. Tab 0 receives matching marker -> accepted
        let meta_c0 = Osc133Metadata {
            version: 1,
            session_id: sid0.clone(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        registry.handle_event(
            tabs[0],
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(meta_c0)),
        );
        assert_eq!(
            registry
                .command_state_machine(tabs[0])
                .unwrap()
                .active_correlation(),
            Some((sid0.as_str(), 1))
        );

        // 4. Tab 1 receives marker with Tab 0's SID -> rejected as alien session!
        let alien_c = Osc133Metadata {
            version: 1,
            session_id: sid0.clone(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        registry.handle_event(
            tabs[1],
            TerminalEvent::ShellIntegration(Osc133Marker::CommandStartedWithMetadata(alien_c)),
        );
        // Active correlation for Tab 1 must be None, and anomaly metric incremented!
        assert_eq!(
            registry
                .command_state_machine(tabs[1])
                .unwrap()
                .active_correlation(),
            None
        );
        assert_eq!(
            registry
                .command_state_machine(tabs[1])
                .unwrap()
                .metrics()
                .desynchronized_metadata_count,
            1
        );

        // 5. Lifecycle removal: removing tab drops the entry and state machine
        assert!(registry.contains(tabs[0]));
        let removed = registry.remove(tabs[0]);
        assert!(removed.is_some());
        assert!(!registry.contains(tabs[0]));
        assert_eq!(registry.session_id(tabs[0]), None);
        assert!(registry.command_state_machine(tabs[0]).is_none());

        // Tab 1 remains unaffected
        assert!(registry.contains(tabs[1]));
        assert_eq!(registry.session_id(tabs[1]), Some(sid1.as_str()));
    }

    #[test]
    fn disabled_integration_produces_no_session_id() {
        use crate::terminal::shell::{Launch, SESSION_ID_ENV, ShellConfig};

        let mut registry = TerminalRegistry::new();
        let tabs = tab_ids(1);

        // PowerShell would be injected on Windows if enabled; Unix spawns its POSIX shell.
        let program = if cfg!(windows) {
            "powershell.exe"
        } else {
            "/bin/sh"
        };
        let launch = Launch::new(
            ShellConfig {
                program: program.to_string(),
                args: vec![],
            },
            false, // Disabled!
            "PC",
            None,
        );
        assert_eq!(launch.session_id(), None);
        assert!(!launch.env.contains_key(SESSION_ID_ENV));

        let (tx, _rx) = unbounded();
        let session = TerminalSession::spawn_launch(launch, 80, 24, 10, 20, tx).unwrap();
        registry.insert(tabs[0], session).ok().unwrap();

        assert_eq!(registry.session_id(tabs[0]), None);
        assert_eq!(
            registry
                .command_state_machine(tabs[0])
                .unwrap()
                .expected_session_id(),
            None
        );
    }
}
