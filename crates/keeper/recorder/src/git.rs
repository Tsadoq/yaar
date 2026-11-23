//! The one place that starts git.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The variables with which a process tells git which repository to use, whatever
/// folder it runs in. A Keeper started from a shell that has one of them set would read
/// that repository in place of the one it was asked about, so no run of git here
/// inherits them.
const REPOSITORY_VARIABLES: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

/// The git program of the user, as the app runs it to read.
///
/// Cheap to clone: it holds only where the program is and the environment it runs in.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
    /// The environment every run gets: the process's own unless a take ended.
    environment: GitEnvironment,
}

/// Where a run of git looks for its repository.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Place<'a> {
    /// In this folder, or above it, as git finds it.
    Folder(&'a Path),
    /// In this git folder, named outright.
    GitDir(&'a Path),
}

/// What one run of git gave.
#[derive(Debug)]
pub(crate) struct Answer {
    /// The command, as one would type it.
    command: String,
    /// Its exit code; none when a signal ended it.
    pub(crate) code: Option<i32>,
    /// What it printed.
    pub(crate) stdout: Vec<u8>,
    /// What it wrote to standard error.
    pub(crate) stderr: String,
}

impl Answer {
    /// What git printed, when it ended well.
    pub(crate) fn printed(self) -> Result<Vec<u8>, GitError> {
        if self.code == Some(0) {
            Ok(self.stdout)
        } else {
            Err(self.failed())
        }
    }

    /// The run as a failure of git.
    pub(crate) fn failed(self) -> GitError {
        GitError::Failed {
            command: self.command,
            code: self.code,
            stderr: self.stderr,
        }
    }

    /// The run as an answer that was not understood.
    pub(crate) fn unreadable(&self, what: impl Into<String>) -> GitError {
        unreadable(&self.command, what)
    }

    /// The command, as one would type it.
    pub(crate) fn command(&self) -> &str {
        &self.command
    }
}

/// An answer of `command` that was not understood.
pub(crate) fn unreadable(command: &str, what: impl Into<String>) -> GitError {
    GitError::Unreadable {
        command: command.to_owned(),
        what: what.into(),
    }
}

/// The environment the Keeper's runs of git get, once the Keeper has taken the user's login
/// shell's (`yaar_link::login`).
///
/// A Keeper started by a Window that the Finder, the Dock or a desktop's menu started has the
/// system's short search path, so the user's git would run the user's hooks, and filters such
/// as `git-lfs`, without the programs of the user's login path. The Keeper therefore takes the
/// login shell's environment once, at its start, on a thread of its own that never holds an
/// answer to a Window, and hands what it got here. Until the take has ended every run of git
/// gets the process's own environment, as before slice 10.
///
/// - A run that reads (`Git::command`) gets the taken environment when the take has ended
///   with one, and the process's own otherwise; it never waits.
/// - A run of the user's own git (`Git::plain`: the creation of a worktree, the commit
///   control), which runs the user's hooks, first waits for the take to have ended, for at
///   most the deadline [`GitEnvironment::waiting`] was given, and then gets the taken
///   environment, or the process's own when the take failed or the wait ran out.
/// - Either way the variables that name another repository stay removed, and the settings a
///   read carries are set over the environment, as before.
///
/// Cheap to clone: every clone is the same environment. A `Git` made by [`Git::find`] or
/// [`Git::at`] has [`GitEnvironment::own`], which never waits and is always the process's
/// own, so that every test and every caller of slices 1 to 9 runs git as before.
#[derive(Debug, Clone)]
pub struct GitEnvironment {
    inner: Arc<Inner>,
}

/// What every clone of one [`GitEnvironment`] shares.
#[derive(Debug)]
struct Inner {
    /// Where the take stands.
    take: Mutex<Take>,
    /// Wakes the runs that wait, when the take ends.
    ended: Condvar,
    /// When the take was announced: the wait of a run is counted from here, so that all the
    /// runs of a take that never ends together wait no longer than the deadline.
    since: Instant,
    /// How long a run of the user's own git waits for the take.
    deadline: Duration,
}

/// Where the take stands.
#[derive(Debug)]
enum Take {
    /// Under way.
    UnderWay,
    /// Ended: with the environment it gave, or with none.
    Ended(Option<Arc<Vec<(OsString, OsString)>>>),
}

impl GitEnvironment {
    /// The process's own environment, for ever: no take was started.
    pub fn own() -> GitEnvironment {
        GitEnvironment::in_state(Take::Ended(None), Duration::ZERO)
    }

    /// A take is under way. A run of the user's own git waits for it at most `deadline`
    /// (the shell's own deadline, `yaar_link::login::SHELL_DEADLINE`, for the Keeper).
    pub fn waiting(deadline: Duration) -> GitEnvironment {
        GitEnvironment::in_state(Take::UnderWay, deadline)
    }

    fn in_state(take: Take, deadline: Duration) -> GitEnvironment {
        GitEnvironment {
            inner: Arc::new(Inner {
                take: Mutex::new(take),
                ended: Condvar::new(),
                since: Instant::now(),
                deadline,
            }),
        }
    }

    /// The take has ended: with the merged environment it gave, or with none when it failed.
    /// Wakes every run that waits. A second call changes nothing.
    pub fn ended(&self, environment: Option<Vec<(OsString, OsString)>>) {
        let mut take = self.lock();
        if matches!(*take, Take::UnderWay) {
            *take = Take::Ended(environment.map(Arc::new));
            drop(take);
            self.inner.ended.notify_all();
        }
    }

    /// The environment a run gets now, without waiting: the taken one once the take ended
    /// with one; none (the process's own) before that, or when it failed.
    pub fn now(&self) -> Option<Vec<(OsString, OsString)>> {
        self.taken().map(|taken| taken.as_ref().clone())
    }

    /// [`GitEnvironment::now`], not copied.
    fn taken(&self) -> Option<Arc<Vec<(OsString, OsString)>>> {
        match &*self.lock() {
            Take::Ended(taken) => taken.clone(),
            Take::UnderWay => None,
        }
    }

    /// What a run of the user's own git gets: waits for the take to end, for at most what
    /// is left of the deadline, then as [`GitEnvironment::now`].
    fn after_waiting(&self) -> Option<Arc<Vec<(OsString, OsString)>>> {
        let left = self
            .inner
            .deadline
            .saturating_sub(self.inner.since.elapsed());
        let (take, _) = self
            .inner
            .ended
            .wait_timeout_while(self.lock(), left, |take| matches!(take, Take::UnderWay))
            .unwrap_or_else(PoisonError::into_inner);
        match &*take {
            Take::Ended(taken) => taken.clone(),
            Take::UnderWay => None,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Take> {
        self.inner
            .take
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Git {
    /// This git, whose runs get `environment` as the doc of [`GitEnvironment`] says.
    pub fn with_environment(self, environment: GitEnvironment) -> Git {
        Git {
            environment,
            ..self
        }
    }

    /// The `git` on the search path of this process.
    pub fn find() -> Result<Git, GitError> {
        let search = std::env::var_os("PATH").ok_or(GitError::NotFound)?;
        std::env::split_paths(&search)
            // An entry that is not a full path means a folder under wherever this
            // process happens to run: not a place to take a program from.
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join("git"))
            .find(|candidate| is_program(candidate))
            .map(Git::at)
            .ok_or(GitError::NotFound)
    }

    /// The git program at this path.
    pub fn at(program: impl Into<PathBuf>) -> Git {
        Git {
            program: program.into(),
            environment: GitEnvironment::own(),
        }
    }

    /// Where the program is.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// A run of git that reads, not yet started, with everything every read carries:
    /// no hook of the hooks folder, no file-system monitor, each hook the configuration
    /// defines turned off by its key in `hooks_off`, no lock that can be done without,
    /// no question at a terminal, and git's own language.
    pub(crate) fn command(&self, place: Place<'_>, hooks_off: &[OsString]) -> Command {
        let taken = self.environment.taken();
        let mut command = self.start(taken.as_deref());
        command.args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ]);
        match place {
            Place::Folder(folder) => {
                command.arg("-C").arg(folder);
            }
            Place::GitDir(git_dir) => {
                let mut option = OsString::from("--git-dir=");
                option.push(git_dir);
                command.arg(option);
            }
        }
        for name in REPOSITORY_VARIABLES {
            command.env_remove(name);
        }
        command
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null());

        // The name of a hook can hold any character, an equals sign too, which `-c`
        // would cut at. Settings handed over in the environment are taken whole. They
        // are numbered after those the environment of this process already holds.
        if !hooks_off.is_empty() {
            let before = match &taken {
                Some(taken) => taken
                    .iter()
                    .find(|(name, _)| name == "GIT_CONFIG_COUNT")
                    .map(|(_, count)| count.to_owned()),
                None => std::env::var_os("GIT_CONFIG_COUNT"),
            }
            .and_then(|count| count.to_str()?.parse::<usize>().ok())
            .unwrap_or(0);
            for (n, key) in hooks_off.iter().enumerate() {
                command
                    .env(format!("GIT_CONFIG_KEY_{}", before + n), key)
                    .env(format!("GIT_CONFIG_VALUE_{}", before + n), "false");
            }
            command.env("GIT_CONFIG_COUNT", (before + hooks_off.len()).to_string());
        }
        command
    }

    /// A run of the user's own git, not yet started, as the user would start it in
    /// `folder`: none of the settings every read carries, so that the user's hooks,
    /// identity, signing and language apply. Only the variables that name another
    /// repository are left out, and nothing is asked at a terminal.
    pub(crate) fn plain(&self, folder: &Path) -> Command {
        let taken = self.environment.after_waiting();
        let mut command = self.start(taken.as_deref());
        command.arg("-C").arg(folder);
        for name in REPOSITORY_VARIABLES {
            command.env_remove(name);
        }
        command.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());
        command
    }

    /// The program, not yet started, in `taken` when the take gave an environment, and in
    /// the process's own otherwise.
    fn start(&self, taken: Option<&Vec<(OsString, OsString)>>) -> Command {
        let mut command = Command::new(&self.program);
        if let Some(taken) = taken {
            command
                .env_clear()
                .envs(taken.iter().map(|(name, value)| (name, value)));
        }
        command
    }

    /// Runs git to its end and gives what it answered, whatever its exit code.
    pub(crate) fn run<I, S>(
        &self,
        place: Place<'_>,
        hooks_off: &[OsString],
        args: I,
    ) -> Result<Answer, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(place, hooks_off);
        command.args(args);
        finish(command)
    }
}

/// Runs a prepared command to its end.
pub(crate) fn finish(mut command: Command) -> Result<Answer, GitError> {
    let shown = shown(&command);
    let output = command.output().map_err(could_not_start)?;
    Ok(Answer {
        command: shown,
        code: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Runs a prepared command to its end with `input` on its standard input.
pub(crate) fn finish_with(mut command: Command, input: &[u8]) -> Result<Answer, GitError> {
    let shown = shown(&command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(could_not_start)?;
    let Some(mut asks) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("git was started without its input").into());
    };
    // Written on a thread of its own, so that git can fill its output while it reads:
    // a pipe that nobody empties would stop both. A write that fails means git ended
    // before it read all, and its exit code says why.
    let output = std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = asks.write_all(input);
        });
        child.wait_with_output()
    })?;
    Ok(Answer {
        command: shown,
        code: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Why git did not start.
pub(crate) fn could_not_start(e: io::Error) -> GitError {
    if e.kind() == io::ErrorKind::NotFound {
        GitError::NotFound
    } else {
        GitError::Io(e)
    }
}

/// The command as one would type it: `git` and what was asked, without the settings
/// every run carries.
pub(crate) fn shown(command: &Command) -> String {
    let mut shown = String::from("git");
    let mut args = command.get_args();
    while let Some(arg) = args.next() {
        if arg == "-c" || arg == "-C" {
            args.next();
        } else {
            shown.push(' ');
            shown.push_str(&arg.to_string_lossy());
        }
    }
    shown
}

fn is_program(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
}

/// Why git gave no answer.
#[derive(Debug)]
pub enum GitError {
    /// There is no git program to run.
    NotFound,
    /// git could not be started, or its output could not be read.
    Io(io::Error),
    /// git ran and failed.
    Failed {
        /// The command, as one would type it.
        command: String,
        /// Its exit code; none when a signal ended it.
        code: Option<i32>,
        /// What it wrote to standard error.
        stderr: String,
    },
    /// git answered with something this crate does not understand.
    Unreadable {
        /// The command, as one would type it.
        command: String,
        /// What was not understood.
        what: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "git was not found"),
            Self::Io(e) => write!(f, "git could not be run: {e}"),
            Self::Failed {
                command,
                code,
                stderr,
            } => match code {
                Some(code) => write!(f, "`{command}` failed with {code}: {}", stderr.trim()),
                None => write!(f, "`{command}` was stopped: {}", stderr.trim()),
            },
            Self::Unreadable { command, what } => {
                write!(f, "`{command}` answered what is not understood: {what}")
            }
        }
    }
}

impl std::error::Error for GitError {}

impl From<io::Error> for GitError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// How long a run that must not wait is given to answer: far more than it needs, far
    /// less than the deadlines the tests hand to the environment.
    const ANSWERS_WITHIN: Duration = Duration::from_secs(5);

    /// How long a run that must wait is watched before the take ends: the one wait of these
    /// tests that is not on a fact. A run that does not wait answers within microseconds,
    /// so this is long enough on a loaded machine too, and a passing run cannot fail on it.
    const WATCHED_FOR: Duration = Duration::from_millis(300);

    fn variables(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    /// What `command` sets and removes, by name: a removed variable is none.
    fn set_in(command: &Command) -> HashMap<OsString, Option<OsString>> {
        command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
            .collect()
    }

    fn value(set: &HashMap<OsString, Option<OsString>>, name: &str) -> Option<OsString> {
        set.get(OsStr::new(name)).cloned().flatten()
    }

    #[test]
    fn the_own_environment_is_none_and_no_end_changes_it() {
        let own = GitEnvironment::own();
        assert_eq!(own.now(), None);
        own.ended(Some(variables(&[("PATH", "/login")])));
        assert_eq!(own.now(), None, "an environment nobody took was taken");
    }

    #[test]
    fn a_take_under_way_gives_none_until_it_ends_and_then_what_it_gave() {
        let waiting = GitEnvironment::waiting(Duration::from_secs(60));
        assert_eq!(waiting.now(), None);
        let taken = variables(&[("PATH", "/login:/usr/bin"), ("X", "1")]);
        waiting.ended(Some(taken.clone()));
        assert_eq!(waiting.now(), Some(taken));

        let failed = GitEnvironment::waiting(Duration::from_secs(60));
        failed.ended(None);
        assert_eq!(failed.now(), None);
    }

    #[test]
    fn a_second_end_changes_nothing() {
        let first = variables(&[("A", "first")]);
        let one = GitEnvironment::waiting(Duration::from_secs(60));
        one.ended(Some(first.clone()));
        one.ended(Some(variables(&[("A", "second")])));
        one.ended(None);
        assert_eq!(one.now(), Some(first));

        let two = GitEnvironment::waiting(Duration::from_secs(60));
        two.ended(None);
        two.ended(Some(variables(&[("A", "late")])));
        assert_eq!(two.now(), None);
    }

    #[test]
    fn every_clone_is_the_same_environment() {
        let waiting = GitEnvironment::waiting(Duration::from_secs(60));
        let clone = waiting.clone();
        let git = Git::at("git").with_environment(waiting);
        clone.ended(Some(variables(&[("PATH", "/login")])));
        let command = git.plain(Path::new("/"));
        assert_eq!(
            value(&set_in(&command), "PATH"),
            Some(OsString::from("/login"))
        );
    }

    #[test]
    fn a_read_gets_the_taken_environment_and_never_waits() {
        let waiting = GitEnvironment::waiting(Duration::from_secs(60));
        let git = Git::at("git").with_environment(waiting.clone());
        // The take is under way and ends only after the read has answered: a read that
        // waited for it would not.
        let (answer_tx, answer) = mpsc::channel();
        let reader = {
            let git = git.clone();
            std::thread::spawn(move || {
                let command = git.command(Place::Folder(Path::new("/")), &[]);
                let _ = answer_tx.send(set_in(&command));
            })
        };
        let before = answer
            .recv_timeout(ANSWERS_WITHIN)
            .expect("a read waited for the take");
        reader.join().expect("the reader ended");
        assert_eq!(
            value(&before, "GIT_OPTIONAL_LOCKS"),
            Some(OsString::from("0"))
        );
        assert_eq!(value(&before, "LOGIN_ONLY"), None, "taken before the end");

        waiting.ended(Some(variables(&[
            ("LOGIN_ONLY", "yes"),
            ("LC_ALL", "de_DE"),
        ])));
        let after = set_in(&git.command(Place::Folder(Path::new("/")), &[]));
        assert_eq!(value(&after, "LOGIN_ONLY"), Some(OsString::from("yes")));
        // What a read carries is set over what was taken.
        assert_eq!(value(&after, "LC_ALL"), Some(OsString::from("C")));
        assert_eq!(
            value(&after, "GIT_OPTIONAL_LOCKS"),
            Some(OsString::from("0"))
        );
    }

    #[test]
    fn the_variables_that_name_another_repository_stay_removed_from_the_taken_environment() {
        let taken = GitEnvironment::waiting(Duration::from_secs(60));
        taken.ended(Some(variables(&[
            ("GIT_DIR", "/elsewhere"),
            ("GIT_WORK_TREE", "/elsewhere"),
            ("PATH", "/login"),
        ])));
        let git = Git::at("git").with_environment(taken);
        for command in [
            git.command(Place::Folder(Path::new("/")), &[]),
            git.plain(Path::new("/")),
        ] {
            let set = set_in(&command);
            assert_eq!(value(&set, "GIT_DIR"), None);
            assert_eq!(value(&set, "GIT_WORK_TREE"), None);
            assert_eq!(value(&set, "PATH"), Some(OsString::from("/login")));
        }
    }

    #[test]
    fn the_hooks_a_read_turns_off_are_numbered_after_the_settings_of_the_environment_it_gets() {
        let taken = GitEnvironment::waiting(Duration::from_secs(60));
        taken.ended(Some(variables(&[
            ("GIT_CONFIG_COUNT", "2"),
            ("GIT_CONFIG_KEY_0", "a.b"),
            ("GIT_CONFIG_VALUE_0", "1"),
            ("GIT_CONFIG_KEY_1", "c.d"),
            ("GIT_CONFIG_VALUE_1", "2"),
        ])));
        let git = Git::at("git").with_environment(taken);
        let hooks = [OsString::from("hook.x.enabled")];
        let set = set_in(&git.command(Place::Folder(Path::new("/")), &hooks));
        assert_eq!(value(&set, "GIT_CONFIG_COUNT"), Some(OsString::from("3")));
        assert_eq!(
            value(&set, "GIT_CONFIG_KEY_2"),
            Some(OsString::from("hook.x.enabled"))
        );
        assert_eq!(
            value(&set, "GIT_CONFIG_VALUE_2"),
            Some(OsString::from("false"))
        );
        // The settings the environment held are the environment's own, and stay.
        assert_eq!(value(&set, "GIT_CONFIG_KEY_1"), Some(OsString::from("c.d")));
    }

    #[test]
    fn the_users_own_git_waits_for_the_take_and_then_gets_what_it_gave() {
        let waiting = GitEnvironment::waiting(Duration::from_secs(60));
        let git = Git::at("git").with_environment(waiting.clone());
        let (started_tx, started) = mpsc::channel();
        let (answer_tx, answer) = mpsc::channel();
        let user = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let command = git.plain(Path::new("/"));
            let _ = answer_tx.send(set_in(&command));
        });
        started
            .recv_timeout(ANSWERS_WITHIN)
            .expect("the run started");
        assert!(
            answer.recv_timeout(WATCHED_FOR).is_err(),
            "the user's own git did not wait for the take"
        );
        waiting.ended(Some(variables(&[("PATH", "/login")])));
        let set = answer
            .recv_timeout(ANSWERS_WITHIN)
            .expect("the wait ended with the take");
        user.join().expect("the run ended");
        assert_eq!(value(&set, "PATH"), Some(OsString::from("/login")));
    }

    #[test]
    fn the_users_own_git_gets_the_process_own_when_the_take_failed_and_does_not_wait_after_it() {
        let waiting = GitEnvironment::waiting(Duration::from_secs(60));
        let git = Git::at("git").with_environment(waiting.clone());
        let (started_tx, started) = mpsc::channel();
        let (answer_tx, answer) = mpsc::channel();
        let user = {
            let git = git.clone();
            std::thread::spawn(move || {
                let _ = started_tx.send(());
                let command = git.plain(Path::new("/"));
                let _ = answer_tx.send(set_in(&command));
            })
        };
        started
            .recv_timeout(ANSWERS_WITHIN)
            .expect("the run started");
        assert!(answer.recv_timeout(WATCHED_FOR).is_err());
        waiting.ended(None);
        let set = answer
            .recv_timeout(ANSWERS_WITHIN)
            .expect("the wait ended with the take");
        user.join().expect("the run ended");
        assert_eq!(value(&set, "PATH"), None);
        assert_eq!(
            value(&set, "GIT_TERMINAL_PROMPT"),
            Some(OsString::from("0"))
        );

        // Later runs find the end at once.
        let (again_tx, again) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = again_tx.send(set_in(&git.plain(Path::new("/"))));
        });
        again
            .recv_timeout(ANSWERS_WITHIN)
            .expect("a run waited for a take that had ended");
    }

    #[test]
    fn the_users_own_git_waits_no_longer_than_the_deadline_and_then_gets_the_process_own() {
        let deadline = Duration::from_millis(400);
        let started = Instant::now();
        let waiting = GitEnvironment::waiting(deadline);
        let git = Git::at("git").with_environment(waiting.clone());
        let command = git.plain(Path::new("/"));
        assert!(
            started.elapsed() >= deadline,
            "the run gave up before the deadline"
        );
        assert_eq!(value(&set_in(&command), "PATH"), None);
        // A take that ends after the wait gave up is still the environment of later reads.
        waiting.ended(Some(variables(&[("PATH", "/late")])));
        assert_eq!(waiting.now(), Some(variables(&[("PATH", "/late")])));
    }

    #[test]
    fn a_git_made_by_find_or_at_runs_in_the_process_own_environment_and_never_waits() {
        let git = Git::at("git");
        let (answer_tx, answer) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = answer_tx.send((
                set_in(&git.plain(Path::new("/"))),
                set_in(&git.command(Place::Folder(Path::new("/")), &[])),
            ));
        });
        let (plain, read) = answer.recv_timeout(ANSWERS_WITHIN).expect("an answer");
        assert_eq!(value(&plain, "PATH"), None);
        assert_eq!(value(&read, "PATH"), None);
    }
}
