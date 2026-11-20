//! A version of a worktree written into a folder as plain files: the old side of a diff,
//! which a second language server reads (ADR-016 point 1).

use std::path::Path;

use crate::states::{RecordError, StateStore};
use crate::worktree::Worktree;

impl StateStore {
    /// Writes every file of `version` into the folder `into`, which must not exist and
    /// whose parent must: `version` is the full id of a state of this store or of a commit
    /// of the real repository, which the store copies first through `worktree` as
    /// [`StateStore::compare`] does; with no worktree only what the store holds can be
    /// written.
    ///
    /// The files are written by git's own checkout of a tree into a folder, with an index
    /// of the store's own made for this call in a temporary file and removed after: git's
    /// `read-tree` of the version into that index, then `checkout-index --all
    /// --prefix=<into>/`, both run in the state store with the real repository's objects as
    /// alternates, every hook off as for every other call here. A file's mode is kept, an
    /// executable one stays executable; a link is written as a link. The folder holds no
    /// `.git` and is no repository. The real repository gains nothing: no worktree, no ref,
    /// no file, no change of its index; the store gains at most the commit it copied, as a
    /// comparison does.
    ///
    /// A version that is not a full id, that neither the store nor the repository holds,
    /// or a write that fails, is an error, and `into` is removed if it was made.
    pub fn export(
        &self,
        worktree: Option<&Worktree>,
        version: &str,
        into: &Path,
    ) -> Result<(), RecordError> {
        let _ = (worktree, version, into);
        todo!()
    }
}
