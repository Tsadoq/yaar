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
use std::sync::{Mutex, PoisonError};

use crate::contents::{Contents, Read};
use crate::git::{Answer, Git, GitError, Place, finish, finish_with, unreadable};
use crate::index::IndexCopy;
use crate::worktree::{
    ChangedPath, CommitId, Worktree, hooks_off, inside, is_full_id, name_status, os_string,
};

// The comparison of any two versions and the reading of a file in any of them: the
// history's package's, beside the recording's.
mod versions;

/// The old value that `git update-ref` takes for "no such ref".
const NO_REF: &str = "0000000000000000000000000000000000000000";

/// Where the refs of the states are, each named by its number.
const STATES: &str = "refs/yaar/states/";
/// Where the refs of the commits kept by [`StateStore::keep_commit`] are.
const KEPT: &str = "refs/yaar/kept/";
/// The file of the store that holds the commit the store last copied whole.
const COMPLETE_HEAD: &str = "yaar-complete-head";

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
}

impl StateStore {
    /// The store in `folder`, a bare repository made there when the folder holds none.
    /// The folder lies in the app's folder, never in a worktree or a repository.
    pub fn open(git: &Git, folder: &Path) -> Result<StateStore, RecordError> {
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
        })
    }

    /// The folder of the store.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Records the state of `worktree` as its files are now, and returns it. When the
    /// tree is that of the newest state, that state is returned and no new state is made,
    /// though objects of the files may have been written into the store: a state is the
    /// files, and the same files are the same state.
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
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let head = worktree.head_commit()?;
        let real_objects = worktree.common_dir().join("objects");

        let (tree, commit, number) = {
            // The copy is gone at the end of this block, whatever happens: a git that was
            // killed leaves its lock in the copy's folder, and that goes with it.
            let index = IndexCopy::of(&worktree.git_dir().join("index")).map_err(GitError::from)?;
            let hooks_off = hooks_off(&self.git, Place::Folder(worktree.root()))?;
            // Every run in the real worktree works on the copy, and writes what it makes
            // into the store: the objects of the real repository are only read.
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
            let paths = differing_paths(&command, &paths.printed()?)?;
            if !paths.is_empty() {
                let mut input = Vec::new();
                for path in &paths {
                    input.extend_from_slice(path);
                    input.push(0);
                }
                finish_with(
                    in_worktree(&[
                        "update-index",
                        "--add",
                        "--remove",
                        "--ignore-skip-worktree-entries",
                        "-z",
                        "--stdin",
                    ]),
                    &input,
                )?
                .printed()?;
            }
            let tree = line(finish(in_worktree(&["write-tree"]))?.printed()?);

            let numbered = self.numbered()?;
            if let Some((_, newest)) = numbered.last()
                && newest.tree == tree
            {
                return Ok(newest.clone());
            }
            let number = numbered.last().map_or(1, |(n, _)| n + 1);
            let message = format!("YAAR state {number}");
            let mut args = vec!["commit-tree", &tree];
            if let Some(head) = &head {
                args.extend(["-p", head.as_str()]);
            }
            args.extend(["-m", &message]);
            let mut command = in_worktree(&args);
            // The app's own identity, so that `user.useConfigOnly` cannot stop it.
            for who in ["AUTHOR", "COMMITTER"] {
                command
                    .env(format!("GIT_{who}_NAME"), "YAAR")
                    .env(format!("GIT_{who}_EMAIL"), "yaar@localhost.invalid");
            }
            (tree, line(finish(command)?.printed()?), number)
        };

        self.complete(&real_objects, head.as_ref().map(CommitId::as_str), &tree)?;
        let name = format!("{STATES}{number:010}");
        // Written only where no ref has this number: all-zero is git's "nothing is there
        // yet". Another handle on the store that took the number first makes this fail
        // with git's words, and the state it wrote stays as it is.
        self.run(None, ["update-ref", &name, &commit, NO_REF])?
            .printed()?;
        Ok(RecordedState {
            id: StateId(commit),
            tree,
            on: head,
        })
    }

    /// Copies a commit of the real repository into the store, with its tree, so that its
    /// files read with the real repository gone, as a thread on the old side of a diff
    /// needs, and names it by the ref `refs/yaar/kept/<commit id>`, so that nothing that
    /// cleans the store up takes it away while it is named. A commit the store holds
    /// already costs nothing.
    pub fn keep_commit(&self, worktree: &Worktree, commit: &CommitId) -> Result<(), RecordError> {
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
    pub fn content(&self, commit: &str, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let (Some(path), true) = (inside(path), is_full_id(commit)) else {
            return Ok(None);
        };
        let mut name = commit.as_bytes().to_vec();
        name.push(b':');
        name.extend_from_slice(path.as_os_str().as_bytes());
        // One git process for the one file: the reader that stays running for many comes
        // with the reader of many files.
        let mut contents = Contents::start_in(self.command(None))?;
        let read = contents.read(&name);
        contents.stop();
        match read? {
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
        let _ = worktree;
        todo!("record, leaving out a nested repository with no commit")
    }

    /// The tree of the files of `worktree` on disk as a recording reads them, written into
    /// this store and named by no commit and no ref, and the folders left out of it, as
    /// for [`Recording::left_out`]: the steps a recording takes up to `git write-tree`, on
    /// a copy of the worktree's list of files, the store's objects written and the real
    /// repository's only read. [`StateStore::record_reporting`] makes its state of this
    /// tree; [`StateStore::compare`] compares a version with it, recording nothing.
    #[expect(dead_code, reason = "written by the recording's package of slice 4")]
    pub(crate) fn tree_on_disk(
        &self,
        worktree: &Worktree,
    ) -> Result<(String, Vec<PathBuf>), RecordError> {
        let _ = worktree;
        todo!("the tree of the files on disk, as a recording reads them")
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
        let answer = self.run(
            None,
            [
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(tree)%00%(parent)",
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
            let [name, id, tree, parent] = fields[..] else {
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
            ));
        }
        all.sort_by_key(|(number, _)| *number);
        Ok(all)
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

/// The paths of the records of `git status --porcelain=v2 -z --no-renames` that git has
/// to be told of for its list of files to hold what `git add --all` would make it hold:
/// those the worktree's side differs in, and the untracked ones. A path that is staged
/// and the same on disk is in the list already.
fn differing_paths(command: &str, printed: &[u8]) -> Result<Vec<Vec<u8>>, GitError> {
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
            paths.push(path.strip_suffix(b"/").unwrap_or(path).to_vec());
        }
    }
    Ok(paths)
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
