//! The servers' environment taken from the user's login shell, shown with scripts that play
//! the shell. No test runs the user's own shell or reads their start-up files.

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

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn write_script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder above")).expect("make the folder");
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("write the script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script executable");
}

fn shell_of(scratch: &Scratch, name: &str, body: &str, deadline: Duration) -> LoginShell {
    let program = scratch.root.join(name);
    write_script(&program, body);
    LoginShell { program, deadline }
}

fn pairs(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    list.iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect()
}

fn value_of<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    environment
        .iter()
        .find(|(have, _)| have == name)
        .map(|(_, value)| value)
}

/// The one wait on no fact: long enough for an early shell to note its start, even at background
/// priority.
const SETTLE: Duration = Duration::from_secs(3);

fn settle() {
    let (_kept, never) = mpsc::channel::<()>();
    let _ = never.recv_timeout(SETTLE);
}

fn make_pipe(path: &Path) {
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("a path with no NUL");
    // SAFETY: `name` is a NUL-terminated path that lives through the call.
    let made = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
    assert_eq!(
        made,
        0,
        "make the pipe: {}",
        std::io::Error::last_os_error()
    );
}

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

/// An orphan is reaped by the system's first process in its own time, with no event to wait for.
fn wait_until_gone(pid: u32, what: &str) {
    let until = Instant::now() + DEADLINE;
    while is_alive(pid) {
        if Instant::now() >= until {
            // SAFETY: a signal to a process of this test's own making.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("{what} (process {pid}) was still there after {DEADLINE:?}");
        }
        std::thread::park_timeout(Duration::from_millis(2));
    }
}

struct KillOnFailure(Vec<u32>);

impl Drop for KillOnFailure {
    fn drop(&mut self) {
        if std::thread::panicking() {
            for pid in &self.0 {
                // SAFETY: a signal to a process of this test's own making.
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
            }
        }
    }
}

struct KillJobOnFailure(PathBuf);

impl Drop for KillJobOnFailure {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let pid = std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        if let Some(pid) = pid {
            // SAFETY: a signal to a process of this test's own making.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

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
    assert!(
        !env.contains("GIT_AUTHOR_NAME"),
        "the shell ran in the test's own environment and not in the Window's: {env}"
    );

    let base: Vec<_> = base
        .into_iter()
        .filter(|(name, _)| name != "HOME")
        .collect();
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
    let base = pairs(&[
        ("HOME", "/"),
        ("PATH", "/usr/bin:/bin"),
        ("PWD", "/windows"),
    ]);
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
    let _guard = KillJobOnFailure(job.clone());

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
    let pids = scratch.root.join("pids");
    make_pipe(&pids);
    let shell = shell_of(
        &scratch,
        "shell",
        &format!(
            "sleep 600 &\n\
             echo \"$$ $!\" > {}\n\
             wait\n",
            quoted(&pids)
        ),
        DEADLINE,
    );

    let (done, outcome) = mpsc::channel();
    std::thread::spawn({
        let base = base.clone();
        move || {
            let started = Instant::now();
            let result = take(&shell, &base);
            let _ = done.send((result, started.elapsed()));
        }
    });
    let said = read_pipe(&pids, "the shell to say its process and its job's");
    let mut numbers = said
        .split_whitespace()
        .map(|number| number.parse::<u32>().expect("a process number"));
    let (shell_pid, job_pid) = (
        numbers.next().expect("the shell's process"),
        numbers.next().expect("the job's process"),
    );
    let _cleanup = KillOnFailure(vec![shell_pid, job_pid]);
    let (result, took) = outcome
        .recv_timeout(2 * DEADLINE)
        .unwrap_or_else(|_| panic!("the call did not end within twice its deadline"));

    assert_eq!(result, Err(ShellError::Deadline(DEADLINE)));
    assert!(
        took >= DEADLINE,
        "the call ended after {took:?}, before its deadline of {DEADLINE:?}"
    );
    assert!(!is_alive(shell_pid), "the shell is still there");
    wait_until_gone(job_pid, "the shell's background job");
}

#[cfg(target_os = "linux")]
#[test]
fn a_shell_ends_when_the_window_that_started_it_is_killed() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let this_test = "a_shell_ends_when_the_window_that_started_it_is_killed";
    if std::env::var_os("YAAR_WINDOW_CHILD").is_some() {
        let scratch = Scratch::new();
        let base = scratch.environment();
        let pid_pipe = scratch.root.join("shell.pid");
        make_pipe(&pid_pipe);
        let shell = shell_of(
            &scratch,
            "never-ending-shell",
            &format!("echo $$ > {}\nexec sleep 600\n", quoted(&pid_pipe)),
            4 * DEADLINE,
        );
        std::thread::spawn(move || {
            let _ = take(&shell, &base);
        });
        let pid = read_pipe(&pid_pipe, "the shell to say its process");
        println!("SHELL {} {}", pid.trim(), scratch.root.display());
        let _ = std::io::stdin().read_line(&mut String::new());
        return;
    }

    let mut window = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", this_test, "--nocapture", "--test-threads=1"])
        .env("YAAR_WINDOW_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the test again as a Window of its own");
    let output = window.stdout.take().expect("the output was piped");
    let said = BufReader::new(output)
        .lines()
        .map_while(Result::ok)
        // The test harness has started a line of its own, so the sentence is not first.
        .find_map(|line| line.split_once("SHELL ").map(|(_, said)| said.to_owned()))
        .expect("the Window said which shell it started");
    let (pid, scratch) = said.split_once(' ').expect("a process and a folder");
    let pid: u32 = pid.parse().expect("a process number");
    assert!(is_alive(pid), "the shell runs while its Window does");

    window.kill().expect("kill the Window");
    window.wait().expect("reap the Window");
    wait_until_gone(pid, "the shell that the killed Window started");
    let _ = std::fs::remove_dir_all(scratch);
}

fn search_path_of(environment: &[(OsString, OsString)]) -> Vec<PathBuf> {
    value_of(environment, "PATH")
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default()
}

struct Rig {
    scratch: Scratch,
    shell_bin: PathBuf,
    runs: PathBuf,
    needed: PathBuf,
    saw: PathBuf,
    env_seen: PathBuf,
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
            needed: scratch.root.join("needed"),
            saw: scratch.root.join("shell.saw"),
            scratch,
            shell_bin,
            env_seen,
            log,
            row,
        }
    }

    fn config(&self, shell: LoginShell) -> yaar_navigator::NavigatorConfig {
        let mut config = self.scratch.config(&self.row);
        config
            .environment
            .push(("STANDIN_KEPT_BY_BASE".into(), "base".into()));
        config.login_shell = Some(shell);
        config.search_path = search_path_of(&config.environment);
        config
    }

    fn shell(&self, deadline: Duration) -> LoginShell {
        shell_of(
            &self.scratch,
            "login-shell",
            &format!(
                "if [ -e {needed} ]; then echo there; else echo 'not there'; fi >> {saw}\n\
                 printf '%s\\n' \"$*\" >> {runs}\n\
                 echo 'STANDIN_NOISE=yes'\n\
                 STANDIN_FROM_SHELL=yes\n\
                 STANDIN_KEPT_BY_BASE=shell\n\
                 HOME=/the/shells/own/home\n\
                 PATH={bin}:$PATH\n\
                 export STANDIN_FROM_SHELL STANDIN_KEPT_BY_BASE HOME PATH\n\
                 exec /bin/sh -c \"$4\"\n",
                needed = quoted(&self.needed),
                saw = quoted(&self.saw),
                runs = quoted(&self.runs),
                bin = quoted(&self.shell_bin),
            ),
            deadline,
        )
    }

    fn project(&self, name: &str) -> Repo {
        self.scratch
            .repository(name, &[("main.sil", "def alpha\n")])
    }

    fn shell_runs(&self) -> Vec<String> {
        std::fs::read_to_string(&self.runs)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

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

fn root_of(id: &yaar_link::WorkspaceId, repository: char, repo: &Repo) -> WorkspaceRoot {
    WorkspaceRoot {
        id: id.clone(),
        repository: repository_id(repository),
        folder: Some(repo.root.clone()),
    }
}

fn wait_logged(path: &Path, event: &str) -> Value {
    let until = Instant::now() + DEADLINE;
    loop {
        if let Some(line) = log_of(path).into_iter().find(|line| line["event"] == event) {
            return line;
        }
        assert!(
            Instant::now() < until,
            "the log {} held no `{event}` line after {DEADLINE:?}",
            path.display()
        );
        std::thread::park_timeout(Duration::from_millis(2));
    }
}

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
    let base_path = value_of(&config.environment, "PATH")
        .expect("a PATH")
        .clone();
    let mut running = Running::start(config);

    show_all(&mut running, roots.clone(), &w_plain);
    settle();
    // The script notes whether `needed` exists as it starts: a shell started before this write
    // notes "not there".
    std::fs::write(&rig.needed, "").expect("say that a server is needed");

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
    assert_eq!(
        std::fs::read_to_string(&rig.saw)
            .expect("the shell noted whether a server was needed")
            .lines()
            .collect::<Vec<_>>(),
        vec!["there"],
        "the shell was started before a server was needed"
    );

    let started = wait_logged(&rig.log, "start");
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

    running.ask(Ask::OnScreen(Some(w_two.clone())));
    running.wait_state(&w_two, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    assert_eq!(rig.shell_runs().len(), 1, "the shell was run again");
    running.end();
}

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
    // The shell is killed at its deadline, which counts from its start.
    let state = running.wait_state(&ws, "standin", None, 2 * DEADLINE, |state| {
        !matches!(state.state, State::Starting)
    });
    assert_eq!(state.state, State::Missing { offer: None });
    assert!(!is_alive(pid), "the shell outlived its deadline");
    running.end();
}

#[test]
fn the_programs_are_looked_for_on_the_windows_search_path_before_the_shells_entries() {
    let rig = Rig::new();
    let window_bin = rig.scratch.root.join("window-bin");
    rig.scratch.put_standin(&window_bin, "standin-lsp");
    let repo = rig.project("project");
    let ws = workspace_id('1');
    let mut config = rig.config(rig.shell(DEADLINE));
    for (name, value) in &mut config.environment {
        if name == "PATH" {
            let entries = std::iter::once(window_bin.clone()).chain(std::env::split_paths(value));
            *value = std::env::join_paths(entries).expect("a search path");
        }
    }
    config.search_path = search_path_of(&config.environment);
    let mut running = Running::start(config);
    show_all(&mut running, vec![root_of(&ws, '1', &repo)], &ws);

    let ready = running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    assert_eq!(
        ready.program,
        Some(window_bin.join("standin-lsp")),
        "the Window's search path comes before the shell's entries"
    );
    assert!(
        !rig.env_seen.exists(),
        "the stand-in of the shell's folder was run"
    );
    running.end();
}

#[test]
fn a_shell_still_running_when_the_navigator_ends_is_killed_with_it() {
    let rig = Rig::new();
    let pipe = rig.scratch.root.join("shell.pid");
    make_pipe(&pipe);
    let shell = shell_of(
        &rig.scratch,
        "slow-shell",
        &format!("echo $$ > {}\nexec sleep 600\n", quoted(&pipe)),
        4 * DEADLINE,
    );
    let repo = rig.project("project");
    let ws = workspace_id('1');
    let mut running = Running::start(rig.config(shell));
    show_all(&mut running, vec![root_of(&ws, '1', &repo)], &ws);
    let pid: u32 = read_pipe(&pipe, "the shell to say its process")
        .trim()
        .parse()
        .expect("a process number");

    running.end();

    wait_until_gone(pid, "the shell");
}
