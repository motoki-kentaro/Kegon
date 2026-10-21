//! End-to-end tests of the Unix Shell Integration: real bash, zsh and fish on a real PTY, from the
//! launch specification down to the verified command events the Dopamine Mode consumes.
//!
//! They start real shells, so they are `#[ignore]`d like the Windows ConPTY ones. Run them where
//! the shell is installed:
//!
//! ```text
//! cargo test --locked -- --ignored unix_shell
//! ```
//!
//! A shell that is not installed fails its test (it is not skipped), so a green run means it ran.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};

use crate::settings::ShellIntegrationMode;
use crate::terminal::cwd::local_host_name;
use crate::terminal::integration::{
    CommandExecutionResult, CommandStateMachine, CompletionReason, MetadataCorrelation,
    Osc133Marker, ShellReportedStatus, VerifiedCommandEvent,
};
use crate::terminal::session::{EventSink, TerminalEvent, TerminalSession};
use crate::terminal::shell::{ResolvedShell, ShellIntegrationLaunch, ShellKind, ShellSpec};
use crate::terminal::unix_integration::UserEnv;

const WAIT: Duration = Duration::from_secs(30);

#[derive(Debug)]
enum Seen {
    Marker(Osc133Marker),
    Directory(PathBuf),
    Exit(Option<i32>),
}

/// One shell on a PTY, with the state machine Kegon would drive from its markers.
struct Run {
    session: TerminalSession,
    rx: Receiver<Seen>,
    sm: CommandStateMachine,
    sid: Option<String>,
    scripts: Option<PathBuf>,
    cwd: PathBuf,
    directories: Vec<PathBuf>,
    /// Declared last, so it is dropped after the session: the session removes the scripts first.
    _cleanup: Cleanup,
}

/// Removes the test's directories when dropped.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for dir in &self.0 {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// What one command line produced, as verified events.
#[derive(Default, Debug)]
struct Outcome {
    started: usize,
    prompts: usize,
    finished: Vec<CommandExecutionResult>,
}

fn find_shell(name: &str) -> PathBuf {
    let fixed = ["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"]
        .into_iter()
        .map(|dir| Path::new(dir).join(name));
    let on_path = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    fixed
        .chain(on_path)
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{name} is not installed; install it to run this test"))
}

fn scratch(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "kegon-unix-shell-e2e-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A startup file of the user's that defines a hook of its own, to prove ours leaves it alone.
fn write_user_files(kind: ShellKind, home: &Path) {
    match kind {
        ShellKind::Bash => {
            // A profile that reads `.bashrc` (as Debian's skeleton does), and a logout file; only a
            // login shell reads them.
            fs::write(
                home.join(".bash_profile"),
                "echo PROFILE_RAN\n[ -f ~/.bashrc ] && . ~/.bashrc\n",
            )
            .unwrap();
            fs::write(
                home.join(".bash_logout"),
                "echo LOGOUT_RAN >> ~/LOGOUT_LOG\n",
            )
            .unwrap();
            fs::write(
                home.join(".bashrc"),
                "PROMPT_COMMAND='echo USER_HOOK_RAN'\nPS1='$ '\n",
            )
        }
        ShellKind::Zsh => fs::write(
            home.join(".zshrc"),
            "precmd() { echo USER_HOOK_RAN }\nPS1='%# '\n",
        ),
        ShellKind::Fish => {
            let config = home.join(".config").join("fish");
            fs::create_dir_all(&config).unwrap();
            fs::write(
                config.join("config.fish"),
                "function __user_hook --on-event fish_prompt\n    echo USER_HOOK_RAN\nend\n",
            )
        }
        _ => unreachable!(),
    }
    .unwrap();
}

fn start(name: &str, kind: ShellKind, mode: ShellIntegrationMode) -> Run {
    start_with(name, kind, mode, None)
}

/// Where the user's interactive startup file of `kind` lives under `home`.
fn rc_file(kind: ShellKind, home: &Path) -> PathBuf {
    match kind {
        ShellKind::Bash => home.join(".bashrc"),
        ShellKind::Zsh => home.join(".zshrc"),
        ShellKind::Fish => home.join(".config").join("fish").join("config.fish"),
        _ => unreachable!(),
    }
}

/// Starts a shell; `rc` replaces the user's startup file that `write_user_files` would write.
fn start_with(name: &str, kind: ShellKind, mode: ShellIntegrationMode, rc: Option<&str>) -> Run {
    start_full(name, kind, mode, rc, false)
}

/// [`start_with`], optionally as a login shell the way the resolver asks for one: `-l`.
fn start_full(
    name: &str,
    kind: ShellKind,
    mode: ShellIntegrationMode,
    rc: Option<&str>,
    login: bool,
) -> Run {
    let program = find_shell(name);
    let base = scratch(name);
    let scripts_base = scratch(&format!("{name}-scripts"));
    let home = base.join("home");
    fs::create_dir_all(&home).unwrap();
    write_user_files(kind, &home);
    if let Some(rc) = rc {
        fs::write(rc_file(kind, &home), rc).unwrap();
    }
    // A directory with a space and non-ASCII text, to exercise the OSC 7 encoding.
    let cwd = base.join("work dir 日本語");
    fs::create_dir_all(&cwd).unwrap();

    let mut spec: ShellSpec = ResolvedShell {
        program: program.to_string_lossy().into_owned(),
        args: if login {
            vec!["-l".to_string()]
        } else {
            vec![]
        },
        kind,
        login,
    }
    .build_spec_for(
        mode,
        &local_host_name(),
        None,
        Some(cwd.clone()),
        Some((&UserEnv::default(), &scripts_base)),
    )
    .expect("launch specification");
    spec.env
        .insert("HOME".to_string(), home.to_string_lossy().into_owned());
    spec.env.insert(
        "XDG_CONFIG_HOME".to_string(),
        home.join(".config").to_string_lossy().into_owned(),
    );
    spec.env
        .insert("KEGON_E2E_ENV".to_string(), "E2E_ENV_VALUE".to_string());
    let sid = spec.session_id.clone();
    let scripts = spec
        .integration_files
        .as_ref()
        .map(|files| files.path().to_path_buf());

    let (tx, rx) = mpsc::channel();
    let sink = EventSink::from_fn(move |event| {
        let _ = match event {
            TerminalEvent::ShellIntegration(marker) => tx.send(Seen::Marker(marker)),
            TerminalEvent::WorkingDirectory(path) => tx.send(Seen::Directory(path)),
            TerminalEvent::ChildExit(code) => tx.send(Seen::Exit(code)),
            _ => Ok(()),
        };
    });
    let session = TerminalSession::spawn_launch(spec, 100, 30, 10, 20, sink).expect("spawn shell");
    let mut sm = CommandStateMachine::new();
    sm.set_expected_session_id(session.session_id().map(str::to_string));
    Run {
        session,
        rx,
        sm,
        sid,
        scripts,
        cwd,
        directories: Vec::new(),
        _cleanup: Cleanup(vec![base, scripts_base]),
    }
}

impl Run {
    /// Feeds markers to the state machine until `done` says so; returns what they verified.
    fn pump_until(
        &mut self,
        what: &str,
        mut done: impl FnMut(&CommandStateMachine, &Outcome) -> bool,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        let deadline = Instant::now() + WAIT;
        while !done(&self.sm, &outcome) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(Seen::Marker(marker)) => match self.sm.transition(marker) {
                    Some(VerifiedCommandEvent::CommandStarted { .. }) => outcome.started += 1,
                    Some(VerifiedCommandEvent::PromptStarted) => outcome.prompts += 1,
                    Some(VerifiedCommandEvent::CommandFinished {
                        execution_result, ..
                    }) => outcome.finished.push(execution_result),
                    _ => {}
                },
                Ok(Seen::Directory(path)) => self.directories.push(path),
                Ok(Seen::Exit(_)) => {}
                Err(_) => panic!("timed out waiting for {what}; screen:\n{}", self.screen()),
            }
        }
        outcome
    }

    fn wait_for_prompt(&mut self) {
        self.pump_until("the first prompt", |_, o| o.prompts > 0);
    }

    /// Types a command line and waits until its result is verified.
    fn run(&mut self, line: &str) -> Outcome {
        self.session.write_input(format!("{line}\r").into_bytes());
        self.pump_until(line, |_, outcome| !outcome.finished.is_empty())
    }

    fn screen(&self) -> String {
        let term = self.session.term().lock();
        let grid = term.grid();
        (0..grid.screen_lines() as i32)
            .map(|line| {
                (0..grid.columns())
                    .map(|column| grid[Point::new(Line(line), Column(column))].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn wait_for_screen(&self, text: &str) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.screen().contains(text) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("{text:?} never appeared; screen:\n{}", self.screen());
    }
}

fn expect_result(outcome: &Outcome, status: ShellReportedStatus, exit: Option<i32>) {
    assert_eq!(
        outcome.started, 1,
        "one command line is one command: {outcome:?}"
    );
    assert_eq!(outcome.finished.len(), 1, "{outcome:?}");
    let result = outcome.finished[0];
    assert_eq!(result.shell_status, status, "{result:?}");
    assert_eq!(result.completion, CompletionReason::DObserved, "{result:?}");
    assert_eq!(
        result.correlation,
        MetadataCorrelation::Matched,
        "{result:?}"
    );
    if let Some(code) = exit {
        assert_eq!(result.osc133_exit, Some(code), "{result:?}");
    }
}

/// The whole contract, for one shell.
fn assert_integration(name: &str, kind: ShellKind) {
    let mut run = start(name, kind, ShellIntegrationMode::Auto);
    let scripts = run.scripts.clone().expect("scripts were written");
    assert!(scripts.is_dir());
    let sid = run.sid.clone().expect("an injected shell has a session id");
    assert_eq!(
        run.session.integration_launch(),
        ShellIntegrationLaunch::Injected
    );
    assert_eq!(run.session.session_id(), Some(sid.as_str()));

    // First prompt: A and B arrive, the directory is reported, the user's own hook still runs.
    run.wait_for_prompt();
    run.wait_for_screen("USER_HOOK_RAN");
    let reported = run
        .directories
        .last()
        .cloned()
        .expect("OSC 7 at the first prompt");
    assert_eq!(
        fs::canonicalize(&reported).unwrap(),
        fs::canonicalize(&run.cwd).unwrap()
    );

    // Success, failure, a missing command.
    expect_result(&run.run("true"), ShellReportedStatus::Success, Some(0));
    expect_result(&run.run("false"), ShellReportedStatus::Failure, Some(1));
    expect_result(
        &run.run("kegon_no_such_command_e2e"),
        ShellReportedStatus::Failure,
        Some(127),
    );
    // Compound and piped lines are one command; the status is the last one's.
    expect_result(
        &run.run("true; false"),
        ShellReportedStatus::Failure,
        Some(1),
    );
    expect_result(
        &run.run("echo piped | cat"),
        ShellReportedStatus::Success,
        Some(0),
    );

    // An empty line is no command: the prompt comes back with a new A and nothing else.
    run.pump_until("the prompt", |_, o| o.prompts > 0);
    run.session.write_input(
        b"
"
        .to_vec(),
    );
    let redraw = run.pump_until("the prompt after an empty line", |_, o| o.prompts > 0);
    assert_eq!(redraw.started, 0);
    assert!(redraw.finished.is_empty());

    // The environment reached the shell.
    expect_result(
        &run.run("echo $KEGON_E2E_ENV"),
        ShellReportedStatus::Success,
        Some(0),
    );
    run.wait_for_screen("E2E_ENV_VALUE");

    // Ctrl+C ends a long command with a failure, never a success.
    run.session.write_input(b"sleep 30\r".to_vec());
    run.pump_until("sleep to start", |sm, _| sm.is_executing());
    run.session.write_input(vec![0x03]);
    let interrupted = run.pump_until("the interrupted command", |_, o| !o.finished.is_empty());
    let result = interrupted.finished[0];
    assert_ne!(
        result.shell_status,
        ShellReportedStatus::Success,
        "{result:?}"
    );
    assert_eq!(
        result.shell_status,
        ShellReportedStatus::Failure,
        "{result:?}"
    );

    // The shell leaving ends the session with its own status.
    run.session.write_input(b"exit 5\r".to_vec());
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match run.rx.recv_timeout(left) {
            Ok(Seen::Exit(code)) => {
                assert_eq!(code, Some(5));
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("the shell did not exit; screen:\n{}", run.screen()),
        }
    }

    // Dropping the session removes the scripts.
    let Run { session, .. } = run;
    drop(session);
    assert!(!scripts.exists(), "scripts were left behind");
}

fn assert_disabled(name: &str, kind: ShellKind) {
    let run = start(name, kind, ShellIntegrationMode::Disabled);
    assert_eq!(
        run.session.integration_launch(),
        ShellIntegrationLaunch::Disabled
    );
    assert_eq!(run.session.session_id(), None);
    assert!(run.scripts.is_none());
    run.session
        .write_input(b"echo DISABLED_STILL_WORKS\r".to_vec());
    run.wait_for_screen("DISABLED_STILL_WORKS");
    // A plain shell emits no markers at all.
    std::thread::sleep(Duration::from_millis(500));
    while let Ok(seen) = run.rx.try_recv() {
        assert!(
            !matches!(seen, Seen::Marker(_)),
            "unexpected marker: {seen:?}"
        );
    }
}

fn assert_tabs_are_separate(name: &str, kind: ShellKind) {
    let mut first = start(name, kind, ShellIntegrationMode::Auto);
    let mut second = start(name, kind, ShellIntegrationMode::Auto);
    assert_ne!(first.sid, second.sid);
    assert_ne!(first.scripts, second.scripts);
    first.wait_for_prompt();
    second.wait_for_prompt();
    // Each tab verifies its own commands under its own identity.
    expect_result(&first.run("true"), ShellReportedStatus::Success, Some(0));
    expect_result(&second.run("false"), ShellReportedStatus::Failure, Some(1));
    // And a status line another tab's shell could print is not this tab's result.
    let foreign = first.sid.clone().unwrap();
    second.session.write_input(
        format!(
            "printf '\\033]133;C;kegon.v=1;kegon.sid={foreign};kegon.seq=1\\007'; \
             printf '\\033]133;D;0;kegon.v=1;kegon.sid={foreign};kegon.seq=1;kegon.ps=success\\007'\r"
        )
        .into_bytes(),
    );
    // The line's own command is still announced under this tab's identity; the forged markers
    // inside it can degrade that result, but never turn it into a verified success.
    let results = second
        .pump_until("the forged line", |_, o| !o.finished.is_empty())
        .finished;
    second.pump_until("the prompt", |_, o| o.prompts > 0);
    assert_eq!(
        results.len(),
        1,
        "one command line, one result: {results:?}"
    );
    let result = results[0];
    assert!(
        !(result.shell_status == ShellReportedStatus::Success
            && result.correlation == MetadataCorrelation::Matched
            && result.osc133_exit == Some(0))
            || second.sm.last_confirmed_seq() == Some(1),
        "{result:?}"
    );
    assert_eq!(
        second.sm.expected_session_id(),
        second.sid.as_deref(),
        "the expected identity never changes"
    );
}

fn assert_status_reaches_the_registry_view(name: &str, kind: ShellKind) {
    let mut run = start(name, kind, ShellIntegrationMode::Auto);
    run.wait_for_prompt();
    assert!(run.sm.has_verified_prompt());
    expect_result(&run.run("true"), ShellReportedStatus::Success, Some(0));
    assert!(run.sm.has_verified_command_execution());
}

macro_rules! shell_tests {
    ($module:ident, $name:literal, $kind:expr) => {
        mod $module {
            use super::*;

            #[test]
            #[ignore = concat!("needs ", $name, " and a PTY; spawns a real shell")]
            fn unix_shell_integration_contract() {
                assert_integration($name, $kind);
            }

            #[test]
            #[ignore = concat!("needs ", $name, " and a PTY; spawns a real shell")]
            fn unix_shell_integration_disabled_is_a_plain_shell() {
                assert_disabled($name, $kind);
            }

            #[test]
            #[ignore = concat!("needs ", $name, " and a PTY; spawns a real shell")]
            fn unix_shell_integration_tabs_do_not_mix() {
                assert_tabs_are_separate($name, $kind);
            }

            #[test]
            #[ignore = concat!("needs ", $name, " and a PTY; spawns a real shell")]
            fn unix_shell_integration_state_machine_view() {
                assert_status_reaches_the_registry_view($name, $kind);
            }
        }
    };
}

shell_tests!(bash, "bash", ShellKind::Bash);
shell_tests!(zsh, "zsh", ShellKind::Zsh);
shell_tests!(fish, "fish", ShellKind::Fish);

/// F-1: a user's `precmd` in any of the ways zsh lets one be defined must neither run twice nor
/// turn a failure into a success.
mod zsh_user_hooks {
    use super::*;

    const COUNTER: &str = "typeset -i n=0
user_hook() { (( n++ )); echo \"HOOK_COUNT=$n\" }
";

    fn variants() -> Vec<(&'static str, String)> {
        vec![
            (
                "precmd function",
                "typeset -i n=0
precmd() { (( n++ )); echo \"HOOK_COUNT=$n\" }
PS1='%# '
"
                .to_string(),
            ),
            (
                "precmd_functions entry",
                format!(
                    "{COUNTER}precmd_functions+=(user_hook)
PS1='%# '
"
                ),
            ),
            (
                "add-zsh-hook",
                format!(
                    "autoload -Uz add-zsh-hook
{COUNTER}add-zsh-hook precmd user_hook
PS1='%# '
"
                ),
            ),
            (
                "precmd function and precmd_functions entry",
                format!(
                    "{COUNTER}typeset -i m=0
precmd() {{ (( m++ )); echo \"PRECMD_COUNT=$m\" }}
precmd_functions+=(user_hook)
PS1='%# '
"
                ),
            ),
        ]
    }

    #[test]
    #[ignore = "needs zsh and a PTY; spawns a real shell"]
    fn unix_shell_zsh_user_precmd_keeps_the_exit_status_and_runs_once() {
        for (name, rc) in variants() {
            let mut run = start_with("zsh", ShellKind::Zsh, ShellIntegrationMode::Auto, Some(&rc));
            run.wait_for_prompt();
            run.wait_for_screen("HOOK_COUNT=1");
            expect_result(&run.run("true"), ShellReportedStatus::Success, Some(0));
            expect_result(&run.run("false"), ShellReportedStatus::Failure, Some(1));
            expect_result(
                &run.run("(exit 42)"),
                ShellReportedStatus::Failure,
                Some(42),
            );
            // Four prompts so far: the hooks ran once for each, never twice.
            run.pump_until("the prompt", |_, o| o.prompts > 0);
            run.wait_for_screen("HOOK_COUNT=4");
            assert!(
                !run.screen().contains("HOOK_COUNT=5"),
                "{name}:
{}",
                run.screen()
            );
        }
    }

    #[test]
    #[ignore = "needs zsh and a PTY; spawns a real shell"]
    fn unix_shell_zsh_precmd_defined_later_is_never_a_success() {
        let mut run = start("zsh", ShellKind::Zsh, ShellIntegrationMode::Auto);
        run.wait_for_prompt();
        // The command that defines `precmd` is the one whose status the new function eats.
        let defining = run.run("precmd() { true }");
        assert_eq!(defining.finished.len(), 1, "{defining:?}");
        assert_ne!(
            defining.finished[0].shell_status,
            ShellReportedStatus::Success
        );
        // From the next prompt on, the function is absorbed and the status is exact again.
        expect_result(&run.run("false"), ShellReportedStatus::Failure, Some(1));
        expect_result(&run.run("true"), ShellReportedStatus::Success, Some(0));
    }
}

/// F-5: the user's prompt keeps seeing `$status` and the whole `$pipestatus`.
mod fish_prompt_statuses {
    use super::*;

    #[test]
    #[ignore = "needs fish and a PTY; spawns a real shell"]
    fn unix_shell_fish_prompt_sees_status_and_every_pipestatus() {
        let rc = "function fish_prompt
    echo \"PS=[$status|$pipestatus]>\"
end
";
        let mut run = start_with(
            "fish",
            ShellKind::Fish,
            ShellIntegrationMode::Auto,
            Some(rc),
        );
        run.wait_for_prompt();
        run.wait_for_screen("PS=[0|0]>");
        for (line, shown, status, exit) in [
            (
                "true | true",
                "PS=[0|0 0]>",
                ShellReportedStatus::Success,
                0,
            ),
            (
                "false | true",
                "PS=[0|1 0]>",
                ShellReportedStatus::Success,
                0,
            ),
            (
                "true | false",
                "PS=[1|0 1]>",
                ShellReportedStatus::Failure,
                1,
            ),
            (
                "false | false",
                "PS=[1|1 1]>",
                ShellReportedStatus::Failure,
                1,
            ),
            (
                "true | false | true",
                "PS=[0|0 1 0]>",
                ShellReportedStatus::Success,
                0,
            ),
        ] {
            expect_result(&run.run(line), status, Some(exit));
            run.wait_for_screen(shown);
        }
    }
}

/// F-4: a login bash is the user's own login shell or nothing: `-l` stays, no integration is
/// injected, and `login_shell`, the profile files and `logout` / `.bash_logout` all work.
mod bash_login {
    use super::*;

    #[test]
    #[ignore = "needs bash and a PTY; spawns a real shell"]
    fn unix_shell_bash_login_shell_is_left_untouched() {
        let run = start_full(
            "bash",
            ShellKind::Bash,
            ShellIntegrationMode::Auto,
            None,
            true,
        );
        assert_eq!(
            run.session.integration_launch(),
            ShellIntegrationLaunch::Unsupported
        );
        assert_eq!(run.session.session_id(), None);
        assert!(run.scripts.is_none());
        // Startup files of a login shell: the profile ran, and it chose to read `.bashrc`.
        let home = run.cwd.parent().unwrap().join("home");
        run.wait_for_screen("PROFILE_RAN");
        run.session
            .write_input(b"shopt -q login_shell && echo LOGIN_SHELL_YES\r".to_vec());
        run.wait_for_screen("LOGIN_SHELL_YES");
        // `logout` exists only in a login shell, and then `.bash_logout` runs, once.
        run.session.write_input(b"logout\r".to_vec());
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match run.rx.recv_timeout(left) {
                Ok(Seen::Exit(_)) => break,
                Ok(Seen::Marker(marker)) => panic!("a login bash emitted {marker:?}"),
                Ok(_) => {}
                Err(_) => panic!(
                    "logout did not end the shell; screen:
{}",
                    run.screen()
                ),
            }
        }
        let logouts = fs::read_to_string(home.join("LOGOUT_LOG")).unwrap_or_default();
        assert_eq!(logouts.lines().count(), 1, "{logouts:?}");
    }
}
