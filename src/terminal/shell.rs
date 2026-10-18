//! Shell resolution for launching terminal sessions.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::env;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Resolved shell executable and optional initial arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    pub program: String,
    pub args: Vec<String>,
}

impl ShellConfig {
    /// Resolves the default shell for the current operating system.
    ///
    /// On Windows, prioritizes PowerShell Core (`pwsh`), falling back to
    /// Windows PowerShell (`powershell.exe`) and finally `cmd.exe`.
    pub fn resolve_default() -> Self {
        #[cfg(target_os = "windows")]
        {
            // 1. Check if `pwsh` is available in PATH or standard location.
            if let Some(pwsh) = find_in_path("pwsh.exe").or_else(find_pwsh_standard_path) {
                return Self {
                    program: pwsh,
                    args: vec![],
                };
            }

            // 2. Fallback to Windows PowerShell (`powershell.exe`).
            if let Some(powershell) = find_in_path("powershell.exe").or_else(find_system_powershell)
            {
                return Self {
                    program: powershell,
                    args: vec![],
                };
            }

            // 3. Ultimate fallback: Command Prompt (`cmd.exe`).
            Self {
                program: env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string()),
                args: vec![],
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            Self {
                program: shell,
                args: vec![],
            }
        }
    }
}

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
///
/// Guarantees:
/// - Fits within `crate::terminal::osc::MAX_SID_LEN` (64 bytes).
/// - Contains only ASCII alphanumeric characters and hyphens (`[a-zA-Z0-9_-]`).
/// - Passes [`crate::terminal::osc::is_valid_session_id`].
/// - Strictly distinct across multiple calls even within the same millisecond or process.
/// - Provides per-session distributed randomness from `RandomState` entropy and an atomic counter.
///
/// ## Security & Trust Boundary Note (L-3)
/// The session identifier is an internal correlation handle for tab disambiguation and subshell
/// filtering. It is NOT an authentication token, credential, or cryptographic secret.
pub fn generate_session_id() -> String {
    let count = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let r1 = RandomState::new().build_hasher().finish();
    let r2 = RandomState::new().build_hasher().finish();
    format!("kegon-{count:x}-{r1:016x}{r2:016x}")
}

/// The shells Shell Integration knows how to make report their working directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    PowerShell,
    Cmd,
}

fn shell_kind(program: &str) -> Option<ShellKind> {
    let stem = Path::new(program)
        .file_stem()?
        .to_string_lossy()
        .to_ascii_lowercase();
    match stem.as_str() {
        "pwsh" | "powershell" => Some(ShellKind::PowerShell),
        "cmd" => Some(ShellKind::Cmd),
        _ => None,
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

/// How a shell is started so that it reports its working directory, with nothing written to the
/// user's profile or configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub config: ShellConfig,
    /// Extra environment for the child only.
    pub env: HashMap<String, String>,
    /// Whether the shell was set up to report its working directory.
    pub integrated: bool,
    /// The OSC 133 integration launch disposition for this session.
    pub integration_launch: ShellIntegrationLaunch,
    /// The session identifier generated for this PTY session, if integration is active.
    pub session_id: Option<String>,
}

impl Launch {
    /// Adds Shell Integration to `config` for shells that support it, and leaves any other shell,
    /// or a shell that already has arguments, exactly as it is.
    ///
    /// * PowerShell (`pwsh`, `powershell`): runs a small script *after* the user's profile via
    ///   `-NoExit -EncodedCommand`. It wraps whatever `prompt` function exists by then and only
    ///   prepends an invisible OSC 7 sequence, so prompt frameworks keep working. No profile is
    ///   modified, and `-Command` text is not subject to the execution policy.
    /// * `cmd.exe`: prefixes the child's `PROMPT` variable with an OSC 9;9 sequence.
    ///
    /// `existing_prompt` is the inherited `PROMPT` value, if any. `enabled == false` returns the
    /// shell untouched.
    pub fn new(
        config: ShellConfig,
        enabled: bool,
        local_host: &str,
        existing_prompt: Option<&str>,
    ) -> Self {
        let integration_launch = if !enabled {
            ShellIntegrationLaunch::Disabled
        } else {
            match shell_kind(&config.program) {
                Some(ShellKind::PowerShell) => ShellIntegrationLaunch::Injected,
                _ => ShellIntegrationLaunch::Unsupported,
            }
        };

        let plain = |config| Self {
            config,
            env: HashMap::new(),
            integrated: false,
            integration_launch,
            session_id: None,
        };
        if !enabled || !config.args.is_empty() {
            return plain(config);
        }
        match shell_kind(&config.program) {
            Some(ShellKind::PowerShell) => {
                let script = powershell_script(local_host);
                let sid = generate_session_id();
                let env = HashMap::from([(SESSION_ID_ENV.to_string(), sid.clone())]);
                Self {
                    config: ShellConfig {
                        args: vec![
                            "-NoExit".to_string(),
                            "-EncodedCommand".to_string(),
                            encode_powershell_command(&script),
                        ],
                        ..config
                    },
                    env,
                    integrated: true,
                    integration_launch,
                    session_id: Some(sid),
                }
            }
            Some(ShellKind::Cmd) => {
                let prompt = format!("$E]9;9;$P$E\\{}", existing_prompt.unwrap_or("$P$G"));
                Self {
                    config,
                    env: HashMap::from([("PROMPT".to_string(), prompt)]),
                    integrated: true,
                    integration_launch,
                    session_id: None,
                }
            }
            None => plain(config),
        }
    }

    /// Returns the session ID generated for this launch, if Shell Integration is active.
    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Resolves a launch from user settings, honoring custom executable paths and integration toggles.
    pub fn from_settings(
        settings: &crate::settings::TerminalSettings,
        existing_prompt: Option<&str>,
    ) -> Result<Self, String> {
        let enabled = match settings.shell_integration {
            crate::settings::ShellIntegrationMode::Auto => {
                !env::var(SHELL_INTEGRATION_ENV).is_ok_and(|v| v.trim() == "0")
            }
            crate::settings::ShellIntegrationMode::Disabled => false,
            crate::settings::ShellIntegrationMode::Unknown(ref raw) => {
                return Err(format!("unknown terminal.shell_integration: {raw:?}"));
            }
        };

        let config = match settings.shell_mode {
            crate::settings::ShellMode::Auto => ShellConfig::resolve_default(),
            crate::settings::ShellMode::Custom => {
                let validated_path = crate::settings::validate_shell_path(&settings.shell_path)
                    .map_err(|err| {
                        format!(
                            "invalid terminal.shell_path {:?}: {:?}",
                            settings.shell_path, err
                        )
                    })?;
                ShellConfig {
                    program: validated_path.to_string_lossy().into_owned(),
                    args: vec![],
                }
            }
            crate::settings::ShellMode::Unknown(ref raw) => {
                return Err(format!("unknown terminal.shell_mode: {raw:?}"));
            }
        };

        Ok(Self::new(
            config,
            enabled,
            &crate::terminal::cwd::local_host_name(),
            existing_prompt,
        ))
    }

    /// The launch for the default shell, honoring [`SHELL_INTEGRATION_ENV`].
    #[allow(dead_code)]
    pub fn resolve_default() -> Self {
        Self::from_settings(
            &crate::settings::TerminalSettings::default(),
            env::var("PROMPT").ok().as_deref(),
        )
        .unwrap_or_else(|_| {
            Self::new(
                ShellConfig::resolve_default(),
                true,
                &crate::terminal::cwd::local_host_name(),
                None,
            )
        })
    }
}

/// PowerShell that injects OSC 133 Shell Integration and OSC 7 working directory reporting.
///
/// The original prompt and readline functions are preserved and invoked so `$?`, `$LASTEXITCODE`,
/// and custom prompt themes (Oh My Posh, Starship) keep working. Everything is guarded by
/// `try`/`catch`, language mode checks, and module checks to ensure graceful degradation.
fn powershell_script(local_host: &str) -> String {
    let host = local_host.replace('\'', "''");
    let template = include_str!("../../assets/shell-integration/powershell.ps1");
    template.replace("@HOST@", &host)
}

/// `-EncodedCommand` takes base64 of the UTF-16LE script, which avoids every quoting problem.
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

fn find_in_path(executable: &str) -> Option<String> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(executable);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_pwsh_standard_path() -> Option<String> {
    if let Ok(pf) = env::var("ProgramFiles") {
        let p = Path::new(&pf).join("PowerShell").join("7").join("pwsh.exe");
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_system_powershell() -> Option<String> {
    if let Ok(windir) = env::var("SystemRoot") {
        let p = Path::new(&windir)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
            assert!(launch.integrated);
            assert_eq!(launch.config.program, program);
            assert_eq!(launch.config.args[..2], ["-NoExit", "-EncodedCommand"]);
            assert_eq!(launch.env.len(), 1);
            let sid = launch
                .env
                .get(SESSION_ID_ENV)
                .expect("must contain KEGON_SESSION_ID");
            assert!(crate::terminal::osc::is_valid_session_id(sid));
            assert_eq!(launch.session_id(), Some(sid.as_str()));
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
        let launch = Launch::new(config("C:\\Windows\\System32\\cmd.exe"), true, "PC", None);
        assert!(launch.config.args.is_empty());
        assert_eq!(launch.env["PROMPT"], "$E]9;9;$P$E\\$P$G");

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
        let bash = config("/bin/bash");
        untouched(&Launch::new(bash.clone(), true, "PC", None), &bash);
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
        // FinalTerm / OSC 133 sequences
        assert!(
            script.contains("133;A"),
            "Must contain prompt start marker (A)"
        );
        assert!(
            script.contains("133;B"),
            "Must contain input start marker (B)"
        );
        assert!(
            script.contains("133;C"),
            "Must contain command start marker (C)"
        );
        assert!(
            script.contains("133;D"),
            "Must contain command finished marker (D)"
        );
        // OSC 7 working directory sequence
        assert!(
            script.contains("]7;"),
            "Must contain OSC 7 directory report"
        );
        assert!(script.contains("MY-HOST"), "Must embed provided hostname");
        // Safety guards
        assert!(
            script.contains("__KegonState"),
            "Must have multi-load state guard"
        );
        assert!(
            script.contains("FullLanguage"),
            "Must check for full language mode"
        );
        assert!(
            script.contains("PSConsoleHostReadLine"),
            "Must hook PSConsoleHostReadLine for command boundaries"
        );
        assert!(
            script.contains("133;C;kegon.v=1;kegon.sid="),
            "Must contain Scheme A OSC 133 C start marker with sid/seq"
        );
        // Option E independent status notification
        assert!(
            script.contains("1337;SetUserVar=kegon_status="),
            "Must contain OSC 1337 status notification"
        );
        assert!(
            script.contains("Seq"),
            "Must maintain monotonic sequence state"
        );
    }

    #[test]
    fn launch_from_settings_auto_auto() {
        let settings = crate::settings::TerminalSettings::default();
        let launch = Launch::from_settings(&settings, None).expect("should resolve auto launch");
        assert!(launch.integrated);
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
        assert!(err.contains("invalid terminal.shell_path"), "{err}");
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
}

/// Locations of the Windows shells Shell Integration supports, for integration tests.
#[cfg(all(test, windows))]
pub(crate) mod installed {
    use super::*;

    pub fn pwsh() -> Option<String> {
        find_in_path("pwsh.exe").or_else(find_pwsh_standard_path)
    }

    pub fn windows_powershell() -> Option<String> {
        find_system_powershell()
    }

    pub fn cmd() -> Option<String> {
        Some(env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string()))
    }
}
