//! The writing side: states of a worktree, recorded without a commit, in a bare state store of the
//! app's own. Git runs in the real repository on a copy of its index with its objects sent to the
//! store; the user's index, refs, stash, reflog and files are left alone.

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

use crate::contents::{Contents, Read, asked, read_kept, stop_kept};
use crate::git::{Answer, Git, GitError, Place, finish, finish_with, unreadable};
use crate::index::IndexCopy;
use crate::worktree::{
    ChangedPath, CommitId, Worktree, hooks_off, inside, is_full_id, name_status, os_string,
};

mod cleaning;
mod export;
mod versions;

pub use cleaning::{Deleted, StoreSizes, sizes};

const NO_REF: &str = "0000000000000000000000000000000000000000";

const STATES: &str = "refs/yaar/states/";
const KEPT: &str = "refs/yaar/kept/";
const COMPLETE_HEAD: &str = "yaar-complete-head";
const COUNTED_FROM: &str = "yaar-counted-from";

/// Within this long, by the store's clock, recording the files of the newest state gives it back;
/// past it the same files become a new state dated now. The clean-up by set time adds this hour.
pub const SAME_STATE_FOR: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateId(String);

impl StateId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedState {
    pub id: StateId,
    pub tree: String,
    pub on: Option<CommitId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    pub state: RecordedState,
    /// Relative to the worktree, ordered by path: repositories inside it that git does not track
    /// and that have no commit yet.
    pub left_out: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum RecordError {
    Git(GitError),
    Store { folder: PathBuf, reason: String },
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

#[derive(Debug)]
pub struct StateStore {
    git: Git,
    folder: PathBuf,
    // A hook defined in the global configuration reaches a bare repository too.
    hooks_off: Vec<OsString>,
    writing: Mutex<()>,
    contents: Mutex<Option<Contents>>,
    clock: Arc<dyn Clock>,
    // Shared by every call that makes the store hold a version and then runs git on it; exclusive
    // for `StateStore::pack`. Taken once per call: a call that needs another public call uses its
    // inner part, as two takes on one thread can deadlock.
    cleaning: RwLock<()>,
}

impl Drop for StateStore {
    fn drop(&mut self) {
        stop_kept(&mut self.contents);
    }
}

impl StateStore {
    pub fn open(git: &Git, folder: &Path) -> Result<StateStore, RecordError> {
        StateStore::open_with(git, folder, Arc::new(SystemClock))
    }

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

    /// None before the first note, or when the file holds no full id. A file of the store, not a
    /// ref: the clean-up never reads or removes it.
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

    pub fn note_counted_from(&self, commit: &CommitId) -> std::io::Result<()> {
        let file = self.folder.join(COUNTED_FROM);
        let beside = self.folder.join(format!("{COUNTED_FROM}.new"));
        fs::write(&beside, format!("{commit}\n"))?;
        fs::rename(&beside, &file)
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The newest state is returned instead when its tree is the same and it was recorded less than
    /// [`SAME_STATE_FOR`] ago. A new state is `refs/yaar/states/<n>`, written only if no ref has
    /// that number: if another handle took it, the recording fails.
    pub fn record(&self, worktree: &Worktree) -> Result<RecordedState, RecordError> {
        Ok(self.record_reporting(worktree)?.state)
    }

    /// Copies a commit of the real repository into the store with its tree, named
    /// `refs/yaar/kept/<commit id>` so that the clean-up does not take it while named.
    pub fn keep_commit(&self, worktree: &Worktree, commit: &CommitId) -> Result<(), RecordError> {
        let _guard = self.reading();
        self.keep_commit_held(worktree, commit)
    }

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

    pub fn states(&self) -> Result<Vec<RecordedState>, GitError> {
        Ok(self
            .numbered()?
            .into_iter()
            .map(|(_, state)| state)
            .collect())
    }

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

    /// None when there is no file there or the store holds no such commit. A link is given as the
    /// path it points at.
    pub fn content(&self, commit: &str, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let (Some(path), true) = (inside(path), is_full_id(commit)) else {
            return Ok(None);
        };
        let mut name = commit.as_bytes().to_vec();
        name.push(b':');
        name.extend_from_slice(path.as_os_str().as_bytes());
        let has_newer_forms = self.git.has_newer_forms();
        let Some(name) = asked(has_newer_forms, name, |args| self.run(None, args))? else {
            return Ok(None);
        };
        match read_kept(
            &self.contents,
            || Contents::start_in(self.command(None), has_newer_forms),
            &name,
        )? {
            Read::Blob(content) => Ok(Some(content)),
            Read::NoBlob => Ok(None),
        }
    }

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
        // The app's own identity, so that `user.useConfigOnly` cannot stop it, and the store's
        // clock, which the clean-up counts a state's age from.
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
        // All-zero is git's "nothing is there yet": a handle that took the number first makes this
        // fail, and the state it wrote stays.
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

    /// The tree of the files of `worktree` on disk as a recording reads them, written into this
    /// store, and the folders left out: a recording's steps up to `git write-tree`. Records
    /// nothing.
    pub(crate) fn tree_on_disk(
        &self,
        worktree: &Worktree,
    ) -> Result<(String, Vec<PathBuf>), RecordError> {
        let real_objects = worktree.common_dir().join("objects");
        let index = IndexCopy::of(&worktree.git_dir().join("index")).map_err(GitError::from)?;
        let hooks_off = hooks_off(&self.git, Place::Folder(worktree.root()))?;
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
        paths.retain(|differing| {
            if differing.folder && self.has_no_commit(worktree.root(), &differing.path) {
                left_out.push(PathBuf::from(os_string(&differing.path)));
                false
            } else {
                true
            }
        });
        left_out.sort();
        // A program beside the recording changes the worktree between git naming a path and its
        // staging: such a path is left out, not a failure.
        let (staged, unstaged): (Vec<_>, Vec<_>) = paths
            .iter()
            .map(|differing| differing.path.as_slice())
            .partition(|path| !is_plain_folder(worktree.root(), path));
        // A path git named as a file that is a folder now leaves the index before the staging,
        // so that the files inside it, named on their own, do not clash with it.
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
            // The whole list in one run of git. A failing run does not say which path failed, so
            // the list is cut in halves until the failing path stands alone. A path that changed
            // under us is left out; any other is the failure.
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
                        pending.push(second);
                        pending.push(first);
                    }
                }
            }
        }
        let tree = line(finish(in_worktree(&["write-tree"]))?.printed()?);
        Ok((tree, left_out))
    }

    fn has_no_commit(&self, root: &Path, folder: &[u8]) -> bool {
        let folder = root.join(os_string(folder));
        if !folder.join(".git").exists() {
            return false;
        }
        let mut command = self.git.command(Place::Folder(&folder), &[]);
        command.args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
        !matches!(finish(command), Ok(answer) if answer.code == Some(0))
    }

    fn command(&self, alternates: Option<&Path>) -> Command {
        let mut command = self
            .git
            .command(Place::GitDir(&self.folder), &self.hooks_off);
        if let Some(objects) = alternates {
            command.env("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects);
        }
        command
    }

    fn command_with<I, S>(&self, alternates: Option<&Path>, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(alternates);
        command.args(args);
        command
    }

    fn run<I, S>(&self, alternates: Option<&Path>, args: I) -> Result<Answer, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        finish(self.command_with(alternates, args))
    }

    fn numbered(&self) -> Result<Vec<(u64, RecordedState)>, GitError> {
        Ok(self
            .numbered_with_times()?
            .into_iter()
            .map(|(number, state, _)| (number, state))
            .collect())
    }

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

    fn is_recent(&self, recorded: Option<u64>) -> bool {
        recorded.is_some_and(|recorded| {
            seconds_of(self.clock.now()).saturating_sub(recorded) < SAME_STATE_FOR.as_secs()
        })
    }

    pub(crate) fn reading(&self) -> std::sync::RwLockReadGuard<'_, ()> {
        self.cleaning.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn tree_of(&self, real_objects: &Path, commit: &str) -> Result<String, GitError> {
        let spec = format!("{commit}^{{tree}}");
        Ok(line(
            self.run(Some(real_objects), ["rev-parse", "--verify", "-q", &spec])?
                .printed()?,
        ))
    }

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
                    let mut ids = vec![head.to_owned(), self.tree_of(real_objects, head)?];
                    ids.extend(self.new_side(real_objects, &last, head)?);
                    self.pack_missing(real_objects, ids)?;
                } else {
                    self.pack_missing(real_objects, vec![head.to_owned()])?;
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
                // The store holds this commit without its parents: the `shallow` file tells git so.
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

    fn pack_prefix(&self) -> String {
        self.folder
            .join("objects/pack/pack")
            .to_string_lossy()
            .into_owned()
    }

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

struct Differing {
    path: Vec<u8>,
    folder: bool,
}

fn differing_paths(command: &str, printed: &[u8]) -> Result<Vec<Differing>, GitError> {
    let mut paths = Vec::new();
    let mut records = printed.split(|byte| *byte == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        // git help status, "Porcelain Format Version 2": how many fields come before the path,
        // whether the worktree's side always differs, whether the path it was renamed from follows.
        let (before, always, original) = match record[0] {
            b'1' => (8, false, false),
            b'2' => (9, false, true),
            b'u' => (10, true, false),
            b'?' => (1, true, false),
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
        if always || fields[1].get(1) != Some(&b'.') {
            let path = fields[before];
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

const FORGET: [&str; 4] = ["update-index", "--force-remove", "-z", "--stdin"];

fn zero_ended(paths: &[&[u8]]) -> Vec<u8> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path);
        input.push(0);
    }
    input
}

fn is_plain_folder(root: &Path, path: &[u8]) -> bool {
    let place = root.join(os_string(path));
    fs::symlink_metadata(&place).is_ok_and(|found| found.is_dir()) && !place.join(".git").exists()
}

/// What tells one incarnation of the file from another: kind, inode, size and times.
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

fn seconds_of(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |after| after.as_secs())
}

fn line(printed: Vec<u8>) -> String {
    String::from_utf8_lossy(&printed).trim_end().to_owned()
}

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
        let printed = b"1 M. N... 100644 100644 100644 aa bb staged.txt\0\
                        1 .M N... 100644 100644 100644 aa bb edited.txt\0\
                        # stash 1\0";

        let paths = differing_paths("git status", printed).expect("read");

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, b"edited.txt");
    }
}
