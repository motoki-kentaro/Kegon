//! Shell Integration for the Unix shells Kegon supports: bash, zsh and fish.
//!
//! Each shell gets its own mechanism, and none of them writes to the user's dotfiles. Kegon
//! creates a private, per-session directory, writes the hook scripts into it, and starts the shell
//! so that it loads them *after* the user's own startup files:
//!
//! | shell | mechanism                                                                    |
//! |-------|------------------------------------------------------------------------------|
//! | bash  | `--rcfile <dir>/bash.sh` (non-login only; bash reads the system rc itself, the script sources `~/.bashrc` and then hooks `PROMPT_COMMAND` and the `DEBUG` trap) |
//! | zsh   | `ZDOTDIR=<dir>`; stand-in `.zshenv`/`.zprofile`/`.zshrc` forward to the user's files, then hook `preexec`/`precmd`; `ZDOTDIR` is handed back afterwards |
//! | fish  | `--init-command 'source "$KEGON_FISH_INIT"'`, which fish runs after `config.fish`; hooks `fish_preexec`/`fish_prompt` |
//!
//! The directory is owned by [`IntegrationFiles`], which removes it when the last copy of the
//! launch specification (and so the terminal session) is dropped, or when the launch fails.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::shell::{SESSION_ID_ENV, ShellKind};

const BASH_SCRIPT: &str = include_str!("../../assets/shell-integration/bash.sh");
const ZSH_ENV_SCRIPT: &str = include_str!("../../assets/shell-integration/zsh/zshenv");
const ZSH_PROFILE_SCRIPT: &str = include_str!("../../assets/shell-integration/zsh/zprofile");
const ZSH_RC_SCRIPT: &str = include_str!("../../assets/shell-integration/zsh/zshrc");
const FISH_SCRIPT: &str = include_str!("../../assets/shell-integration/fish.fish");

/// Path of the fish script, read by the fixed `--init-command` text.
pub const FISH_INIT_ENV: &str = "KEGON_FISH_INIT";
/// The user's own `ZDOTDIR` (empty when unset) that the stand-in zsh files forward to.
pub const USER_ZDOTDIR_ENV: &str = "KEGON_USER_ZDOTDIR";

const FISH_INIT_COMMAND: &str = "source \"$KEGON_FISH_INIT\"";

/// Parts of the parent environment the injection depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserEnv {
    /// `ZDOTDIR` as the user has it, if set.
    pub zdotdir: Option<String>,
    /// `fish_features` as the user has it, if set.
    pub fish_features: Option<String>,
}

impl UserEnv {
    /// Reads the values from the process environment.
    pub fn from_process() -> Self {
        Self {
            zdotdir: std::env::var("ZDOTDIR").ok(),
            fish_features: std::env::var("fish_features").ok(),
        }
    }
}

/// What it takes to start a shell with its integration loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The complete argument list for the shell.
    pub args: Vec<String>,
    /// Environment for the child process; always holds the session identifier.
    pub env: HashMap<String, String>,
    /// The scripts' directory, deleted once nothing refers to it any more.
    pub files: IntegrationFiles,
}

/// A private directory with the hook scripts of one session.
#[derive(Debug, Clone)]
pub struct IntegrationFiles(Arc<ScriptDir>);

#[derive(Debug)]
struct ScriptDir {
    path: PathBuf,
}

impl Drop for ScriptDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl PartialEq for IntegrationFiles {
    fn eq(&self, other: &Self) -> bool {
        self.0.path == other.0.path
    }
}

impl Eq for IntegrationFiles {}

impl IntegrationFiles {
    /// The directory holding the scripts.
    pub fn path(&self) -> &Path {
        &self.0.path
    }
}

/// Where the per-session directory is created: the user's runtime directory when there is one,
/// otherwise the system temporary directory.
pub fn default_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// Whether Kegon can integrate this shell kind at all.
pub fn supports(kind: ShellKind) -> bool {
    matches!(kind, ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish)
}

/// Prepares the integration of a shell, or returns `None` when the shell cannot be integrated
/// safely: an unsupported kind, arguments Kegon does not understand, or files that could not be
/// created. The caller then starts the plain shell and reports `Unsupported`.
///
/// `args` and `login` are what the resolver chose. Only no arguments, or the resolver's own login
/// argument `-l`, are accepted; anything else could change how the shell starts in ways the
/// scripts do not account for.
pub fn prepare(
    kind: ShellKind,
    args: &[String],
    login: bool,
    local_host: &str,
    session_id: &str,
    user: &UserEnv,
    base: &Path,
) -> Option<Plan> {
    if !supports(kind) {
        return None;
    }
    // A login bash ignores --rcfile, and imitating a login shell would lose `logout`,
    // `.bash_logout` and `shopt login_shell`. The user's login contract wins: it stays plain.
    if kind == ShellKind::Bash && login {
        return None;
    }
    let only_login_arg = login && args == ["-l"];
    if !args.is_empty() && !only_login_arg {
        return None;
    }
    let files = create_dir(base, session_id).ok()?;
    let dir = files.path().to_path_buf();
    let mut env = HashMap::from([(SESSION_ID_ENV.to_string(), session_id.to_string())]);

    let plan_args = match kind {
        ShellKind::Bash => {
            let rc = dir.join("bash.sh");
            write_private(
                &rc,
                &BASH_SCRIPT.replace("@HOST@", &posix_quote(local_host)),
            )
            .ok()?;
            vec!["--rcfile".to_string(), rc.to_string_lossy().into_owned()]
        }
        ShellKind::Zsh => {
            write_private(&dir.join(".zshenv"), ZSH_ENV_SCRIPT).ok()?;
            write_private(&dir.join(".zprofile"), ZSH_PROFILE_SCRIPT).ok()?;
            write_private(
                &dir.join(".zshrc"),
                &ZSH_RC_SCRIPT.replace("@HOST@", &posix_quote(local_host)),
            )
            .ok()?;
            env.insert("ZDOTDIR".to_string(), dir.to_string_lossy().into_owned());
            env.insert(
                USER_ZDOTDIR_ENV.to_string(),
                user.zdotdir.clone().unwrap_or_default(),
            );
            args.to_vec()
        }
        ShellKind::Fish => {
            let script = dir.join("kegon.fish");
            write_private(
                &script,
                &FISH_SCRIPT.replace("@HOST@", &fish_quote(local_host)),
            )
            .ok()?;
            env.insert(
                FISH_INIT_ENV.to_string(),
                script.to_string_lossy().into_owned(),
            );
            // fish 4 marks prompts with OSC 133 by itself; its markers carry no session identity
            // and would be taken for a second, unverified integration.
            env.insert(
                "fish_features".to_string(),
                match user.fish_features.as_deref().map(str::trim) {
                    Some(features) if !features.is_empty() => {
                        format!("{features},no-mark-prompt")
                    }
                    _ => "no-mark-prompt".to_string(),
                },
            );
            let mut fish_args = args.to_vec();
            fish_args.push("--init-command".to_string());
            fish_args.push(FISH_INIT_COMMAND.to_string());
            fish_args
        }
        _ => return None,
    };

    Some(Plan {
        args: plan_args,
        env,
        files,
    })
}

/// Creates `<base>/kegon-shell-<session id>`, readable by the owner only. `mkdir` fails when the
/// name exists, even as a symbolic link, so nothing planted in a shared directory can be reused.
fn create_dir(base: &Path, session_id: &str) -> io::Result<IntegrationFiles> {
    let path = base.join(format!("kegon-shell-{session_id}"));
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&path)?;
    // Owning the directory from here on removes it again if a later step fails.
    Ok(IntegrationFiles(Arc::new(ScriptDir { path })))
}

/// Writes a new file, readable by the owner only; never overwrites or follows an existing one.
fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(contents.as_bytes())
}

/// Quotes `value` as the *contents* of a POSIX single-quoted string (bash, zsh).
fn posix_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

/// Quotes `value` as the *contents* of a fish single-quoted string.
fn fish_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::integration::{
        CommandExecutionResult, CommandStateMachine, CompletionReason, MetadataCorrelation,
        ShellReportedStatus, VerifiedCommandEvent,
    };
    use crate::terminal::osc::{OscEvent, OscScanner};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kegon-unix-integration-test-{tag}-{}",
            super::super::shell::generate_session_id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn plan(kind: ShellKind, shell_args: &[&str], login: bool, base: &Path) -> Option<Plan> {
        prepare(
            kind,
            &args(shell_args),
            login,
            "host",
            &super::super::shell::generate_session_id(),
            &UserEnv::default(),
            base,
        )
    }

    #[test]
    fn only_bash_zsh_and_fish_are_integrated() {
        let base = scratch("kinds");
        for kind in [
            ShellKind::PowerShell,
            ShellKind::WindowsPowerShell,
            ShellKind::Cmd,
            ShellKind::Other,
        ] {
            assert!(!supports(kind));
            assert!(plan(kind, &[], false, &base).is_none(), "{kind:?}");
        }
        for kind in [ShellKind::Bash, ShellKind::Zsh, ShellKind::Fish] {
            assert!(supports(kind));
            assert!(plan(kind, &[], false, &base).is_some(), "{kind:?}");
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn unknown_arguments_leave_the_shell_alone() {
        let base = scratch("args");
        assert!(plan(ShellKind::Bash, &["--norc"], false, &base).is_none());
        assert!(plan(ShellKind::Zsh, &["-f"], false, &base).is_none());
        // `-l` is the resolver's login argument, and only with the login intent.
        assert!(plan(ShellKind::Zsh, &["-l"], false, &base).is_none());
        assert!(plan(ShellKind::Zsh, &["-l"], true, &base).is_some());
        assert!(plan(ShellKind::Fish, &["-l", "-x"], true, &base).is_none());
        // Nothing is left behind for a shell that was not integrated.
        assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn bash_loads_the_script_with_rcfile_and_never_in_a_login_shell() {
        let base = scratch("bash");
        let plain = plan(ShellKind::Bash, &[], false, &base).unwrap();
        let rc = plain.files.path().join("bash.sh");
        assert_eq!(plain.args, ["--rcfile", rc.to_str().unwrap()]);
        assert!(plain.env[SESSION_ID_ENV].starts_with("kegon-"));
        assert!(!plain.env.contains_key("KEGON_SHELL_LOGIN"));

        // A login bash keeps its login contract: no integration, no change at all (--rcfile
        // would be ignored, and imitating login would lose `logout` and `.bash_logout`).
        assert!(plan(ShellKind::Bash, &["-l"], true, &base).is_none());
        assert!(plan(ShellKind::Bash, &[], true, &base).is_none());
        assert_eq!(
            fs::read_dir(&base).unwrap().count(),
            1,
            "only the plain bash's directory"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn zsh_keeps_its_arguments_and_forwards_to_the_users_zdotdir() {
        let base = scratch("zsh");
        let user = UserEnv {
            zdotdir: Some("/home/me/.config/zsh".to_string()),
            fish_features: None,
        };
        let login = prepare(
            ShellKind::Zsh,
            &args(&["-l"]),
            true,
            "host",
            "kegon-1-zsh",
            &user,
            &base,
        )
        .unwrap();
        assert_eq!(login.args, ["-l"]);
        assert_eq!(login.env["ZDOTDIR"], login.files.path().to_str().unwrap());
        assert_eq!(login.env[USER_ZDOTDIR_ENV], "/home/me/.config/zsh");
        for name in [".zshenv", ".zprofile", ".zshrc"] {
            assert!(login.files.path().join(name).is_file(), "{name}");
        }

        let plain = plan(ShellKind::Zsh, &[], false, &base).unwrap();
        assert!(plain.args.is_empty());
        assert_eq!(plain.env[USER_ZDOTDIR_ENV], "");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn fish_runs_a_fixed_init_command_and_turns_its_own_prompt_marks_off() {
        let base = scratch("fish");
        let fish = plan(ShellKind::Fish, &["-l"], true, &base).unwrap();
        assert_eq!(fish.args, ["-l", "--init-command", FISH_INIT_COMMAND]);
        assert_eq!(
            fish.env[FISH_INIT_ENV],
            fish.files.path().join("kegon.fish").to_str().unwrap()
        );
        assert_eq!(fish.env["fish_features"], "no-mark-prompt");

        let user = UserEnv {
            zdotdir: None,
            fish_features: Some("qmark-noglob".to_string()),
        };
        let kept = prepare(
            ShellKind::Fish,
            &[],
            false,
            "host",
            "kegon-1-def",
            &user,
            &base,
        )
        .unwrap();
        assert_eq!(kept.env["fish_features"], "qmark-noglob,no-mark-prompt");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn host_names_are_quoted_for_each_shell() {
        assert_eq!(posix_quote("o'brien"), "o'\\''brien");
        assert_eq!(fish_quote("o'brien\\pc"), "o\\'brien\\\\pc");
        let base = scratch("quote");
        let bash = plan_with_host(ShellKind::Bash, "a'b", &base);
        let script = fs::read_to_string(bash.files.path().join("bash.sh")).unwrap();
        assert!(script.contains("__kegon_host='a'\\''b'"));
        assert!(!script.contains("@HOST@"));
        let fish = plan_with_host(ShellKind::Fish, "a'b", &base);
        let script = fs::read_to_string(fish.files.path().join("kegon.fish")).unwrap();
        assert!(script.contains("set -g __kegon_host 'a\\'b'"));
        assert!(!script.contains("@HOST@"));
        let zsh = plan_with_host(ShellKind::Zsh, "a'b", &base);
        let script = fs::read_to_string(zsh.files.path().join(".zshrc")).unwrap();
        assert!(script.contains("__kegon_host='a'\\''b'"));
        fs::remove_dir_all(base).unwrap();
    }

    fn plan_with_host(kind: ShellKind, host: &str, base: &Path) -> Plan {
        prepare(
            kind,
            &[],
            false,
            host,
            &super::super::shell::generate_session_id(),
            &UserEnv::default(),
            base,
        )
        .unwrap()
    }

    #[test]
    fn every_script_speaks_the_kegon_dialect() {
        // fish has no input-start mark: its prompt stays the user's own (see fish.fish).
        for (name, script, input_mark) in [
            ("bash", BASH_SCRIPT, true),
            ("zsh", ZSH_RC_SCRIPT, true),
            ("fish", FISH_SCRIPT, false),
        ] {
            for marker in [
                "133;A",
                "133;C;kegon.v=1;kegon.sid=",
                "133;D;",
                "kegon.seq=",
                "kegon.ps=",
                "]7;file://",
            ] {
                assert!(script.contains(marker), "{name} lacks {marker}");
            }
            assert_eq!(script.contains("133;B"), input_mark, "{name}");
            assert!(script.contains("@HOST@"), "{name} lacks the host slot");
        }
    }

    #[test]
    fn scripts_take_the_session_id_out_of_the_environment() {
        assert!(BASH_SCRIPT.contains("unset KEGON_SESSION_ID"));
        assert!(ZSH_ENV_SCRIPT.contains("unset KEGON_SESSION_ID"));
        assert!(FISH_SCRIPT.contains("set -e KEGON_SESSION_ID"));
    }

    #[test]
    fn scripts_never_touch_the_users_dotfiles() {
        // Reading and sourcing the user's files is the point; writing to them is never done.
        for script in [
            BASH_SCRIPT,
            ZSH_ENV_SCRIPT,
            ZSH_PROFILE_SCRIPT,
            ZSH_RC_SCRIPT,
            FISH_SCRIPT,
        ] {
            for line in script.lines().filter(|l| !l.trim_start().starts_with('#')) {
                assert!(!line.contains(">>"), "appends: {line}");
                assert!(!line.contains("> ~"), "writes to home: {line}");
                assert!(!line.contains("> \"$HOME"), "writes to home: {line}");
                assert!(!line.contains("rm "), "removes files: {line}");
            }
        }
    }

    #[test]
    fn the_directory_is_private_new_and_removed_with_the_last_owner() {
        let base = scratch("lifetime");
        let first = plan(ShellKind::Bash, &[], false, &base).unwrap();
        let dir = first.files.path().to_path_buf();
        assert!(dir.join("bash.sh").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join("bash.sh")), 0o600);
        }
        // A clone (as a launch specification is cloned) keeps the directory alive.
        let copy = first.files.clone();
        drop(first);
        assert!(dir.exists());
        drop(copy);
        assert!(!dir.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn an_existing_name_is_never_reused() {
        let base = scratch("exists");
        let taken = base.join("kegon-shell-kegon-1-abc");
        fs::create_dir(&taken).unwrap();
        fs::write(taken.join("planted"), "x").unwrap();
        let attempt = prepare(
            ShellKind::Bash,
            &[],
            false,
            "host",
            "kegon-1-abc",
            &UserEnv::default(),
            &base,
        );
        assert!(attempt.is_none());
        // The other party's directory is untouched.
        assert!(taken.join("planted").is_file());
        fs::remove_dir_all(base).unwrap();
    }

    /// Feeds `bytes` as PTY output through the real scanner and state machine.
    fn verified(
        sm: &mut CommandStateMachine,
        scanner: &mut OscScanner,
        bytes: &str,
    ) -> Vec<VerifiedCommandEvent> {
        scanner
            .scan(bytes.as_bytes())
            .into_iter()
            .filter_map(|event| match event {
                OscEvent::ShellIntegration(marker) => sm.transition(marker),
                _ => None,
            })
            .collect()
    }

    fn started(seq: u64, sid: &str) -> String {
        format!("]133;C;kegon.v=1;kegon.sid={sid};kegon.seq={seq}")
    }

    fn finished(code: i32, seq: u64, sid: &str) -> String {
        let ps = if code == 0 { "success" } else { "failure" };
        format!("]133;D;{code};kegon.v=1;kegon.sid={sid};kegon.seq={seq};kegon.ps={ps}")
    }

    fn machine(sid: &str) -> (CommandStateMachine, OscScanner) {
        let mut sm = CommandStateMachine::new();
        sm.set_expected_session_id(Some(sid.to_string()));
        (sm, OscScanner::new("host".to_string()))
    }

    fn result_of(events: &[VerifiedCommandEvent]) -> Option<CommandExecutionResult> {
        events.iter().find_map(|event| match event {
            VerifiedCommandEvent::CommandFinished {
                execution_result, ..
            } => Some(*execution_result),
            _ => None,
        })
    }

    #[test]
    fn the_scripts_markers_are_a_verified_success_or_failure() {
        let (mut sm, mut scanner) = machine("kegon-1-me");
        verified(&mut sm, &mut scanner, "]133;A]133;B");
        verified(&mut sm, &mut scanner, &started(1, "kegon-1-me"));
        let ok = verified(&mut sm, &mut scanner, &finished(0, 1, "kegon-1-me"));
        let ok = result_of(&ok).expect("finished");
        assert_eq!(ok.shell_status, ShellReportedStatus::Success);
        assert_eq!(ok.osc133_exit, Some(0));
        assert_eq!(ok.completion, CompletionReason::DObserved);
        assert_eq!(ok.correlation, MetadataCorrelation::Matched);

        verified(&mut sm, &mut scanner, &started(2, "kegon-1-me"));
        let bad = verified(&mut sm, &mut scanner, &finished(7, 2, "kegon-1-me"));
        let bad = result_of(&bad).expect("finished");
        assert_eq!(bad.shell_status, ShellReportedStatus::Failure);
        assert_eq!(bad.osc133_exit, Some(7));
        assert_eq!(bad.correlation, MetadataCorrelation::Matched);
    }

    #[test]
    fn another_tabs_identity_never_becomes_a_verified_result() {
        let (mut sm, mut scanner) = machine("kegon-1-me");
        verified(&mut sm, &mut scanner, &started(1, "kegon-2-other"));
        let events = verified(&mut sm, &mut scanner, &finished(0, 1, "kegon-2-other"));
        let result = result_of(&events).expect("a boundary is still seen");
        assert_ne!(result.correlation, MetadataCorrelation::Matched);
        assert_eq!(result.shell_status, ShellReportedStatus::Unknown);
    }

    #[test]
    fn a_marker_typed_by_hand_is_not_a_finished_command() {
        let (mut sm, mut scanner) = machine("kegon-1-me");
        // `printf` of a D with nothing running, even with the right identity: nothing to finish.
        let events = verified(&mut sm, &mut scanner, &finished(0, 1, "kegon-1-me"));
        assert!(result_of(&events).is_none());
        // A forged D for a command that did start, with a made-up sequence number: not adopted.
        verified(&mut sm, &mut scanner, &started(1, "kegon-1-me"));
        let forged = verified(&mut sm, &mut scanner, &finished(0, 99, "kegon-1-me"));
        assert!(result_of(&forged).is_none());
        // The real D still completes it, once; a replay finds nothing running.
        let real = verified(&mut sm, &mut scanner, &finished(3, 1, "kegon-1-me"));
        assert_eq!(
            result_of(&real).map(|r| r.shell_status),
            Some(ShellReportedStatus::Failure)
        );
        let replay = verified(&mut sm, &mut scanner, &finished(0, 1, "kegon-1-me"));
        assert!(result_of(&replay).is_none());
    }

    #[test]
    fn an_interrupted_command_is_not_a_success() {
        let (mut sm, mut scanner) = machine("kegon-1-me");
        verified(&mut sm, &mut scanner, &started(1, "kegon-1-me"));
        // The prompt comes back without a D: the shell never reported how it ended.
        let events = verified(&mut sm, &mut scanner, "]133;A");
        let result = result_of(&events).expect("closed by the prompt");
        assert_eq!(result.completion, CompletionReason::MissingD);
        assert_eq!(result.shell_status, ShellReportedStatus::Unknown);
    }

    #[cfg(unix)]
    #[test]
    fn unix_bash_script_reports_commands_and_keeps_the_users_hooks() {
        let Some(bash) = installed_bash() else {
            eprintln!("bash is not installed: skipped");
            return;
        };
        let base = scratch("bash-run");
        let home = base.join("home");
        fs::create_dir(&home).unwrap();
        fs::write(
            home.join(".bashrc"),
            "PROMPT_COMMAND='echo USER_PROMPT_COMMAND'\ntrap 'echo USER_DEBUG_TRAP' DEBUG\n",
        )
        .unwrap();
        let plan = prepare(
            ShellKind::Bash,
            &[],
            false,
            "testhost",
            "kegon-9-feed",
            &UserEnv::default(),
            &base,
        )
        .unwrap();
        let output = run_shell(
            &bash,
            &plan.args,
            &plan.env,
            &home,
            "true\n\n\nfalse\nls /nonexistent-kegon-path 2>/dev/null; echo piped | cat\nexit 3\n",
        );
        let markers = markers(&output);
        assert_eq!(
            markers.iter().filter(|m| m.starts_with("C;")).count(),
            4,
            "{markers:?}"
        );
        let finished: Vec<_> = markers.iter().filter(|m| m.starts_with("D;")).collect();
        assert_eq!(finished.len(), 3, "{markers:?}");
        assert!(finished[0].starts_with("D;0;kegon.v=1;kegon.sid=kegon-9-feed;kegon.seq=1;"));
        assert!(finished[0].ends_with("kegon.ps=success"));
        assert!(finished[1].starts_with("D;1;"));
        assert!(finished[1].ends_with("kegon.ps=failure"));
        assert!(finished[2].starts_with("D;0;") && finished[2].contains("kegon.seq=3;"));
        // What bash printed is what the state machine verifies.
        let (mut sm, mut scanner) = machine("kegon-9-feed");
        let results: Vec<_> = verified(&mut sm, &mut scanner, &output)
            .iter()
            .filter_map(|e| result_of(std::slice::from_ref(e)))
            .collect();
        let statuses: Vec<_> = results.iter().map(|r| r.shell_status).collect();
        assert_eq!(
            statuses,
            [
                ShellReportedStatus::Success,
                ShellReportedStatus::Failure,
                ShellReportedStatus::Success
            ],
            "{output:?}"
        );
        assert!(
            results
                .iter()
                .all(|r| r.correlation == MetadataCorrelation::Matched)
        );
        assert!(output.contains("USER_PROMPT_COMMAND"), "{output}");
        assert!(output.contains("USER_DEBUG_TRAP"), "{output}");
        assert!(output.contains("]7;file://testhost/"), "{output}");
        // Dropping the plan deletes the scripts.
        let dir = plan.files.path().to_path_buf();
        drop(plan);
        assert!(!dir.exists());
        fs::remove_dir_all(base).unwrap();
    }

    /// bash with `rc` as the user's `~/.bashrc`, run on `input`; returns what it printed.
    #[cfg(unix)]
    fn run_integrated_bash(
        rc: &str,
        input: &str,
        extra_args: &[&str],
    ) -> Option<(String, PathBuf)> {
        let bash = installed_bash()?;
        let base = scratch("bash-rc");
        let home = base.join("home");
        fs::create_dir(&home).unwrap();
        fs::write(home.join(".bashrc"), rc).unwrap();
        let plan = prepare(
            ShellKind::Bash,
            &[],
            false,
            "testhost",
            "kegon-9-feed",
            &UserEnv::default(),
            &base,
        )
        .unwrap();
        let mut args = plan.args.clone();
        args.extend(extra_args.iter().map(|a| a.to_string()));
        let output = run_shell(&bash, &args, &plan.env, &home, input);
        Some((output, base))
    }

    /// F-2: the input-start mark must survive a PROMPT_COMMAND that assigns PS1 every time.
    #[cfg(unix)]
    #[test]
    fn unix_bash_input_mark_survives_prompt_commands_that_rewrite_ps1() {
        let array_capable = "if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )); then";
        for (name, rc) in [
            ("fixed PS1", "PS1='fixed> '\n".to_string()),
            (
                "string PROMPT_COMMAND rewriting PS1",
                "PROMPT_COMMAND='PS1=\"dyn> \"; echo USER_PC'\n".to_string(),
            ),
            (
                "array PROMPT_COMMAND rewriting PS1",
                format!(
                    "{array_capable}\n  PROMPT_COMMAND=('PS1=\"arr> \"' 'echo USER_PC')\nelse\n  PROMPT_COMMAND='PS1=\"arr> \"; echo USER_PC'\nfi\n"
                ),
            ),
        ] {
            let Some((output, base)) = run_integrated_bash(&rc, "true\nfalse\nexit\n", &[]) else {
                eprintln!("bash is not installed: skipped");
                return;
            };
            let marks = markers(&output);
            let count = |tag: &str| marks.iter().filter(|m| m.as_str() == tag).count();
            assert!(count("A") >= 3, "{name}: {marks:?}");
            // Exactly one B per prompt: never lost, never repeated.
            assert_eq!(count("B"), count("A"), "{name}: {marks:?}");
            // The user's own PROMPT_COMMAND ran, and the exit statuses are intact.
            if name != "fixed PS1" {
                assert!(output.contains("USER_PC"), "{name}: {output}");
            }
            let (mut sm, mut scanner) = machine("kegon-9-feed");
            let statuses: Vec<_> = verified(&mut sm, &mut scanner, &output)
                .iter()
                .filter_map(|e| result_of(std::slice::from_ref(e)))
                .map(|r| r.shell_status)
                .collect();
            assert_eq!(
                statuses,
                [ShellReportedStatus::Success, ShellReportedStatus::Failure],
                "{name}"
            );
            fs::remove_dir_all(base).unwrap();
        }
    }

    /// F-3: bash reads its system rc (`/etc/bash.bashrc` on Debian) by itself even with
    /// `--rcfile`, so the integration must neither skip it nor read it a second time.
    #[cfg(unix)]
    #[test]
    fn unix_bash_reads_the_system_rc_once_and_the_users_rc_once() {
        let Some(bash) = installed_bash() else {
            eprintln!("bash is not installed: skipped");
            return;
        };
        // `-x` with a PS4 that names the file each traced command comes from.
        let trace = |args: &[&str], env: &HashMap<String, String>, home: &Path| {
            let mut env = env.clone();
            env.insert("PS4".to_string(), "+TRACE:${BASH_SOURCE[0]}: ".to_string());
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            run_shell(&bash, &args, &env, home, "exit\n")
        };
        let from = |output: &str, file: &str| {
            output
                .lines()
                // xtrace repeats the first PS4 character once per level of sourcing.
                .map(|l| l.trim_start_matches('+'))
                .filter(|l| l.starts_with("TRACE:") && l.contains(&format!("{file}:")))
                .count()
        };
        let base = scratch("sysrc");
        let home = base.join("home");
        fs::create_dir(&home).unwrap();
        fs::write(home.join(".bashrc"), "KEGON_TEST_RC_LINE=1\n").unwrap();
        // `home/.bashrc`, as the trace names it (the temporary directory itself differs).
        let bashrc = "home/.bashrc";

        let plain = trace(&["-x"], &HashMap::new(), &home);
        let plan = prepare(
            ShellKind::Bash,
            &[],
            false,
            "testhost",
            "kegon-9-sysrc",
            &UserEnv::default(),
            &base,
        )
        .unwrap();
        let mut args: Vec<&str> = plan.args.iter().map(String::as_str).collect();
        args.push("-x");
        let integrated = trace(&args, &plan.env, &home);

        // The user's file ran exactly as often as in a plain bash: once.
        assert_eq!(from(&plain, bashrc), 1, "{plain}");
        assert_eq!(from(&integrated, bashrc), 1, "{integrated}");
        // So did the system file, when this system has one that bash reads.
        let system = "/etc/bash.bashrc";
        let plain_system = from(&plain, system);
        if plain_system == 0 {
            eprintln!("this bash reads no {system}: the system-rc check is skipped");
        }
        assert_eq!(from(&integrated, system), plain_system, "{integrated}");
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    /// Runs the shell interactively with `input` on a pipe (no terminal needed) and returns
    /// everything it printed, stdout and stderr in the order they were written (as on a terminal).
    fn run_shell(
        shell: &Path,
        args: &[String],
        env: &HashMap<String, String>,
        home: &Path,
        input: &str,
    ) -> String {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sh")
            .args(["-c", "exec \"$@\" 2>&1", "sh"])
            .arg(shell)
            .args(args)
            .arg("-i")
            .envs(env)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[cfg(unix)]
    /// The `133;` payloads in `output`, in order.
    fn markers(output: &str) -> Vec<String> {
        output
            .split("\x1b]133;")
            .skip(1)
            .filter_map(|rest| rest.split('\x07').next())
            .map(str::to_string)
            .collect()
    }

    #[cfg(unix)]
    fn installed_bash() -> Option<PathBuf> {
        ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .or_else(|| {
                std::env::var_os("PATH").and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|d| d.join("bash"))
                        .find(|p| p.is_file())
                })
            })
    }
}
