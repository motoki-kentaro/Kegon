//! Shell resolution for launching terminal sessions.

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
}
