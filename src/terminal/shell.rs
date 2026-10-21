//! Shell resolution and launch specification for terminal sessions.
//!
//! This module separates:
//! - Determining which shell program to launch ([`ShellResolver`])
//! - Encapsulating the complete launch specification ([`ShellSpec`])
//! - Applying shell integration hooks ([`ResolvedShell::build_spec`] / [`ShellIntegrationLaunch`])
//!
//! ## Precedence
//! - **Windows**: custom setting -> `pwsh` (PATH or Program Files) -> `powershell` (PATH or System32) -> `cmd.exe` (ComSpec)
//! - **Unix**: custom setting -> `$SHELL` (if absolute & executable) -> user login shell (if absolute & executable) -> platform fallback (`/bin/bash`, `/bin/sh`; macOS: `/bin/zsh`, `/bin/bash`, `/bin/sh`)

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::env;
#[cfg(any(windows, test))]
use std::ffi::OsString;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::settings::{ShellIntegrationMode, ShellMode, TerminalSettings, validate_shell_path};
use crate::terminal::unix_integration::{self, IntegrationFiles, UserEnv};

/// Environment variable that turns Shell Integration off when set to `0`.
pub const SHELL_INTEGRATION_ENV: &str = "KEGON_SHELL_INTEGRATION";

/// Environment variable used to pass the session identifier to the shell child process.
///
/// ## Security & Trust Boundary Note (Phase 2A)
/// This environment variable is NOT an authentication secret or credential.
/// It is an internal correlation handle used to prevent cross-tab contamination,
/// reject orphaned subshell markers, and maintain sequence monotonicity.
pub const SESSION_ID_ENV: &str = "KEGON_SESSION_ID";

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generates a unique, collision-resistant session identifier for a terminal PTY session.
///
/// Format: `kegon-<counter_hex>-<random_hex16><random_hex16>` (e.g. `kegon-1-0123456789abcdef0123456789abcdef`).
pub fn generate_session_id() -> String {
    let count = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let r1 = RandomState::new().build_hasher().finish();
    let r2 = RandomState::new().build_hasher().finish();
    format!("kegon-{count:x}-{r1:016x}{r2:016x}")
}

/// Identified family/flavor of a shell executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellKind {
    PowerShell,
    WindowsPowerShell,
    Cmd,
    Bash,
    Zsh,
    Fish,
    Other,
}

impl ShellKind {
    /// Classifies a shell program by inspecting its filename (stem).
    pub fn from_program(program: &str) -> Self {
        let stem = Path::new(program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match stem.as_str() {
            "pwsh" => Self::PowerShell,
            "powershell" => Self::WindowsPowerShell,
            "cmd" => Self::Cmd,
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            "fish" => Self::Fish,
            _ => Self::Other,
        }
    }

    /// Whether this shell supports PowerShell-style OSC 133 encoded script injection.
    #[cfg(windows)]
    pub fn supports_powershell_integration(&self) -> bool {
        matches!(self, Self::PowerShell | Self::WindowsPowerShell)
    }

    /// Whether this shell supports cmd.exe PROMPT prefixing.
    #[allow(dead_code)]
    pub fn supports_cmd_integration(&self) -> bool {
        matches!(self, Self::Cmd)
    }
}

/// The OSC 133 integration injection disposition decided at launch time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIntegrationLaunch {
    /// Injected OSC 133 hooks into PowerShell.
    Injected,
    /// Explicitly disabled by user configuration or environment.
    Disabled,
    /// Shell does not support OSC 133 hooks (e.g. cmd.exe or generic executable).
    Unsupported,
}

/// Errors that can occur when resolving a shell to launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellResolveError {
    /// An explicit custom path was invalid or not executable.
    CustomPathInvalid { path: String, reason: String },
    /// No usable shell executable was found.
    NoShellFound { searched: Vec<String> },
    /// An unknown shell mode was specified in settings.
    UnknownMode(String),
}

impl fmt::Display for ShellResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CustomPathInvalid { path, reason } => {
                // Preserves the exact legacy format expected by diagnostics and tests:
                // "invalid terminal.shell_path <debug_path>: <reason>"
                write!(f, "invalid terminal.shell_path {path:?}: {reason}")
            }
            Self::NoShellFound { searched } => {
                write!(
                    f,
                    "no usable shell found (searched: {})",
                    searched.join(", ")
                )
            }
            Self::UnknownMode(raw) => {
                write!(f, "unknown terminal.shell_mode: {raw}")
            }
        }
    }
}

impl std::error::Error for ShellResolveError {}

/// Legacy shell program and argument holder, preserved for backwards compatibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    pub program: String,
    pub args: Vec<String>,
}

impl ShellConfig {
    /// Resolves the default shell for the current operating system.
    #[allow(dead_code)]
    pub fn resolve_default() -> Self {
        let resolver = ShellResolver::new();
        match resolver.resolve_default() {
            Ok(resolved) => Self {
                program: resolved.program,
                args: resolved.args,
            },
            Err(_) => {
                #[cfg(target_os = "windows")]
                {
                    Self {
                        program: env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string()),
                        args: vec![],
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    Self {
                        program: env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
                        args: vec![],
                    }
                }
            }
        }
    }
}

/// The raw resolved shell program, arguments, and metadata before integration injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShell {
    /// Executable path or command name.
    pub program: String,
    /// Arguments to launch the shell with.
    pub args: Vec<String>,
    /// Identified kind of the shell.
    pub kind: ShellKind,
    /// Whether the shell was resolved as a login shell intent (e.g. on macOS).
    pub login: bool,
}

impl ResolvedShell {
    /// Builds the final [`ShellSpec`] with integration hooks, environment variables,
    /// and session configuration.
    ///
    /// ## Invariant (F-1)
    /// `integration_launch == Injected` is returned IF AND ONLY IF the integration script
    /// was actually injected into the launch specification. If integration cannot be injected
    /// (e.g. existing arguments present, shell kind unsupported, or on Unix where pwsh injection
    /// is deferred to Slice 3), `integration_launch` MUST be [`ShellIntegrationLaunch::Unsupported`]
    /// (or [`ShellIntegrationLaunch::Disabled`] if toggled off).
    pub fn build_spec(
        self,
        integration_mode: ShellIntegrationMode,
        local_host: &str,
        existing_prompt: Option<&str>,
        working_directory: Option<PathBuf>,
    ) -> Result<ShellSpec, String> {
        // bash, zsh and fish are integrated on Unix only: on Windows the same names are MSYS or
        // Git builds whose paths and startup files the scripts do not account for.
        let unix = cfg!(unix).then(|| (UserEnv::from_process(), unix_integration::default_base()));
        self.build_spec_for(
            integration_mode,
            local_host,
            existing_prompt,
            working_directory,
            unix.as_ref().map(|(user, base)| (user, base.as_path())),
        )
    }

    /// [`Self::build_spec`] with the Unix integration's inputs given: the user's environment and
    /// the directory for the hook scripts, or `None` to leave bash, zsh and fish unintegrated.
    pub(crate) fn build_spec_for(
        self,
        integration_mode: ShellIntegrationMode,
        local_host: &str,
        existing_prompt: Option<&str>,
        working_directory: Option<PathBuf>,
        unix: Option<(&UserEnv, &Path)>,
    ) -> Result<ShellSpec, String> {
        let enabled = match integration_mode {
            ShellIntegrationMode::Auto => {
                !env::var(SHELL_INTEGRATION_ENV).is_ok_and(|v| v.trim() == "0")
            }
            ShellIntegrationMode::Disabled => false,
            ShellIntegrationMode::Unknown(ref raw) => {
                return Err(format!("unknown terminal.shell_integration: {raw:?}"));
            }
        };

        if !enabled {
            return Ok(ShellSpec {
                config: ShellConfig {
                    program: self.program,
                    args: self.args,
                },
                env: HashMap::new(),
                working_directory,
                kind: self.kind,
                login: self.login,
                integrated: false,
                integration_launch: ShellIntegrationLaunch::Disabled,
                session_id: None,
                integration_files: None,
            });
        }

        // Only inject PowerShell integration on Windows where ConPTY-targeted -EncodedCommand
        // script is implemented, and only when no conflicting arguments exist.
        #[cfg(target_os = "windows")]
        let can_inject_powershell =
            self.args.is_empty() && self.kind.supports_powershell_integration();
        #[cfg(not(target_os = "windows"))]
        let can_inject_powershell = false;

        let can_inject_cmd = self.args.is_empty() && self.kind.supports_cmd_integration();

        // `None` (an unknown argument, or scripts that could not be written) leaves the plain
        // shell, reported as `Unsupported`.
        let unix_plan = unix.and_then(|(user, base)| {
            let sid = generate_session_id();
            unix_integration::prepare(
                self.kind, &self.args, self.login, local_host, &sid, user, base,
            )
            .map(|plan| (sid, plan))
        });

        if can_inject_powershell {
            let script = powershell_script(local_host);
            let sid = generate_session_id();
            let env = HashMap::from([(SESSION_ID_ENV.to_string(), sid.clone())]);
            Ok(ShellSpec {
                config: ShellConfig {
                    program: self.program,
                    args: vec![
                        "-NoExit".to_string(),
                        "-EncodedCommand".to_string(),
                        encode_powershell_command(&script),
                    ],
                },
                env,
                working_directory,
                kind: self.kind,
                login: self.login,
                integrated: true,
                integration_launch: ShellIntegrationLaunch::Injected,
                session_id: Some(sid),
                integration_files: None,
            })
        } else if can_inject_cmd {
            let prompt = format!("$E]9;9;$P$E\\{}", existing_prompt.unwrap_or("$P$G"));
            Ok(ShellSpec {
                config: ShellConfig {
                    program: self.program,
                    args: vec![],
                },
                env: HashMap::from([("PROMPT".to_string(), prompt)]),
                working_directory,
                kind: self.kind,
                login: self.login,
                integrated: true,
                integration_launch: ShellIntegrationLaunch::Unsupported, // cmd has no OSC 133 hooks
                session_id: None,
                integration_files: None,
            })
        } else if let Some((sid, plan)) = unix_plan {
            Ok(ShellSpec {
                config: ShellConfig {
                    program: self.program,
                    args: plan.args,
                },
                env: plan.env,
                working_directory,
                kind: self.kind,
                login: self.login,
                integrated: true,
                integration_launch: ShellIntegrationLaunch::Injected,
                session_id: Some(sid),
                integration_files: Some(plan.files),
            })
        } else {
            // Integration is supported neither by shell nor by argument context.
            // Guarantees integrated=false and integration_launch=Unsupported.
            Ok(ShellSpec {
                config: ShellConfig {
                    program: self.program,
                    args: self.args,
                },
                env: HashMap::new(),
                working_directory,
                kind: self.kind,
                login: self.login,
                integrated: false,
                integration_launch: ShellIntegrationLaunch::Unsupported,
                session_id: None,
                integration_files: None,
            })
        }
    }
}

/// Complete specification for launching a terminal shell process.
///
/// ## Login Shell Contract (Slice 1 / Slice 2 Interface)
/// `args` is the authoritative source for command-line arguments passed to the shell process.
/// On platforms like macOS where a login shell is desired, `args` already contains `["-l"]`.
/// The `login` field is an informational metadata flag indicating the launch intent.
/// Platform PTY adapters (Slice 2) MUST pass `args` as-is to the PTY spawner, and MUST NOT
/// inject a second `-l` argument or alter argv[0] based solely on `login`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    /// Shell executable and initial arguments.
    pub config: ShellConfig,
    /// Extra environment variables for the child process.
    pub env: HashMap<String, String>,
    /// Initial working directory, if explicitly set.
    pub working_directory: Option<PathBuf>,
    /// Identified kind of the shell.
    pub kind: ShellKind,
    /// Whether this shell was resolved as a login shell intent.
    pub login: bool,
    /// Whether the shell was set up to report its working directory.
    pub integrated: bool,
    /// The OSC 133 semantic prompt integration disposition.
    pub integration_launch: ShellIntegrationLaunch,
    /// The session identifier generated for this session, if integration is active.
    pub session_id: Option<String>,
    /// Hook scripts of a Unix shell's integration; deleted when the last copy of this spec (and so
    /// the session) is dropped.
    pub integration_files: Option<IntegrationFiles>,
}

/// Backwards compatibility alias for [`ShellSpec`].
pub type Launch = ShellSpec;

impl ShellSpec {
    /// Whether the shell's own `OSC 133;C` / `D` (those without a `kegon.*` parameter) must be
    /// kept away from the command state machine.
    ///
    /// Only for a fish that Kegon actually integrated, under a session identity: fish 4 prints
    /// identity-less command markers by itself beside Kegon's own, which would turn Kegon's into a
    /// duplicate. Every other shell and every launch without integration reports all markers.
    pub fn drops_unmarked_command_markers(&self) -> bool {
        self.kind == ShellKind::Fish
            && self.integration_launch == ShellIntegrationLaunch::Injected
            && self.session_id.is_some()
    }

    /// Creates a ShellSpec directly (used by existing tests and callers).
    #[allow(dead_code)]
    pub fn new(
        config: ShellConfig,
        enabled: bool,
        local_host: &str,
        existing_prompt: Option<&str>,
    ) -> Self {
        let kind = ShellKind::from_program(&config.program);
        let resolved = ResolvedShell {
            program: config.program,
            args: config.args,
            kind,
            login: false,
        };
        let mode = if enabled {
            ShellIntegrationMode::Auto
        } else {
            ShellIntegrationMode::Disabled
        };
        resolved
            .build_spec(mode, local_host, existing_prompt, None)
            .expect("valid mode")
    }

    /// Resolves and builds a ShellSpec according to user settings.
    pub fn from_settings(
        settings: &TerminalSettings,
        existing_prompt: Option<&str>,
    ) -> Result<Self, String> {
        let resolver = ShellResolver::new();
        let resolved = resolver
            .resolve_from_settings(settings)
            .map_err(|e| e.to_string())?;

        resolved.build_spec(
            settings.shell_integration.clone(),
            &crate::terminal::cwd::local_host_name(),
            existing_prompt,
            None,
        )
    }

    /// Resolves the default shell for the platform and builds a ShellSpec.
    #[allow(dead_code)]
    pub fn resolve_default() -> Self {
        Self::from_settings(
            &TerminalSettings::default(),
            env::var("PROMPT").ok().as_deref(),
        )
        .unwrap_or_else(|_| {
            let resolver = ShellResolver::new();
            let resolved = resolver.resolve_default().unwrap_or_else(|_| {
                #[cfg(target_os = "windows")]
                {
                    ResolvedShell {
                        program: "cmd.exe".to_string(),
                        args: vec![],
                        kind: ShellKind::Cmd,
                        login: false,
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    ResolvedShell {
                        program: "/bin/sh".to_string(),
                        args: vec![],
                        kind: ShellKind::Other,
                        login: false,
                    }
                }
            });
            resolved
                .build_spec(
                    ShellIntegrationMode::Auto,
                    &crate::terminal::cwd::local_host_name(),
                    None,
                    None,
                )
                .expect("valid default spec")
        })
    }

    /// Returns the executable path or command name.
    #[allow(dead_code)]
    pub fn program(&self) -> &str {
        &self.config.program
    }

    /// Returns the command arguments.
    #[allow(dead_code)]
    pub fn args(&self) -> &[String] {
        &self.config.args
    }

    /// Sets or replaces the initial working directory for this [`ShellSpec`].
    pub fn with_working_directory(mut self, working_directory: Option<PathBuf>) -> Self {
        self.working_directory = working_directory;
        self
    }

    /// Returns the session ID generated for this launch, if Shell Integration is active.
    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Returns a [`ShellConfig`] view for compatibility with existing tests and callers.
    #[allow(dead_code)]
    pub fn config(&self) -> ShellConfig {
        ShellConfig {
            program: self.config.program.clone(),
            args: self.config.args.clone(),
        }
    }
}

/// Interface for querying the OS environment and file system during shell resolution.
pub trait ShellEnvironment {
    /// Returns the value of an environment variable.
    fn var(&self, key: &str) -> Option<String>;
    /// Returns the raw OsString value of an environment variable (Windows resolver: `PATH`).
    #[cfg(any(windows, test))]
    fn var_os(&self, key: &str) -> Option<OsString>;
    /// Checks whether `path` exists and is a regular file (Windows resolver).
    #[cfg(any(windows, test))]
    fn is_file(&self, path: &Path) -> bool;
    /// Checks whether `path` exists, is a regular file, and is executable.
    fn is_executable(&self, path: &Path) -> bool;
    /// Queries the current user's login shell from the system user database (e.g. getpwuid).
    #[allow(dead_code)]
    fn login_shell(&self) -> Option<String>;
}

/// Standard production environment querying the real OS.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsEnvironment;

impl ShellEnvironment for OsEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        env::var(key).ok()
    }

    #[cfg(any(windows, test))]
    fn var_os(&self, key: &str) -> Option<OsString> {
        env::var_os(key)
    }

    #[cfg(any(windows, test))]
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_executable(&self, path: &Path) -> bool {
        #[cfg(target_os = "windows")]
        {
            if !path.is_file() {
                return false;
            }
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = path.metadata() {
                meta.is_file() && (meta.permissions().mode() & 0o111 != 0)
            } else {
                false
            }
        }
        #[cfg(not(any(windows, unix)))]
        {
            path.is_file()
        }
    }

    fn login_shell(&self) -> Option<String> {
        #[cfg(unix)]
        {
            os_login_shell()
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

#[cfg(unix)]
fn os_login_shell() -> Option<String> {
    unsafe {
        let uid = libc::getuid();
        let mut pwd = std::mem::zeroed::<libc::passwd>();
        let mut buf = vec![0 as libc::c_char; 1024];
        let mut result: *mut libc::passwd = std::ptr::null_mut();

        loop {
            let res = libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result);
            if res == libc::ERANGE {
                buf.resize(buf.len() * 2, 0);
            } else if res == 0 && !result.is_null() {
                if pwd.pw_shell.is_null() {
                    return None;
                }
                let c_str = std::ffi::CStr::from_ptr(pwd.pw_shell);
                let shell_str = c_str.to_str().ok()?.trim();
                if shell_str.is_empty() {
                    return None;
                }
                return Some(shell_str.to_string());
            } else {
                return None;
            }
        }
    }
}

/// Resolves which shell executable and arguments should be launched.
#[derive(Debug, Clone)]
pub struct ShellResolver<E: ShellEnvironment = OsEnvironment> {
    env: E,
}

impl ShellResolver<OsEnvironment> {
    /// Creates a new ShellResolver using the real OS environment.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { env: OsEnvironment }
    }
}

impl Default for ShellResolver<OsEnvironment> {
    fn default() -> Self {
        Self::new()
    }
}

/// Checks whether a path is an absolute Unix path (begins with '/').
///
/// On Unix hosts this delegates to [Path::is_absolute]. On non-Unix hosts (e.g. Windows
/// test environments running Unix resolution tests), it checks whether the path begins with '/'.
fn is_unix_absolute_path(path: &Path) -> bool {
    #[cfg(unix)]
    {
        path.is_absolute()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().starts_with('/')
    }
}
impl<E: ShellEnvironment> ShellResolver<E> {
    /// Creates a ShellResolver with a custom or mock environment.
    #[allow(dead_code)]
    pub fn with_env(env: E) -> Self {
        Self { env }
    }

    /// Access the underlying environment.
    #[allow(dead_code)]
    pub fn env(&self) -> &E {
        &self.env
    }

    /// Resolves the default shell for the current operating system.
    pub fn resolve_default(&self) -> Result<ResolvedShell, ShellResolveError> {
        #[cfg(target_os = "windows")]
        {
            self.resolve_windows()
        }
        #[cfg(target_os = "macos")]
        {
            self.resolve_unix(true)
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            self.resolve_unix(false)
        }
        #[cfg(not(any(windows, unix)))]
        {
            self.resolve_unix(false)
        }
    }

    /// Resolves an explicitly configured custom shell executable path.
    ///
    /// Validates the path using [`validate_shell_path`], enforcing single-path rules
    /// and rejecting relative paths, directories, or unsupported extensions.
    pub fn resolve_custom(&self, shell_path: &str) -> Result<ResolvedShell, ShellResolveError> {
        let validated = validate_shell_path(shell_path).map_err(|err| {
            ShellResolveError::CustomPathInvalid {
                path: shell_path.to_string(),
                reason: format!("{err:?}"),
            }
        })?;
        let path_str = validated.to_string_lossy().into_owned();
        let kind = ShellKind::from_program(&path_str);
        Ok(ResolvedShell {
            program: path_str,
            args: vec![],
            kind,
            login: false,
        })
    }

    /// Resolves a shell according to the given [`TerminalSettings`].
    pub fn resolve_from_settings(
        &self,
        settings: &TerminalSettings,
    ) -> Result<ResolvedShell, ShellResolveError> {
        match settings.shell_mode {
            ShellMode::Auto => self.resolve_default(),
            ShellMode::Custom => self.resolve_custom(&settings.shell_path),
            ShellMode::Unknown(ref raw) => Err(ShellResolveError::UnknownMode(format!("{raw:?}"))),
        }
    }

    /// Resolves default shell using Windows precedence:
    /// 1. `pwsh.exe` in PATH or Program Files
    /// 2. `powershell.exe` in PATH or System32
    /// 3. `%ComSpec%` or `cmd.exe`
    ///
    /// Only `resolve_default` on Windows calls this. Unix test builds keep it so the precedence
    /// tests that do not depend on Windows path syntax run there too.
    #[cfg(any(windows, test))]
    pub fn resolve_windows(&self) -> Result<ResolvedShell, ShellResolveError> {
        let mut searched = Vec::new();

        // 1. pwsh in PATH or standard Program Files location
        searched.push("pwsh.exe".to_string());
        if let Some(pwsh) = self
            .find_in_path("pwsh.exe")
            .or_else(|| self.find_pwsh_standard())
        {
            return Ok(ResolvedShell {
                program: pwsh,
                args: vec![],
                kind: ShellKind::PowerShell,
                login: false,
            });
        }

        // 2. powershell in PATH or System32
        searched.push("powershell.exe".to_string());
        if let Some(ps) = self
            .find_in_path("powershell.exe")
            .or_else(|| self.find_system_powershell())
        {
            return Ok(ResolvedShell {
                program: ps,
                args: vec![],
                kind: ShellKind::WindowsPowerShell,
                login: false,
            });
        }

        // 3. ComSpec or cmd.exe
        searched.push("cmd.exe".to_string());
        let cmd = self
            .env
            .var("ComSpec")
            .unwrap_or_else(|| "cmd.exe".to_string());
        Ok(ResolvedShell {
            program: cmd,
            args: vec![],
            kind: ShellKind::Cmd,
            login: false,
        })
    }

    /// Resolves default shell using Unix precedence:
    /// 1. `$SHELL` (if absolute & executable)
    /// 2. User login shell from system database (if absolute & executable)
    /// 3. Platform safe fallbacks (macOS: `/bin/zsh`, `/bin/bash`, `/bin/sh`; Linux: `/bin/bash`, `/bin/sh`)
    ///
    /// ## Path Security (F-4)
    /// Relative paths in `$SHELL` or login shell are rejected to avoid cwd confusion and PATH spoofing.
    #[allow(dead_code)]
    pub fn resolve_unix(&self, is_macos: bool) -> Result<ResolvedShell, ShellResolveError> {
        let mut searched = Vec::new();

        // 1. $SHELL environment variable
        searched.push("$SHELL".to_string());
        if let Some(shell_env) = self.env.var("SHELL") {
            let trimmed = shell_env.trim();
            if !trimmed.is_empty() {
                let p = Path::new(trimmed);
                // Security: must be absolute path to avoid cwd confusion or PATH spoofing.
                if is_unix_absolute_path(p) && self.env.is_executable(p) {
                    let kind = ShellKind::from_program(trimmed);
                    let login = is_macos;
                    let args = if login {
                        vec!["-l".to_string()]
                    } else {
                        vec![]
                    };
                    return Ok(ResolvedShell {
                        program: trimmed.to_string(),
                        args,
                        kind,
                        login,
                    });
                }
            }
        }

        // 2. User login shell from system database
        searched.push("login_shell".to_string());
        if let Some(login_shell) = self.env.login_shell() {
            let trimmed = login_shell.trim();
            if !trimmed.is_empty() {
                let p = Path::new(trimmed);
                // Security: must be absolute path to avoid cwd confusion or PATH spoofing.
                if is_unix_absolute_path(p) && self.env.is_executable(p) {
                    let kind = ShellKind::from_program(trimmed);
                    let login = is_macos;
                    let args = if login {
                        vec!["-l".to_string()]
                    } else {
                        vec![]
                    };
                    return Ok(ResolvedShell {
                        program: trimmed.to_string(),
                        args,
                        kind,
                        login,
                    });
                }
            }
        }

        // 3. Platform safe fallback list (always absolute paths)
        let fallbacks: &[&str] = if is_macos {
            &["/bin/zsh", "/bin/bash", "/bin/sh"]
        } else {
            &["/bin/bash", "/bin/sh"]
        };

        for candidate in fallbacks {
            searched.push((*candidate).to_string());
            let p = Path::new(candidate);
            if is_unix_absolute_path(p) && self.env.is_executable(p) {
                let kind = ShellKind::from_program(candidate);
                let login = is_macos;
                let args = if login {
                    vec!["-l".to_string()]
                } else {
                    vec![]
                };
                return Ok(ResolvedShell {
                    program: (*candidate).to_string(),
                    args,
                    kind,
                    login,
                });
            }
        }

        Err(ShellResolveError::NoShellFound { searched })
    }

    #[cfg(any(windows, test))]
    fn find_in_path(&self, executable: &str) -> Option<String> {
        let path_var = self.env.var_os("PATH")?;
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(executable);
            if self.env.is_file(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        None
    }

    #[cfg(any(windows, test))]
    fn find_pwsh_standard(&self) -> Option<String> {
        if let Some(pf) = self.env.var("ProgramFiles") {
            let p = Path::new(&pf).join("PowerShell").join("7").join("pwsh.exe");
            if self.env.is_file(&p) {
                return Some(p.to_string_lossy().into_owned());
            }
        }
        None
    }

    #[cfg(any(windows, test))]
    fn find_system_powershell(&self) -> Option<String> {
        if let Some(windir) = self.env.var("SystemRoot") {
            let p = Path::new(&windir)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe");
            if self.env.is_file(&p) {
                return Some(p.to_string_lossy().into_owned());
            }
        }
        None
    }
}

/// PowerShell script template that injects OSC 133 Shell Integration and OSC 7 directory reporting.
fn powershell_script(local_host: &str) -> String {
    let host = local_host.replace('\'', "''");
    let template = include_str!("../../assets/shell-integration/powershell.ps1");
    template.replace("@HOST@", &host)
}

/// `-EncodedCommand` takes base64 of the UTF-16LE script, which avoids quoting problems.
pub(super) fn encode_powershell_command(script: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use std::collections::HashSet;

    /// Mock environment for deterministic, hermetic tests of shell resolution across platforms.
    #[derive(Debug, Clone, Default)]
    pub struct MockEnvironment {
        pub vars: HashMap<String, String>,
        pub os_vars: HashMap<String, OsString>,
        pub files: HashSet<PathBuf>,
        pub executables: HashSet<PathBuf>,
        pub login_shell: Option<String>,
    }

    impl MockEnvironment {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_var(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
            self.vars.insert(key.into(), val.into());
            self
        }

        pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
            self.files.insert(path.into());
            self
        }

        pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
            let p = path.into();
            self.files.insert(p.clone());
            self.executables.insert(p);
            self
        }

        pub fn with_login_shell(mut self, shell: impl Into<String>) -> Self {
            self.login_shell = Some(shell.into());
            self
        }
    }

    impl ShellEnvironment for MockEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn var_os(&self, key: &str) -> Option<OsString> {
            self.os_vars
                .get(key)
                .cloned()
                .or_else(|| self.vars.get(key).map(OsString::from))
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }

        fn is_executable(&self, path: &Path) -> bool {
            self.executables.contains(path)
        }

        fn login_shell(&self) -> Option<String> {
            self.login_shell.clone()
        }
    }

    /// A launch whose Shell Integration was injected, with its own session id, for tests of what
    /// happens downstream of injection (SID wiring, status, Dopamine) with synthetic markers.
    ///
    /// Windows injects into PowerShell for real. On Unix the tests must not depend on which shell
    /// is installed or how the user configured it, so there an inert child (`/bin/cat`: it runs
    /// nothing typed into it and prints no markers) is declared injected. Production code never
    /// builds such a spec.
    pub fn injected_launch(local_host: &str) -> ShellSpec {
        #[cfg(windows)]
        {
            let config = ShellConfig {
                program: "powershell.exe".to_string(),
                args: vec![],
            };
            ShellSpec::new(config, true, local_host, None)
        }
        #[cfg(not(windows))]
        {
            let _ = local_host;
            let sid = generate_session_id();
            ShellSpec {
                config: ShellConfig {
                    program: "/bin/cat".to_string(),
                    args: vec![],
                },
                env: HashMap::from([(SESSION_ID_ENV.to_string(), sid.clone())]),
                working_directory: None,
                kind: ShellKind::Other,
                login: false,
                integrated: true,
                integration_launch: ShellIntegrationLaunch::Injected,
                session_id: Some(sid),
                integration_files: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockEnvironment;
    use super::*;

    #[test]
    fn resolves_default_shell_successfully() {
        let config = ShellConfig::resolve_default();
        assert!(!config.program.is_empty());
    }

    fn config(program: &str) -> ShellConfig {
        ShellConfig {
            program: program.to_string(),
            args: vec![],
        }
    }

    #[test]
    fn powershell_gets_an_encoded_prompt_wrapper_and_session_id_env() {
        for program in [
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
            "powershell.exe",
        ] {
            let launch = Launch::new(config(program), true, "PC", None);
            assert_eq!(launch.config.program, program);
            #[cfg(windows)]
            {
                assert!(launch.integrated);
                assert_eq!(launch.integration_launch, ShellIntegrationLaunch::Injected);
                assert_eq!(launch.config.args[..2], ["-NoExit", "-EncodedCommand"]);
                assert_eq!(launch.env.len(), 1);
                let sid = launch
                    .env
                    .get(SESSION_ID_ENV)
                    .expect("must contain KEGON_SESSION_ID");
                assert!(crate::terminal::osc::is_valid_session_id(sid));
                assert_eq!(launch.session_id(), Some(sid.as_str()));
            }
            // Unix PowerShell integration is deferred to Slice 3: nothing is injected.
            #[cfg(not(windows))]
            {
                assert!(!launch.integrated);
                assert_eq!(
                    launch.integration_launch,
                    ShellIntegrationLaunch::Unsupported
                );
                assert!(launch.config.args.is_empty());
                assert!(launch.env.is_empty());
                assert_eq!(launch.session_id(), None);
            }
        }
    }

    #[test]
    fn session_id_generation_is_valid_unique_and_unpredictable() {
        let sid1 = generate_session_id();
        let sid2 = generate_session_id();
        assert_ne!(sid1, sid2);
        assert!(crate::terminal::osc::is_valid_session_id(&sid1));
        assert!(crate::terminal::osc::is_valid_session_id(&sid2));
        assert!(sid1.starts_with("kegon-"));
        assert!(sid2.starts_with("kegon-"));
        assert!(sid1.len() <= crate::terminal::osc::MAX_SID_LEN);
        assert!(sid2.len() <= crate::terminal::osc::MAX_SID_LEN);
    }

    #[test]
    fn cmd_gets_a_prompt_prefix_that_keeps_the_existing_prompt() {
        // `\` separates the path only under Windows path syntax.
        #[cfg(windows)]
        {
            let launch = Launch::new(config("C:\\Windows\\System32\\cmd.exe"), true, "PC", None);
            assert!(launch.config.args.is_empty());
            assert_eq!(launch.env["PROMPT"], "$E]9;9;$P$E\\$P$G");
        }
        let bare = Launch::new(config("cmd.exe"), true, "PC", None);
        assert!(bare.config.args.is_empty());
        assert_eq!(bare.env["PROMPT"], "$E]9;9;$P$E\\$P$G");

        let custom = Launch::new(config("cmd.exe"), true, "PC", Some("$T $P$G"));
        assert_eq!(custom.env["PROMPT"], "$E]9;9;$P$E\\$T $P$G");
    }

    #[test]
    fn other_shells_disabled_integration_and_custom_args_are_left_alone() {
        let untouched = |launch: &Launch, original: &ShellConfig| {
            assert!(!launch.integrated);
            assert_eq!(&launch.config, original);
            assert!(launch.env.is_empty());
        };
        // `sh` is integrated nowhere (bash, zsh and fish are, on Unix).
        let sh = config("/bin/sh");
        untouched(&Launch::new(sh.clone(), true, "PC", None), &sh);
        let pwsh = config("pwsh.exe");
        untouched(&Launch::new(pwsh.clone(), false, "PC", None), &pwsh);
        let with_args = ShellConfig {
            program: "pwsh.exe".to_string(),
            args: vec!["-NoProfile".to_string()],
        };
        untouched(
            &Launch::new(with_args.clone(), true, "PC", None),
            &with_args,
        );
    }

    #[test]
    fn encodes_powershell_commands_as_utf16le_base64() {
        assert_eq!(encode_powershell_command("a"), "YQA=");
        assert_eq!(encode_powershell_command("ab"), "YQBiAA==");
        assert_eq!(encode_powershell_command("abc"), "YQBiAGMA");
    }

    #[test]
    fn script_embeds_the_host_name_safely() {
        assert!(powershell_script("O'Brien-PC").contains("'O''Brien-PC'"));
    }

    #[test]
    fn powershell_script_contains_osc133_and_safety_guards() {
        let script = powershell_script("MY-HOST");
        assert!(script.contains("133;A"));
        assert!(script.contains("133;B"));
        assert!(script.contains("133;C"));
        assert!(script.contains("133;D"));
        assert!(script.contains("]7;"));
        assert!(script.contains("MY-HOST"));
        assert!(script.contains("__KegonState"));
        assert!(script.contains("FullLanguage"));
        assert!(script.contains("PSConsoleHostReadLine"));
        assert!(script.contains("133;C;kegon.v=1;kegon.sid="));
        assert!(script.contains("1337;SetUserVar=kegon_status="));
        assert!(script.contains("Seq"));
    }

    #[test]
    fn launch_from_settings_auto_auto() {
        let settings = crate::settings::TerminalSettings::default();
        let launch = Launch::from_settings(&settings, None).expect("should resolve auto launch");
        // Windows resolves PowerShell or cmd.exe, both set up to report their directory.
        #[cfg(windows)]
        assert!(launch.integrated);
        // On Unix, bash, zsh and fish are integrated and any other shell is left plain; either
        // way the invariant between the three fields holds.
        #[cfg(not(windows))]
        {
            let integrated_kind = matches!(
                launch.kind,
                ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish
            );
            assert_eq!(launch.session_id().is_some(), launch.integrated);
            if launch.integrated {
                assert!(integrated_kind);
                assert_eq!(launch.integration_launch, ShellIntegrationLaunch::Injected);
            } else {
                assert_eq!(
                    launch.integration_launch,
                    ShellIntegrationLaunch::Unsupported
                );
            }
        }
    }

    #[test]
    fn launch_from_settings_auto_disabled() {
        let mut settings = crate::settings::TerminalSettings::default();
        settings.shell_integration = crate::settings::ShellIntegrationMode::Disabled;
        let launch = Launch::from_settings(&settings, None).expect("should resolve");
        assert!(!launch.integrated);
        assert!(launch.config.args.is_empty());
        assert!(launch.env.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn launch_from_settings_custom_cmd() {
        let mut settings = crate::settings::TerminalSettings::default();
        settings.shell_mode = crate::settings::ShellMode::Custom;
        settings.shell_path = r"C:\Windows\System32\cmd.exe".to_string();
        settings.shell_integration = crate::settings::ShellIntegrationMode::Auto;

        let launch = Launch::from_settings(&settings, Some("$P$G")).expect("cmd launch");
        assert!(launch.integrated);
        assert_eq!(launch.config.program, r"C:\Windows\System32\cmd.exe");
        assert!(launch.config.args.is_empty());
        assert!(launch.env.contains_key("PROMPT"));
    }

    #[cfg(windows)]
    #[test]
    fn launch_from_settings_custom_powershell_and_disabled() {
        if let Some(pwsh_path) = installed::pwsh().or_else(installed::windows_powershell) {
            let mut settings = crate::settings::TerminalSettings::default();
            settings.shell_mode = crate::settings::ShellMode::Custom;
            settings.shell_path = pwsh_path.clone();
            settings.shell_integration = crate::settings::ShellIntegrationMode::Auto;

            let launch_auto = Launch::from_settings(&settings, None).expect("pwsh auto launch");
            assert!(launch_auto.integrated);
            assert_eq!(launch_auto.config.args.len(), 3);
            assert_eq!(launch_auto.config.args[0], "-NoExit");
            assert_eq!(launch_auto.config.args[1], "-EncodedCommand");

            settings.shell_integration = crate::settings::ShellIntegrationMode::Disabled;
            let launch_dis = Launch::from_settings(&settings, None).expect("pwsh disabled launch");
            assert!(!launch_dis.integrated);
            assert!(launch_dis.config.args.is_empty());
            assert!(launch_dis.env.is_empty());
        }
    }

    #[test]
    fn launch_from_settings_custom_non_shell_exe_is_untouched() {
        let notepad_path = r"C:\Windows\System32\notepad.exe".to_string();
        if std::path::Path::new(&notepad_path).exists() {
            let mut settings = crate::settings::TerminalSettings::default();
            settings.shell_mode = crate::settings::ShellMode::Custom;
            settings.shell_path = notepad_path.clone();
            settings.shell_integration = crate::settings::ShellIntegrationMode::Auto;

            let launch = Launch::from_settings(&settings, None).expect("other exe launch");
            assert!(!launch.integrated);
            assert_eq!(launch.config.program, notepad_path);
            assert!(launch.config.args.is_empty());
            assert!(launch.env.is_empty());
        }
    }

    #[test]
    fn launch_from_settings_custom_invalid_path_errors() {
        let mut settings = crate::settings::TerminalSettings::default();
        settings.shell_mode = crate::settings::ShellMode::Custom;
        settings.shell_path =
            r"C:\Windows\System32\definitely_not_a_real_shell_xyz.exe".to_string();

        let err = Launch::from_settings(&settings, None).unwrap_err();
        // F-3: Verifies exact legacy diagnostic format preservation
        assert!(
            err.contains("invalid terminal.shell_path"),
            "Expected 'invalid terminal.shell_path', got: {err}"
        );
    }

    #[test]
    fn launch_from_settings_unknown_integration_errors() {
        let mut settings = crate::settings::TerminalSettings::default();
        settings.shell_integration = crate::settings::ShellIntegrationMode::Unknown(
            toml::Value::String("bogus".to_string()),
        );

        let err = Launch::from_settings(&settings, None).unwrap_err();
        assert!(err.contains("unknown terminal.shell_integration"), "{err}");
    }

    // --- Slice 1: Unit tests for ShellResolver & ShellSpec ---

    #[test]
    fn shell_kind_classification() {
        assert_eq!(ShellKind::from_program("pwsh"), ShellKind::PowerShell);
        assert_eq!(ShellKind::from_program("pwsh.exe"), ShellKind::PowerShell);
        assert_eq!(
            ShellKind::from_program("/usr/bin/pwsh"),
            ShellKind::PowerShell
        );

        assert_eq!(
            ShellKind::from_program("powershell"),
            ShellKind::WindowsPowerShell
        );
        assert_eq!(
            ShellKind::from_program("powershell.exe"),
            ShellKind::WindowsPowerShell
        );

        assert_eq!(ShellKind::from_program("cmd"), ShellKind::Cmd);
        assert_eq!(ShellKind::from_program("cmd.exe"), ShellKind::Cmd);

        // `\` separates the path only under Windows path syntax.
        #[cfg(windows)]
        {
            assert_eq!(
                ShellKind::from_program(r"C:\Program Files\PowerShell\7\pwsh.exe"),
                ShellKind::PowerShell
            );
            assert_eq!(
                ShellKind::from_program(
                    r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
                ),
                ShellKind::WindowsPowerShell
            );
            assert_eq!(
                ShellKind::from_program(r"C:\Windows\System32\cmd.exe"),
                ShellKind::Cmd
            );
        }

        assert_eq!(ShellKind::from_program("bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_program("/bin/bash"), ShellKind::Bash);
        assert_eq!(
            ShellKind::from_program("/usr/local/bin/bash"),
            ShellKind::Bash
        );

        assert_eq!(ShellKind::from_program("zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::from_program("/bin/zsh"), ShellKind::Zsh);

        assert_eq!(ShellKind::from_program("fish"), ShellKind::Fish);
        assert_eq!(ShellKind::from_program("/usr/bin/fish"), ShellKind::Fish);

        assert_eq!(ShellKind::from_program("sh"), ShellKind::Other);
        assert_eq!(ShellKind::from_program("/bin/sh"), ShellKind::Other);
        assert_eq!(ShellKind::from_program("nu"), ShellKind::Other);
    }

    // These three search directories with the host's `std::path` and `env::split_paths`
    // (`;`-separated, `\`-joined only on Windows), so their Windows paths mean nothing elsewhere.
    // The ComSpec fallbacks below do not touch paths and run everywhere.
    #[cfg(windows)]
    #[test]
    fn windows_resolver_prefers_pwsh_in_path() {
        let env = MockEnvironment::new()
            .with_var("PATH", r"C:\tools;C:\Windows\System32")
            .with_file(r"C:\tools\pwsh.exe")
            .with_file(r"C:\Windows\System32\cmd.exe");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_windows().expect("should resolve");
        assert_eq!(res.program, r"C:\tools\pwsh.exe");
        assert_eq!(res.kind, ShellKind::PowerShell);
        assert_eq!(res.args, Vec::<String>::new());
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolver_finds_pwsh_in_program_files() {
        let env = MockEnvironment::new()
            .with_var("ProgramFiles", r"C:\Program Files")
            .with_file(r"C:\Program Files\PowerShell\7\pwsh.exe")
            .with_var("ComSpec", r"C:\Windows\System32\cmd.exe");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_windows().expect("should resolve");
        assert_eq!(res.program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(res.kind, ShellKind::PowerShell);
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolver_falls_back_to_powershell() {
        let env = MockEnvironment::new()
            .with_var("SystemRoot", r"C:\Windows")
            .with_file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
            .with_var("ComSpec", r"C:\Windows\System32\cmd.exe");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_windows().expect("should resolve");
        assert_eq!(
            res.program,
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        );
        assert_eq!(res.kind, ShellKind::WindowsPowerShell);
    }

    #[test]
    fn windows_resolver_falls_back_to_cmd() {
        let env = MockEnvironment::new().with_var("ComSpec", r"C:\Windows\System32\cmd.exe");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_windows().expect("should resolve");
        assert_eq!(res.program, r"C:\Windows\System32\cmd.exe");
        assert_eq!(res.kind, ShellKind::Cmd);
    }

    #[test]
    fn windows_resolver_comspec_missing_defaults_to_cmd() {
        let env = MockEnvironment::new();
        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_windows().expect("fallback to cmd");
        assert_eq!(res.program, "cmd.exe");
        assert_eq!(res.kind, ShellKind::Cmd);
    }

    // --- F-2: Custom shell resolution unification tests ---

    #[test]
    fn custom_shell_windows_rejects_non_exe() {
        let resolver = ShellResolver::new();
        for invalid in [
            r"C:\Windows\System32\test.cmd",
            r"C:\Windows\System32\test.bat",
            r"C:\Windows\System32\test.com",
            r"C:\Windows\System32\test",
        ] {
            let err = resolver.resolve_custom(invalid).unwrap_err();
            assert!(
                matches!(err, ShellResolveError::CustomPathInvalid { .. }),
                "expected rejection of {invalid}"
            );
        }
    }

    #[test]
    fn custom_shell_does_not_fallback_to_auto_on_failure() {
        let resolver = ShellResolver::new();
        let settings = TerminalSettings {
            shell_mode: ShellMode::Custom,
            shell_path: r"C:\Nonexistent\shell.exe".to_string(),
            ..Default::default()
        };
        let err = resolver.resolve_from_settings(&settings).unwrap_err();
        assert!(matches!(err, ShellResolveError::CustomPathInvalid { .. }));
    }

    // --- F-4: Unix absolute path enforcement & fallback tests ---

    #[test]
    fn unix_resolver_prefers_valid_absolute_shell_env() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "/usr/local/bin/fish")
            .with_executable("/usr/local/bin/fish")
            .with_login_shell("/bin/bash")
            .with_executable("/bin/bash");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_unix(false).expect("resolve fish");
        assert_eq!(res.program, "/usr/local/bin/fish");
        assert_eq!(res.kind, ShellKind::Fish);
        assert!(!res.login);
        assert!(res.args.is_empty());
    }

    #[test]
    fn unix_resolver_rejects_relative_shell_env_and_falls_back() {
        for relative in ["bash", "./bash", "../bin/bash"] {
            let env = MockEnvironment::new()
                .with_var("SHELL", relative)
                .with_executable(relative) // even if file exists relative to cwd
                .with_login_shell("/bin/bash")
                .with_executable("/bin/bash");

            let resolver = ShellResolver::with_env(env);
            let res = resolver
                .resolve_unix(false)
                .unwrap_or_else(|_| panic!("fallback for {relative}"));
            assert_eq!(
                res.program, "/bin/bash",
                "Relative SHELL={relative} must be rejected in favor of login shell"
            );
        }
    }

    #[test]
    fn unix_resolver_rejects_relative_login_shell_and_uses_platform_fallback() {
        let env = MockEnvironment::new()
            .with_login_shell("bash")
            .with_executable("bash")
            .with_executable("/bin/sh");

        let resolver = ShellResolver::with_env(env);
        let res = resolver
            .resolve_unix(false)
            .expect("must fallback to platform /bin/sh");
        assert_eq!(res.program, "/bin/sh");
    }

    #[test]
    fn unix_resolver_macos_applies_login_arg() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "/bin/zsh")
            .with_executable("/bin/zsh");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_unix(true).expect("resolve zsh on macos");
        assert_eq!(res.program, "/bin/zsh");
        assert_eq!(res.kind, ShellKind::Zsh);
        assert!(res.login);
        assert_eq!(res.args, vec!["-l"]);
    }

    #[test]
    fn unix_resolver_falls_back_when_shell_env_not_executable() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "/opt/broken/shell")
            .with_file("/opt/broken/shell") // not executable
            .with_login_shell("/bin/bash")
            .with_executable("/bin/bash");

        let resolver = ShellResolver::with_env(env);
        let res = resolver
            .resolve_unix(false)
            .expect("fallback to login shell");
        assert_eq!(res.program, "/bin/bash");
        assert_eq!(res.kind, ShellKind::Bash);
    }

    #[test]
    fn unix_resolver_falls_back_when_shell_env_missing() {
        let env = MockEnvironment::new()
            .with_login_shell("/bin/zsh")
            .with_executable("/bin/zsh");

        let resolver = ShellResolver::with_env(env);
        let res = resolver
            .resolve_unix(false)
            .expect("fallback to login shell");
        assert_eq!(res.program, "/bin/zsh");
    }

    #[test]
    fn unix_resolver_falls_back_when_shell_env_empty() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "   ")
            .with_login_shell("/bin/zsh")
            .with_executable("/bin/zsh");

        let resolver = ShellResolver::with_env(env);
        let res = resolver
            .resolve_unix(false)
            .expect("fallback to login shell");
        assert_eq!(res.program, "/bin/zsh");
    }

    #[test]
    fn unix_resolver_falls_back_to_platform_defaults_linux() {
        let env = MockEnvironment::new().with_executable("/bin/sh");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_unix(false).expect("fallback to /bin/sh");
        assert_eq!(res.program, "/bin/sh");
        assert_eq!(res.kind, ShellKind::Other);
    }

    #[test]
    fn unix_resolver_falls_back_to_platform_defaults_macos() {
        let env = MockEnvironment::new().with_executable("/bin/zsh");

        let resolver = ShellResolver::with_env(env);
        let res = resolver.resolve_unix(true).expect("fallback to /bin/zsh");
        assert_eq!(res.program, "/bin/zsh");
        assert_eq!(res.kind, ShellKind::Zsh);
        assert!(res.login);
        assert_eq!(res.args, vec!["-l"]);
    }

    #[test]
    fn unix_resolver_returns_error_when_all_fail() {
        let env = MockEnvironment::new();
        let resolver = ShellResolver::with_env(env);
        let err = resolver.resolve_unix(false).unwrap_err();
        assert!(matches!(err, ShellResolveError::NoShellFound { .. }));
    }

    #[test]
    fn unix_resolver_handles_path_with_spaces() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "/Users/john doe/bin/my shell")
            .with_executable("/Users/john doe/bin/my shell");

        let resolver = ShellResolver::with_env(env);
        let res = resolver
            .resolve_unix(false)
            .expect("resolve path with spaces");
        assert_eq!(res.program, "/Users/john doe/bin/my shell");
    }

    // --- F-1: Invariant & State Consistency Tests ---

    #[test]
    fn unix_pwsh_on_macos_does_not_claim_injected_integration() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "/opt/homebrew/bin/pwsh")
            .with_executable("/opt/homebrew/bin/pwsh");

        let resolver = ShellResolver::with_env(env);
        let resolved = resolver
            .resolve_unix(true) // macos -> args = ["-l"], login = true
            .expect("resolve macos pwsh");

        assert_eq!(resolved.program, "/opt/homebrew/bin/pwsh");
        assert_eq!(resolved.kind, ShellKind::PowerShell);
        assert!(resolved.login);
        assert_eq!(resolved.args, vec!["-l"]);

        // Build spec with Auto integration mode
        let spec = resolved
            .build_spec(ShellIntegrationMode::Auto, "HOST", None, None)
            .expect("build spec");

        // F-1 INVARIANT: integration_launch == Injected MUST ONLY occur when script is actually injected.
        assert_eq!(
            spec.integration_launch,
            ShellIntegrationLaunch::Unsupported,
            "macOS pwsh with -l must be Unsupported, never Injected"
        );
        assert!(
            !spec.integrated,
            "integrated must be false when not injected"
        );
        assert_eq!(
            spec.session_id, None,
            "session_id must be None when not injected"
        );
        assert_eq!(
            spec.args(),
            &["-l"],
            "arguments must be preserved untouched"
        );
    }

    #[test]
    fn unix_pwsh_login_shell_on_macos_is_unsupported() {
        let env = MockEnvironment::new()
            .with_login_shell("/bin/pwsh")
            .with_executable("/bin/pwsh");

        let resolver = ShellResolver::with_env(env);
        let resolved = resolver
            .resolve_unix(true)
            .expect("resolve macos login pwsh");
        let spec = resolved
            .build_spec(ShellIntegrationMode::Auto, "HOST", None, None)
            .expect("build spec");

        assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
        assert!(!spec.integrated);
        assert_eq!(spec.session_id, None);
    }

    #[test]
    fn linux_plain_pwsh_has_unsupported_integration_on_unix() {
        let env = MockEnvironment::new()
            .with_var("SHELL", "/usr/bin/pwsh")
            .with_executable("/usr/bin/pwsh");
        let resolver = ShellResolver::with_env(env);
        let resolved = resolver
            .resolve_unix(false) // linux -> login = false, args = []
            .expect("resolve linux pwsh");
        assert_eq!(resolved.program, "/usr/bin/pwsh");
        assert_eq!(resolved.kind, ShellKind::PowerShell);
        assert!(resolved.args.is_empty());

        let spec = resolved
            .build_spec(ShellIntegrationMode::Auto, "HOST", None, None)
            .expect("build spec");

        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
            assert!(!spec.integrated);
            assert_eq!(spec.session_id, None);
        }
        #[cfg(target_os = "windows")]
        {
            // On Windows host, Windows PowerShell injection activates for empty args
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Injected);
            assert!(spec.integrated);
            assert!(spec.session_id.is_some());
        }
    }

    #[test]
    fn windows_pwsh_plain_is_injected() {
        let resolved = ResolvedShell {
            program: r"C:\Program Files\PowerShell\7\pwsh.exe".to_string(),
            args: vec![],
            kind: ShellKind::PowerShell,
            login: false,
        };

        let spec = resolved
            .build_spec(ShellIntegrationMode::Auto, "HOST", None, None)
            .expect("build spec");

        #[cfg(target_os = "windows")]
        {
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Injected);
            assert!(spec.integrated);
            assert!(spec.session_id.is_some());
            assert_eq!(spec.args()[0], "-NoExit");
            assert_eq!(spec.args()[1], "-EncodedCommand");
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
            assert!(!spec.integrated);
        }
    }

    #[test]
    fn windows_pwsh_with_user_args_is_unsupported() {
        let resolved = ResolvedShell {
            program: "pwsh.exe".to_string(),
            args: vec!["-NoProfile".to_string()],
            kind: ShellKind::PowerShell,
            login: false,
        };

        let spec = resolved
            .build_spec(ShellIntegrationMode::Auto, "HOST", None, None)
            .expect("build spec");

        assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
        assert!(!spec.integrated);
        assert_eq!(spec.session_id, None);
        assert_eq!(spec.args(), &["-NoProfile"]);
    }

    #[test]
    fn resolved_shell_does_not_inject_powershell_into_unix_shells() {
        let resolved = ResolvedShell {
            program: "/bin/zsh".to_string(),
            args: vec!["-l".to_string()],
            kind: ShellKind::Zsh,
            login: true,
        };

        // Without the Unix inputs (as on Windows) a Unix shell is left exactly as resolved.
        let spec = resolved
            .build_spec_for(ShellIntegrationMode::Auto, "HOST", None, None, None)
            .expect("build spec");

        assert_eq!(spec.program(), "/bin/zsh");
        assert_eq!(spec.args(), &["-l"]);
        assert!(spec.env.is_empty());
        assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
        assert!(!spec.integrated);
        assert_eq!(spec.session_id, None);
        assert_eq!(spec.integration_files, None);
    }

    // --- Slice 3: Unix Shell Integration contract ---

    fn unix_spec(
        program: &str,
        args: &[&str],
        login: bool,
        mode: ShellIntegrationMode,
    ) -> ShellSpec {
        let base = std::env::temp_dir();
        ResolvedShell {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            kind: ShellKind::from_program(program),
            login,
        }
        .build_spec_for(
            mode,
            "HOST",
            None,
            None,
            Some((&UserEnv::default(), base.as_path())),
        )
        .expect("build spec")
    }

    /// The invariant the rest of Kegon relies on to trust OSC 133 events.
    fn assert_injection_invariant(spec: &ShellSpec) {
        if spec.integration_launch == ShellIntegrationLaunch::Injected {
            assert!(spec.integrated);
            let sid = spec
                .session_id
                .as_deref()
                .expect("injected has a session id");
            assert_eq!(spec.env.get(SESSION_ID_ENV).map(String::as_str), Some(sid));
            assert!(spec.integration_files.is_some());
        } else {
            assert!(!spec.integrated || spec.kind == ShellKind::Cmd);
            assert_eq!(spec.session_id, None);
            assert_eq!(spec.integration_files, None);
        }
    }

    #[test]
    fn auto_integrates_bash_zsh_and_fish_and_nothing_else() {
        if env::var(SHELL_INTEGRATION_ENV).is_ok_and(|v| v.trim() == "0") {
            return;
        }
        for program in ["/bin/bash", "/usr/bin/zsh", "/usr/local/bin/fish"] {
            let spec = unix_spec(program, &[], false, ShellIntegrationMode::Auto);
            assert_eq!(
                spec.integration_launch,
                ShellIntegrationLaunch::Injected,
                "{program}"
            );
            assert_injection_invariant(&spec);
            assert_eq!(spec.program(), program);
        }
        for program in ["/bin/sh", "/usr/bin/dash", "/usr/bin/nu", "/opt/odd/shell"] {
            let spec = unix_spec(program, &[], false, ShellIntegrationMode::Auto);
            assert_eq!(
                spec.integration_launch,
                ShellIntegrationLaunch::Unsupported,
                "{program}"
            );
            assert_injection_invariant(&spec);
            assert!(spec.args().is_empty() && spec.env.is_empty());
        }
    }

    #[test]
    fn disabled_stays_disabled_for_every_shell() {
        for program in ["/bin/bash", "/bin/zsh", "/usr/bin/fish", "/bin/sh"] {
            let spec = unix_spec(program, &["-l"], true, ShellIntegrationMode::Disabled);
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Disabled);
            assert_injection_invariant(&spec);
            // Exactly what the resolver chose, with no environment added.
            assert_eq!(spec.args(), &["-l"]);
            assert!(spec.env.is_empty());
        }
    }

    #[test]
    fn login_is_applied_once_and_only_through_args() {
        if env::var(SHELL_INTEGRATION_ENV).is_ok_and(|v| v.trim() == "0") {
            return;
        }
        let count = |spec: &ShellSpec| spec.args().iter().filter(|a| *a == "-l").count();
        // zsh and fish keep the resolver's `-l`, once.
        let zsh = unix_spec("/bin/zsh", &["-l"], true, ShellIntegrationMode::Auto);
        assert_eq!(count(&zsh), 1);
        assert!(zsh.login);
        let fish = unix_spec("/usr/bin/fish", &["-l"], true, ShellIntegrationMode::Auto);
        assert_eq!(count(&fish), 1);
        // A login bash is a real login shell or nothing: untouched, `Unsupported`.
        let bash = unix_spec("/bin/bash", &["-l"], true, ShellIntegrationMode::Auto);
        assert_eq!(bash.args(), &["-l"]);
        assert!(bash.login && bash.env.is_empty());
        assert_eq!(bash.integration_launch, ShellIntegrationLaunch::Unsupported);
        assert_injection_invariant(&bash);
        // Plain Linux start: no login anywhere.
        let linux = unix_spec("/bin/bash", &[], false, ShellIntegrationMode::Auto);
        assert_eq!(linux.integration_launch, ShellIntegrationLaunch::Injected);
        assert!(!linux.args().iter().any(|a| a == "-l"));
    }

    #[test]
    fn user_arguments_make_the_shell_unsupported_not_half_integrated() {
        for (program, args) in [
            ("/bin/bash", vec!["--norc"]),
            ("/bin/zsh", vec!["-f"]),
            ("/usr/bin/fish", vec!["--private"]),
        ] {
            let spec = unix_spec(program, &args, false, ShellIntegrationMode::Auto);
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
            assert_injection_invariant(&spec);
            assert_eq!(spec.args(), args.as_slice());
        }
    }

    #[test]
    fn every_session_gets_its_own_identity_and_scripts() {
        if env::var(SHELL_INTEGRATION_ENV).is_ok_and(|v| v.trim() == "0") {
            return;
        }
        let first = unix_spec("/bin/bash", &[], false, ShellIntegrationMode::Auto);
        let second = unix_spec("/bin/bash", &[], false, ShellIntegrationMode::Auto);
        assert_ne!(first.session_id, second.session_id);
        assert_ne!(
            first.integration_files.as_ref().unwrap().path(),
            second.integration_files.as_ref().unwrap().path()
        );
        assert_ne!(first.args(), second.args());
        let dir = first
            .integration_files
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();
        // Cloning the spec shares the scripts; the last owner removes them.
        let copy = first.clone();
        drop(first);
        assert!(dir.exists());
        drop(copy);
        assert!(!dir.exists());
    }

    #[test]
    fn the_unix_scripts_are_never_used_on_windows() {
        let resolved = ResolvedShell {
            program: r"C:\Program Files\Gitinash.exe".to_string(),
            args: vec![],
            kind: ShellKind::Bash,
            login: false,
        };
        let spec = resolved
            .build_spec(ShellIntegrationMode::Auto, "HOST", None, None)
            .expect("build spec");
        if cfg!(windows) {
            assert_eq!(spec.integration_launch, ShellIntegrationLaunch::Unsupported);
            assert_injection_invariant(&spec);
        }
    }

    // --- F-3: Error display format preservation test ---

    #[test]
    fn shell_resolve_error_display_formatting() {
        let err1 = ShellResolveError::CustomPathInvalid {
            path: r"C:\Windows\bad.exe".to_string(),
            reason: "NotFound".to_string(),
        };
        // F-3: Legacy format compatibility
        assert_eq!(
            err1.to_string(),
            r#"invalid terminal.shell_path "C:\\Windows\\bad.exe": NotFound"#
        );

        let err2 = ShellResolveError::NoShellFound {
            searched: vec!["$SHELL".to_string(), "/bin/sh".to_string()],
        };
        assert!(err2.to_string().contains("no usable shell found"));

        let err3 = ShellResolveError::UnknownMode("xyz".to_string());
        assert_eq!(err3.to_string(), "unknown terminal.shell_mode: xyz");
    }

    #[test]
    fn shell_spec_accessors_and_invariance() {
        let spec = ShellSpec::new(config("pwsh.exe"), true, "MY-PC", None);
        assert_eq!(spec.program(), "pwsh.exe");
        #[cfg(target_os = "windows")]
        {
            assert_eq!(spec.args().len(), 3);
            assert!(spec.session_id().is_some());
            assert!(spec.integrated);
        }
        assert_eq!(spec.kind, ShellKind::PowerShell);

        let config_view = spec.config();
        assert_eq!(config_view.program, "pwsh.exe");
    }

    #[test]
    fn only_an_injected_fish_with_an_identity_drops_unmarked_command_markers() {
        let mut spec = ShellSpec::new(config("pwsh.exe"), true, "MY-PC", None);
        let mut with = |kind, launch, sid: Option<&str>| {
            spec.kind = kind;
            spec.integration_launch = launch;
            spec.session_id = sid.map(str::to_string);
            spec.drops_unmarked_command_markers()
        };
        let injected = ShellIntegrationLaunch::Injected;
        assert!(with(ShellKind::Fish, injected, Some("kegon-1-a")));
        // Not injected, or no identity to expect: nothing is filtered.
        assert!(!with(
            ShellKind::Fish,
            ShellIntegrationLaunch::Disabled,
            None
        ));
        assert!(!with(
            ShellKind::Fish,
            ShellIntegrationLaunch::Unsupported,
            None
        ));
        assert!(!with(ShellKind::Fish, injected, None));
        // Other shells, injected or not.
        for kind in [
            ShellKind::Bash,
            ShellKind::Zsh,
            ShellKind::PowerShell,
            ShellKind::Cmd,
        ] {
            assert!(!with(kind, injected, Some("kegon-1-a")), "{kind:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_unix_launches_drop_unmarked_markers_only_for_integrated_fish() {
        let auto = ShellIntegrationMode::Auto;
        let fish = unix_spec("/usr/bin/fish", &[], false, auto.clone());
        assert_eq!(fish.integration_launch, ShellIntegrationLaunch::Injected);
        assert!(fish.drops_unmarked_command_markers());
        let login = unix_spec("/usr/bin/fish", &["-l"], true, auto.clone());
        assert!(login.drops_unmarked_command_markers());
        let disabled = unix_spec("/usr/bin/fish", &[], false, ShellIntegrationMode::Disabled);
        assert!(!disabled.drops_unmarked_command_markers());
        for shell in ["/bin/bash", "/usr/bin/zsh"] {
            let spec = unix_spec(shell, &[], false, auto.clone());
            assert!(!spec.drops_unmarked_command_markers(), "{shell}");
        }
    }
}

/// Locations of the Windows shells Shell Integration supports, for integration tests.
#[cfg(all(test, windows))]
pub(crate) mod installed {
    use super::*;

    pub fn pwsh() -> Option<String> {
        let resolver = ShellResolver::new();
        resolver
            .find_in_path("pwsh.exe")
            .or_else(|| resolver.find_pwsh_standard())
    }

    pub fn windows_powershell() -> Option<String> {
        let resolver = ShellResolver::new();
        resolver.find_system_powershell()
    }

    pub fn cmd() -> Option<String> {
        Some(env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string()))
    }
}
