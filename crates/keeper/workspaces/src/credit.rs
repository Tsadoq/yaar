//! Who is credited with a changed path: the rule, with no file, no watcher and no clock of
//! its own. Every moment is given by the caller.

use std::collections::{BTreeMap, BTreeSet};
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

/// How long a moment seen is kept to be judged again. The report that a command began
/// reaches the Keeper after the command's first writes can, since the hook that carries it
/// is a process of its own; a moment older than this has every command that could hold it
/// reported, and is judged for good.
const GRACE: Duration = Duration::from_secs(5);

/// How many things are noted between two times the ledger lets go of what it has judged
/// for good, so that a build that writes thousands of files costs each of them little.
const NOTES_PER_COMPACTION: usize = 1024;

/// What the ledger holds of one path, or of one folder that appeared.
#[derive(Debug)]
struct Noted {
    // The sessions that wrote it with their own tool, in the order they did.
    named: Vec<String>,
    // The sessions credited with the moments already judged for good, in order.
    judged: Vec<String>,
    // The moments the watcher saw it change, not yet judged for good.
    pending: Vec<Instant>,
    // When anything was last noted of it.
    last: Instant,
}

impl Noted {
    fn new(at: Instant) -> Noted {
        Noted {
            named: Vec::new(),
            judged: Vec::new(),
            pending: Vec::new(),
            last: at,
        }
    }

    fn touch(&mut self, at: Instant) {
        self.last = self.last.max(at);
    }
}

/// The ledger of one worktree: the writes the agents' tools named, the shell commands of
/// each session with when they began and ended, and when each path was seen changing.
#[derive(Debug, Default)]
pub struct Ledger {
    // What is held of each file.
    files: BTreeMap<PathBuf, Noted>,
    // What is held of each folder that appeared, as a new one or moved in: the system does
    // not always tell of what is written in it at once, or at all if it was moved in whole.
    folders: BTreeMap<PathBuf, Noted>,
    // The shell commands of each session, the oldest first.
    commands: BTreeMap<String, Vec<Window>>,
    // How many things were noted since the ledger last let go of what it had judged.
    noted: usize,
}

impl Ledger {
    /// A ledger that has noted nothing.
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// `session` wrote `path`, relative to the worktree, with its own tool, at `at`.
    pub fn wrote(&mut self, session: &str, path: &Path, at: Instant) {
        let noted = self
            .files
            .entry(path.to_owned())
            .or_insert_with(|| Noted::new(at));
        noted.touch(at);
        if !noted.named.iter().any(|named| named == session) {
            noted.named.push(session.to_owned());
        }
        self.tick(at);
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
        let noted = self
            .files
            .entry(path.to_owned())
            .or_insert_with(|| Noted::new(at));
        noted.touch(at);
        noted.pending.push(at);
        self.tick(at);
    }

    /// The watcher saw the folder `path`, relative to the worktree, appear at `at`, made
    /// there or moved in. A file below it that the watcher never saw change is credited as
    /// the folder is: the system may not have been watching the folder yet when the file
    /// was written, or the file came with the folder.
    pub fn appeared(&mut self, path: &Path, at: Instant) {
        let noted = self
            .folders
            .entry(path.to_owned())
            .or_insert_with(|| Noted::new(at));
        noted.touch(at);
        noted.pending.push(at);
        self.tick(at);
    }

    /// Who is credited with the change of `path`, judged at `now`: the sessions whose own
    /// tool named it; else, for each time the watcher saw it change, the one session whose
    /// shell command ran then, the command counted from its beginning to [`SETTLE`] after
    /// its end, and none when no command or more than one ran then. A command that has
    /// not ended runs until `now`. A path the watcher never saw change is judged by the
    /// folders it lies in that the watcher saw appear, the same way. Empty: no agent made
    /// it.
    pub fn credit(&self, path: &Path, now: Instant) -> Vec<String> {
        if let Some(noted) = self.files.get(path) {
            return if noted.named.is_empty() {
                self.judge(noted, now)
            } else {
                noted.named.clone()
            };
        }
        let mut credited = Vec::new();
        for folder in path.ancestors().skip(1) {
            if let Some(noted) = self.folders.get(folder) {
                join(&mut credited, self.judge(noted, now));
            }
        }
        credited
    }

    /// The sessions the moments of `noted` credit, those judged for good first.
    fn judge(&self, noted: &Noted, now: Instant) -> Vec<String> {
        let mut credited = noted.judged.clone();
        for &seen in &noted.pending {
            if let Some(session) = self.sole_session(seen, now) {
                join(&mut credited, [session.to_owned()]);
            }
        }
        credited
    }

    /// The one session whose command runs at `seen`, judged at `now`; none when no
    /// command or more than one session's runs then.
    fn sole_session(&self, seen: Instant, now: Instant) -> Option<&str> {
        let mut running = self
            .commands
            .iter()
            .filter(|(_, windows)| windows.iter().any(|window| window.holds(seen, now)));
        let (session, _) = running.next()?;
        running.next().is_none().then_some(session.as_str())
    }

    /// The paths the ledger holds something of: those a tool named and those the watcher
    /// saw, each once.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    /// Forgets what was noted of `path`: it is committed, or no longer changed.
    pub fn forget(&mut self, path: &Path) {
        self.files.remove(path);
        self.folders.remove(path);
    }

    /// Forgets what was noted of what is no longer changed: every path not in `changed`,
    /// and every folder that holds none of them, unless it was noted at or after `asked`.
    /// A change is listed from what git says at the moment it was asked, and a write made
    /// since is not in that list yet; forgetting it would make it a change no agent made.
    pub fn forget_unchanged(&mut self, changed: &BTreeSet<&Path>, asked: Instant) {
        let holders: BTreeSet<&Path> = changed
            .iter()
            .flat_map(|path| path.ancestors().skip(1))
            .collect();
        self.files
            .retain(|path, noted| changed.contains(path.as_path()) || noted.last >= asked);
        self.folders
            .retain(|path, noted| holders.contains(path.as_path()) || noted.last >= asked);
    }

    /// Lets go of what is judged for good as of `now`. A moment seen more than a grace
    /// time before is credited to the session whose command held it, and kept as that;
    /// a path with nothing credited and nothing to judge is the same as a path never
    /// seen, and goes; so does a command that ended too long ago to hold any moment still
    /// to be judged. What [`Ledger::credit`] says does not change.
    pub fn compact(&mut self, now: Instant) {
        self.noted = 0;
        let Some(cutoff) = now.checked_sub(GRACE) else {
            return;
        };
        let mut files = std::mem::take(&mut self.files);
        let mut folders = std::mem::take(&mut self.folders);
        for noted in files.values_mut().chain(folders.values_mut()) {
            let (settled, pending): (Vec<Instant>, Vec<Instant>) = noted
                .pending
                .iter()
                .copied()
                .partition(|&seen| seen <= cutoff);
            let sessions: Vec<String> = settled
                .into_iter()
                .filter_map(|seen| self.sole_session(seen, now).map(str::to_owned))
                .collect();
            join(&mut noted.judged, sessions);
            noted.pending = pending;
        }
        let empty = |noted: &Noted| {
            noted.named.is_empty() && noted.judged.is_empty() && noted.pending.is_empty()
        };
        folders.retain(|_, noted| !empty(noted));
        // A file under a folder that is kept stays, whatever it holds: without its own
        // entry it would be judged by the folder.
        files.retain(|path, noted| {
            !empty(noted) || path.ancestors().skip(1).any(|up| folders.contains_key(up))
        });
        self.files = files;
        self.folders = folders;
        let last_to_matter = cutoff.checked_sub(SETTLE);
        for windows in self.commands.values_mut() {
            windows.retain(|window| match (window.ended, last_to_matter) {
                (Some(ended), Some(last)) => ended >= last,
                _ => true,
            });
        }
        self.commands.retain(|_, windows| !windows.is_empty());
    }

    /// Counts a thing noted at `at`, and lets go of what is judged once enough were.
    fn tick(&mut self, at: Instant) {
        self.noted += 1;
        if self.noted >= NOTES_PER_COMPACTION {
            self.compact(at);
        }
    }
}

/// Appends to `credited` each session of `more` it does not hold yet.
fn join(credited: &mut Vec<String>, more: impl IntoIterator<Item = String>) {
    for session in more {
        if !credited.contains(&session) {
            credited.push(session);
        }
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
        ledger.wrote("b", Path::new("x.txt"), start);
        ledger.wrote("a", Path::new("x.txt"), start);
        ledger.wrote("b", Path::new("x.txt"), start);
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
        ledger.wrote("a", Path::new("x.txt"), at(start, 15));
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
        ledger.wrote("a", Path::new("x.txt"), start);
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("y.txt"), at(start, 10));
        ledger.forget(Path::new("x.txt"));
        ledger.forget(Path::new("y.txt"));
        assert!(by(&ledger, "x.txt", at(start, 20)).is_empty());
        assert!(by(&ledger, "y.txt", at(start, 20)).is_empty());
    }

    fn paths_of(ledger: &Ledger) -> Vec<String> {
        ledger
            .paths()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    fn changed<'a>(paths: &[&'a str]) -> BTreeSet<&'a Path> {
        paths.iter().map(|path| Path::new(*path)).collect()
    }

    #[test]
    fn what_was_noted_after_the_changes_were_asked_for_is_not_forgotten_with_what_is_not_listed() {
        // `git status` runs for a while, and a write lands and is noted before the list
        // reaches the ledger: it is not in the list, and is still no stale credit.
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("early.txt"), at(start, 10));
        ledger.seen(Path::new("late.txt"), at(start, 30));
        ledger.wrote("a", Path::new("named-late.txt"), at(start, 31));
        ledger.wrote("a", Path::new("named-early.txt"), at(start, 11));
        let asked = at(start, 20);
        ledger.forget_unchanged(&changed(&["listed.txt"]), asked);
        let now = at(start, 1000);
        assert_eq!(by(&ledger, "late.txt", now), ["a"]);
        assert_eq!(by(&ledger, "named-late.txt", now), ["a"]);
        assert!(by(&ledger, "early.txt", now).is_empty(), "forgotten");
        assert!(by(&ledger, "named-early.txt", now).is_empty(), "forgotten");
        assert_eq!(paths_of(&ledger), ["late.txt", "named-late.txt"]);
    }

    #[test]
    fn a_path_listed_as_changed_is_kept_whenever_it_was_noted() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("listed.txt"), at(start, 10));
        ledger.forget_unchanged(&changed(&["listed.txt"]), at(start, 20));
        assert_eq!(by(&ledger, "listed.txt", at(start, 1000)), ["a"]);
    }

    #[test]
    fn a_folder_that_holds_a_listed_change_is_kept_and_another_is_not() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.appeared(Path::new("out"), at(start, 10));
        ledger.appeared(Path::new("gone"), at(start, 11));
        ledger.forget_unchanged(&changed(&["out/deep/f.txt"]), at(start, 20));
        let now = at(start, 1000);
        assert_eq!(by(&ledger, "out/deep/f.txt", now), ["a"]);
        assert!(by(&ledger, "gone/f.txt", now).is_empty());
    }

    #[test]
    fn a_file_the_watcher_never_saw_in_a_folder_that_appeared_is_credited_as_the_folder_is() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.appeared(Path::new("out"), at(start, 10));
        ledger.ended("a", at(start, 30));
        let now = at(start, 5000);
        assert_eq!(by(&ledger, "out/f.txt", now), ["a"]);
        assert_eq!(by(&ledger, "out/deep/f.txt", now), ["a"]);
        assert!(by(&ledger, "other/f.txt", now).is_empty());
        assert!(by(&ledger, "outer/f.txt", now).is_empty());
    }

    #[test]
    fn a_file_the_watcher_saw_in_a_folder_that_appeared_is_judged_by_its_own_moments() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.appeared(Path::new("out"), at(start, 10));
        ledger.ended("a", at(start, 30));
        // Edited by hand long after the command ended.
        ledger.seen(Path::new("out/by-hand.txt"), at(start, 4000));
        assert!(by(&ledger, "out/by-hand.txt", at(start, 5000)).is_empty());
    }

    #[test]
    fn a_folder_that_appeared_while_two_commands_ran_credits_none_of_its_files() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.began("b", at(start, 5));
        ledger.appeared(Path::new("out"), at(start, 10));
        assert!(by(&ledger, "out/f.txt", at(start, 20)).is_empty());
    }

    /// Everything `credit` says of the paths of `ledger`, to compare before and after.
    fn credits_of(ledger: &Ledger, paths: &[&str], now: Instant) -> Vec<Vec<String>> {
        paths.iter().map(|path| by(ledger, path, now)).collect()
    }

    #[test]
    fn letting_go_of_what_is_judged_changes_nothing_credit_says() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("a.txt"), at(start, 10));
        ledger.appeared(Path::new("out"), at(start, 12));
        ledger.ended("a", at(start, 100));
        ledger.began("b", at(start, 200));
        ledger.began("a", at(start, 205));
        ledger.seen(Path::new("both.txt"), at(start, 210));
        ledger.seen(Path::new("b-only.txt"), at(start, 202));
        ledger.ended("a", at(start, 300));
        ledger.ended("b", at(start, 310));
        ledger.seen(Path::new("by-hand.txt"), at(start, 20_000));
        ledger.wrote("b", Path::new("named.txt"), at(start, 20));
        let paths = [
            "a.txt",
            "out/f.txt",
            "both.txt",
            "b-only.txt",
            "by-hand.txt",
            "named.txt",
            "never.txt",
        ];
        let now = at(start, 30_000);
        let before = credits_of(&ledger, &paths, now);
        assert_eq!(before[0], ["a"]);
        assert_eq!(before[1], ["a"]);
        assert!(before[2].is_empty(), "both.txt: two sessions ran");
        assert_eq!(before[3], ["b"]);
        ledger.compact(now);
        assert_eq!(credits_of(&ledger, &paths, now), before);
        // And a moment seen since is judged as it would have been: the commands that
        // ended long ago hold nothing new.
        ledger.seen(Path::new("a.txt"), at(start, 30_500));
        assert_eq!(by(&ledger, "a.txt", at(start, 31_000)), ["a"]);
        ledger.seen(Path::new("later.txt"), at(start, 30_600));
        assert!(by(&ledger, "later.txt", at(start, 31_000)).is_empty());
    }

    #[test]
    fn letting_go_drops_the_paths_with_nothing_credited_and_keeps_the_rest() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("credited.txt"), at(start, 10));
        ledger.ended("a", at(start, 20));
        ledger.seen(Path::new("by-hand.txt"), at(start, 1000));
        ledger.seen(Path::new("fresh.txt"), at(start, 9000));
        ledger.wrote("a", Path::new("named.txt"), at(start, 30));
        ledger.compact(at(start, 10_000));
        // `fresh.txt` is younger than the grace time and is still to be judged.
        assert_eq!(
            paths_of(&ledger),
            ["credited.txt", "fresh.txt", "named.txt"]
        );
    }

    #[test]
    fn a_ledger_that_is_never_asked_does_not_keep_every_path_it_saw() {
        // A build writes thousands of files nobody asks about.
        let start = Instant::now();
        let mut ledger = Ledger::new();
        for n in 0..NOTES_PER_COMPACTION - 1 {
            ledger.seen(Path::new(&format!("target/{n}.o")), at(start, n as u64));
        }
        assert_eq!(ledger.paths().count(), NOTES_PER_COMPACTION - 1);
        ledger.seen(Path::new("late.txt"), at(start, 60_000));
        assert_eq!(paths_of(&ledger), ["late.txt"]);
    }

    #[test]
    fn a_file_without_credit_under_a_folder_that_is_kept_stays_judged_by_its_own_moments() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.appeared(Path::new("out"), at(start, 10));
        ledger.ended("a", at(start, 20));
        ledger.seen(Path::new("out/by-hand.txt"), at(start, 9000));
        ledger.compact(at(start, 20_000));
        assert!(by(&ledger, "out/by-hand.txt", at(start, 21_000)).is_empty());
        assert_eq!(by(&ledger, "out/never-seen.txt", at(start, 21_000)), ["a"]);
    }

    #[test]
    fn a_moment_younger_than_the_grace_time_is_judged_by_a_command_reported_since() {
        // The report of a command's beginning can reach the ledger after the write it
        // made: the moment is still waiting to be judged.
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.seen(Path::new("x.txt"), at(start, 1000));
        ledger.compact(at(start, 3000));
        ledger.began("a", at(start, 900));
        assert_eq!(by(&ledger, "x.txt", at(start, 4000)), ["a"]);
    }

    #[test]
    fn a_command_that_ended_still_holds_a_moment_not_yet_judged_when_the_ledger_lets_go() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.seen(Path::new("x.txt"), at(start, 10));
        ledger.ended("a", at(start, 20));
        ledger.compact(at(start, 1000));
        assert_eq!(by(&ledger, "x.txt", at(start, 1000)), ["a"]);
    }

    #[test]
    fn a_command_that_ended_long_ago_is_not_kept() {
        let start = Instant::now();
        let mut ledger = Ledger::new();
        ledger.began("a", at(start, 0));
        ledger.ended("a", at(start, 20));
        ledger.began("b", at(start, 30));
        ledger.ended("b", at(start, 40));
        ledger.began("c", at(start, 50));
        let held = |ledger: &Ledger| format!("{ledger:?}").matches("Window").count();
        assert_eq!(held(&ledger), 3);
        ledger.compact(at(start, 4000));
        assert_eq!(
            held(&ledger),
            3,
            "inside the grace time, and one has not ended"
        );
        ledger.compact(at(start, 60_000));
        assert_eq!(held(&ledger), 1, "only the command that runs on");
    }
}
