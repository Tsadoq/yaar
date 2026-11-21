//! The environment of the language servers, taken from the user's login shell (NAV-16), and
//! the function that takes it, shown with shells the tests write themselves: a script that
//! plays the shell, records its arguments, prints what the test wants and then runs the
//! command it was given. No test runs the user's own shell or reads their start-up files.
//!
//! Every wait is for a fact, with a deadline: a state the Navigator tells, the end of a call
//! on a thread of its own, the line a script writes into a pipe. A clause that says nothing
//! happened is read after a barrier: the Navigator handles one event at a time and tells one
//! list of its servers for each `Workspaces` and each `OnScreen`, so a list told for a later
//! ask comes after what the earlier ones made it do.

mod support;

use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::*;
use yaar_navigator::shell::{COMMAND, LoginShell, MARKER, ShellError, take};
use yaar_navigator::{Ask, State, Told, WorkspaceRoot};

/// A deadline short enough for a test of a shell that outlasts it, and long enough for a
/// script to have started on a loaded machine.
const SHORT: Duration = Duration::from_secs(5);

/// `path` quoted for a `/bin/sh` script.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Writes the executable `/bin/sh` script `path` with `body`, its folder made first.
fn write_script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder above")).expect("make the folder");
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("write the script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script executable");
}

/// A shell of the test's: the script `<scratch>/<name>`.
fn shell_of(scratch: &Scratch, name: &str, body: &str, deadline: Duration) -> LoginShell {
    let program = scratch.root.join(name);
    write_script(&program, body);
    LoginShell { program, deadline }
}

/// The variables as `take` gives them, for a comparison.
fn pairs(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    list.iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect()
}

/// The value of a variable in what `take` gave.
fn value_of<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    environment
        .iter()
        .find(|(have, _)| have == name)
        .map(|(_, value)| value)
}

/// A named pipe at `path`: a script that writes into it blocks until the test reads.
fn make_pipe(path: &Path) {
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("a path with no NUL");
    // SAFETY: `name` is a NUL-terminated path that lives through the call.
    let made = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
    assert_eq!(made, 0, "make the pipe: {}", std::io::Error::last_os_error());
}

/// What a script writes into the named pipe `pipe`, read on a thread of its own so that the
/// wait has a deadline: the pipe's reader blocks until a writer opens it.
fn read_pipe(pipe: &Path, what: &str) -> String {
    let (sent, received) = mpsc::channel();
    let pipe = pipe.to_path_buf();
    std::thread::spawn(move || {
        let _ = sent.send(std::fs::read_to_string(pipe));
    });
    received
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| panic!("waited {DEADLINE:?} for {what}"))
        .expect("read the pipe")
}

/// Waits until no process `pid` runs. A process that lost its parent is reaped by the
/// system's first process, in its own time, and has no event to wait for, so this looks
/// again at the one fact until it holds, up to [`DEADLINE`].
fn wait_until_gone(pid: u32, what: &str) {
    let until = Instant::now() + DEADLINE;
    while is_alive(pid) {
        assert!(
            Instant::now() < until,
            "{what} (process {pid}) was still there after {DEADLINE:?}"
        );
        std::thread::park_timeout(Duration::from_millis(2));
    }
}

// ---------------------------------------------------------------------------------------
// `take`
// ---------------------------------------------------------------------------------------

#[test]
fn what_the_shell_prints_before_the_marker_is_noise_and_what_follows_is_the_environment() {
    let scratch = Scratch::new();
    let base = scratch.environment();
    let shell = shell_of(
        &scratch,
        "shell",
        &format!(
            "echo 'NOISE=before'\n\
             printf 'MORE_NOISE=%s\\0' 'no newline'\n\
             printf '%s' '{MARKER}'\n\
             printf 'TAKEN=yes\\0'\n"
        ),
        DEADLINE,
    );

    let taken = take(&shell, &base).expect("an environment");

    assert_eq!(
        value_of(&taken, "TAKEN"),
        Some(&OsString::from("yes")),
        "what follows the marker is taken: {taken:?}"
    );
    assert_eq!(value_of(&taken, "NOISE"), None, "noise was taken");
    assert_eq!(value_of(&taken, "MORE_NOISE"), None, "noise was taken");
    assert_eq!(
        &taken[..base.len()],
        &base[..],
        "the Window's own variables come first, as they were"
    );
}

#[test]
fn the_shell_runs_in_the_windows_environment_and_no_other_and_in_the_folder_home_names() {
    let scratch = Scratch::new();
    let base = scratch.environment();
    let seen = scratch.root.join("seen");
    let shell = shell_of(
        &scratch,
        "shell",
        &format!(
            "pwd > {cwd}\n\
             env > {env}\n\
             printf '%s' '{MARKER}'\n",
            cwd = quoted(&seen.with_extension("cwd")),
            env = quoted(&seen.with_extension("env")),
        ),
        DEADLINE,
    );

    take(&shell, &base).expect("an environment");

    let cwd = std::fs::read_to_string(seen.with_extension("cwd")).expect("the folder it ran in");
    assert_eq!(
        Path::new(cwd.trim_end()).canonicalize().unwrap(),
        scratch.root.join("home").canonicalize().unwrap(),
        "the shell runs in the folder HOME names"
    );
    let env = std::fs::read_to_string(seen.with_extension("env")).expect("its environment");
    assert!(
        env.lines()
            .any(|line| line == format!("HOME={}", scratch.root.join("home").display())),
        "the Window's HOME: {env}"
    );
    // `Scratch::new` set these in this process; the shell must not have them.
    assert!(
        !env.contains("GIT_AUTHOR_NAME"),
        "the shell ran in the test's own environment and not in the Window's: {env}"
    );

    // With no HOME, the root.
    let base: Vec<_> = base.into_iter().filter(|(name, _)| name != "HOME").collect();
    take(&shell, &base).expect("an environment");
    let cwd = std::fs::read_to_string(seen.with_extension("cwd")).expect("the folder it ran in");
    assert_eq!(
        Path::new(cwd.trim_end()).canonicalize().unwrap(),
        Path::new("/").canonicalize().unwrap(),
        "with no HOME the shell runs in /"
    );
}

#[test]
fn the_variables_that_describe_the_shells_run_and_the_entries_that_are_no_variable_are_left_out() {
    let scratch = Scratch::new();
    let base = pairs(&[("HOME", "/"), ("PATH", "/usr/bin:/bin"), ("PWD", "/windows")]);
    let shell = shell_of(
        &scratch,
        "shell",
        &format!(
            "printf '%s' '{MARKER}'\n\
             printf 'PWD=/shells\\0OLDPWD=/old\\0SHLVL=7\\0_=/usr/bin/env\\0'\n\
             printf 'NOEQUALS\\0=x\\0'\n\
             printf 'KEPT=value=with=equals\\0PATH=/c:/usr/bin:/d\\0'\n"
        ),
        DEADLINE,
    );

    let taken = take(&shell, &base).expect("an environment");

    assert_eq!(
        taken,
        pairs(&[
            ("HOME", "/"),
            ("PATH", "/usr/bin:/bin:/c:/d"),
            ("PWD", "/windows"),
            ("KEPT", "value=with=equals"),
        ])
    );
}

#[test]
fn a_shell_that_cannot_be_started_ends_badly_or_prints_no_environment_gives_the_reason() {
    let scratch = Scratch::new();
    let base = scratch.environment();

    let absent = LoginShell {
        program: scratch.root.join("no-such-shell"),
        deadline: DEADLINE,
    };
    assert!(
        matches!(take(&absent, &base), Err(ShellError::Start(_))),
        "a program that is not there"
    );

    let status = shell_of(
        &scratch,
        "status",
        &format!("printf '%s' '{MARKER}'\nenv -0\nexit 3\n"),
        DEADLINE,
    );
    assert_eq!(take(&status, &base), Err(ShellError::Status(Some(3))));

    let signalled = shell_of(&scratch, "signalled", "kill -9 $$\n", DEADLINE);
    assert_eq!(take(&signalled, &base), Err(ShellError::Status(None)));

    let silent = shell_of(&scratch, "silent", "echo noise\nenv -0\n", DEADLINE);
    assert_eq!(take(&silent, &base), Err(ShellError::NoMarker));
}

#[test]
fn a_job_a_start_up_file_left_in_the_background_does_not_hold_the_output_open() {
    let scratch = Scratch::new();
    let base = scratch.environment();
    let job = scratch.root.join("job.pid");
    // The job keeps the shell's standard output, as a start-up file's `ssh-agent` or `sleep &`
    // does; the shell itself ends at once.
    let shell = shell_of(
        &scratch,
        "shell",
        &format!(
            "sleep 600 &\n\
             echo $! > {}\n\
             printf '%s' '{MARKER}'\n\
             printf 'TAKEN=yes\\0'\n",
            quoted(&job)
        ),
        DEADLINE,
    );

    // A read that waited for the job would run out the deadline and fail.
    let taken = take(&shell, &base).expect("an environment, not the deadline");
    assert_eq!(value_of(&taken, "TAKEN"), Some(&OsString::from("yes")));

    let pid: u32 = std::fs::read_to_string(&job)
        .expect("the job's process")
        .trim()
        .parse()
        .expect("a process number");
    wait_until_gone(pid, "the background job");
}

#[test]
fn a_shell_that_outlasts_its_deadline_is_killed_with_everything_it_started() {
    let scratch = Scratch::new();
    let base = scratch.environment();
    let (shell_pid, job_pid) = (scratch.root.join("shell.pid"), scratch.root.join("job.pid"));
    let shell = shell_of(
        &scratch,
        "shell",
        &format!(
            "echo $$ > {}\n\
             sleep 600 &\n\
             echo $! > {}\n\
             wait\n",
            quoted(&shell_pid),
            quoted(&job_pid)
        ),
        SHORT,
    );

    let started = Instant::now();
    let outcome = take(&shell, &base);
    let took = started.elapsed();

    assert_eq!(outcome, Err(ShellError::Deadline(SHORT)));
    assert!(
        took >= SHORT,
        "the call ended after {took:?}, before its deadline of {SHORT:?}"
    );
    let read = |file: &Path| -> u32 {
        std::fs::read_to_string(file)
            .unwrap_or_else(|_| panic!("the script did not start within {SHORT:?}"))
            .trim()
            .parse()
            .expect("a process number")
    };
    // The shell is reaped before the call returns; its job is the group's other member.
    assert!(!is_alive(read(&shell_pid)), "the shell is still there");
    wait_until_gone(read(&job_pid), "the shell's background job");
}

// ---------------------------------------------------------------------------------------
// NAV-16
// ---------------------------------------------------------------------------------------

/// What a scratch of the NAV-16 tests holds: the shell-bin folder where the shell puts the
/// stand-in on the path, the files the scripts write, and the registry row.
struct Rig {
    scratch: Scratch,
    shell_bin: PathBuf,
    /// One line for each run of the shell: its arguments.
    runs: PathBuf,
    /// The whole environment the last run of the server's program had.
    env_seen: PathBuf,
    /// The stand-in's log.
    log: PathBuf,
    row: String,
}

impl Rig {
    fn new() -> Rig {
        let scratch = Scratch::new();
        let shell_bin = scratch.root.join("shell-bin");
        let (env_seen, log) = (
            scratch.root.join("server.env"),
            scratch.root.join("standin.log"),
        );
        // The stand-in logs a few variables only: this script, which stands in its place on
        // the path, writes the whole environment it was run in and then becomes the
        // stand-in.
        write_script(
            &shell_bin.join("standin-lsp"),
            &format!(
                "env > {}\nexec {} \"$@\"\n",
                quoted(&env_seen),
                quoted(&standin())
            ),
        );
        let row = standin_row(
            "standin",
            &["--log", &log.display().to_string()],
            "answer",
            "",
        );
        Rig {
            runs: scratch.root.join("shell.runs"),
            scratch,
            shell_bin,
            env_seen,
            log,
            row,
        }
    }

    /// The configuration of the Window's own: the scratch's environment (its search path
    /// has no stand-in) with a variable the shell will also set, and `shell` as the login
    /// shell.
    fn config(&self, shell: LoginShell) -> yaar_navigator::NavigatorConfig {
        let mut config = self.scratch.config(&self.row);
        config
            .environment
            .push(("STANDIN_KEPT_BY_BASE".into(), "base".into()));
        config.login_shell = Some(shell);
        config
    }

    /// A script of the shell that records its arguments, prints noise, sets variables and
    /// the path, and then runs the command it was given, as a login shell does.
    fn shell(&self, deadline: Duration) -> LoginShell {
        shell_of(
            &self.scratch,
            "login-shell",
            &format!(
                "printf '%s\\n' \"$*\" >> {runs}\n\
                 echo 'STANDIN_NOISE=yes'\n\
                 STANDIN_FROM_SHELL=yes\n\
                 STANDIN_KEPT_BY_BASE=shell\n\
                 HOME=/the/shells/own/home\n\
                 PATH={bin}:$PATH\n\
                 export STANDIN_FROM_SHELL STANDIN_KEPT_BY_BASE HOME PATH\n\
                 exec /bin/sh -c \"$4\"\n",
                runs = quoted(&self.runs),
                bin = quoted(&self.shell_bin),
            ),
            deadline,
        )
    }

    /// A repository with a `main.sil`, so that the stand-in's row has a server to run.
    fn project(&self, name: &str) -> Repo {
        self.scratch
            .repository(name, &[("main.sil", "def alpha\n")])
    }

    /// The lines the shell recorded: one for each run.
    fn shell_runs(&self) -> Vec<String> {
        std::fs::read_to_string(&self.runs)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

/// Tells the Navigator the workspaces and then the one on screen, and reads the list each
/// ask makes it tell, so that what follows is read after both were handled.
fn show_all(running: &mut Running, roots: Vec<WorkspaceRoot>, on_screen: &yaar_link::WorkspaceId) {
    running.ask(Ask::Workspaces(roots));
    running.wait_told("the list for the workspaces", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    });
    running.ask(Ask::OnScreen(Some(on_screen.clone())));
    running.wait_told("the list for the workspace on screen", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    });
}

/// What a workspace of the repository named by `repository` is, for the Navigator.
fn root_of(id: &yaar_link::WorkspaceId, repository: char, repo: &Repo) -> WorkspaceRoot {
    WorkspaceRoot {
        id: id.clone(),
        repository: repository_id(repository),
        folder: Some(repo.root.clone()),
    }
}

/// Every line of the stand-in's log, as the JSON it is; none when it is not there.
fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

#[test]
fn the_servers_run_in_the_windows_environment_with_what_the_shell_prints_added_taken_once_and_not_before_a_server_is_needed()
 {
    let rig = Rig::new();
    let plain = rig.scratch.repository("plain", &[("notes.txt", "none\n")]);
    let (one, two) = (rig.project("one"), rig.project("two"));
    let (w_plain, w_one, w_two) = (workspace_id('1'), workspace_id('2'), workspace_id('3'));
    let roots = vec![
        root_of(&w_plain, '1', &plain),
        root_of(&w_one, '2', &one),
        root_of(&w_two, '3', &two),
    ];
    let config = rig.config(rig.shell(DEADLINE));
    let base_path = value_of(&config.environment, "PATH").expect("a PATH").clone();
    let mut running = Running::start(config);

    // Never before a server is needed: a workspace with no file of any row on screen.
    show_all(&mut running, roots.clone(), &w_plain);
    assert_eq!(
        rig.shell_runs(),
        Vec::<String>::new(),
        "the shell was run before a server was needed"
    );

    // The first server needed.
    running.ask(Ask::OnScreen(Some(w_one.clone())));
    let ready = running.wait_state(&w_one, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(
        ready.program,
        Some(rig.shell_bin.join("standin-lsp")),
        "the stand-in was found on the shell's path"
    );
    assert_eq!(
        rig.shell_runs(),
        vec![format!("-l -i -c {COMMAND}")],
        "the shell runs once, as the launchers run it"
    );

    // The stand-in wrote its `start` line before it answered the request that made it
    // ready, and the script wrote the environment before it became the stand-in.
    let started = log_of(&rig.log)
        .into_iter()
        .find(|line| line["event"] == "start")
        .expect("the stand-in's start line");
    assert_eq!(started["env"]["STANDIN_FROM_SHELL"], "yes");
    assert_eq!(
        started["env"]["STANDIN_KEPT_BY_BASE"], "base",
        "the Window's value of a variable both have"
    );
    assert_eq!(
        started["env"].get("STANDIN_NOISE"),
        None,
        "noise before the marker was taken"
    );
    assert_eq!(
        started["env"]["HOME"],
        rig.scratch.root.join("home").display().to_string(),
        "the Window's own HOME, not the shell's"
    );
    let env = std::fs::read(&rig.env_seen).expect("the environment of the server");
    let path_line = env
        .split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"PATH="))
        .expect("a PATH in the server's environment");
    let expected = std::env::join_paths(
        std::env::split_paths(&base_path)
            .chain(std::iter::once(rig.shell_bin.clone()))
            .collect::<Vec<_>>(),
    )
    .expect("a search path");
    assert_eq!(
        path_line,
        expected.as_bytes(),
        "the Window's search path, then the shell's entries it lacks"
    );

    // Once: a second server, started after the shell reported.
    running.ask(Ask::OnScreen(Some(w_two.clone())));
    running.wait_state(&w_two, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(rig.shell_runs().len(), 1, "the shell was run again");
    running.end();
}

/// A Navigator with the shell `shell`, a workspace with a file of the stand-in's row on
/// screen, and the state it comes to rest in: the stand-in lies only in the shell's folder,
/// so a shell whose environment is not taken leaves it missing. Gives the state, and the
/// Navigator ended.
fn rest_with(rig: &Rig, shell: LoginShell, deadline: Duration) -> State {
    let repo = rig.project("project");
    let ws = workspace_id('1');
    let mut running = Running::start(rig.config(shell));
    show_all(&mut running, vec![root_of(&ws, '1', &repo)], &ws);
    let state = running
        .wait_state(&ws, "standin", None, deadline, |state| {
            !matches!(state.state, State::Starting)
        })
        .state;
    running.end();
    state
}

#[test]
fn a_shell_that_fails_leaves_the_windows_own_environment_and_is_not_run_again() {
    let rig = Rig::new();
    let shell = shell_of(
        &rig.scratch,
        "failing-shell",
        &format!(
            "printf '%s\\n' \"$*\" >> {runs}\n\
             PATH={bin}:$PATH\n\
             export PATH\n\
             printf '%s' '{MARKER}'\n\
             env -0\n\
             exit 3\n",
            runs = quoted(&rig.runs),
            bin = quoted(&rig.shell_bin),
        ),
        DEADLINE,
    );
    let repo = rig.project("project");
    let (ws, other) = (workspace_id('1'), workspace_id('2'));
    let second = rig.project("second");
    let mut running = Running::start(rig.config(shell));
    show_all(
        &mut running,
        vec![root_of(&ws, '1', &repo), root_of(&other, '2', &second)],
        &ws,
    );
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Missing { offer: None }
    });
    // A second server after the failure: the shell is not tried again.
    running.ask(Ask::OnScreen(Some(other.clone())));
    running.wait_state(&other, "standin", None, DEADLINE, |state| {
        state.state == State::Missing { offer: None }
    });
    assert_eq!(rig.shell_runs().len(), 1, "a failed shell was run again");
    assert!(
        !rig.env_seen.exists(),
        "the stand-in on the shell's path was found and run"
    );
    running.end();
}

#[test]
fn a_shell_that_prints_no_environment_leaves_the_windows_own() {
    let rig = Rig::new();
    let shell = shell_of(
        &rig.scratch,
        "silent-shell",
        &format!(
            "PATH={bin}:$PATH\n\
             export PATH\n\
             echo noise\n\
             echo 'PATH={bin}:/usr/bin'\n",
            bin = quoted(&rig.shell_bin),
        ),
        DEADLINE,
    );

    let state = rest_with(&rig, shell, DEADLINE);

    assert_eq!(state, State::Missing { offer: None });
    assert!(
        !rig.env_seen.exists(),
        "the stand-in on the shell's path was found and run"
    );
}

#[test]
fn a_shell_that_outlasts_its_deadline_leaves_the_windows_own_and_is_gone() {
    let rig = Rig::new();
    let pipe = rig.scratch.root.join("shell.pid");
    make_pipe(&pipe);
    let shell = shell_of(
        &rig.scratch,
        "slow-shell",
        &format!("echo $$ > {}\nexec sleep 600\n", quoted(&pipe)),
        DEADLINE,
    );
    let repo = rig.project("project");
    let ws = workspace_id('1');
    let mut running = Running::start(rig.config(shell));
    show_all(&mut running, vec![root_of(&ws, '1', &repo)], &ws);

    let pid: u32 = read_pipe(&pipe, "the shell to say its process")
        .trim()
        .parse()
        .expect("a process number");
    // The shell is killed at its deadline, which counts from its start: past it, the
    // Navigator moves on with its own environment.
    let state = running.wait_state(&ws, "standin", None, 2 * DEADLINE, |state| {
        !matches!(state.state, State::Starting)
    });
    assert_eq!(state.state, State::Missing { offer: None });
    // The call reaped the shell before it reported, and the report came before this state.
    assert!(!is_alive(pid), "the shell outlived its deadline");
    running.end();
}
