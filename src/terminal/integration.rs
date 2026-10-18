//! Shell integration command execution state machine for OSC 133 sequences.
//!
//! Tracks command lifecycle (Prompt, Editing, Executing, Finished) on a per-terminal basis,
//! ensuring robust handling of anomalies, out-of-order markers, and missing events.

use std::fmt;

fn debug_log(msg: impl FnOnce() -> String) {
    if std::env::var_os("KEGON_DEBUG_SHELL_INTEGRATION").is_some() {
        eprintln!("[OSC133] {}", msg());
    }
}

/// Alias for [`VerifiedCommandEvent`] reflecting command execution events.
#[allow(dead_code)]
pub type CommandExecutionEvent = VerifiedCommandEvent;

/// Status reported directly by the shell runtime (e.g. `$?` in PowerShell or future shell integrations).
///
/// This represents what the shell itself observed/reported at the prompt boundary,
/// completely orthogonal to numeric process exit codes (`osc133_exit` or `native_exit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellReportedStatus {
    /// Shell explicitly reported success (e.g. `$? == True`).
    Success,
    /// Shell explicitly reported failure (e.g. `$? == False`).
    Failure,
    /// Shell status is unknown, unobserved, desynchronized, or unsupported.
    #[default]
    Unknown,
}

impl ShellReportedStatus {
    /// Returns true if the shell explicitly reported success.
    #[allow(dead_code)]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns true if the shell explicitly reported failure.
    #[allow(dead_code)]
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failure)
    }

    /// Returns true if the shell reported status is unknown or unobserved.
    #[allow(dead_code)]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// String identifier for diagnostics and serialization.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Unknown => "unknown",
        }
    }
}

/// The observed boundary condition through which a command execution was completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompletionReason {
    /// Command execution finished cleanly with an observed OSC 133 D marker.
    #[default]
    DObserved,
    /// Next prompt (A or B) was observed while a command was active, indicating an interrupted command or missing D.
    MissingD,
    /// Child shell or terminal process terminated while a command was active.
    ShellExited,
}

impl CompletionReason {
    /// Returns true if the command completed normally with an observed D marker.
    #[allow(dead_code)]
    pub fn is_d_observed(&self) -> bool {
        matches!(self, Self::DObserved)
    }

    /// Legacy helper: returns true if the command completed cleanly with an observed D marker.
    #[allow(dead_code)]
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::DObserved)
    }
}

/// Status of metadata correlation for a completed command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetadataCorrelation {
    /// Valid, uncorrupted metadata with matching session ID and monotonic sequence number
    /// was successfully validated and correlated to this command execution.
    Matched,
    /// Standard OSC 133 C or D without metadata (legacy shell integration).
    #[default]
    Unprovided,
    /// Metadata was transmitted or attempted, but failed correlation or validation
    /// (e.g. sequence regression/desynchronization, session ID mismatch, corrupt payload, or boundary anomaly).
    Mismatched,
}

impl MetadataCorrelation {
    /// Returns true if metadata was successfully matched.
    #[allow(dead_code)]
    pub fn is_matched(&self) -> bool {
        matches!(self, Self::Matched)
    }
}

/// Comprehensive, orthogonal result model for finished command executions.
///
/// Distinguishes between:
/// 1. `shell_status`: The shell runtime's own success/failure report (e.g. `$?`).
/// 2. `osc133_exit`: External numeric exit code notified via OSC 133 D marker.
/// 3. `native_exit`: OS-verified native process exit code (None if unverified).
/// 4. `completion`: The boundary / observation condition for termination.
/// 5. `correlation`: The metadata correlation state between start and finish.
///
/// ## Structural Evolution Note (N-6)
/// `CommandExecutionResult` currently aggregates all 5 orthogonal dimensions in a single flat struct
/// for ergonomics and zero-overhead copying. Decomposition into sub-structures (e.g. exit code grouping
/// vs correlation metadata) is explicitly deferred to future refactoring when additional consumers
/// are implemented in Phase 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandExecutionResult {
    /// Status reported directly by the shell runtime, if available and correlated.
    pub shell_status: ShellReportedStatus,
    /// Numeric exit code carried in OSC 133 D marker, if present.
    pub osc133_exit: Option<i32>,
    /// Directly observed OS native process exit code, if available.
    pub native_exit: Option<i32>,
    /// The boundary condition through which the command was closed.
    pub completion: CompletionReason,
    /// Correlation status between C and D metadata.
    pub correlation: MetadataCorrelation,
}

impl CommandExecutionResult {
    /// Creates a new execution result with the specified properties.
    pub const fn new(
        shell_status: ShellReportedStatus,
        osc133_exit: Option<i32>,
        native_exit: Option<i32>,
        completion: CompletionReason,
        correlation: MetadataCorrelation,
    ) -> Self {
        Self {
            shell_status,
            osc133_exit,
            native_exit,
            completion,
            correlation,
        }
    }

    /// Derives the numeric command result from the OSC 133 exit code.
    pub fn numeric_result(&self) -> CommandResult {
        match self.osc133_exit {
            Some(0) => CommandResult::Success,
            Some(code) => CommandResult::Failure(code),
            None => CommandResult::Unknown,
        }
    }
}

/// Metadata parsed from extended OSC 133 markers.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Osc133Metadata {
    /// Protocol version (must be 1 for current specification).
    pub version: u32,
    /// Injected shell session identifier (e.g. 16-hex characters).
    pub session_id: String,
    /// Monotonically increasing command sequence number within the session.
    pub seq: u64,
    /// Shell-reported status parsed from `kegon.ps` (or future shell status keys).
    pub shell_status: ShellReportedStatus,
}

impl fmt::Debug for Osc133Metadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Osc133Metadata")
            .field("version", &self.version)
            .field("session_id", &"<redacted>")
            .field("seq", &self.seq)
            .field("shell_status", &self.shell_status)
            .finish()
    }
}

/// Raw marker decoded from an OSC 133 escape sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Osc133Marker {
    /// OSC 133 ; A - Prompt start.
    PromptStarted,
    /// OSC 133 ; B - Command line input start (editing active).
    InputStarted,
    /// OSC 133 ; C - Command execution start (without metadata).
    CommandStarted,
    /// OSC 133 ; D [; <exit_code>] - Command execution finished (without metadata).
    CommandFinished(Option<i32>),
    /// OSC 133 ; C ; <metadata> - Command execution start with metadata.
    CommandStartedWithMetadata(Osc133Metadata),
    /// OSC 133 ; D [; <exit_code>] ; <metadata> - Command execution finished with metadata.
    CommandFinishedWithMetadata(Option<i32>, Osc133Metadata),
    /// OSC 1337 ; SetUserVar=kegon_status=<Base64> - Independent status notification (Option E).
    IndependentMetadata(Osc133Metadata),
    /// OSC 1337 ; SetUserVar=kegon_status=<Invalid> - Malformed/unparsable target kegon_status metadata.
    InvalidMetadata,
}

/// The result of a finished command execution based on numeric exit code.
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

/// Active command correlation tracking session identifier and sequence number.
#[derive(Clone, PartialEq, Eq)]
pub struct ActiveCorrelation {
    pub session_id: String,
    pub seq: u64,
}

impl fmt::Debug for ActiveCorrelation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveCorrelation")
            .field("session_id", &"<redacted>")
            .field("seq", &self.seq)
            .finish()
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
    /// Command execution completed with the recorded single authoritative result.
    Finished {
        command_id: u64,
        execution_result: CommandExecutionResult,
    },
}

impl CommandState {
    /// Returns the comprehensive execution result if in Finished state.
    #[allow(dead_code)]
    pub fn execution_result(&self) -> Option<&CommandExecutionResult> {
        match self {
            Self::Finished {
                execution_result, ..
            } => Some(execution_result),
            _ => None,
        }
    }

    /// Derives the numeric command result if in Finished state.
    #[allow(dead_code)]
    pub fn numeric_result(&self) -> Option<CommandResult> {
        self.execution_result().map(|r| r.numeric_result())
    }
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
        execution_result: CommandExecutionResult,
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
    /// D received with an older seq than the active executing command (stale D).
    StaleFinished,
    /// Session ID, sequence number, or protocol version desynchronized.
    DesynchronizedMetadata,
}

/// Metric counters tracking state transitions and protocol anomalies without logging sensitive data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateMachineMetrics {
    pub prompt_count: u64,
    pub input_count: u64,
    pub command_count: u64,
    pub finished_count: u64,
    pub verified_command_starts: u64,
    pub verified_command_finishes: u64,
    pub orphaned_d_count: u64,
    pub duplicate_c_count: u64,
    pub missing_d_count: u64,
    pub shell_exit_interrupted_count: u64,
    pub stale_d_count: u64,
    pub desynchronized_metadata_count: u64,
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
#[derive(Clone)]
pub struct CommandStateMachine {
    state: CommandState,
    next_command_id: u64,
    metrics: StateMachineMetrics,
    last_verified_event: Option<VerifiedCommandEvent>,
    last_execution_result: Option<CommandExecutionResult>,
    active_correlation: Option<ActiveCorrelation>,
    current_session_id: Option<String>,
    last_confirmed_seq: Option<u64>,
    expected_session_id: Option<String>,
    pending_metadata: Option<Osc133Metadata>,
    pending_corrupted: bool,
    duplicate_c: bool,
}

impl fmt::Debug for CommandStateMachine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandStateMachine")
            .field("state", &self.state)
            .field("next_command_id", &self.next_command_id)
            .field("metrics", &self.metrics)
            .field("last_verified_event", &self.last_verified_event)
            .field("last_execution_result", &self.last_execution_result)
            .field("active_correlation", &self.active_correlation)
            .field(
                "current_session_id",
                &self.current_session_id.as_ref().map(|_| "<redacted>"),
            )
            .field("last_confirmed_seq", &self.last_confirmed_seq)
            .field(
                "expected_session_id",
                &self.expected_session_id.as_ref().map(|_| "<redacted>"),
            )
            .field("pending_metadata", &self.pending_metadata)
            .field("pending_corrupted", &self.pending_corrupted)
            .field("duplicate_c", &self.duplicate_c)
            .finish()
    }
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
            last_execution_result: None,
            active_correlation: None,
            current_session_id: None,
            last_confirmed_seq: None,
            expected_session_id: None,
            pending_metadata: None,
            pending_corrupted: false,
            duplicate_c: false,
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

    /// Active correlation session ID and sequence number if a command is currently executing.
    #[allow(dead_code)]
    pub fn active_correlation(&self) -> Option<(&str, u64)> {
        self.active_correlation
            .as_ref()
            .map(|c| (c.session_id.as_str(), c.seq))
    }

    /// Sets the expected session ID for this terminal instance.
    ///
    /// ## Phase 2 Connection Contract & Trust Boundary (N-2)
    /// In Phase 1 Slice 1, this method provides an explicit verification boundary but is
    /// intentionally unconnected to PTY spawning.
    ///
    /// In Phase 2:
    /// 1. Kegon will generate an unpredictable per-PTY session ID (e.g. 128-bit random hex / UUIDv4).
    /// 2. Kegon injects this SID into the shell environment upon spawn (e.g. via `KEGON_SESSION_ID`).
    /// 3. The shell integration hook reads this SID and transmits it in OSC 133 C/D metadata (`kegon.sid`).
    /// 4. The state machine validates all incoming markers against `expected_session_id`, rejecting
    ///    any command start or finish with a mismatched SID as `DesynchronizedMetadata`.
    /// 5. On PTY respawn or session restart, `reset_session()` is invoked to clear old correlation.
    ///
    /// If `None`, any valid initial session ID will be accepted upon first observation.
    #[allow(dead_code)]
    pub fn set_expected_session_id(&mut self, session_id: Option<String>) {
        self.expected_session_id = session_id;
    }

    /// Returns the currently active session ID observed from the shell hooks, if any.
    #[allow(dead_code)]
    pub fn current_session_id(&self) -> Option<&str> {
        self.current_session_id.as_deref()
    }

    /// Returns the expected session ID configured by Kegon, if any.
    #[allow(dead_code)]
    pub fn expected_session_id(&self) -> Option<&str> {
        self.expected_session_id.as_deref()
    }

    /// Returns the last confirmed sequence number for the current session, if any.
    #[allow(dead_code)]
    pub fn last_confirmed_seq(&self) -> Option<u64> {
        self.last_confirmed_seq
    }

    /// Returns the pending Option E status metadata for the active command, if any.
    #[allow(dead_code)]
    pub fn pending_metadata(&self) -> Option<&Osc133Metadata> {
        self.pending_metadata.as_ref()
    }

    /// Resets session tracking state (e.g. upon PTY spawn or full session reset).
    #[allow(dead_code)]
    pub fn reset_session(&mut self) {
        self.state = CommandState::Unknown;
        self.current_session_id = None;
        self.last_confirmed_seq = None;
        self.active_correlation = None;
        self.pending_metadata = None;
        self.pending_corrupted = false;
        self.duplicate_c = false;
        self.last_verified_event = None;
        self.last_execution_result = None;
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

    /// Pure step function computing new state, next command id, correlation, session tracking, verified event, and anomaly.
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub fn step(
        current: CommandState,
        next_id: u64,
        active_correlation: Option<ActiveCorrelation>,
        current_session_id: Option<String>,
        last_confirmed_seq: Option<u64>,
        expected_session_id: Option<String>,
        pending_metadata: Option<Osc133Metadata>,
        pending_corrupted: bool,
        duplicate_c: bool,
        marker: Osc133Marker,
    ) -> (
        CommandState,
        u64,
        Option<ActiveCorrelation>,
        Option<String>,
        Option<u64>,
        Option<Osc133Metadata>,
        bool,
        bool,
        Option<VerifiedCommandEvent>,
        TransitionAnomaly,
    ) {
        match marker {
            Osc133Marker::PromptStarted => {
                match current {
                    CommandState::Executing { command_id } => {
                        // Previous command was not closed with D before prompt returned (aborted/interrupted/MissingD).
                        //
                        // N-4 Sequence Consumption Policy:
                        // `last_confirmed_seq` is NOT advanced because the completion of this command was never
                        // confirmed by a legitimate D marker.
                        // When the user enters the next command, the shell hook will increment its internal sequence
                        // counter ($global:__kegon_seq++). Because next_seq > active_seq >= last_confirmed_seq,
                        // strict monotonic sequence progression is naturally maintained without desynchronization.
                        let execution_result = CommandExecutionResult::new(
                            ShellReportedStatus::Unknown,
                            None,
                            None,
                            CompletionReason::MissingD,
                            MetadataCorrelation::Unprovided,
                        );
                        (
                            CommandState::Prompt,
                            next_id,
                            None,
                            current_session_id,
                            last_confirmed_seq,
                            None,
                            false,
                            false,
                            Some(VerifiedCommandEvent::CommandFinished {
                                command_id,
                                execution_result,
                            }),
                            TransitionAnomaly::MissingFinished,
                        )
                    }
                    _ => (
                        CommandState::Prompt,
                        next_id,
                        active_correlation,
                        current_session_id,
                        last_confirmed_seq,
                        None,
                        false,
                        false,
                        Some(VerifiedCommandEvent::PromptStarted),
                        TransitionAnomaly::None,
                    ),
                }
            }
            Osc133Marker::InputStarted => {
                match current {
                    CommandState::Executing { command_id } => {
                        // B received while executing: previous command abandoned without D marker (MissingD).
                        // Sequence consumption policy (N-4): `last_confirmed_seq` is kept unchanged.
                        let execution_result = CommandExecutionResult::new(
                            ShellReportedStatus::Unknown,
                            None,
                            None,
                            CompletionReason::MissingD,
                            MetadataCorrelation::Unprovided,
                        );
                        (
                            CommandState::Editing,
                            next_id,
                            None,
                            current_session_id,
                            last_confirmed_seq,
                            None,
                            false,
                            false,
                            Some(VerifiedCommandEvent::CommandFinished {
                                command_id,
                                execution_result,
                            }),
                            TransitionAnomaly::MissingFinished,
                        )
                    }
                    CommandState::Editing => {
                        // Redundant B while already editing
                        (
                            CommandState::Editing,
                            next_id,
                            active_correlation,
                            current_session_id,
                            last_confirmed_seq,
                            pending_metadata,
                            pending_corrupted,
                            duplicate_c,
                            None,
                            TransitionAnomaly::None,
                        )
                    }
                    _ => (
                        CommandState::Editing,
                        next_id,
                        active_correlation,
                        current_session_id,
                        last_confirmed_seq,
                        None,
                        false,
                        false,
                        Some(VerifiedCommandEvent::InputStarted),
                        TransitionAnomaly::None,
                    ),
                }
            }
            Osc133Marker::CommandStarted | Osc133Marker::CommandStartedWithMetadata(_) => {
                let incoming_meta = match marker {
                    Osc133Marker::CommandStartedWithMetadata(m) => Some(m),
                    _ => None,
                };
                match current {
                    CommandState::Executing { .. } => {
                        // Duplicate C: previous command is still executing; record anomaly without
                        // re-assigning command_id or emitting duplicate CommandStarted event.
                        // Invalidate correlation so that subsequent D cannot match if metadata was involved.
                        let corrupted = pending_corrupted
                            || active_correlation.is_some()
                            || incoming_meta.is_some()
                            || pending_metadata.is_some();
                        (
                            current,
                            next_id,
                            None,
                            current_session_id,
                            last_confirmed_seq,
                            None,
                            corrupted,
                            true,
                            None,
                            TransitionAnomaly::DuplicateCommand,
                        )
                    }
                    _ => {
                        let new_id = next_id.wrapping_add(1);
                        let mut new_correlation = None;
                        let mut new_session_id = current_session_id;
                        let mut new_confirmed_seq = last_confirmed_seq;
                        let mut anomaly = TransitionAnomaly::None;

                        if let Some(meta) = incoming_meta {
                            // Check against expected session ID if configured (Phase 2 hook point)
                            let sid_valid = match &expected_session_id {
                                Some(expected) => expected == &meta.session_id,
                                None => true,
                            };

                            if sid_valid {
                                if new_session_id.as_deref() == Some(&meta.session_id) {
                                    // Same session: check seq strictly increasing
                                    if let Some(last_seq) = new_confirmed_seq {
                                        if meta.seq <= last_seq {
                                            // Seq regressed or reused!
                                            anomaly = TransitionAnomaly::DesynchronizedMetadata;
                                        } else {
                                            new_correlation = Some(ActiveCorrelation {
                                                session_id: meta.session_id,
                                                seq: meta.seq,
                                            });
                                        }
                                    } else {
                                        // First seq in this session
                                        new_correlation = Some(ActiveCorrelation {
                                            session_id: meta.session_id,
                                            seq: meta.seq,
                                        });
                                    }
                                } else {
                                    // New/changed session ID: adopt and reset confirmed seq
                                    new_session_id = Some(meta.session_id.clone());
                                    new_confirmed_seq = None;
                                    new_correlation = Some(ActiveCorrelation {
                                        session_id: meta.session_id,
                                        seq: meta.seq,
                                    });
                                }
                            } else {
                                // Mismatched expected session ID
                                anomaly = TransitionAnomaly::DesynchronizedMetadata;
                            }
                        }

                        (
                            CommandState::Executing { command_id: new_id },
                            new_id,
                            new_correlation,
                            new_session_id,
                            new_confirmed_seq,
                            None,
                            false,
                            false,
                            Some(VerifiedCommandEvent::CommandStarted { command_id: new_id }),
                            anomaly,
                        )
                    }
                }
            }
            Osc133Marker::IndependentMetadata(meta) => match current {
                CommandState::Executing { .. } => {
                    if duplicate_c || pending_metadata.is_some() || pending_corrupted {
                        // Duplicate metadata or metadata after duplicate C within the same command execution!
                        (
                            current,
                            next_id,
                            active_correlation,
                            current_session_id,
                            last_confirmed_seq,
                            None,
                            true,
                            duplicate_c,
                            None,
                            TransitionAnomaly::DesynchronizedMetadata,
                        )
                    } else if let Some(active) = &active_correlation {
                        // Scheme A: Command execution started with C carrying active correlation (sid, seq).
                        // Requirement: C and status metadata's sid/seq must match completely to adopt status.
                        let sid_matches = active.session_id == meta.session_id;
                        let sid_expected = match &expected_session_id {
                            Some(expected) => expected == &meta.session_id,
                            None => true,
                        };
                        let seq_matches = active.seq == meta.seq;

                        if !sid_matches || !sid_expected {
                            (
                                current,
                                next_id,
                                active_correlation,
                                current_session_id,
                                last_confirmed_seq,
                                None,
                                true,
                                duplicate_c,
                                None,
                                TransitionAnomaly::DesynchronizedMetadata,
                            )
                        } else if !seq_matches {
                            let anomaly = if meta.seq < active.seq {
                                TransitionAnomaly::StaleFinished
                            } else {
                                TransitionAnomaly::DesynchronizedMetadata
                            };
                            (
                                current,
                                next_id,
                                active_correlation,
                                current_session_id,
                                last_confirmed_seq,
                                None,
                                true,
                                duplicate_c,
                                None,
                                anomaly,
                            )
                        } else {
                            // Exact match on both session_id and monotonic sequence number!
                            (
                                current,
                                next_id,
                                active_correlation,
                                current_session_id,
                                last_confirmed_seq,
                                Some(meta),
                                false,
                                duplicate_c,
                                None,
                                TransitionAnomaly::None,
                            )
                        }
                    } else {
                        // Command was started by legacy C (without metadata).
                        // Under Scheme A rules: C and status metadata must match completely to adopt status.
                        // Since legacy C provided no sid/seq, metadata cannot match. Corrupt and degrade to Unknown.
                        (
                            current,
                            next_id,
                            active_correlation,
                            current_session_id,
                            last_confirmed_seq,
                            None,
                            true,
                            duplicate_c,
                            None,
                            TransitionAnomaly::DesynchronizedMetadata,
                        )
                    }
                }
                _ => (
                    current,
                    next_id,
                    active_correlation,
                    current_session_id,
                    last_confirmed_seq,
                    None,
                    false,
                    false,
                    None,
                    TransitionAnomaly::OrphanedFinished,
                ),
            },
            Osc133Marker::CommandFinished(raw_code)
            | Osc133Marker::CommandFinishedWithMetadata(raw_code, _) => {
                let incoming_meta = match marker {
                    Osc133Marker::CommandFinishedWithMetadata(_, m) => Some(m),
                    _ => None,
                };
                match current {
                    CommandState::Executing { command_id } => {
                        if pending_corrupted {
                            let execution_result = CommandExecutionResult::new(
                                ShellReportedStatus::Unknown,
                                raw_code,
                                None,
                                CompletionReason::DObserved,
                                MetadataCorrelation::Mismatched,
                            );
                            (
                                CommandState::Finished {
                                    command_id,
                                    execution_result,
                                },
                                next_id,
                                None,
                                current_session_id,
                                last_confirmed_seq,
                                None,
                                false,
                                false,
                                Some(VerifiedCommandEvent::CommandFinished {
                                    command_id,
                                    execution_result,
                                }),
                                TransitionAnomaly::DesynchronizedMetadata,
                            )
                        } else if let Some(meta) = pending_metadata {
                            let is_consistent = match &incoming_meta {
                                Some(m) => m == &meta,
                                None => true,
                            };
                            if !is_consistent {
                                let execution_result = CommandExecutionResult::new(
                                    ShellReportedStatus::Unknown,
                                    raw_code,
                                    None,
                                    CompletionReason::DObserved,
                                    MetadataCorrelation::Mismatched,
                                );
                                (
                                    CommandState::Finished {
                                        command_id,
                                        execution_result,
                                    },
                                    next_id,
                                    None,
                                    current_session_id,
                                    last_confirmed_seq,
                                    None,
                                    false,
                                    false,
                                    Some(VerifiedCommandEvent::CommandFinished {
                                        command_id,
                                        execution_result,
                                    }),
                                    TransitionAnomaly::DesynchronizedMetadata,
                                )
                            } else {
                                let execution_result = CommandExecutionResult::new(
                                    meta.shell_status,
                                    raw_code,
                                    None,
                                    CompletionReason::DObserved,
                                    MetadataCorrelation::Matched,
                                );
                                (
                                    CommandState::Finished {
                                        command_id,
                                        execution_result,
                                    },
                                    next_id,
                                    None,
                                    Some(meta.session_id),
                                    Some(meta.seq),
                                    None,
                                    false,
                                    false,
                                    Some(VerifiedCommandEvent::CommandFinished {
                                        command_id,
                                        execution_result,
                                    }),
                                    TransitionAnomaly::None,
                                )
                            }
                        } else {
                            match (active_correlation, incoming_meta) {
                                (Some(active), Some(meta)) => {
                                    if active.session_id == meta.session_id
                                        && active.seq == meta.seq
                                    {
                                        // Matched!
                                        let execution_result = CommandExecutionResult::new(
                                            meta.shell_status,
                                            raw_code,
                                            None,
                                            CompletionReason::DObserved,
                                            MetadataCorrelation::Matched,
                                        );
                                        (
                                            CommandState::Finished {
                                                command_id,
                                                execution_result,
                                            },
                                            next_id,
                                            None,
                                            current_session_id,
                                            Some(meta.seq),
                                            None,
                                            false,
                                            false,
                                            Some(VerifiedCommandEvent::CommandFinished {
                                                command_id,
                                                execution_result,
                                            }),
                                            TransitionAnomaly::None,
                                        )
                                    } else {
                                        // Mismatched metadata on D (stale seq, future seq, or alien session ID - N-1).
                                        // Under N-1, any mismatched metadata on D MUST NOT complete the active command,
                                        // MUST NOT adopt the numeric exit code, and MUST NOT advance last_confirmed_seq.
                                        // Active correlation is preserved so that the subsequent legitimate D can complete the command.
                                        let anomaly = if active.session_id == meta.session_id
                                            && meta.seq < active.seq
                                        {
                                            TransitionAnomaly::StaleFinished
                                        } else {
                                            TransitionAnomaly::DesynchronizedMetadata
                                        };
                                        (
                                            current,
                                            next_id,
                                            Some(active),
                                            current_session_id,
                                            last_confirmed_seq,
                                            None,
                                            false,
                                            duplicate_c,
                                            None,
                                            anomaly,
                                        )
                                    }
                                }
                                (Some(_active), None) => {
                                    // Active command expected metadata, but legacy D arrived
                                    let execution_result = CommandExecutionResult::new(
                                        ShellReportedStatus::Unknown,
                                        raw_code,
                                        None,
                                        CompletionReason::DObserved,
                                        MetadataCorrelation::Mismatched,
                                    );
                                    (
                                        CommandState::Finished {
                                            command_id,
                                            execution_result,
                                        },
                                        next_id,
                                        None,
                                        current_session_id,
                                        last_confirmed_seq,
                                        None,
                                        false,
                                        false,
                                        Some(VerifiedCommandEvent::CommandFinished {
                                            command_id,
                                            execution_result,
                                        }),
                                        TransitionAnomaly::None,
                                    )
                                }
                                (None, Some(_meta)) => {
                                    // Active command had no metadata, but metadata D arrived
                                    let execution_result = CommandExecutionResult::new(
                                        ShellReportedStatus::Unknown,
                                        raw_code,
                                        None,
                                        CompletionReason::DObserved,
                                        MetadataCorrelation::Mismatched,
                                    );
                                    (
                                        CommandState::Finished {
                                            command_id,
                                            execution_result,
                                        },
                                        next_id,
                                        None,
                                        current_session_id,
                                        last_confirmed_seq,
                                        None,
                                        false,
                                        false,
                                        Some(VerifiedCommandEvent::CommandFinished {
                                            command_id,
                                            execution_result,
                                        }),
                                        TransitionAnomaly::DesynchronizedMetadata,
                                    )
                                }
                                (None, None) => {
                                    // Standard legacy C + legacy D
                                    let execution_result = CommandExecutionResult::new(
                                        ShellReportedStatus::Unknown,
                                        raw_code,
                                        None,
                                        CompletionReason::DObserved,
                                        MetadataCorrelation::Unprovided,
                                    );
                                    (
                                        CommandState::Finished {
                                            command_id,
                                            execution_result,
                                        },
                                        next_id,
                                        None,
                                        current_session_id,
                                        last_confirmed_seq,
                                        None,
                                        false,
                                        false,
                                        Some(VerifiedCommandEvent::CommandFinished {
                                            command_id,
                                            execution_result,
                                        }),
                                        TransitionAnomaly::None,
                                    )
                                }
                            }
                        }
                    }
                    _ => {
                        // Orphaned D without executing command
                        (
                            current,
                            next_id,
                            active_correlation,
                            current_session_id,
                            last_confirmed_seq,
                            None,
                            false,
                            false,
                            None,
                            TransitionAnomaly::OrphanedFinished,
                        )
                    }
                }
            }
            Osc133Marker::InvalidMetadata => match current {
                CommandState::Executing { .. } => {
                    // M-3: Parsing failure on target kegon_status metadata.
                    // Corrupts the active command, degrades shell status to Unknown,
                    // and prevents subsequent metadata from reviving correlation.
                    (
                        current,
                        next_id,
                        active_correlation,
                        current_session_id,
                        last_confirmed_seq,
                        None,
                        true,
                        duplicate_c,
                        None,
                        TransitionAnomaly::DesynchronizedMetadata,
                    )
                }
                _ => (
                    current,
                    next_id,
                    active_correlation,
                    current_session_id,
                    last_confirmed_seq,
                    None,
                    false,
                    false,
                    None,
                    TransitionAnomaly::DesynchronizedMetadata,
                ),
            },
        }
    }

    /// Advances the state machine with an incoming OSC 133 marker.
    ///
    /// Returns `Some(VerifiedCommandEvent)` if this marker triggered a verified event.
    pub fn transition(&mut self, marker: Osc133Marker) -> Option<VerifiedCommandEvent> {
        let before = self.state;
        let (
            new_state,
            new_id,
            new_correlation,
            new_sid,
            new_confirmed_seq,
            new_pending,
            new_corrupted,
            new_duplicate_c,
            event,
            anomaly,
        ) = Self::step(
            self.state,
            self.next_command_id,
            self.active_correlation.take(),
            self.current_session_id.clone(),
            self.last_confirmed_seq,
            self.expected_session_id.clone(),
            self.pending_metadata.take(),
            self.pending_corrupted,
            self.duplicate_c,
            marker.clone(),
        );

        self.state = new_state;
        self.next_command_id = new_id;
        self.active_correlation = new_correlation;
        self.current_session_id = new_sid;
        self.last_confirmed_seq = new_confirmed_seq;
        self.pending_metadata = new_pending;
        self.pending_corrupted = new_corrupted;
        self.duplicate_c = new_duplicate_c;
        if event.is_some() {
            self.last_verified_event = event;
        }

        if let Some(VerifiedCommandEvent::CommandFinished {
            execution_result, ..
        }) = event
        {
            self.last_execution_result = Some(execution_result);
            self.metrics.verified_command_finishes += 1;
        } else if let Some(VerifiedCommandEvent::CommandStarted { .. }) = event {
            self.metrics.verified_command_starts += 1;
        }

        match marker {
            Osc133Marker::PromptStarted => self.metrics.prompt_count += 1,
            Osc133Marker::InputStarted => self.metrics.input_count += 1,
            Osc133Marker::CommandStarted | Osc133Marker::CommandStartedWithMetadata(_) => {
                self.metrics.command_count += 1
            }
            Osc133Marker::CommandFinished(_) | Osc133Marker::CommandFinishedWithMetadata(_, _) => {
                self.metrics.finished_count += 1
            }
            Osc133Marker::IndependentMetadata(_) | Osc133Marker::InvalidMetadata => {}
        }

        match anomaly {
            TransitionAnomaly::None => {}
            TransitionAnomaly::OrphanedFinished => self.metrics.orphaned_d_count += 1,
            TransitionAnomaly::DuplicateCommand => self.metrics.duplicate_c_count += 1,
            TransitionAnomaly::MissingFinished => self.metrics.missing_d_count += 1,
            TransitionAnomaly::StaleFinished => self.metrics.stale_d_count += 1,
            TransitionAnomaly::DesynchronizedMetadata => {
                self.metrics.desynchronized_metadata_count += 1;
            }
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
            self.active_correlation = None;
            self.pending_metadata = None;
            self.pending_corrupted = false;
            self.duplicate_c = false;
            let execution_result = CommandExecutionResult::new(
                ShellReportedStatus::Unknown,
                None,
                None,
                CompletionReason::ShellExited,
                MetadataCorrelation::Unprovided,
            );
            self.state = CommandState::Finished {
                command_id,
                execution_result,
            };
            let event = VerifiedCommandEvent::CommandFinished {
                command_id,
                execution_result,
            };
            self.last_verified_event = Some(event);
            self.last_execution_result = Some(execution_result);
            debug_log(|| {
                format!(
                    "shell exit interrupted active command: command_id={command_id}, execution_result={execution_result:?}"
                )
            });
            Some(event)
        } else {
            None
        }
    }

    /// Returns the result of the last finished command based on numeric exit code.
    #[allow(dead_code)]
    pub fn last_result(&self) -> Option<CommandResult> {
        self.last_execution_result.map(|r| r.numeric_result())
    }

    /// Returns the comprehensive execution result for the last finished command, if any.
    #[allow(dead_code)]
    pub fn last_execution_result(&self) -> Option<CommandExecutionResult> {
        self.last_execution_result
    }

    /// Returns true if any verified command execution (legitimate C or D) has occurred.
    pub fn has_verified_command_execution(&self) -> bool {
        self.metrics.verified_command_starts > 0 || self.metrics.verified_command_finishes > 0
    }

    /// Returns true if any verified prompt marker (A or B) has been received.
    pub fn has_verified_prompt(&self) -> bool {
        self.metrics.prompt_count > 0 || self.metrics.input_count > 0
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
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            Some(0),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                execution_result: expected_exec,
            }
        );
        assert!(!sm.is_executing());
        assert_eq!(sm.current_command_id(), None);
        assert_eq!(sm.last_execution_result(), Some(expected_exec));
        assert_eq!(sm.last_result(), Some(CommandResult::Success));
    }

    #[test]
    fn normal_lifecycle_failure_code() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStarted);

        // D;127 -> Failure(127)
        let ev = sm.transition(Osc133Marker::CommandFinished(Some(127)));
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            Some(127),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                execution_result: expected_exec,
            }
        );
        assert_eq!(sm.last_execution_result(), Some(expected_exec));
        assert_eq!(sm.last_result(), Some(CommandResult::Failure(127)));
    }

    #[test]
    fn normal_lifecycle_missing_exit_code_is_unknown_not_success() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStarted);

        // D (None) -> Unknown result (MUST NOT be treated as Success)
        let ev = sm.transition(Osc133Marker::CommandFinished(None));
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );
        assert_eq!(sm.last_execution_result(), Some(expected_exec));
        assert_eq!(sm.last_result(), Some(CommandResult::Unknown));
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
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            Some(0),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev1,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );

        // Duplicate D: ignored, no second event fired
        let ev2 = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(ev2, None);
        assert_eq!(sm.metrics().orphaned_d_count, 1);
    }

    #[test]
    fn duplicate_c_preserves_active_id_and_completes_with_subsequent_d() {
        let mut sm = CommandStateMachine::new();
        let ev1 = sm.transition(Osc133Marker::CommandStarted);
        assert_eq!(
            ev1,
            Some(VerifiedCommandEvent::CommandStarted { command_id: 1 })
        );
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 1 });

        // Duplicate C: active command ID 1 is preserved, no new event emitted, anomaly counted
        let ev2 = sm.transition(Osc133Marker::CommandStarted);
        assert_eq!(ev2, None);
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 1 });
        assert_eq!(sm.metrics().duplicate_c_count, 1);
        assert_eq!(sm.metrics().verified_command_starts, 1);

        // Subsequent D;0 cleanly completes command 1
        let ev3 = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            Some(0),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev3,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                execution_result: expected_exec,
            }
        );
        assert_eq!(sm.metrics().verified_command_finishes, 1);
    }

    #[test]
    fn missing_d_recovered_by_prompt_start() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::CommandStarted);
        assert!(sm.is_executing());

        // A arrives while still executing: previous command closed as Unknown, prompt recovered
        let ev = sm.transition(Osc133Marker::PromptStarted);
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            None,
            None,
            CompletionReason::MissingD,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );
        assert_eq!(*sm.state(), CommandState::Prompt);
        assert_eq!(sm.metrics().missing_d_count, 1);
        assert_eq!(sm.last_execution_result(), Some(expected_exec));
    }

    #[test]
    fn shell_exit_aborts_active_command_as_unknown() {
        let mut sm = CommandStateMachine::new();
        sm.transition(Osc133Marker::CommandStarted);
        assert!(sm.is_executing());

        let ev = sm.on_shell_exit();
        let expected_exec = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            None,
            None,
            CompletionReason::ShellExited,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected_exec,
            })
        );
        assert_eq!(
            *sm.state(),
            CommandState::Finished {
                command_id: 1,
                execution_result: expected_exec,
            }
        );
        assert_eq!(sm.metrics().shell_exit_interrupted_count, 1);
        assert_eq!(sm.last_execution_result(), Some(expected_exec));

        // When not executing, on_shell_exit does nothing
        assert_eq!(sm.on_shell_exit(), None);
    }

    #[test]
    fn last_result_tracks_confirmed_command_results() {
        let mut sm = CommandStateMachine::new();
        assert_eq!(sm.last_result(), None);
        assert_eq!(sm.last_execution_result(), None);

        // Orphaned D does not record a last result
        sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(sm.last_result(), None);
        assert_eq!(sm.last_execution_result(), None);

        // Command 1 -> D;0
        sm.transition(Osc133Marker::CommandStarted);
        sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(sm.last_result(), Some(CommandResult::Success));

        // Prompt arrives -> last_result remains 0
        sm.transition(Osc133Marker::PromptStarted);
        assert_eq!(sm.last_result(), Some(CommandResult::Success));

        // Command 2 -> D;42
        sm.transition(Osc133Marker::CommandStarted);
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
    fn metadata_correlation_matching_and_status_roundtrip() {
        let mut sm = CommandStateMachine::new();

        // 1. Success with matching sid and seq
        let start_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_A".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(start_meta));
        assert_eq!(sm.active_correlation(), Some(("SESSION_A", 1)));

        let finish_meta_succ = Osc133Metadata {
            version: 1,
            session_id: "SESSION_A".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            None,
            finish_meta_succ,
        ));
        let expected = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected,
            })
        );
        assert_eq!(sm.last_execution_result(), Some(expected));
        assert_eq!(sm.active_correlation(), None);

        // 2. Failure with matching sid and seq
        let start_meta2 = Osc133Metadata {
            version: 1,
            session_id: "SESSION_A".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(start_meta2));
        assert_eq!(sm.active_correlation(), Some(("SESSION_A", 2)));

        let finish_meta_fail = Osc133Metadata {
            version: 1,
            session_id: "SESSION_A".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Failure,
        };
        let ev2 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            None,
            finish_meta_fail,
        ));
        let expected2 = CommandExecutionResult::new(
            ShellReportedStatus::Failure,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected2,
            })
        );
        assert_eq!(sm.last_execution_result(), Some(expected2));
    }

    #[test]
    fn numeric_exit_and_shell_status_orthogonality() {
        let mut sm = CommandStateMachine::new();

        // Native exit 0 with PowerShell failure (e.g. cmdlet error or redirection failure)
        let start_meta = Osc133Metadata {
            version: 1,
            session_id: "S1".into(),
            seq: 10,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(start_meta));

        let finish_meta = Osc133Metadata {
            version: 1,
            session_id: "S1".into(),
            seq: 10,
            shell_status: ShellReportedStatus::Failure,
        };
        let ev = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            Some(0),
            finish_meta,
        ));
        let expected = CommandExecutionResult::new(
            ShellReportedStatus::Failure,
            Some(0),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected,
            })
        );
        assert_eq!(sm.last_result(), Some(CommandResult::Success));
        assert_eq!(sm.last_execution_result(), Some(expected));

        // Native exit 42 with PowerShell success (e.g. exit 42; Write-Output ok)
        let start_meta2 = Osc133Metadata {
            version: 1,
            session_id: "S1".into(),
            seq: 11,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(start_meta2));

        let finish_meta2 = Osc133Metadata {
            version: 1,
            session_id: "S1".into(),
            seq: 11,
            shell_status: ShellReportedStatus::Success,
        };
        let ev2 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            Some(42),
            finish_meta2,
        ));
        let expected2 = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            Some(42),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected2,
            })
        );
        assert_eq!(sm.last_result(), Some(CommandResult::Failure(42)));
        assert_eq!(sm.last_execution_result(), Some(expected2));
    }

    #[test]
    fn stale_d_does_not_abort_active_command_and_later_matching_d_completes() {
        let mut sm = CommandStateMachine::new();

        // Command 2 starts with seq=2
        let start_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_STALE_TEST".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(start_meta));
        assert_eq!(sm.active_correlation(), Some(("SESSION_STALE_TEST", 2)));

        // Late D arrives with seq=1 (stale D from previous command)
        let stale_d_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_STALE_TEST".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev_stale = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            Some(0),
            stale_d_meta,
        ));

        // Must NOT complete command 2, must not fire event, must record stale anomaly
        assert_eq!(ev_stale, None);
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 1 });
        assert_eq!(sm.active_correlation(), Some(("SESSION_STALE_TEST", 2)));
        assert_eq!(sm.metrics().stale_d_count, 1);

        // Current legitimate D arrives with seq=2
        let current_d_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_STALE_TEST".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        };
        let ev_curr = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            None,
            current_d_meta,
        ));

        // Now command completes normally!
        let expected = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev_curr,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected,
            })
        );
        assert_eq!(sm.last_execution_result(), Some(expected));
        assert_eq!(sm.active_correlation(), None);
    }

    #[test]
    fn foreign_session_d_does_not_abort_active_command() {
        let mut sm = CommandStateMachine::new();

        // Active command in SESSION_PARENT with seq=1
        let start_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_PARENT".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(start_meta));
        assert_eq!(sm.active_correlation(), Some(("SESSION_PARENT", 1)));

        // D from child/alien session arrives
        let alien_d_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_CHILD".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev_alien = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            None,
            alien_d_meta,
        ));

        // Must NOT complete parent command
        assert_eq!(ev_alien, None);
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 1 });
        assert_eq!(sm.active_correlation(), Some(("SESSION_PARENT", 1)));
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);

        // Legitimate parent D arrives -> completes
        let parent_d_meta = Osc133Metadata {
            version: 1,
            session_id: "SESSION_PARENT".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev_parent = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            None,
            parent_d_meta,
        ));
        assert!(ev_parent.is_some());
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
    }

    #[test]
    fn seq_regression_or_reuse_in_same_session_rejects_metadata() {
        let mut sm = CommandStateMachine::new();

        // Command 1: C(seq=5) -> D(seq=5) completes cleanly
        let meta_c1 = Osc133Metadata {
            version: 1,
            session_id: "S_MONO".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(meta_c1));
        let meta_d1 = Osc133Metadata {
            version: 1,
            session_id: "S_MONO".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Success,
        };
        sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, meta_d1));
        assert_eq!(sm.last_confirmed_seq(), Some(5));

        // Command 2: C arrives with seq=4 (regression!) in same session
        let meta_c2_regress = Osc133Metadata {
            version: 1,
            session_id: "S_MONO".into(),
            seq: 4,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(meta_c2_regress));

        // Correlation must be rejected!
        assert_eq!(sm.active_correlation(), None);
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);

        // Subsequent D cannot match -> finishes with Mismatched and Unknown
        let meta_d2 = Osc133Metadata {
            version: 1,
            session_id: "S_MONO".into(),
            seq: 4,
            shell_status: ShellReportedStatus::Success,
        };
        let ev2 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(Some(0), meta_d2));
        let expected2 = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            Some(0),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Mismatched,
        );
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected2,
            })
        );
    }

    #[test]
    fn session_change_resets_seq_tracking() {
        let mut sm = CommandStateMachine::new();

        // Session 1 confirmed up to seq 10
        let c1 = Osc133Metadata {
            version: 1,
            session_id: "S1".into(),
            seq: 10,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c1));
        let d1 = Osc133Metadata {
            version: 1,
            session_id: "S1".into(),
            seq: 10,
            shell_status: ShellReportedStatus::Success,
        };
        sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, d1));
        assert_eq!(sm.last_confirmed_seq(), Some(10));
        assert_eq!(sm.current_session_id(), Some("S1"));

        // New session S2 starts with seq=1 (lower than 10, but allowed because session changed!)
        let c2 = Osc133Metadata {
            version: 1,
            session_id: "S2".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c2));
        assert_eq!(sm.current_session_id(), Some("S2"));
        assert_eq!(sm.active_correlation(), Some(("S2", 1)));

        let d2 = Osc133Metadata {
            version: 1,
            session_id: "S2".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev = sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, d2));
        let expected = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected,
            })
        );
    }

    #[test]
    fn expected_session_id_verification() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("EXPECTED_SID".into()));
        assert_eq!(sm.expected_session_id(), Some("EXPECTED_SID"));

        // C with mismatched SID
        let rogue_c = Osc133Metadata {
            version: 1,
            session_id: "ROGUE_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(rogue_c));
        assert_eq!(sm.active_correlation(), None);
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);

        sm.transition(Osc133Marker::CommandFinished(None));

        // C with matching SID
        let valid_c = Osc133Metadata {
            version: 1,
            session_id: "EXPECTED_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(valid_c));
        assert_eq!(sm.active_correlation(), Some(("EXPECTED_SID", 1)));

        let valid_d = Osc133Metadata {
            version: 1,
            session_id: "EXPECTED_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev = sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, valid_d));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
        assert!(ev.is_some());
    }

    #[test]
    fn legacy_and_metadata_cross_combinations() {
        let mut sm = CommandStateMachine::new();

        // 1. C with metadata, D legacy (no metadata) -> Mismatched
        let meta_c = Osc133Metadata {
            version: 1,
            session_id: "S_CROSS".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(meta_c));
        let ev1 = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(
            ev1,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    Some(0),
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Mismatched,
                ),
            })
        );

        // 2. C legacy, D with metadata -> Mismatched
        sm.transition(Osc133Marker::CommandStarted);
        let meta_d = Osc133Metadata {
            version: 1,
            session_id: "S_CROSS".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        };
        let ev2 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, meta_d));
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    None,
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Mismatched,
                ),
            })
        );
    }

    #[test]
    fn debug_log_sanitization_redacts_session_id() {
        let meta = Osc133Metadata {
            version: 1,
            session_id: "SECRET_PASSWORD_OR_SID_12345".into(),
            seq: 42,
            shell_status: ShellReportedStatus::Success,
        };
        let meta_debug = format!("{meta:?}");
        assert!(!meta_debug.contains("SECRET_PASSWORD_OR_SID_12345"));
        assert!(meta_debug.contains("<redacted>"));

        let corr = ActiveCorrelation {
            session_id: "SECRET_PASSWORD_OR_SID_12345".into(),
            seq: 42,
        };
        let corr_debug = format!("{corr:?}");
        assert!(!corr_debug.contains("SECRET_PASSWORD_OR_SID_12345"));
        assert!(corr_debug.contains("<redacted>"));

        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("EXPECTED_SECRET_SID".into()));
        sm.transition(Osc133Marker::CommandStartedWithMetadata(meta));
        let sm_debug = format!("{sm:?}");
        assert!(!sm_debug.contains("SECRET_PASSWORD_OR_SID_12345"));
        assert!(!sm_debug.contains("EXPECTED_SECRET_SID"));
        assert!(sm_debug.contains("<redacted>"));
    }

    #[test]
    fn future_seq_d_does_not_abort_active_command_and_subsequent_commands_succeed() {
        // N-1 Regression Test:
        // 1. sid=S_N1, seq=1 completes cleanly.
        // 2. sid=S_N1, seq=2 starts executing.
        // 3. sid=S_N1, seq=1_000_000 arrives (future seq D).
        //    Must NOT abort command 2, must NOT adopt numeric exit code, must NOT advance last_confirmed_seq.
        // 4. sid=S_N1, seq=2 legitimate D arrives -> completes with Matched and correct status.
        // 5. sid=S_N1, seq=3 executes and completes normally.
        let mut sm = CommandStateMachine::new();

        // Step 1: seq=1 completes cleanly
        let c1 = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c1));
        let d1 = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        sm.transition(Osc133Marker::CommandFinishedWithMetadata(Some(0), d1));
        assert_eq!(sm.last_confirmed_seq(), Some(1));

        // Step 2: seq=2 starts executing
        let c2 = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c2));
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 2 });
        assert_eq!(sm.active_correlation(), Some(("S_N1", 2)));

        // Step 3: rogue future seq D arrives (seq=1_000_000, exit code 99, status failure)
        let d_future = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 1_000_000,
            shell_status: ShellReportedStatus::Failure,
        };
        let ev_future = sm.transition(Osc133Marker::CommandFinishedWithMetadata(
            Some(99),
            d_future,
        ));

        // Must NOT abort active command 2, must not emit event, must not advance last_confirmed_seq
        assert_eq!(ev_future, None);
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 2 });
        assert_eq!(sm.active_correlation(), Some(("S_N1", 2)));
        assert_eq!(sm.last_confirmed_seq(), Some(1));
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);

        // Step 4: legitimate seq=2 D arrives (no exit code, status success)
        let d2 = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        };
        let ev2 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, d2));

        let expected2 = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected2,
            })
        );
        assert_eq!(sm.last_execution_result(), Some(expected2));
        assert_eq!(sm.active_correlation(), None);
        assert_eq!(sm.last_confirmed_seq(), Some(2));

        // Step 5: seq=3 starts and finishes normally
        let c3 = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c3));
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 3 });
        assert_eq!(sm.active_correlation(), Some(("S_N1", 3)));

        let d3 = Osc133Metadata {
            version: 1,
            session_id: "S_N1".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Success,
        };
        let ev3 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(Some(0), d3));
        let expected3 = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            Some(0),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev3,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 3,
                execution_result: expected3,
            })
        );
        assert_eq!(sm.last_confirmed_seq(), Some(3));
        // Verify desynchronized count did not increment again
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);
    }

    #[test]
    fn missing_d_does_not_advance_seq_and_next_command_resumes_monotonically() {
        // N-4 Regression Test:
        // Command 1: C(seq=1) -> D(seq=1) completes cleanly.
        // Command 2: C(seq=2) -> PromptStarted (MissingD, e.g. Ctrl+C).
        //   last_confirmed_seq must remain 1.
        // Command 3: Shell increments hook seq to 3 ($global:__kegon_seq++).
        //   C(seq=3) is accepted because 3 > 1.
        //   D(seq=3) completes cleanly with Matched.
        let mut sm = CommandStateMachine::new();

        let c1 = Osc133Metadata {
            version: 1,
            session_id: "S_N4".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c1));
        let d1 = Osc133Metadata {
            version: 1,
            session_id: "S_N4".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, d1));
        assert_eq!(sm.last_confirmed_seq(), Some(1));

        // Command 2 starts
        let c2 = Osc133Metadata {
            version: 1,
            session_id: "S_N4".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c2));
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 2 });

        // User hits Ctrl+C; shell prompt returns without D
        let ev_abort = sm.transition(Osc133Marker::PromptStarted);
        let expected_abort = CommandExecutionResult::new(
            ShellReportedStatus::Unknown,
            None,
            None,
            CompletionReason::MissingD,
            MetadataCorrelation::Unprovided,
        );
        assert_eq!(
            ev_abort,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected_abort,
            })
        );
        // N-4: last_confirmed_seq was NOT advanced to 2
        assert_eq!(sm.last_confirmed_seq(), Some(1));
        assert_eq!(sm.active_correlation(), None);

        // Command 3 starts with hook seq=3
        let c3 = Osc133Metadata {
            version: 1,
            session_id: "S_N4".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Unknown,
        };
        sm.transition(Osc133Marker::CommandStartedWithMetadata(c3));
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 3 });
        assert_eq!(sm.active_correlation(), Some(("S_N4", 3)));

        // D3 completes cleanly
        let d3 = Osc133Metadata {
            version: 1,
            session_id: "S_N4".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Success,
        };
        let ev3 = sm.transition(Osc133Marker::CommandFinishedWithMetadata(None, d3));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
        assert!(ev3.is_some());
        assert_eq!(sm.last_confirmed_seq(), Some(3));
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

    #[test]
    fn scheme_a_happy_path_with_monotonic_seq_and_status() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("PS_E_1".into()));

        // Command 1: success, codeless D
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "PS_E_1".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 1 });

        // Option E status arrives before D
        let meta1 = Osc133Metadata {
            version: 1,
            session_id: "PS_E_1".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        };
        let ev_meta = sm.transition(Osc133Marker::IndependentMetadata(meta1.clone()));
        assert_eq!(ev_meta, None);
        assert_eq!(sm.pending_metadata(), Some(&meta1));

        // Codeless D arrives
        let ev1 = sm.transition(Osc133Marker::CommandFinished(None));
        let expected1 = CommandExecutionResult::new(
            ShellReportedStatus::Success,
            None,
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev1,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: expected1,
            })
        );
        assert_eq!(sm.last_confirmed_seq(), Some(1));
        assert_eq!(sm.pending_metadata(), None);

        // Command 2: failure, numeric exit code 1
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "PS_E_1".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        }));
        assert_eq!(*sm.state(), CommandState::Executing { command_id: 2 });

        let meta2 = Osc133Metadata {
            version: 1,
            session_id: "PS_E_1".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Failure,
        };
        sm.transition(Osc133Marker::IndependentMetadata(meta2));
        let ev2 = sm.transition(Osc133Marker::CommandFinished(Some(1)));
        let expected2 = CommandExecutionResult::new(
            ShellReportedStatus::Failure,
            Some(1),
            None,
            CompletionReason::DObserved,
            MetadataCorrelation::Matched,
        );
        assert_eq!(
            ev2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: expected2,
            })
        );
        assert_eq!(sm.last_confirmed_seq(), Some(2));
    }

    #[test]
    fn scheme_a_c_and_metadata_seq_matched() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_MATCH".into()));

        // Command with seq=2
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_MATCH".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_MATCH".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        }));
        let ev_d = sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            ev_d,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Success,
                    None,
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Matched,
                ),
            })
        );
        assert_eq!(sm.last_confirmed_seq(), Some(2));
    }

    #[test]
    fn scheme_a_c_and_metadata_seq_mismatched_yields_unknown() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_MISMATCH".into()));

        // C has seq=2
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_MISMATCH".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        }));
        // Metadata has seq=3 (mismatch!)
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_MISMATCH".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Success,
        }));
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);

        let ev_d = sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(
            ev_d,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    Some(0),
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Mismatched,
                ),
            })
        );
        // last_confirmed_seq must not advance
        assert_eq!(sm.last_confirmed_seq(), None);
    }

    #[test]
    fn scheme_a_initial_seq_higher_than_one() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_INIT_5".into()));

        // First command starts at seq=5
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_INIT_5".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_INIT_5".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
        assert_eq!(
            sm.last_execution_result().unwrap().shell_status,
            ShellReportedStatus::Success
        );
        assert_eq!(sm.last_confirmed_seq(), Some(5));
    }

    #[test]
    fn scheme_a_resynchronizes_after_seq_gap() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_GAP".into()));

        // Command 1 at seq=1
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_GAP".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_GAP".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm.last_confirmed_seq(), Some(1));

        // Command 2 has gap in seq (e.g. subshell ran 3 commands): seq=5
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_GAP".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_GAP".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Failure,
        }));
        sm.transition(Osc133Marker::CommandFinished(Some(1)));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
        assert_eq!(
            sm.last_execution_result().unwrap().shell_status,
            ShellReportedStatus::Failure
        );
        assert_eq!(sm.last_confirmed_seq(), Some(5));
    }

    #[test]
    fn scheme_a_recovery_after_dropped_c_and_dropped_d() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_DROP".into()));

        // 1. Initial command completes cleanly at seq=1
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm.last_confirmed_seq(), Some(1));

        // 2. C is dropped for Command 2: state is Editing
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        // Meta and D arrive without C -> treated as orphaned
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm.metrics().orphaned_d_count, 2);
        assert_eq!(sm.last_confirmed_seq(), Some(1));

        // 3. Command 3 starts with C(seq=3) and recovers immediately!
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 3,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
        assert_eq!(sm.last_confirmed_seq(), Some(3));

        // 4. D is dropped for Command 4:
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 4,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 4,
            shell_status: ShellReportedStatus::Success,
        }));
        // D never arrives; prompt arrives (Ctrl+C / timeout)
        let ev_drop = sm.transition(Osc133Marker::PromptStarted);
        assert_eq!(
            sm.last_execution_result().unwrap().completion,
            CompletionReason::MissingD
        );
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Unprovided
        );
        assert_eq!(
            sm.last_execution_result().unwrap().shell_status,
            ShellReportedStatus::Unknown
        );
        // last_confirmed_seq remains at 3
        assert_eq!(sm.last_confirmed_seq(), Some(3));
        assert!(ev_drop.is_some());

        // 5. Command 5 starts with C(seq=5) and recovers immediately!
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DROP".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
        assert_eq!(sm.last_confirmed_seq(), Some(5));
    }

    #[test]
    fn scheme_a_duplicate_c_duplicate_d_duplicate_metadata() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_DUPS".into()));

        // 1. Duplicate C
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DUPS".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        // Duplicate C arrives while executing
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DUPS".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        assert_eq!(sm.metrics().duplicate_c_count, 1);
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DUPS".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Mismatched
        );
        assert_eq!(sm.last_confirmed_seq(), None);

        // 2. Duplicate metadata
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DUPS".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DUPS".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        }));
        // Duplicate metadata arrives
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_DUPS".into(),
            seq: 2,
            shell_status: ShellReportedStatus::Success,
        }));
        assert_eq!(sm.metrics().desynchronized_metadata_count, 3);
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Mismatched
        );
        assert_eq!(sm.last_confirmed_seq(), None);

        // 3. Duplicate D
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm.metrics().orphaned_d_count, 1);
    }

    #[test]
    fn scheme_a_normal_metadata_cannot_revive_after_invalid_metadata() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_REVIVE".into()));

        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_REVIVE".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        // InvalidMetadata arrives (Base64 decode or metadata parse failure)
        sm.transition(Osc133Marker::InvalidMetadata);
        assert_eq!(sm.metrics().desynchronized_metadata_count, 1);

        // Normal valid metadata arrives: must NOT revive
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_REVIVE".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        assert_eq!(sm.metrics().desynchronized_metadata_count, 2);

        sm.transition(Osc133Marker::CommandFinished(Some(0)));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Mismatched
        );
        assert_eq!(
            sm.last_execution_result().unwrap().shell_status,
            ShellReportedStatus::Unknown
        );
        assert_eq!(sm.last_confirmed_seq(), None);
    }

    #[test]
    fn scheme_a_stale_future_and_alien_sid() {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some("SID_ORIGIN".into()));

        // Establish seq=5
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 5,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm.last_confirmed_seq(), Some(5));

        // Case 1: Stale seq on C (seq=4 <= 5)
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 4,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 4,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Mismatched
        );
        assert_eq!(sm.last_confirmed_seq(), Some(5));

        // Case 2: Alien SID on metadata
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 6,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "ALIEN_SID".into(),
            seq: 6,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Mismatched
        );
        assert_eq!(sm.last_confirmed_seq(), Some(5));

        // Case 3: Future seq on metadata (C has seq=7, metadata has seq=100)
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 7,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "SID_ORIGIN".into(),
            seq: 100,
            shell_status: ShellReportedStatus::Success,
        }));
        sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            sm.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Mismatched
        );
        assert_eq!(sm.last_confirmed_seq(), Some(5));
    }

    #[test]
    fn scheme_a_legacy_osc133_compatibility() {
        let mut sm = CommandStateMachine::new();

        // 1. Legacy C (without metadata) + legacy D (without metadata)
        sm.transition(Osc133Marker::CommandStarted);
        let ev_d = sm.transition(Osc133Marker::CommandFinished(Some(42)));
        assert_eq!(
            ev_d,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 1,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    Some(42),
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Unprovided,
                ),
            })
        );
        assert_eq!(sm.last_confirmed_seq(), None);

        // 2. Legacy C + Option E metadata (cannot match, degrades to Mismatched)
        sm.transition(Osc133Marker::PromptStarted);
        sm.transition(Osc133Marker::InputStarted);
        sm.transition(Osc133Marker::CommandStarted);
        sm.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "LEGACY_TEST".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        let ev_d2 = sm.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(
            ev_d2,
            Some(VerifiedCommandEvent::CommandFinished {
                command_id: 2,
                execution_result: CommandExecutionResult::new(
                    ShellReportedStatus::Unknown,
                    None,
                    None,
                    CompletionReason::DObserved,
                    MetadataCorrelation::Mismatched,
                ),
            })
        );
        assert_eq!(sm.last_confirmed_seq(), None);
    }

    #[test]
    fn scheme_a_multi_tab_and_pty_reset() {
        // Tab A
        let mut sm_a = CommandStateMachine::new();
        sm_a.set_expected_session_id(Some("TAB_A_SID".into()));

        // Tab B
        let mut sm_b = CommandStateMachine::new();
        sm_b.set_expected_session_id(Some("TAB_B_SID".into()));

        // Tab A executes command 1
        sm_a.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "TAB_A_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm_a.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "TAB_A_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        sm_a.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm_a.last_confirmed_seq(), Some(1));

        // Tab B executes command 1 with its own SID
        sm_b.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "TAB_B_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Failure,
        }));
        sm_b.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "TAB_B_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Failure,
        }));
        sm_b.transition(Osc133Marker::CommandFinished(Some(1)));
        assert_eq!(sm_b.last_confirmed_seq(), Some(1));
        assert_eq!(
            sm_b.last_execution_result().unwrap().shell_status,
            ShellReportedStatus::Failure
        );

        // Tab A is completely isolated from Tab B
        assert_eq!(
            sm_a.last_execution_result().unwrap().shell_status,
            ShellReportedStatus::Success
        );

        // PTY respawn on Tab A: reset_session cleans tracking and previous results
        sm_a.reset_session();
        sm_a.set_expected_session_id(Some("TAB_A_NEW_SID".into()));
        assert_eq!(sm_a.last_confirmed_seq(), None);
        assert_eq!(sm_a.current_session_id(), None);
        assert_eq!(sm_a.last_execution_result(), None);
        assert_eq!(sm_a.last_verified_event(), None);
        assert_eq!(sm_a.state(), &CommandState::Unknown);

        // New PTY starts cleanly at seq=1
        sm_a.transition(Osc133Marker::CommandStartedWithMetadata(Osc133Metadata {
            version: 1,
            session_id: "TAB_A_NEW_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Unknown,
        }));
        sm_a.transition(Osc133Marker::IndependentMetadata(Osc133Metadata {
            version: 1,
            session_id: "TAB_A_NEW_SID".into(),
            seq: 1,
            shell_status: ShellReportedStatus::Success,
        }));
        sm_a.transition(Osc133Marker::CommandFinished(None));
        assert_eq!(sm_a.last_confirmed_seq(), Some(1));
        assert_eq!(
            sm_a.last_execution_result().unwrap().correlation,
            MetadataCorrelation::Matched
        );
    }
}
