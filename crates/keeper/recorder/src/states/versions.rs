//! Any two versions of a worktree compared, and a file read in any of them: states of
//! the store, commits of the real repository, and the files on disk.

use std::path::Path;

use crate::states::{RecordError, StateStore};
use crate::worktree::{ChangedPath, Worktree};

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
    pub fn compare(
        &self,
        worktree: Option<&Worktree>,
        from: &str,
        to: Option<&str>,
    ) -> Result<Vec<ChangedPath>, RecordError> {
        let _ = (worktree, from, to);
        todo!("the paths that differ between any two versions")
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
        let _ = (worktree, version, path);
        todo!("the content of a file in any version")
    }
}
