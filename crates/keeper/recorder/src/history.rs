//! The history of a worktree: its commits and its recorded states, in the order they
//! happened.

use crate::states::{RecordError, RecordedState, StateStore};
use crate::worktree::{Commit, CommitId, Worktree};

/// One entry of the history of a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A commit of the branch checked out.
    Commit(Commit),
    /// A state recorded in the worktree's state store.
    State(RecordedState),
}

/// The history of `worktree` since the commit `since`, oldest first: the commits of the
/// branch checked out after `since`, following first parents, and the states of `store`
/// taken on `since` or on one of those commits, in the order they happened.
///
/// A state was taken on the commit that was checked out then, [`RecordedState::on`], and
/// so after that commit and before the next one: it comes right after its commit, after
/// the states of that commit taken before it, and before the next commit. States taken on
/// `since` come first. A state taken on a commit that is not in this list, as one of a
/// branch the user switched away from, is not listed. With `since` none, the history
/// starts at the first commit; with nothing committed, it holds the states alone.
///
/// The commits are told in git's words, never by date: commits made in the same second,
/// as tests make them, keep their order.
pub fn history(
    worktree: &Worktree,
    store: &StateStore,
    since: Option<&CommitId>,
) -> Result<Vec<Entry>, RecordError> {
    let _ = (worktree, store, since);
    todo!("the commits and the states, in the order they happened")
}
