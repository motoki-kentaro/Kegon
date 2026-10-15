//! Shell resolution for launching terminal sessions.

use std::collections::HashMap;
use std::env;
use std::path::Path;

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

/// How a shell is started so that it reports its working directory, with nothing written to the
/// user's profile or configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub config: ShellConfig,
    /// Extra environment for the child only.
    pub env: HashMap<String, String>,
    /// Whether the shell was set up to report its working directory.
    pub integrated: bool,
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
        let plain = |config| Self {
            config,
            env: HashMap::new(),
            integrated: false,
        };
        if !enabled || !config.args.is_empty() {
            return plain(config);
        }
        match shell_kind(&config.program) {
            Some(ShellKind::PowerShell) => {
                let script = powershell_script(local_host);
                Self {
                    config: ShellConfig {
                        args: vec![
                            "-NoExit".to_string(),
                            "-EncodedCommand".to_string(),
                            encode_powershell_command(&script),
                        ],
                        ..config
                    },
                    env: HashMap::new(),
                    integrated: true,
                }
            }
            Some(ShellKind::Cmd) => {
                let prompt = format!("$E]9;9;$P$E\\{}", existing_prompt.unwrap_or("$P$G"));
                Self {
                    config,
                    env: HashMap::from([("PROMPT".to_string(), prompt)]),
                    integrated: true,
                }
            }
            None => plain(config),
        }
    }

    /// The launch for the default shell, honoring [`SHELL_INTEGRATION_ENV`].
    pub fn resolve_default() -> Self {
        let enabled = !env::var(SHELL_INTEGRATION_ENV).is_ok_and(|v| v.trim() == "0");
        Self::new(
            ShellConfig::resolve_default(),
            enabled,
            &crate::terminal::cwd::local_host_name(),
            env::var("PROMPT").ok().as_deref(),
        )
    }
}

/// PowerShell that wraps the current `prompt` so each prompt also reports the working directory.
///
/// The original prompt runs first, so `$?` and `$LASTEXITCODE` are what it expects. Everything is
/// inside `try`/`catch`: any failure (constrained language mode, a broken prompt) leaves the shell
/// as the profile made it.
fn powershell_script(local_host: &str) -> String {
    let host = local_host.replace('\'', "''");
    let template = r#"try {
  $global:__KegonPrompt = $function:prompt
  $global:__KegonHost = '@HOST@'
  if ($null -ne $global:__KegonPrompt) {
    function global:prompt {
      $original = & $global:__KegonPrompt
      try {
        $location = $ExecutionContext.SessionState.Path.CurrentLocation
        if ($location.Provider.Name -eq 'FileSystem') {
          $builder = New-Object System.Text.StringBuilder
          foreach ($byte in [System.Text.Encoding]::UTF8.GetBytes($location.ProviderPath.Replace('\', '/'))) {
            if (($byte -ge 48 -and $byte -le 57) -or ($byte -ge 65 -and $byte -le 90) -or ($byte -ge 97 -and $byte -le 122) -or $byte -in 45, 46, 95, 126, 47, 58) {
              [void]$builder.Append([char]$byte)
            } else {
              [void]$builder.AppendFormat('%{0:X2}', $byte)
            }
          }
          $path = $builder.ToString()
          if ($path.StartsWith('//')) { $uri = 'file://' + $path } else { $uri = 'file://' + $global:__KegonHost + '/' + $path }
          $esc = [char]27
          return ($esc + ']7;' + $uri + $esc + '\') + $original
        }
      } catch { }
      return $original
    }
  }
} catch { }"#;
    template.replace("@HOST@", &host)
}

/// `-EncodedCommand` takes base64 of the UTF-16LE script, which avoids every quoting problem.
fn encode_powershell_command(script: &str) -> String {
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
    fn powershell_gets_an_encoded_prompt_wrapper_and_no_env() {
        for program in [
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
            "powershell.exe",
        ] {
            let launch = Launch::new(config(program), true, "PC", None);
            assert!(launch.integrated);
            assert_eq!(launch.config.program, program);
            assert_eq!(launch.config.args[..2], ["-NoExit", "-EncodedCommand"]);
            assert!(launch.env.is_empty());
        }
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
