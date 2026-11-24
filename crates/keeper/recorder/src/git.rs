use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

/// Variables that tell git which repository to use. A Keeper started from a shell that sets one
/// would read that repository instead of the one asked about, so no run inherits them.
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

#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
    environment: GitEnvironment,
    has_newer_forms: Arc<OnceLock<bool>>,
}

/// The oldest git known to have every form the reads ask for first: `-z` after `worktree list`, the
/// atom `ahead-behind` of `for-each-ref`, `-Z` after `cat-file --batch`. Ubuntu 22.04 (2.34) and
/// Debian 12 (2.39) each lack at least one; gits from 2.40 to 2.42 were not checked and are asked
/// the older way too.
const NEWER: (u32, u32) = (2, 43);

#[derive(Debug, Clone, Copy)]
pub(crate) enum Place<'a> {
    Folder(&'a Path),
    GitDir(&'a Path),
}

#[derive(Debug)]
pub(crate) struct Answer {
    command: String,
    /// Its exit code; none when a signal ended it.
    pub(crate) code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: String,
}

impl Answer {
    pub(crate) fn printed(self) -> Result<Vec<u8>, GitError> {
        if self.code == Some(0) {
            Ok(self.stdout)
        } else {
            Err(self.failed())
        }
    }

    pub(crate) fn failed(self) -> GitError {
        GitError::Failed {
            command: self.command,
            code: self.code,
            stderr: self.stderr,
        }
    }

    pub(crate) fn unreadable(&self, what: impl Into<String>) -> GitError {
        unreadable(&self.command, what)
    }

    pub(crate) fn command(&self) -> &str {
        &self.command
    }
}

pub(crate) fn unreadable(command: &str, what: impl Into<String>) -> GitError {
    GitError::Unreadable {
        command: command.to_owned(),
        what: what.into(),
    }
}

/// The environment of the Keeper's runs of git, set once the login shell's is taken: from the
/// Finder the Keeper has a short search path, so hooks and filters such as `git-lfs` would miss the
/// user's programs. Reads never wait for it; the user's own git waits up to a deadline.
#[derive(Debug, Clone)]
pub struct GitEnvironment {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    take: Mutex<Take>,
    ended: Condvar,
    deadline: Duration,
}

#[derive(Debug)]
enum Take {
    UnderWay,
    Ended(Option<Arc<Vec<(OsString, OsString)>>>),
}

impl GitEnvironment {
    pub fn own() -> GitEnvironment {
        GitEnvironment::in_state(Take::Ended(None), Duration::ZERO)
    }

    pub fn waiting(deadline: Duration) -> GitEnvironment {
        GitEnvironment::in_state(Take::UnderWay, deadline)
    }

    fn in_state(take: Take, deadline: Duration) -> GitEnvironment {
        GitEnvironment {
            inner: Arc::new(Inner {
                take: Mutex::new(take),
                ended: Condvar::new(),
                deadline,
            }),
        }
    }

    pub fn ended(&self, environment: Option<Vec<(OsString, OsString)>>) {
        let mut take = self.lock();
        if matches!(*take, Take::UnderWay) {
            *take = Take::Ended(environment.map(Arc::new));
            drop(take);
            self.inner.ended.notify_all();
        }
    }

    /// None means the process's own environment.
    pub fn now(&self) -> Option<Vec<(OsString, OsString)>> {
        self.taken().map(|taken| taken.as_ref().clone())
    }

    fn taken(&self) -> Option<Arc<Vec<(OsString, OsString)>>> {
        match &*self.lock() {
            Take::Ended(taken) => taken.clone(),
            Take::UnderWay => None,
        }
    }

    fn after_waiting(&self) -> Option<Arc<Vec<(OsString, OsString)>>> {
        let (take, _) = self
            .inner
            .ended
            .wait_timeout_while(self.lock(), self.inner.deadline, |take| {
                matches!(take, Take::UnderWay)
            })
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
    pub fn with_environment(self, environment: GitEnvironment) -> Git {
        Git {
            environment,
            ..self
        }
    }

    pub fn find() -> Result<Git, GitError> {
        let search = std::env::var_os("PATH").ok_or(GitError::NotFound)?;
        std::env::split_paths(&search)
            // An entry that is not a full path is relative to wherever this process runs.
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join("git"))
            .find(|candidate| is_program(candidate))
            .map(Git::at)
            .ok_or(GitError::NotFound)
    }

    pub fn at(program: impl Into<PathBuf>) -> Git {
        Git {
            program: program.into(),
            environment: GitEnvironment::own(),
            has_newer_forms: Arc::new(OnceLock::new()),
        }
    }

    /// A version that cannot be read counts as new: git then fails in its own words.
    pub(crate) fn has_newer_forms(&self) -> bool {
        *self.has_newer_forms.get_or_init(|| {
            let mut command = self.command(Place::Folder(Path::new("/")), &[]);
            command.arg("version");
            let Ok(answer) = finish(command) else {
                return true;
            };
            says_newer_forms(&String::from_utf8_lossy(&answer.stdout))
        })
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// A run that reads, not yet started; `hooks_off` holds the keys of the hooks the configuration
    /// defines, each turned off.
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

        // A hook name can hold an equals sign, which `-c` would cut at: the settings go in the
        // environment, numbered after those the environment the run gets already holds.
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

    /// The user's own git as the user would start it in `folder`: the user's hooks, identity,
    /// signing and language apply. Only variables that name another repository are left out.
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

    fn start(&self, taken: Option<&Vec<(OsString, OsString)>>) -> Command {
        let mut command = Command::new(&self.program);
        if let Some(taken) = taken {
            command
                .env_clear()
                .envs(taken.iter().map(|(name, value)| (name, value)));
        }
        command
    }

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

fn says_newer_forms(version: &str) -> bool {
    let mut numbers = version
        .strip_prefix("git version ")
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_digit())
        .map(|number| number.parse::<u32>());
    match (numbers.next(), numbers.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= NEWER,
        _ => true,
    }
}

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
    // Written on a thread of its own so that git can fill its output while it reads: a pipe nobody
    // empties would stop both. A failed write means git ended before it read all.
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

pub(crate) fn could_not_start(e: io::Error) -> GitError {
    if e.kind() == io::ErrorKind::NotFound {
        GitError::NotFound
    } else {
        GitError::Io(e)
    }
}

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

#[derive(Debug)]
pub enum GitError {
    NotFound,
    Io(io::Error),
    Failed {
        command: String,
        code: Option<i32>,
        stderr: String,
    },
    Unreadable {
        command: String,
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

    const ANSWERS_WITHIN: Duration = Duration::from_secs(5);

    const QUICKLY: Duration = Duration::from_secs(1);

    /// Long enough that a run that does not wait has answered even on a loaded machine.
    const WATCHED_FOR: Duration = Duration::from_millis(300);

    /// Many reads in a row, so that even a short wait on each would add up past `QUICKLY`.
    const READS: usize = 20;

    fn variables(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

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
    fn a_git_has_the_newer_forms_from_2_43_on_and_one_that_says_nothing_clear_is_taken_to_have_them()
     {
        for old in [
            "git version 2.34.1\n",
            "git version 2.39.5\n",
            "git version 2.39.5 (Apple Git-154)\n",
            "git version 2.9.5\n",
            "git version 1.8.3.1\n",
        ] {
            assert!(!says_newer_forms(old), "{old:?}");
        }
        for new in [
            "git version 2.43.0\n",
            "git version 2.47.3\n",
            "git version 2.51.0\n",
            "git version 2.54.0 (Apple Git-160)\n",
            "git version 2.100.0\n",
            "git version 3.0.0\n",
            "git version 2.45.GIT\n",
            "git version 2.43.0.windows.1\n",
            "not git\n",
            "git version \n",
            "",
        ] {
            assert!(says_newer_forms(new), "{new:?}");
        }
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
        // The take ends only after the read has answered: a read that waited for it would not.
        let (answer_tx, answer) = mpsc::channel();
        let reader = {
            let git = git.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let mut command = git.command(Place::Folder(Path::new("/")), &[]);
                for _ in 1..READS {
                    command = git.command(Place::Folder(Path::new("/")), &[]);
                }
                let _ = answer_tx.send((set_in(&command), started.elapsed()));
            })
        };
        let (before, took) = answer
            .recv_timeout(ANSWERS_WITHIN)
            .expect("a read waited for the take");
        reader.join().expect("the reader ended");
        assert!(
            took < QUICKLY,
            "{READS} reads took {took:?} while the take was under way: a read waited"
        );
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
        // Long enough that a wait of twice the deadline is told from one of the deadline.
        let deadline = Duration::from_secs(2);
        let waiting = GitEnvironment::waiting(deadline);
        let git = Git::at("git").with_environment(waiting.clone());
        let (answer_tx, answer) = mpsc::channel();
        let started = Instant::now();
        let until = started + deadline + Duration::from_secs(1);
        let user = std::thread::spawn(move || {
            let command = git.plain(Path::new("/"));
            let _ = answer_tx.send((set_in(&command), started.elapsed()));
        });
        let (set, waited) = answer
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| {
                panic!(
                    "the run waited more than {:?} for a deadline of {deadline:?}",
                    started.elapsed()
                )
            });
        user.join().expect("the run ended");
        assert!(waited >= deadline, "the run gave up before the deadline");
        assert!(
            waited < deadline + Duration::from_secs(1),
            "the run waited {waited:?} for a deadline of {deadline:?}"
        );
        assert_eq!(value(&set, "PATH"), None);
        waiting.ended(Some(variables(&[("PATH", "/late")])));
        assert_eq!(waiting.now(), Some(variables(&[("PATH", "/late")])));
    }

    #[test]
    fn a_git_made_by_find_or_at_runs_in_the_process_own_environment_and_never_waits() {
        let (answer_tx, answer) = mpsc::channel();
        std::thread::spawn(move || {
            let found = Git::find().expect("git on the search path");
            for git in [Git::at("git"), found] {
                let started = Instant::now();
                let plain = set_in(&git.plain(Path::new("/")));
                let read = set_in(&git.command(Place::Folder(Path::new("/")), &[]));
                let _ = answer_tx.send((plain, read, started.elapsed()));
            }
        });
        for _ in 0..2 {
            let (plain, read, took) = answer.recv_timeout(ANSWERS_WITHIN).expect("an answer");
            assert_eq!(value(&plain, "PATH"), None);
            assert_eq!(value(&read, "PATH"), None);
            assert!(took < QUICKLY, "a run took {took:?}: it waited");
        }
    }
}
