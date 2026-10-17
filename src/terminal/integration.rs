//! Shell integration command execution state machine for OSC 133 sequences.
//!
//! Tracks command lifecycle (Prompt, Editing, Executing, Finished) on a per-terminal basis,
//! ensuring robust handling of anomalies, out-of-order markers, and missing events.

fn debug_log(msg: impl FnOnce() -> String) {
    if std::env::var_os("KEGON_DEBUG_SHELL_INTEGRATION").is_some() {
        eprintln!("[OSC133] {}", msg());
    }
}

/// Alias for [`VerifiedCommandEvent`] reflecting command execution events.
#[allow(dead_code)]
pub type CommandExecutionEvent = VerifiedCommandEvent;

/// Raw marker decoded from an OSC 133 escape sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osc133Marker {
    /// OSC 133 ; A - Prompt start.
    PromptStarted,
    /// OSC 133 ; B - Command line input start (editing active).
    InputStarted,
    /// OSC 133 ; C - Command execution start.
    CommandStarted,
    /// OSC 133 ; D [; <exit_code>] - Command execution finished.
    CommandFinished(Option<i32>),
}

/// The result of a finished command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResult {
    /// Command completed with explicit exit code 0.
    Success,
    /// Command completed with a non-zero exit code.
    Failure(i32),
    /// Command finished without an explicit exit code, or was interrupted/inferred.
    Unknown,
}

impl CommandResult {
    /// Returns true if the command explicitly succeeded with exit code 0.
    #[allow(dead_code)]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Explicit exit code if available.
    #[allow(dead_code)]
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Success => Some(0),
            Self::Failure(code) => Some(*code),
            Self::Unknown => None,
        }
    }
}

/// Per-terminal command execution lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandState {
    /// Initial or desynchronized state before any verified prompt is observed.
    #[default]
    Unknown,
    /// Shell prompt is actively rendering.
    Prompt,
    /// Prompt rendering complete; user input line editing is active.
    Editing,
    /// Command execution is underway.
    Executing { command_id: u64 },
    /// Command execution completed with the recorded result.
    Finished {
        command_id: u64,
        result: CommandResult,
    },
}

/// Verified, transition-guarded event dispatched only when valid state transitions occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedCommandEvent {
    /// Valid prompt began.
    PromptStarted,
    /// Input prompt ready for editing.
    InputStarted,
    /// A command legitimately started execution.
    CommandStarted { command_id: u64 },
    /// An active command finished execution with the verified result.
    CommandFinished {
        command_id: u64,
        result: CommandResult,
    },
}

/// Anomalies detected during state transitions, tracked for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionAnomaly {
    /// Normal transition without any protocol anomaly.
    None,
    /// D received when no matching command was executing (orphaned or duplicate D).
    OrphanedFinished,
    /// C received while a command was already executing (duplicate C).
    DuplicateCommand,
    /// A or B received while a command was executing (missing D / interrupted).
    MissingFinished,
}

/// Metric counters tracking state transitions and protocol anomalies without logging sensitive data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateMachineMetrics {
    pub prompt_count: u64,
    pub input_count: u64,
    pub command_count: u64,
    pub finished_count: u64,
    pub orphaned_d_count: u64,
    pub duplicate_c_count: u64,
    pub missing_d_count: u64,
    pub shell_exit_interrupted_count: u64,
}

/// Tab-level operational status of Shell Integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIntegrationStatus {
    /// Active: received both prompt (A/B) and command execution (C/D) markers.
    Active,
    /// Active (Prompt only): received prompt markers (A/B), but command execution markers (C/D) have not been observed.
    ActivePromptOnly,
    /// Waiting: injected/applied at launch, waiting for first marker from the shell.
    Waiting,
    /// Disabled: explicitly disabled by user configuration.
    Disabled,
    /// Unsupported: shell executable does not support OSC 133 (e.g. cmd.exe or non-PowerShell executable).
    Unsupported,
    /// Unavailable: shell process exited or failed without emitting any markers.
    Unavailable,
}

impl ShellIntegrationStatus {
    /// True if the session has received verified OSC 133 prompt or command markers.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active | Self::ActivePromptOnly)
    }
}

/// Pure state machine coordinating OSC 133 markers into verified command events.
#[derive(Debug, Clone)]
pub struct CommandStateMachine {
    state: CommandState,
    next_command_id: u64,
    metrics: StateMachineMetrics,
    last_verified_event: Option<VerifiedCommandEvent>,
    last_result: Option<CommandResult>,
}

impl Default for CommandStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandStateMachine {
    /// Creates a fresh state machine in `Unknown` state with zero command count.
    pub fn new() -> Self {
        Self {
            state: CommandState::Unknown,
            next_command_id: 0,
            metrics: StateMachineMetrics::default(),
            last_verified_event: None,
            last_result: None,
        }
    }

    /// Current execution state.
    pub fn state(&self) -> &CommandState {
        &self.state
    }

    /// Whether a command is currently executing.
    #[allow(dead_code)]
    pub fn is_executing(&self) -> bool {
        matches!(self.state, CommandState::Executing { .. })
    }

    /// Returns the active command ID if currently executing.
    #[allow(dead_code)]
    pub fn current_command_id(&self) -> Option<u64> {
        match self.state {
            CommandState::Executing { command_id } => Some(command_id),
            _ => None,
        }
    }

    /// The most recent verified command event emitted by this state machine.
    #[allow(dead_code)]
    pub fn last_verified_event(&self) -> Option<VerifiedCommandEvent> {
        self.last_verified_event
    }

    /// Lifetime metrics of transitions and anomalies.
    #[allow(dead_code)]
    pub fn metrics(&self) -> &StateMachineMetrics {
        &self.metrics
    }

    /// Pure step function computing new state, next command id, verified event, and anomaly.
    pub fn step(
        current: CommandState,
        next_id: u64,
        marker: Osc133Marker,
    ) -> (
        CommandState,
        u64,
        Option<VerifiedCommandEvent>,
        TransitionAnomaly,
    ) {
        match marker {
            Osc133Marker::PromptStarted => {
                match current {
                    CommandState::Executing { command_id } => {
                        // Previous command was not closed with D before prompt returned (aborted/interrupted)
                        (
                            CommandState::Prompt,
                            next_id,
                            Some(VerifiedCommandEvent::CommandFinished {
                                command_id,
                                result: CommandResult::Unknown,
                            }),
                            TransitionAnomaly::MissingFinished,
                        )
                    }
                    _ => (
                        CommandState::Prompt,
                        next_id,
                        Some(VerifiedCommandEvent::PromptStarted),
                        TransitionAnomaly::None,
                    ),
                }
            }
            Osc133Marker::InputStarted => {
                match current {
                    CommandState::Executing { command_id } => {
                        // B received while executing: previous command abandoned
                        (
                            CommandState::Editing,
                            next_id,
                            Some(VerifiedCommandEvent::CommandFinished {
                                command_id,
                                result: CommandResult::Unknown,
                            }),
                            TransitionAnomaly::MissingFinished,
                        )
                    }
                    CommandState::Editing => {
                        // Redundant B while already editing
                        (
                            CommandState::Editing,
                            next_id,
                            None,
                            TransitionAnomaly::None,
                        )
                    }
                    _ => (
                        CommandState::Editing,
                        next_id,
                        Some(VerifiedCommandEvent::InputStarted),
                        TransitionAnomaly::None,
                    ),
                }
            }
            Osc133Marker::CommandStarted => {
                let new_id = next_id.wrapping_add(1);
                match current {
                    CommandState::Executing { .. } => {
                        // Duplicate C: previous command did not receive D before new one started
                        (
                            CommandState::Executing { command_id: new_id },
                            new_id,
                            Some(VerifiedCommandEvent::CommandStarted { command_id: new_id }),
                            TransitionAnomaly::DuplicateCommand,
                        )
                    }
                    _ => (
                        CommandState::Executing { command_id: new_id },
                        new_id,
                        Some(VerifiedCommandEvent::CommandStarted { command_id: new_id }),
                        TransitionAnomaly::None,
                    ),
                }
            }
            Osc133Marker::CommandFinished(raw_code) => {
                match current {
                    CommandState::Executing { command_id } => {
                        let result = match raw_code {
                            Some(0) => CommandResult::Success,
                            Some(code) => CommandResult::Failure(code),
                            None => CommandResult::Unknown,
                        };
                        (
                            CommandState::Finished { command_id, result },
                            next_id,
                            Some(VerifiedCommandEvent::CommandFinished { command_id, result }),
                            TransitionAnomaly::None,
                        )
                    }
                    _ => {
                        // D received without an active executing command (orphaned D or duplicate D)
                        // Never fire a success/failure event when no matching C exists!
                        (current, next_id, None, TransitionAnomaly::OrphanedFinished)
                    }
                }
            }
        }
    }

    /// Advances the state machine with an incoming OSC 133 marker.
    ///
    /// Returns `Some(VerifiedCommandEvent)` if this marker triggered a verified event.
    pub fn transition(&mut self, marker: Osc133Marker) -> Option<VerifiedCommandEvent> {
        let before = self.state;
        let (new_state, new_id, event, anomaly) =
            Self::step(self.state, self.next_command_id, marker);

        self.state = new_state;
        self.next_command_id = new_id;
        self.last_verified_event = event;

        if let Some(VerifiedCommandEvent::CommandFinished { result, .. }) = event {
            self.last_result = Some(result);
        }

        match marker {
            Osc133Marker::PromptStarted => self.metrics.prompt_count += 1,
            Osc133Marker::InputStarted => self.metrics.input_count += 1,
            Osc133Marker::CommandStarted => self.metrics.command_count += 1,
            Osc133Marker::CommandFinished(_) => self.metrics.finished_count += 1,
        }

        match anomaly {
            TransitionAnomaly::None => {}
            TransitionAnomaly::OrphanedFinished => self.metrics.orphaned_d_count += 1,
            TransitionAnomaly::DuplicateCommand => self.metrics.duplicate_c_count += 1,
            TransitionAnomaly::MissingFinished => self.metrics.missing_d_count += 1,
        }

        debug_log(|| {
            format!(
                "transition: marker={marker:?}, before={before:?}, after={:?}, event={event:?}, anomaly={anomaly:?}",
                self.state
            )
        });

        event
    }

    /// Handles unexpected child shell termination while a command may be executing.
    ///
    /// If a command was executing, marks it as interrupted (`CommandResult::Unknown`)
    /// rather than borrowing the shell's process exit code.
    pub fn on_shell_exit(&mut self) -> Option<VerifiedCommandEvent> {
        if let CommandState::Executing { command_id } = self.state {
            self.metrics.shell_exit_interrupted_count += 1;
            let result = CommandResult::Unknown;
            self.state = CommandState::Finished { command_id, result };
            let event = VerifiedCommandEvent::CommandFinished { command_id, result };
            self.last_verified_event = Some(event);
            self.last_result = Some(result);
            debug_log(|| {
                format!(
                    "shell exit interrupted active command: command_id={command_id}, result={result:?}"
                )
            });
            Some(event)
        } else {
            None
        }
    }

    /// Returns the result of the last finished command, if any.
    pub fn last_result(&self) -> Option<CommandResult> {
        self.last_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_lifecycle_success() {
        let mut sm = CommandStateMachine::new();
        assert_eq!(*sm.state(), CommandState::Unknown);

        // A -> PromptStarted
        let ev = sm.transition(Osc133Marker::PromptStarted);
        assert_eq!(ev, Some(VerifiedCommandEvent::PromptStarted));
        assert_eq!(*sm.state(), CommandState::Prompt);

        // B -> InputStarted
        let ev = sm.transition(Osc133Marker::InputStarted);
        assert_eq!(ev, Some(VerifiedCommandEvent::InputStarted));
        assert_eq!(*sm.state(), CommandState::Editing);

        // C -> CommandStarted (ID 1)
        let ev = sm.transition(Osc133Marker::CommandStarted);
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandStarted { command_id: 1 })
        );
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 1 });
        assert!(sm.is_executing());
        assert_eq!(sm.current_command_id(), Some(1));

        // D;0 -> CommandFinished Success
        let ev = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Success
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                result: CommandResult::Success
            }
        );
        assert!(!sm.is_executing());
        assert_eq!(sm.current_command_id(), None);
    }

    #[test]
    fn normal_lifecycle_failure_code() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStarted);

        // D;127 -> Failure(127)
        let ev = sm.transition(Osc133Marker::CommandFinished(Some(127)));
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Failure(127)
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                result: CommandResult::Failure(127)
            }
        );
    }

    #[test]
    fn normal_lifecycle_missing_exit_code_is_unknown_not_success() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStarted);

        // D (None) -> Unknown result (MUST NOT be treated as Success)
        let ev = sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Unknown
            })
        );
        assert_ne!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Success
            })
        );
    }

    #[test]
    fn orphaned_d_without_c_is_ignored_and_fires_no_event() {
        let mut sm = CommandStateMachine::new();
        assert_eq!(*sm.state(), CommandState::Unknown);

        // D;0 without C
        let ev = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(ev, None, "Orphaned D must not emit any verified event");
        assert_eq!(*sm.state(), CommandState::Unknown);
        assert_eq!(sm.metrics().orphaned_d_count, 1);

        // Even in Editing state, D;0 without C is rejected
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        let ev = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(ev, None);
        assert_eq!(*sm.state(), CommandState::Editing);
        assert_eq!(sm.metrics().orphaned_d_count, 2);
    }

    #[test]
    fn duplicate_d_is_safely_ignored() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::CommandStarted);

        // First D finishes command
        let ev1 = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(
            ev1,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Success
            })
        );

        // Duplicate D: ignored, no second event fired
        let ev2 = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(ev2, None);
        assert_eq!(sm.metrics().orphaned_d_count, 1);
    }

    #[test]
    fn duplicate_c_aborts_previous_command_and_starts_new_id() {
        let mut sm = CommandStateMachine::new();
        let ev1 = sm.transition(Osc133Marker::CommandStarted);
        assert_eq!(
            ev1,
            Some(VerifiedCommandEvent::CommandStarted { command_id: 1 })
        );

        // Duplicate C: previous command is superseded, new command ID issued
        let ev2 = sm.transition(Osc133Marker::CommandStarted);
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandStarted { command_id: 2 })
        );
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 2 });
        assert_eq!(sm.metrics().duplicate_c_count, 1);
    }

    #[test]
    fn missing_d_recovered_by_prompt_start() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::CommandStarted);
        assert!(sm.is_executing());

        // A arrives while still executing: previous command closed as Unknown, prompt recovered
        let ev = sm.transition(Osc133Marker::PromptStarted);
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Unknown
            })
        );
        assert_eq!(*sm.state(), CommandState::Prompt);
        assert_eq!(sm.metrics().missing_d_count, 1);
    }

    #[test]
    fn shell_exit_aborts_active_command_as_unknown() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::CommandStarted);
        assert!(sm.is_executing());

        let ev = sm.on_shell_exit();
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                result: CommandResult::Unknown
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                result: CommandResult::Unknown
            }
        );
        assert_eq!(sm.metrics().shell_exit_interrupted_count, 1);

        // When not executing, on_shell_exit does nothing
        assert_eq!(sm.on_shell_exit(), None);
    }

    #[test]
    fn last_result_tracks_confirmed_command_results() {
        let mut sm = CommandStateMachine::new();
        assert_eq!(sm.last_result(), None);

        // Orphaned D does not record a last result
        sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(sm.last_result(), None);

        // Command 1 -> D;0
        sm.transition(Osc133Marker::CommandStarted);
        sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(sm.last_result(), Some(CommandResult::Success));

        // Prompt arrives -> last_result remains 0
        sm.transition(Osc133Marker::PromptStarted);
        assert_eq!(sm.last_result(), Some(CommandResult::Success));

        // Command 2 -> D;42
        sm.transition(Osc133Marker::CommandStarted);
        // While executing, last_result is still the previous finished command
        assert_eq!(sm.last_result(), Some(CommandResult::Success));
        sm.transition(Osc133Marker::CommandFinished(Some(42)));
        assert_eq!(sm.last_result(), Some(CommandResult::Failure(42)));

        // Command 3 -> D (None)
        sm.transition(Osc133Marker::CommandStarted);
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm.last_result(), Some(CommandResult::Unknown));

        // Command 4 -> interrupted by next prompt A
        sm.transition(Osc133Marker::CommandStarted);
        sm.transition(Osc133Marker::PromptStarted);
        assert_eq!(sm.last_result(), Some(CommandResult::Unknown));

        // Command 5 -> interrupted by shell exit
        sm.transition(Osc133Marker::CommandStarted);
        sm.on_shell_exit();
        assert_eq!(sm.last_result(), Some(CommandResult::Unknown));
    }

    #[test]
    fn shell_integration_status_is_active() {
        assert!(ShellIntegrationStatus::Active.is_active());
        assert!(ShellIntegrationStatus::ActivePromptOnly.is_active());
        assert!(!ShellIntegrationStatus::Waiting.is_active());
        assert!(!ShellIntegrationStatus::Disabled.is_active());
        assert!(!ShellIntegrationStatus::Unsupported.is_active());
        assert!(!ShellIntegrationStatus::Unavailable.is_active());
    }
}
