//! Any two versions of a worktree compared, and a file read in any of them: states of
//! the store, commits of the real repository, and the files on disk.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::git::{GitError, Place};
use crate::states::{RecordError, StateStore, not_a_commit};
use crate::worktree::{ChangedPath, Commit, CommitId, Worktree, is_full_id, name_status};

impl StateStore {
    /// The paths that differ between two versions of the worktree, ordered by path, renames
    /// paired as for [`StateStore::changed_paths`]: `from` and `to` are each the full id of
    /// a state of this store or of a commit of the real repository, and `to` none is the
    /// files on disk. A commit of the real repository the store does not hold is copied
    /// into it first, as [`StateStore::keep_commit`] does, through `worktree`; with no
    /// worktree, as when it is gone, only what the store holds can be compared. What is
    /// given equals what `git diff` of the two trees names.
    ///
    /// A comparison with the files on disk reads them as a recording would, on a copy of
    /// the worktree's list of files, and records no state.
    ///
    /// A version that is not a full id, that neither the store nor the real repository
    /// holds, or that the store lacks when there is no worktree, is an error and changes
    /// nothing in the store.
    pub fn compare(
        &self,
        worktree: Option<&Worktree>,
        from: &str,
        to: Option<&str>,
    ) -> Result<Vec<ChangedPath>, RecordError> {
        self.hold(worktree, from)?;
        let changes = match to {
            Some(to) => {
                self.hold(worktree, to)?;
                // The objects of the real repository are not needed: a version the store
                // holds is whole in it.
                self.run(None, diff_tree_args(from, to))?
            }
            None => {
                let Some(worktree) = worktree else {
                    return Err(RecordError::Store {
                        folder: self.folder.clone(),
                        reason: "the files on disk are compared through the worktree, \
                                 and no worktree is given"
                            .to_owned(),
                    });
                };
                let (tree, _) = self.tree_on_disk(worktree)?;
                // A blob of the files on disk that the real repository holds already was
                // not written into the store: git reads it from there to pair a rename.
                let real_objects = worktree.common_dir().join("objects");
                self.run(Some(&real_objects), diff_tree_args(from, &tree))?
            }
        };
        let command = changes.command().to_owned();
        Ok(name_status(&command, &changes.printed()?)?
            .into_iter()
            .map(|(path, change)| ChangedPath {
                path: PathBuf::from(path),
                change,
            })
            .collect())
    }

    /// The content of the file at `path` in `version`, the full id of a state of this
    /// store or of a commit of the real repository, copied into the store first as for
    /// [`StateStore::compare`]; none when the version holds no file at that path.
    pub fn content_in(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        path: &Path,
    ) -> Result<Option<Vec<u8>>, RecordError> {
        self.hold(worktree, version)?;
        Ok(self.content(version, path)?)
    }

    /// Makes the store hold `version`, a state or a commit, so that it reads with the real
    /// repository gone: a commit the store has not got is copied from the repository of
    /// `worktree`.
    fn hold(&self, worktree: Option<&Worktree>, version: &str) -> Result<(), RecordError> {
        if !is_full_id(version) {
            return Err(not_a_commit(version).into());
        }
        // A state is held, so is the commit a state was taken on and a commit kept before:
        // those need nothing of the real repository, which may be gone by now.
        let held = self.run(None, ["cat-file", "-e", &format!("{version}^{{commit}}")])?;
        if held.code == Some(0) {
            return Ok(());
        }
        match worktree {
            Some(worktree) => self.keep_commit(worktree, &CommitId::new(version)),
            None => Err(RecordError::Store {
                folder: self.folder.clone(),
                reason: format!(
                    "{version} is not held, and with no worktree there is no repository to \
                     copy it from"
                ),
            }),
        }
    }

    /// The commits of the branch checked out in `worktree` that come after `since`, in the
    /// order they happened, oldest first, following first parents; none where nothing is
    /// committed. With `since` none, from the first commit.
    ///
    /// A walk along first parents alone cannot be told where to stop: when the branch took
    /// in a commit its base had gained, `since` is reached through a second parent, and
    /// the walk would go on to the root. So the commits that come after `since` are named
    /// first, whatever way they are reached, and the walk along first parents keeps those.
    pub(crate) fn first_parent_commits(
        &self,
        worktree: &Worktree,
        since: Option<&CommitId>,
    ) -> Result<Vec<Commit>, GitError> {
        if worktree.head_commit()?.is_none() {
            return Ok(Vec::new());
        }
        // Plumbing that reads, and runs no hook: nothing of the configuration's is to be
        // turned off.
        let read = |args: &[&str]| {
            self.git
                .run(Place::Folder(worktree.root()), &[], args)?
                .printed()
        };
        let lines = |printed: &[u8]| -> Vec<Vec<u8>> {
            printed
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(<[u8]>::to_vec)
                .collect()
        };
        let mut after = None;
        if let Some(since) = since {
            let left_out = format!("^{since}");
            let reached: HashSet<Vec<u8>> = lines(&read(&["rev-list", "HEAD", &left_out])?)
                .into_iter()
                .collect();
            if reached.is_empty() {
                return Ok(Vec::new());
            }
            after = Some(reached);
        }
        let limit = after
            .as_ref()
            .map(|after| format!("--max-count={}", after.len()));
        let mut args = vec![
            "rev-list",
            "--first-parent",
            "--no-commit-header",
            "--format=%H%x00%s",
        ];
        args.extend(limit.as_deref());
        args.push("HEAD");
        let mut commits = Vec::new();
        for line in lines(&read(&args)?) {
            let mut fields = line.splitn(2, |byte| *byte == 0);
            let (Some(id), Some(summary)) = (fields.next(), fields.next()) else {
                return Err(crate::git::unreadable(
                    &format!("git {}", args.join(" ")),
                    format!("the line {:?}", String::from_utf8_lossy(&line)),
                ));
            };
            if after.as_ref().is_none_or(|after| after.contains(id)) {
                commits.push(Commit {
                    id: CommitId::new(String::from_utf8_lossy(id).into_owned()),
                    summary: String::from_utf8_lossy(summary).into_owned(),
                });
            }
        }
        commits.reverse();
        Ok(commits)
    }
}

/// The arguments of git's tree diff as `compare` runs it: the paths that differ, with
/// what happened to each, renames paired.
fn diff_tree_args<'a>(from: &'a str, to: &'a str) -> [&'a str; 7] {
    ["diff-tree", "-r", "-z", "-M", "--name-status", from, to]
}
