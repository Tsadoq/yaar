//! Who is credited with a changed path: the rule, with no file, no watcher and no clock of
//! its own. Every moment is given by the caller.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yaar_recorder::Change;

/// How long after a shell command ended a change seen still falls within it. The system
/// tells of a write some milliseconds after it lands, about 12 ms on macOS and at once
/// with inotify. It is short, since a change seen within it of a command that ended while
/// another session's command runs counts for both, and makes a change no agent made: in
/// the recording of two sessions in one folder, one session's command began 135 ms after
/// the other's ended.
pub const SETTLE: Duration = Duration::from_millis(50);

/// One uncommitted change of a worktree, with who made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditedPath {
    /// The path, relative to the folder of the worktree.
    pub path: PathBuf,
    /// What happened to it, from the commit checked out to the files on disk.
    pub change: Change,
    /// The sessions it is credited to, by their ids, in the order they were first credited;
    /// empty for a change no agent made.
    pub by: Vec<String>,
}

/// One shell command of a session: when it began, and when it ended, if it has.
#[derive(Debug, Clone, Copy)]
struct Window {
    began: Instant,
    ended: Option<Instant>,
}

impl Window {
    /// Whether a change seen at `seen` falls within the command, judged at `now`.
    fn holds(&self, seen: Instant, now: Instant) -> bool {
        let last = match self.ended {
            Some(ended) => ended + SETTLE,
            None => now,
        };
        self.began <= seen && seen <= last
    }
}

/// The ledger of one worktree: the writes the agents' tools named, the shell commands of
/// each session with when they began and ended, and when each path was seen changing.
#[derive(Debug, Default)]
pub struct Ledger {
    // The sessions that wrote each path with their own tool, in the order they did.
    named: BTreeMap<PathBuf, Vec<String>>,
    // The moments the watcher saw each path change.
    seen: BTreeMap<PathBuf, Vec<Instant>>,
    // The shell commands of each session, the oldest first.
    commands: BTreeMap<String, Vec<Window>>,
}

impl Ledger {
    /// A ledger that has noted nothing.
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// `session` wrote `path`, relative to the worktree, with its own tool.
    pub fn wrote(&mut self, session: &str, path: &Path) {
        let sessions = self.named.entry(path.to_owned()).or_default();
        if !sessions.iter().any(|named| named == session) {
            sessions.push(session.to_owned());
        }
    }

    /// A shell command of `session` began at `at`.
    pub fn began(&mut self, session: &str, at: Instant) {
        self.commands
            .entry(session.to_owned())
            .or_default()
            .push(Window {
                began: at,
                ended: None,
            });
    }

    /// The shell command of `session` ended at `at`.
    pub fn ended(&mut self, session: &str, at: Instant) {
        // Of several commands that run at once, the one that began first is taken to end
        // first: the report of an end does not say which command it is. A report of an
        // end for a command never reported begun closes nothing.
        let open = self
            .commands
            .get_mut(session)
            .and_then(|windows| windows.iter_mut().find(|window| window.ended.is_none()));
        if let Some(window) = open {
            window.ended = Some(at.max(window.began));
        }
    }

    /// The watcher saw `path`, relative to the worktree, change at `at`.
    pub fn seen(&mut self, path: &Path, at: Instant) {
        self.seen.entry(path.to_owned()).or_default().push(at);
    }

    /// Who is credited with the change of `path`, judged at `now`: the sessions whose own
    /// tool named it; else, for each time the watcher saw it change, the one session whose
    /// shell command ran then, the command counted from its beginning to [`SETTLE`] after
    /// its end, and none when no command or more than one ran then. A command that has
    /// not ended runs until `now`. Empty: no agent made it.
    pub fn credit(&self, path: &Path, now: Instant) -> Vec<String> {
        if let Some(named) = self.named.get(path) {
            return named.clone();
        }
        let mut credited: Vec<String> = Vec::new();
        for &seen in self.seen.get(path).into_iter().flatten() {
            let mut running = self
                .commands
                .iter()
                .filter(|(_, windows)| windows.iter().any(|window| window.holds(seen, now)));
            if let (Some((session, _)), None) = (running.next(), running.next())
                && !credited.contains(session)
            {
                credited.push(session.clone());
            }
        }
        credited
    }

    /// The paths the ledger holds something of: those a tool named and those the watcher
    /// saw, each once.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        let mut paths: Vec<&Path> = self
            .named
            .keys()
            .chain(self.seen.keys())
            .map(PathBuf::as_path)
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.into_iter()
    }

    /// Forgets what was noted of `path`: it is committed, or no longer changed.
    pub fn forget(&mut self, path: &Path) {
        self.named.remove(path);
        self.seen.remove(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A moment of the test's own clock: `n` milliseconds after `start`.
    fn at(start: Instant, n: u64) -> Instant {
        start + ms(n)
    }

    fn by(ledger: &Ledger, path: &str, now: Instant) -> Vec<String> {
        ledger.credit(Path::new(path), now)
    }

    #[test]
    fn a_path_nobody_named_and_the_watcher_never_saw_is_no_agents() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", start);
        assert!(by(&ledger, "x.txt", at(start, 10)).is_empty());
    }

    #[test]
    fn a_write_a_tool_named_is_credited_to_each_session_that_named_it_once_in_order() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.wrote("b", Path::new("x.txt"));
        ledger.wrote("a", Path::new("x.txt"));
        ledger.wrote("b", Path::new("x.txt"));
        assert_eq!(by(&ledger, "x.txt", start), ["b", "a"]);
        assert!(by(&ledger, "y.txt", start).is_empty());
    }

    #[test]
    fn a_change_seen_while_one_command_runs_is_credited_to_its_session() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("x.txt"), at(start, 20));
        ledger.ended("a", at(start, 30));
        assert_eq!(by(&ledger, "x.txt", at(start, 1000)), ["a"]);
    }

    #[test]
    fn a_change_seen_while_two_commands_run_is_no_agents() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.began("b", at(start, 5));
        ledger.seen(Path::new("x.txt"), at(start, 20));
        ledger.ended("a", at(start, 30));
        ledger.ended("b", at(start, 40));
        assert!(by(&ledger, "x.txt", at(start, 1000)).is_empty());
    }

    #[test]
    fn two_commands_of_one_session_at_once_are_one_session_running() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.began("a", at(start, 5));
        ledger.seen(Path::new("x.txt"), at(start, 20));
        assert_eq!(by(&ledger, "x.txt", at(start, 30)), ["a"]);
    }

    #[test]
    fn a_command_counts_from_its_beginning_to_the_settle_time_after_its_end() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 100));
        ledger.ended("a", at(start, 200));
        let end_of_it = 200 + SETTLE.as_millis() as u64;
        for (path, moment, credited) in [
            ("before.txt", 99, false),
            ("first.txt", 100, true),
            ("last.txt", end_of_it, true),
            ("after.txt", end_of_it + 1, false),
        ] {
            ledger.seen(Path::new(path), at(start, moment));
            assert_eq!(
                !by(&ledger, path, at(start, 5000)).is_empty(),
                credited,
                "{path} seen at {moment} ms"
            );
        }
    }

    #[test]
    fn a_change_seen_just_after_one_command_ended_and_before_the_next_began_is_the_first_ones() {
        // The recording's gap: one session's command ends and the other's begins 135 ms
        // later. The write the system tells of late still belongs to the first.
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.ended("a", at(start, 100));
        ledger.began("b", at(start, 235));
        ledger.seen(Path::new("x.txt"), at(start, 112));
        ledger.seen(Path::new("y.txt"), at(start, 100 + 60));
        ledger.seen(Path::new("z.txt"), at(start, 240));
        let now = at(start, 5000);
        assert_eq!(by(&ledger, "x.txt", now), ["a"]);
        assert!(by(&ledger, "y.txt", now).is_empty(), "past the settle time");
        assert_eq!(by(&ledger, "z.txt", now), ["b"]);
    }

    #[test]
    fn a_command_that_has_not_ended_runs_until_now() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("x.txt"), at(start, 1000));
        assert_eq!(by(&ledger, "x.txt", at(start, 1000)), ["a"]);
        // Judged before the change was seen, the command has not reached it yet.
        assert!(by(&ledger, "x.txt", at(start, 999)).is_empty());
    }

    #[test]
    fn a_path_written_by_commands_of_two_sessions_in_turn_is_credited_to_both() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("shared.log"), at(start, 10));
        ledger.ended("a", at(start, 20));
        ledger.began("b", at(start, 500));
        ledger.seen(Path::new("shared.log"), at(start, 510));
        ledger.ended("b", at(start, 520));
        assert_eq!(by(&ledger, "shared.log", at(start, 5000)), ["a", "b"]);
    }

    #[test]
    fn a_name_from_a_tool_settles_it_whatever_the_watcher_saw() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("b", at(start, 0));
        ledger.seen(Path::new("x.txt"), at(start, 10));
        ledger.wrote("a", Path::new("x.txt"));
        assert_eq!(by(&ledger, "x.txt", at(start, 20)), ["a"]);
    }

    #[test]
    fn an_end_nobody_saw_begin_closes_nothing_and_ends_the_oldest_open_command() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.ended("a", at(start, 0));
        ledger.began("a", at(start, 10));
        ledger.began("a", at(start, 20));
        ledger.ended("a", at(start, 30));
        // The first command ended at 30 ms; the second runs on.
        ledger.seen(Path::new("late.txt"), at(start, 5000));
        assert_eq!(by(&ledger, "late.txt", at(start, 6000)), ["a"]);
        ledger.ended("a", at(start, 40));
        ledger.seen(Path::new("later.txt"), at(start, 7000));
        assert!(by(&ledger, "later.txt", at(start, 8000)).is_empty());
    }

    #[test]
    fn forgetting_a_path_leaves_it_no_agents_again() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.wrote("a", Path::new("x.txt"));
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("y.txt"), at(start, 10));
        ledger.forget(Path::new("x.txt"));
        ledger.forget(Path::new("y.txt"));
        assert!(by(&ledger, "x.txt", at(start, 20)).is_empty());
        assert!(by(&ledger, "y.txt", at(start, 20)).is_empty());
    }
}
