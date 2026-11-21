//! The writing side: the states of a worktree, recorded without a commit.
//!
//! A state holds what git would commit if the user ran `git add -A` and committed: every
//! tracked file and every untracked file git does not ignore, as git's own filters and
//! line-ending rules make them. It is a commit, but in a state store of the app's own, a
//! bare repository in the app's folder, one for each worktree: never in the real
//! repository.
//!
//! How a state is taken, in short. The user's own git runs in the real repository, on a
//! copy of the worktree's list of files made for this one state, and with every object it
//! writes sent to the state store; the real repository's objects are read beside the
//! store's. `git status` names the paths that differ, those paths are staged into the
//! copy, the tree is written, and a commit of it with the app's own identity is made and
//! named by a ref of the store. The store then copies whatever of that state, and of the
//! commit checked out, it does not hold yet, so that every state reads with the real
//! repository deleted.
//!
//! What recording leaves alone: the user's list of files, their refs, branches, stash and
//! reflog, every working file, and the real repository's objects. Every run turns off the
//! hooks as reading does (the hooks folder, each hook the configuration defines, the
//! file-system monitor), and none depends on the user's identity.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::clock::{Clock, SystemClock};

use crate::contents::{Contents, Read, read_kept, stop_kept};
use crate::git::{Answer, Git, GitError, Place, finish, finish_with, unreadable};
use crate::index::IndexCopy;
use crate::worktree::{
    ChangedPath, CommitId, Worktree, hooks_off, inside, is_full_id, name_status, os_string,
};

// The comparison of any two versions and the reading of a file in any of them: the
// history's package's, beside the recording's.
mod export;
mod versions;
// Deleting what nothing refers to, and the room a store takes.
mod cleaning;

pub use cleaning::{Deleted, StoreSizes, sizes};

/// The old value that `git update-ref` takes for "no such ref".
const NO_REF: &str = "0000000000000000000000000000000000000000";

/// Where the refs of the states are, each named by its number.
const STATES: &str = "refs/yaar/states/";
/// Where the refs of the commits kept by [`StateStore::keep_commit`] are.
const KEPT: &str = "refs/yaar/kept/";
/// The file of the store that holds the commit the store last copied whole.
const COMPLETE_HEAD: &str = "yaar-complete-head";
/// The file of the store that holds the commit the changes of the worktree were last
/// counted from, as the Keeper noted it ([`StateStore::note_counted_from`]).
const COUNTED_FROM: &str = "yaar-counted-from";

/// How long a recording of files that are those of the newest state gives that state back
/// rather than a new one, by the store's clock: one hour. Past it, the same files are
/// recorded as a new state dated now, so that a turn that changed nothing has a state as
/// young as the turn, and the clean-up by the set time, which counts from the time a state
/// was recorded, never takes it earlier than the set time after the turn. The clean-up by
/// the set time adds this hour to the set time for that reason.
pub const SAME_STATE_FOR: Duration = Duration::from_secs(60 * 60);

/// Names one recorded state: the full id of its commit in the state store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateId(String);

impl StateId {
    /// The id with these digits.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The digits of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One recorded state, as the store lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedState {
    /// Its id.
    pub id: StateId,
    /// The full id of its git tree object: two states hold the same files when their
    /// trees are the same. A tree is not a state: it is never given where a state or a
    /// commit is asked for, as to [`StateStore::paths`].
    pub tree: String,
    /// The commit that was checked out in the worktree when it was taken; none where
    /// nothing was committed yet.
    pub on: Option<CommitId>,
}

/// What [`StateStore::record_reporting`] gives: the state, and what it left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// The state recorded, or the newest one when the files are those it holds.
    pub state: RecordedState,
    /// The folders left out of the state, relative to the folder of the worktree, ordered
    /// by path: repositories inside the worktree that git does not track and that have no
    /// commit yet.
    pub left_out: Vec<PathBuf>,
}

/// Why a state could not be recorded.
#[derive(Debug)]
pub enum RecordError {
    /// git failed, or the store could not be read or written.
    Git(GitError),
    /// The folder of the store cannot be made or written.
    Store {
        /// The folder of the store.
        folder: PathBuf,
        /// What went wrong, as a sentence.
        reason: String,
    },
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => e.fmt(f),
            Self::Store { folder, reason } => {
                write!(f, "the state store {}: {reason}", folder.display())
            }
        }
    }
}

impl std::error::Error for RecordError {}

impl From<GitError> for RecordError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// The recorded states of one worktree, in a store of their own.
///
/// It may be used from several threads at once; states of the one worktree are taken one
/// after the other, never two at a time. Reading a state needs nothing of the real
/// repository: not its objects, not its folder.
#[derive(Debug)]
pub struct StateStore {
    git: Git,
    folder: PathBuf,
    // The setting that turns off each hook the configuration defined when this was
    // opened: a hook defined in the global configuration reaches a bare repository too.
    hooks_off: Vec<OsString>,
    // Held while a state is taken or a commit is copied: one at a time.
    writing: Mutex<()>,
    // The running process that hands out file contents, once one was asked for. It has a
    // lock of its own, so that a read never waits for a recording.
    contents: Mutex<Option<Contents>>,
    // Where the time a state is recorded at is read.
    clock: Arc<dyn Clock>,
    // The guard of the clean-up: taken shared by every call that makes the store hold a
    // version and then runs git on it, for the whole call, and exclusively by
    // `StateStore::pack`, which removes objects. Taken once per call, never twice on one
    // thread: a call that calls another public call of the store calls its inner part.
    // `cleaning.rs`'s module doc says which calls take it.
    cleaning: RwLock<()>,
}

impl Drop for StateStore {
    fn drop(&mut self) {
        stop_kept(&mut self.contents);
    }
}

impl StateStore {
    /// The store in `folder`, a bare repository made there when the folder holds none.
    /// The folder lies in the app's folder, never in a worktree or a repository. The time a
    /// state is recorded at is the machine's: [`StateStore::open_with`] and
    /// [`SystemClock`].
    pub fn open(git: &Git, folder: &Path) -> Result<StateStore, RecordError> {
        StateStore::open_with(git, folder, Arc::new(SystemClock))
    }

    /// [`StateStore::open`], with the time a state is recorded at read from `clock`: each
    /// state commit made from now on carries `clock`'s time as its committer date, to the
    /// second, which [`StateStore::recorded_times`] gives back.
    pub fn open_with(
        git: &Git,
        folder: &Path,
        clock: Arc<dyn Clock>,
    ) -> Result<StateStore, RecordError> {
        if !folder.join("HEAD").exists() {
            fs::create_dir_all(folder).map_err(|e| RecordError::Store {
                folder: folder.to_owned(),
                reason: format!("it cannot be made: {e}"),
            })?;
            let mut init = git.command(Place::Folder(folder), &[]);
            init.args(["init", "--quiet", "--bare"]);
            finish(init)?.printed()?;
        }
        Ok(StateStore {
            git: git.clone(),
            folder: folder.to_owned(),
            hooks_off: hooks_off(git, Place::GitDir(folder))?,
            writing: Mutex::new(()),
            contents: Mutex::new(None),
            clock,
            cleaning: RwLock::new(()),
        })
    }

    /// Every state of the store with the time it was recorded at, the oldest first, as
    /// [`StateStore::states`] lists them: the committer date of its commit, to the second.
    /// A state recorded before slice 9 carries the machine's time of then.
    ///
    /// For the time to be the clock's, a recording sets `GIT_COMMITTER_DATE` and
    /// `GIT_AUTHOR_DATE` to `@<seconds since the epoch> +0000` of [`Clock::now`] on the
    /// `commit-tree` it runs; this reads `%(committerdate:unix)` with `git for-each-ref`.
    pub fn recorded_times(&self) -> Result<Vec<(StateId, SystemTime)>, GitError> {
        self.numbered_with_times()?
            .into_iter()
            .map(|(_, state, seconds)| {
                let seconds = seconds.ok_or_else(|| {
                    unreadable(
                        "a read of the state store",
                        format!("the state {} has no readable time", state.id),
                    )
                })?;
                Ok((state.id, UNIX_EPOCH + Duration::from_secs(seconds)))
            })
            .collect()
    }

    /// The full ids of the commits the store keeps by [`StateStore::keep_commit`] (each named
    /// by a ref `refs/yaar/kept/<id>`), ordered by id: one `git for-each-ref` of that folder
    /// of refs. What the clean-up at once may let go besides states.
    pub fn kept_commits(&self) -> Result<Vec<String>, GitError> {
        let answer = self.run(None, ["for-each-ref", "--format=%(refname)", KEPT])?;
        let command = answer.command().to_owned();
        let printed = answer.printed()?;
        let mut kept = Vec::new();
        for name in printed.split(|byte| *byte == b'\n') {
            if name.is_empty() {
                continue;
            }
            let id = std::str::from_utf8(name)
                .ok()
                .and_then(|name| name.strip_prefix(KEPT))
                .filter(|id| is_full_id(id))
                .ok_or_else(|| {
                    unreadable(
                        &command,
                        format!("the ref {:?}", String::from_utf8_lossy(name)),
                    )
                })?;
            kept.push(id.to_owned());
        }
        kept.sort();
        Ok(kept)
    }

    /// The commit the changes of the worktree were last counted from, as
    /// [`StateStore::note_counted_from`] noted it; none before the first note, or when the
    /// file holds no full id. A file of the store, `yaar-counted-from`, not a ref: the
    /// commit is noted, not copied, and nothing of the clean-up reads or removes it.
    pub fn counted_from(&self) -> std::io::Result<Option<CommitId>> {
        match fs::read_to_string(self.folder.join(COUNTED_FROM)) {
            Ok(text) => {
                let id = text.trim();
                Ok(is_full_id(id).then(|| CommitId::new(id)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Notes `commit` as the one the changes of the worktree are counted from now, read back
    /// by [`StateStore::counted_from`], also by a store opened anew after the Keeper started
    /// again. Written whole or not at all: into a file beside it, then renamed over it.
    pub fn note_counted_from(&self, commit: &CommitId) -> std::io::Result<()> {
        let file = self.folder.join(COUNTED_FROM);
        let beside = self.folder.join(format!("{COUNTED_FROM}.new"));
        fs::write(&beside, format!("{commit}\n"))?;
        fs::rename(&beside, &file)
    }

    /// The folder of the store.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Records the state of `worktree` as its files are now, and returns it. When the
    /// tree is that of the newest state and that state was recorded less than
    /// [`SAME_STATE_FOR`] ago by the store's clock, that state is returned and no new state
    /// is made, though objects of the files may have been written into the store: a state
    /// is the files, and the same files a moment later are the same state. Past
    /// [`SAME_STATE_FOR`], or with a newest state whose time cannot be read, the same tree
    /// is recorded as a new state dated now (its message names its own number, so its id
    /// differs from the older one's).
    ///
    /// A new state is named by the ref `refs/yaar/states/<n>`, `n` the highest number of
    /// the store's states plus 1, written with 10 digits. The ref is written only if no
    /// ref has that number: when another handle on the store took it between the reading
    /// of the numbers and the writing, the recording fails with git's words,
    /// [`RecordError::Git`], and replaces no state. It is not tried again.
    ///
    /// The real repository is left as it was: its list of files, refs, stash, reflog,
    /// objects and working files. No hook of the user runs and none can stop it. A
    /// repository inside the worktree is recorded as the commit it is on; one with no
    /// commit yet is left out of the state, as [`StateStore::record_reporting`] says,
    /// which gives the same state and names what it left out.
    pub fn record(&self, worktree: &Worktree) -> Result<RecordedState, RecordError> {
        Ok(self.record_reporting(worktree)?.state)
    }

    /// Copies a commit of the real repository into the store, with its tree, so that its
    /// files read with the real repository gone, as a thread on the old side of a diff
    /// needs, and names it by the ref `refs/yaar/kept/<commit id>`, so that nothing that
    /// cleans the store up takes it away while it is named. A commit the store holds
    /// already costs nothing.
    pub fn keep_commit(&self, worktree: &Worktree, commit: &CommitId) -> Result<(), RecordError> {
        let _guard = self.reading();
        self.keep_commit_held(worktree, commit)
    }

    /// [`StateStore::keep_commit`] for a caller that holds the guard of the clean-up.
    pub(crate) fn keep_commit_held(
        &self,
        worktree: &Worktree,
        commit: &CommitId,
    ) -> Result<(), RecordError> {
        if !is_full_id(commit.as_str()) {
            return Err(not_a_commit(commit.as_str()).into());
        }
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let name = format!("{KEPT}{commit}");
        if self.run(None, ["rev-parse", "-q", "--verify", &name])?.code == Some(0) {
            return Ok(());
        }
        let real_objects = worktree.common_dir().join("objects");
        let tree = self.tree_of(&real_objects, commit.as_str())?;
        self.complete(&real_objects, Some(commit.as_str()), &tree)?;
        self.run(None, ["update-ref", &name, commit.as_str()])?
            .printed()?;
        Ok(())
    }

    /// Every state of the store, the oldest first.
    pub fn states(&self) -> Result<Vec<RecordedState>, GitError> {
        Ok(self
            .numbered()?
            .into_iter()
            .map(|(_, state)| state)
            .collect())
    }

    /// Every file of a state or of a kept commit, ordered by the bytes of its path, as
    /// git orders paths. `commit` is the full id of a commit: a [`StateId`]'s, or one given
    /// to [`StateStore::keep_commit`]; never that of a git tree object.
    pub fn paths(&self, commit: &str) -> Result<Vec<PathBuf>, GitError> {
        if !is_full_id(commit) {
            return Err(not_a_commit(commit));
        }
        let _guard = self.reading();
        let printed = self
            .run(
                None,
                ["ls-tree", "-r", "-z", "--name-only", "--full-tree", commit],
            )?
            .printed()?;
        Ok(printed
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(os_string(path)))
            .collect())
    }

    /// The content of one file of a state or of a kept commit, `commit` named as for
    /// [`StateStore::paths`]; none when it has no file there, or when the store holds no
    /// such state or commit. A link is given as the path it points at, as git stores it.
    ///
    /// Answered by the one process this store keeps for contents, started at the first
    /// read and ended with the store; a process that has died is started again once. It
    /// sees what the store comes to hold after it started. A read never waits for a state
    /// being recorded.
    pub fn content(&self, commit: &str, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let (Some(path), true) = (inside(path), is_full_id(commit)) else {
            return Ok(None);
        };
        let mut name = commit.as_bytes().to_vec();
        name.push(b':');
        name.extend_from_slice(path.as_os_str().as_bytes());
        // Asked of the one process the store keeps, which holds no lock of recording.
        match read_kept(
            &self.contents,
            || Contents::start_in(self.command(None)),
            &name,
        )? {
            Read::Blob(content) => Ok(Some(content)),
            Read::NoBlob => Ok(None),
        }
    }

    /// The paths that differ between two states, or kept commits, of the store, each named
    /// by the full id of its commit as for [`StateStore::paths`], ordered by path, each
    /// with what happened to it on the way from `from` to `to`, renames paired by git's
    /// rule.
    pub fn changed_paths(&self, from: &str, to: &str) -> Result<Vec<ChangedPath>, GitError> {
        for commit in [from, to] {
            if !is_full_id(commit) {
                return Err(not_a_commit(commit));
            }
        }
        let _guard = self.reading();
        let answer = self.run(
            None,
            ["diff-tree", "-r", "-z", "-M", "--name-status", from, to],
        )?;
        let command = answer.command().to_owned();
        Ok(name_status(&command, &answer.printed()?)?
            .into_iter()
            .map(|(path, change)| ChangedPath {
                path: PathBuf::from(path),
                change,
            })
            .collect())
    }

    /// Records the state of `worktree` as [`StateStore::record`] does, and says what it
    /// left out. A repository inside the worktree that git does not track and that has no
    /// commit yet cannot be recorded: it is left out of the state, with what is inside it,
    /// and named in [`Recording::left_out`], where a recording would otherwise fail. A
    /// repository inside the worktree with a commit is recorded as that commit, as before.
    ///
    /// Files that a program creates and deletes beside the recording do not make it fail:
    /// a path that is gone by the time it is staged is left out of the state, one that
    /// appeared is in it or not, as the moment found it.
    pub fn record_reporting(&self, worktree: &Worktree) -> Result<Recording, RecordError> {
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let head = worktree.head_commit()?;
        let real_objects = worktree.common_dir().join("objects");
        let (tree, left_out) = self.tree_on_disk(worktree)?;

        let numbered = self.numbered_with_times()?;
        if let Some((_, newest, recorded)) = numbered.last()
            && newest.tree == tree
            && self.is_recent(*recorded)
        {
            return Ok(Recording {
                state: newest.clone(),
                left_out,
            });
        }
        let number = numbered.last().map_or(1, |(n, _, _)| n + 1);
        let message = format!("YAAR state {number}");
        let mut args = vec!["commit-tree", &tree];
        if let Some(head) = &head {
            args.extend(["-p", head.as_str()]);
        }
        args.extend(["-m", &message]);
        let mut command = self.command(Some(&real_objects));
        command.args(args);
        // The app's own identity, so that `user.useConfigOnly` cannot stop it, and the
        // time of the store's clock, which the clean-up counts a state's age from.
        let date = format!("@{} +0000", seconds_of(self.clock.now()));
        for who in ["AUTHOR", "COMMITTER"] {
            command
                .env(format!("GIT_{who}_NAME"), "YAAR")
                .env(format!("GIT_{who}_EMAIL"), "yaar@localhost.invalid")
                .env(format!("GIT_{who}_DATE"), &date);
        }
        let commit = line(finish(command)?.printed()?);

        self.complete(&real_objects, head.as_ref().map(CommitId::as_str), &tree)?;
        let name = format!("{STATES}{number:010}");
        // Written only where no ref has this number: all-zero is git's "nothing is there
        // yet". Another handle on the store that took the number first makes this fail
        // with git's words, and the state it wrote stays as it is.
        self.run(None, ["update-ref", &name, &commit, NO_REF])?
            .printed()?;
        Ok(Recording {
            state: RecordedState {
                id: StateId(commit),
                tree,
                on: head,
            },
            left_out,
        })
    }

    /// The tree of the files of `worktree` on disk as a recording reads them, written into
    /// this store and named by no commit and no ref, and the folders left out of it, as
    /// for [`Recording::left_out`]: the steps a recording takes up to `git write-tree`, on
    /// a copy of the worktree's list of files, the store's objects written and the real
    /// repository's only read. [`StateStore::record_reporting`] makes its state of this
    /// tree; [`StateStore::compare`] compares a version with it, recording nothing.
    pub(crate) fn tree_on_disk(
        &self,
        worktree: &Worktree,
    ) -> Result<(String, Vec<PathBuf>), RecordError> {
        let real_objects = worktree.common_dir().join("objects");
        // The copy is gone at the end of this function, whatever happens: a git that was
        // killed leaves its lock in the copy's folder, and that goes with it.
        let index = IndexCopy::of(&worktree.git_dir().join("index")).map_err(GitError::from)?;
        let hooks_off = hooks_off(&self.git, Place::Folder(worktree.root()))?;
        // Every run in the real worktree works on the copy, and writes what it makes into
        // the store: the objects of the real repository are only read.
        let in_worktree = |args: &[&str]| {
            let mut command = self.git.command(Place::Folder(worktree.root()), &hooks_off);
            command
                .env("GIT_INDEX_FILE", index.file())
                .env("GIT_OBJECT_DIRECTORY", self.folder.join("objects"))
                .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &real_objects)
                .args(args);
            command
        };
        let paths = finish(in_worktree(&[
            "-c",
            "status.showUntrackedFiles=all",
            "status",
            "--porcelain=v2",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ]))?;
        let command = paths.command().to_owned();
        let mut paths = differing_paths(&command, &paths.printed()?)?;
        let mut left_out = Vec::new();
        // A repository inside the worktree is staged as the commit it is on, which a
        // repository with no commit has none of: git would refuse the whole list.
        paths.retain(|differing| {
            if differing.folder && self.has_no_commit(worktree.root(), &differing.path) {
                left_out.push(PathBuf::from(os_string(&differing.path)));
                false
            } else {
                true
            }
        });
        left_out.sort();
        // A program that works beside the recording changes the worktree between the
        // moment git named a path and the moment it is staged: such a path is not a
        // failure, it is what the worktree looks like by then.
        let (staged, unstaged): (Vec<_>, Vec<_>) = paths
            .iter()
            .map(|differing| differing.path.as_slice())
            .partition(|path| !is_plain_folder(worktree.root(), path));
        // A path git named as a file that is a folder now: its files, if they appeared
        // before git looked, were named on their own. What the index held of it goes
        // first, so that those files do not clash with it.
        if !unstaged.is_empty() {
            finish_with(in_worktree(&FORGET), &zero_ended(&unstaged))?.printed()?;
        }
        if !staged.is_empty() {
            let stage = |paths: &[&[u8]]| {
                finish_with(
                    in_worktree(&[
                        "update-index",
                        "--add",
                        "--remove",
                        "--replace",
                        "--ignore-skip-worktree-entries",
                        "-z",
                        "--stdin",
                    ]),
                    &zero_ended(paths),
                )?
                .printed()
            };
            // The whole list at once, which is one run of git. A run that fails says
            // nothing of which path made it fail, and git stages one path after another
            // until that one, so the list is cut in halves until the failing path stands
            // alone: a few runs where one for each path would be tens of thousands. A path
            // that fails alone is either one that a program beside us changed after git
            // named it, which is left out as the worktree has it now, or one that fails
            // for itself, as a clean filter that errors does, and that is the failure. The
            // two are told apart by the path itself, looked at before the run and after
            // it: one that is a file again by the time it is looked at after was still
            // changed in between, which only a look before shows.
            let mut pending: Vec<&[&[u8]]> = vec![&staged];
            while let Some(range) = pending.pop() {
                let before = match range {
                    [path] => Some(fingerprint(worktree.root(), path)),
                    _ => None,
                };
                let Err(e) = stage(range) else { continue };
                match range {
                    [path] => {
                        let touched = before
                            .is_some_and(|before| before != fingerprint(worktree.root(), path));
                        if !touched && !changed_under_us(worktree.root(), path) {
                            return Err(e.into());
                        }
                        finish_with(in_worktree(&FORGET), &zero_ended(&[path]))?.printed()?;
                    }
                    _ => {
                        let (first, second) = range.split_at(range.len() / 2);
                        // The first half is taken first, the paths in the order git named
                        // them.
                        pending.push(second);
                        pending.push(first);
                    }
                }
            }
        }
        let tree = line(finish(in_worktree(&["write-tree"]))?.printed()?);
        Ok((tree, left_out))
    }

    /// Whether `folder`, a path below `root` that git names as a folder of its own, is a
    /// repository that has no commit to be recorded as. Anything git cannot read a commit
    /// of counts as none: a recording that failed for it would fail for as long as the
    /// folder is there.
    fn has_no_commit(&self, root: &Path, folder: &[u8]) -> bool {
        let folder = root.join(os_string(folder));
        if !folder.join(".git").exists() {
            return false;
        }
        let mut command = self.git.command(Place::Folder(&folder), &[]);
        command.args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
        !matches!(finish(command), Ok(answer) if answer.code == Some(0))
    }

    /// A run of git in the store, not yet started; with the objects of a repository
    /// beside the store's own where `alternates` names them.
    fn command(&self, alternates: Option<&Path>) -> Command {
        let mut command = self
            .git
            .command(Place::GitDir(&self.folder), &self.hooks_off);
        if let Some(objects) = alternates {
            command.env("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects);
        }
        command
    }

    /// A run of git in the store with its arguments, not yet started.
    fn command_with<I, S>(&self, alternates: Option<&Path>, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(alternates);
        command.args(args);
        command
    }

    /// One run of git in the store, to its end.
    fn run<I, S>(&self, alternates: Option<&Path>, args: I) -> Result<Answer, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        finish(self.command_with(alternates, args))
    }

    /// Every state with its number, in the order of the numbers.
    fn numbered(&self) -> Result<Vec<(u64, RecordedState)>, GitError> {
        Ok(self
            .numbered_with_times()?
            .into_iter()
            .map(|(number, state, _)| (number, state))
            .collect())
    }

    /// Every state with its number and the seconds since the epoch of its committer date,
    /// in the order of the numbers, from one run of git. The date is none where git gave
    /// nothing readable.
    fn numbered_with_times(&self) -> Result<Vec<(u64, RecordedState, Option<u64>)>, GitError> {
        let answer = self.run(
            None,
            [
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(tree)%00%(parent)%00%(committerdate:unix)",
                STATES,
            ],
        )?;
        let command = answer.command().to_owned();
        let printed = answer.printed()?;
        let mut all = Vec::new();
        for record in printed
            .split(|byte| *byte == b'\n')
            .filter(|record| !record.is_empty())
        {
            let fields: Vec<&[u8]> = record.split(|byte| *byte == 0).collect();
            let [name, id, tree, parent, time] = fields[..] else {
                return Err(unreadable(
                    &command,
                    format!("the line {:?}", String::from_utf8_lossy(record)),
                ));
            };
            let number = std::str::from_utf8(name)
                .ok()
                .and_then(|name| name.strip_prefix(STATES))
                .and_then(|number| number.parse::<u64>().ok())
                .ok_or_else(|| {
                    unreadable(
                        &command,
                        format!("the ref {:?}", String::from_utf8_lossy(name)),
                    )
                })?;
            let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
            all.push((
                number,
                RecordedState {
                    id: StateId(text(id)),
                    tree: text(tree),
                    on: (!parent.is_empty()).then(|| CommitId::new(text(parent))),
                },
                text(time).parse::<u64>().ok(),
            ));
        }
        all.sort_by_key(|(number, _, _)| *number);
        Ok(all)
    }

    /// Whether a state recorded at `recorded` (seconds since the epoch, none where it is
    /// not known) is younger than [`SAME_STATE_FOR`] by the store's clock. A time ahead of
    /// the clock is as young as can be.
    fn is_recent(&self, recorded: Option<u64>) -> bool {
        recorded.is_some_and(|recorded| {
            seconds_of(self.clock.now()).saturating_sub(recorded) < SAME_STATE_FOR.as_secs()
        })
    }

    /// The guard of the clean-up taken for reading, for the whole call of the one that
    /// holds it. A thread that failed while holding it leaves it as sound as it was: the
    /// guard protects no data.
    pub(crate) fn reading(&self) -> std::sync::RwLockReadGuard<'_, ()> {
        self.cleaning.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The id of the tree of a commit that the real repository or the store holds.
    fn tree_of(&self, real_objects: &Path, commit: &str) -> Result<String, GitError> {
        let spec = format!("{commit}^{{tree}}");
        Ok(line(
            self.run(Some(real_objects), ["rev-parse", "--verify", "-q", &spec])?
                .printed()?,
        ))
    }

    /// Makes the store hold what a state of `tree`, taken on `head`, reads: the commit
    /// checked out with its whole tree, which the store keeps as a commit with no
    /// history, and every object of `tree`.
    fn complete(
        &self,
        real_objects: &Path,
        head: Option<&str>,
        tree: &str,
    ) -> Result<(), GitError> {
        if let Some(head) = head {
            let marker = self.folder.join(COMPLETE_HEAD);
            let last = fs::read_to_string(&marker)
                .map(|last| last.trim().to_owned())
                .unwrap_or_default();
            if last != head {
                let last_is_held = !last.is_empty()
                    && self
                        .run(None, ["cat-file", "-e", &format!("{last}^{{tree}}")])?
                        .code
                        == Some(0);
                if last_is_held {
                    // What the last head held, the store holds: only what differs is new.
                    let mut ids = vec![head.to_owned(), self.tree_of(real_objects, head)?];
                    ids.extend(self.new_side(real_objects, &last, head)?);
                    self.pack_missing(real_objects, ids)?;
                } else {
                    self.pack_missing(real_objects, vec![head.to_owned()])?;
                    // git walks the whole tree itself and reuses the real pack's data.
                    let tree_of_head = self.tree_of(real_objects, head)?;
                    let mut input = tree_of_head.into_bytes();
                    input.push(b'\n');
                    finish_with(
                        self.command_with(
                            Some(real_objects),
                            ["pack-objects", "--revs", "-q", &self.pack_prefix()],
                        ),
                        &input,
                    )?
                    .printed()?;
                }
                // The commit has no parents here, and git has to be told so.
                self.note_shallow(head)?;
                fs::write(&marker, format!("{head}\n"))?;
            }
            let mut ids = vec![tree.to_owned()];
            ids.extend(self.new_side(real_objects, head, tree)?);
            self.pack_missing(real_objects, ids)
        } else {
            let listed = self
                .run(
                    Some(real_objects),
                    ["rev-list", "--objects", "--no-object-names", tree],
                )?
                .printed()?;
            let ids = listed
                .split(|byte| *byte == b'\n')
                .filter(|id| !id.is_empty())
                .map(|id| String::from_utf8_lossy(id).into_owned())
                .collect();
            self.pack_missing(real_objects, ids)
        }
    }

    /// The objects `to` has that `from` has not, as git's tree diff lists them: every
    /// tree and blob on the new side of a path that differs, a repository inside the
    /// worktree left out.
    fn new_side(&self, real_objects: &Path, from: &str, to: &str) -> Result<Vec<String>, GitError> {
        let printed = self
            .run(
                Some(real_objects),
                [
                    "diff-tree",
                    "-r",
                    "-t",
                    "--no-renames",
                    "--no-abbrev",
                    from,
                    to,
                ],
            )?
            .printed()?;
        Ok(printed
            .split(|byte| *byte == b'\n')
            .filter_map(|record| {
                // `:<mode> <mode> <id> <id> <status>`, a tab, the path.
                let meta = record.split(|byte| *byte == b'\t').next()?;
                let meta = std::str::from_utf8(meta).ok()?.strip_prefix(':')?;
                let mut fields = meta.split(' ');
                let (_, mode, _, id) = (
                    fields.next()?,
                    fields.next()?,
                    fields.next()?,
                    fields.next()?,
                );
                (mode != "160000" && id.bytes().any(|digit| digit != b'0')).then(|| id.to_owned())
            })
            .collect())
    }

    /// Packs into the store each of these objects that it does not hold itself, reading
    /// them from the real repository.
    fn pack_missing(&self, real_objects: &Path, mut ids: Vec<String>) -> Result<(), GitError> {
        ids.sort();
        ids.dedup();
        let asked = ids.join("\n") + "\n";
        // The store alone is asked: an object it has on loan from the real repository
        // would vanish with it.
        let answer = finish_with(
            self.command_with(None, ["cat-file", "--batch-check"]),
            asked.as_bytes(),
        )?;
        let command = answer.command().to_owned();
        let mut missing = Vec::new();
        for record in answer.printed()?.split(|byte| *byte == b'\n') {
            if let Some(id) = record.strip_suffix(b" missing") {
                missing.extend_from_slice(id);
                missing.push(b'\n');
            } else if !record.is_empty() && !record.contains(&b' ') {
                return Err(unreadable(
                    &command,
                    format!("the answer {:?}", String::from_utf8_lossy(record)),
                ));
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        finish_with(
            self.command_with(
                Some(real_objects),
                ["pack-objects", "-q", &self.pack_prefix()],
            ),
            &missing,
        )?
        .printed()?;
        Ok(())
    }

    /// Where git names the packs it writes into the store.
    fn pack_prefix(&self) -> String {
        self.folder
            .join("objects/pack/pack")
            .to_string_lossy()
            .into_owned()
    }

    /// Tells the store that this commit stands without its history.
    fn note_shallow(&self, commit: &str) -> Result<(), GitError> {
        let file = self.folder.join("shallow");
        let known = fs::read(&file).unwrap_or_default();
        if known
            .split(|byte| *byte == b'\n')
            .any(|known| known == commit.as_bytes())
        {
            return Ok(());
        }
        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(file)?;
        file.write_all(format!("{commit}\n").as_bytes())?;
        Ok(())
    }
}

/// One path `git status` named that git has to be told of.
struct Differing {
    /// The path, relative to the worktree, without a slash at its end.
    path: Vec<u8>,
    /// Whether git named it with a slash: a repository inside the worktree.
    folder: bool,
}

/// The paths of the records of `git status --porcelain=v2 -z --no-renames` that git has
/// to be told of for its list of files to hold what `git add --all` would make it hold:
/// those the worktree's side differs in, and the untracked ones. A path that is staged
/// and the same on disk is in the list already.
fn differing_paths(command: &str, printed: &[u8]) -> Result<Vec<Differing>, GitError> {
    let mut paths = Vec::new();
    let mut records = printed.split(|byte| *byte == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        // git help status, "Porcelain Format Version 2": how many fields come before the
        // path, whether the record is one the worktree's side always differs in, and
        // whether the path it was renamed from follows. A path may hold a space, so it is
        // all that is left.
        let (before, always, original) = match record[0] {
            b'1' => (8, false, false),
            b'2' => (9, false, true),
            b'u' => (10, true, false),
            b'?' => (1, true, false),
            // A header, such as the stash count a user's configuration asks git to print:
            // it names no file.
            b'#' => continue,
            _ => (0, false, false),
        };
        let fields: Vec<&[u8]> = record.splitn(before + 1, |byte| *byte == b' ').collect();
        if before == 0 || fields.len() != before + 1 {
            return Err(unreadable(
                command,
                format!("the record {:?}", String::from_utf8_lossy(record)),
            ));
        }
        if original {
            records.next();
        }
        // `XY`: the staged side, then the worktree's, which is a dot where it is the same.
        if always || fields[1].get(1) != Some(&b'.') {
            let path = fields[before];
            // A repository inside the worktree is listed as its folder, with a slash.
            paths.push(match path.strip_suffix(b"/") {
                Some(folder) => Differing {
                    path: folder.to_vec(),
                    folder: true,
                },
                None => Differing {
                    path: path.to_vec(),
                    folder: false,
                },
            });
        }
    }
    Ok(paths)
}

/// The arguments of the run that takes paths out of a list of files whatever is on disk.
const FORGET: [&str; 4] = ["update-index", "--force-remove", "-z", "--stdin"];

/// `paths`, each followed by a zero byte, as `git update-index -z --stdin` reads them.
fn zero_ended(paths: &[&[u8]]) -> Vec<u8> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path);
        input.push(0);
    }
    input
}

/// Whether `path`, below `root`, is a folder now that is no repository: git cannot be
/// told to add one, only the files in it.
fn is_plain_folder(root: &Path, path: &[u8]) -> bool {
    let place = root.join(os_string(path));
    fs::symlink_metadata(&place).is_ok_and(|found| found.is_dir()) && !place.join(".git").exists()
}

/// What tells one incarnation of the file at `path`, below `root`, from another: its kind,
/// its inode, its size and its times, or nothing where there is no file. A path that was
/// deleted and made anew has another one.
fn fingerprint(root: &Path, path: &[u8]) -> Option<(u32, u64, u64, i64, i64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    let found = fs::symlink_metadata(root.join(os_string(path))).ok()?;
    Some((
        found.mode(),
        found.ino(),
        found.size(),
        found.mtime(),
        found.mtime_nsec(),
        found.ctime(),
        found.ctime_nsec(),
    ))
}

/// Whether `path`, below `root`, is gone or is a folder now that is no repository: what a
/// path that failed to stage looks like when a program beside the recording changed it
/// after git named it.
fn changed_under_us(root: &Path, path: &[u8]) -> bool {
    let place = root.join(os_string(path));
    match fs::symlink_metadata(&place) {
        // A folder above it that is a file now leaves no such path either.
        Err(e) => matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        ),
        Ok(found) => found.is_dir() && !place.join(".git").exists(),
    }
}

/// The whole seconds `time` lies after the epoch; none before it.
fn seconds_of(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |after| after.as_secs())
}

/// The first line git printed, without its line end.
fn line(printed: Vec<u8>) -> String {
    String::from_utf8_lossy(&printed).trim_end().to_owned()
}

/// The refusal of something that is not the full id of a commit.
fn not_a_commit(given: &str) -> GitError {
    unreadable(
        "a read of the state store",
        format!("{given:?} is not the full id of a commit"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_git_names_with_a_slash_is_a_folder_and_loses_its_slash() {
        let printed = b"? nested/\0? plain.txt\0? with space/\0";

        let paths = differing_paths("git status", printed).expect("read");

        let seen: Vec<(&[u8], bool)> = paths
            .iter()
            .map(|differing| (differing.path.as_slice(), differing.folder))
            .collect();
        assert_eq!(
            seen,
            vec![
                (&b"nested"[..], true),
                (&b"plain.txt"[..], false),
                (&b"with space"[..], true)
            ]
        );
    }

    #[test]
    fn a_staged_path_that_is_the_same_on_disk_is_not_named_again() {
        // `1 XY sub mH mI mW hH hI path`: the worktree's side is a dot where it is the same.
        let printed = b"1 M. N... 100644 100644 100644 aa bb staged.txt\0\
                        1 .M N... 100644 100644 100644 aa bb edited.txt\0\
                        # stash 1\0";

        let paths = differing_paths("git status", printed).expect("read");

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, b"edited.txt");
    }
}
