//! Comparing any two versions of a worktree (states, commits, files on disk) and reading a file in
//! any.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::git::{GitError, Place};
use crate::states::{RecordError, StateStore, not_a_commit};
use crate::worktree::{ChangedPath, Commit, CommitId, Worktree, is_full_id, name_status};

impl StateStore {
    /// The paths that differ between `from` and `to`, each the full id of a state or of a commit of
    /// the real repository; `to` none is the files on disk, read on a copy of the index. A commit
    /// the store lacks is copied in through `worktree`.
    pub fn compare(
        &self,
        worktree: Option<&Worktree>,
        from: &str,
        to: Option<&str>,
    ) -> Result<Vec<ChangedPath>, RecordError> {
        let _guard = self.reading();
        self.compare_held(worktree, from, to)
    }

    pub(crate) fn compare_held(
        &self,
        worktree: Option<&Worktree>,
        from: &str,
        to: Option<&str>,
    ) -> Result<Vec<ChangedPath>, RecordError> {
        match to {
            Some(to) => self.hold(worktree, &[from, to])?,
            None => self.hold(worktree, &[from])?,
        }
        let changes = match to {
            Some(to) => self.run(None, diff_tree_args(from, to))?,
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
                // A blob of the files on disk that the real repository holds already was not
                // written into the store: git reads it from there to pair a rename.
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

    /// None when the version holds no file at `path`.
    pub fn content_in(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        path: &Path,
    ) -> Result<Option<Vec<u8>>, RecordError> {
        let _guard = self.reading();
        self.hold(worktree, &[version])?;
        Ok(self.content(version, path)?)
    }

    /// Every version is checked before any is copied, so a refusal leaves the store as it was. The
    /// caller holds the clean-up guard.
    fn hold(&self, worktree: Option<&Worktree>, versions: &[&str]) -> Result<(), RecordError> {
        let mut to_keep = Vec::new();
        for version in versions {
            if !is_full_id(version) {
                return Err(not_a_commit(version).into());
            }
            // A state, the commit it was taken on and a kept commit need nothing of the real
            // repository, which may be gone.
            let spec = format!("{version}^{{commit}}");
            if self.run(None, ["cat-file", "-e", &spec])?.code == Some(0) {
                continue;
            }
            let Some(worktree) = worktree else {
                return Err(RecordError::Store {
                    folder: self.folder.clone(),
                    reason: format!(
                        "{version} is not held, and with no worktree there is no repository \
                         to copy it from"
                    ),
                });
            };
            let real_objects = worktree.common_dir().join("objects");
            if self
                .run(Some(&real_objects), ["cat-file", "-e", &spec])?
                .code
                != Some(0)
            {
                return Err(RecordError::Store {
                    folder: self.folder.clone(),
                    reason: format!("{version} is no commit of the repository"),
                });
            }
            to_keep.push((worktree, *version));
        }
        for (worktree, version) in to_keep {
            self.keep_commit_held(worktree, &CommitId::new(version))?;
        }
        Ok(())
    }

    /// A first-parent walk cannot tell where to stop: when the branch took in a commit its base had
    /// gained, `since` is reached through a second parent. So the commits after `since` are named
    /// first, by any route, and the walk keeps those.
    pub(crate) fn first_parent_commits(
        &self,
        worktree: &Worktree,
        since: Option<&CommitId>,
    ) -> Result<Vec<Commit>, GitError> {
        if worktree.head_commit()?.is_none() {
            return Ok(Vec::new());
        }
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

fn diff_tree_args<'a>(from: &'a str, to: &'a str) -> [&'a str; 7] {
    ["diff-tree", "-r", "-z", "-M", "--name-status", from, to]
}
