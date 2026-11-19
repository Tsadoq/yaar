//! Who is credited with a changed path: the rule, with no file, no watcher and no clock of
//! its own. Every moment is given by the caller.

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

/// The ledger of one worktree: the writes the agents' tools named, the shell commands of
/// each session with when they began and ended, and when each path was seen changing.
#[derive(Debug, Default)]
pub struct Ledger {
    _todo: (),
}

impl Ledger {
    /// A ledger that has noted nothing.
    pub fn new() -> Ledger {
        todo!("an empty ledger")
    }

    /// `session` wrote `path`, relative to the worktree, with its own tool.
    pub fn wrote(&mut self, session: &str, path: &Path) {
        let _ = (session, path);
        todo!("note a named write")
    }

    /// A shell command of `session` began at `at`.
    pub fn began(&mut self, session: &str, at: Instant) {
        let _ = (session, at);
        todo!("note a command that began")
    }

    /// The shell command of `session` ended at `at`.
    pub fn ended(&mut self, session: &str, at: Instant) {
        let _ = (session, at);
        todo!("note a command that ended")
    }

    /// The watcher saw `path`, relative to the worktree, change at `at`.
    pub fn seen(&mut self, path: &Path, at: Instant) {
        let _ = (path, at);
        todo!("note a change the watcher saw")
    }

    /// Who is credited with the change of `path`, judged at `now`: the sessions whose own
    /// tool named it; else, for each time the watcher saw it change, the one session whose
    /// shell command ran then, the command counted from its beginning to [`SETTLE`] after
    /// its end, and none when no command or more than one ran then. A command that has
    /// not ended runs until `now`. Empty: no agent made it.
    pub fn credit(&self, path: &Path, now: Instant) -> Vec<String> {
        let _ = (path, now);
        todo!("who made the change of one path")
    }

    /// Forgets what was noted of `path`: it is committed, or no longer changed.
    pub fn forget(&mut self, path: &Path) {
        let _ = path;
        todo!("forget a path")
    }
}
