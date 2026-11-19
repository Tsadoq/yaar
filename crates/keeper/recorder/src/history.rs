//! The history of a worktree: its commits and its recorded states, in the order they
//! happened.

use std::collections::HashMap;

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
    let commits = store.first_parent_commits(worktree, since)?;
    let mut taken_on: HashMap<Option<CommitId>, Vec<RecordedState>> = HashMap::new();
    for state in store.states()? {
        taken_on.entry(state.on.clone()).or_default().push(state);
    }
    let mut entries = Vec::new();
    let mut put_states_on = |commit: Option<CommitId>, entries: &mut Vec<Entry>| {
        // Taken out, so that a state is listed once and one that is taken on a commit
        // that is not listed is never reached.
        entries.extend(
            taken_on
                .remove(&commit)
                .into_iter()
                .flatten()
                .map(Entry::State),
        );
    };
    // Before the first commit listed: what was taken on `since`, or, from the root, what
    // was taken before anything was committed.
    put_states_on(since.cloned(), &mut entries);
    for commit in commits {
        let id = commit.id.clone();
        entries.push(Entry::Commit(commit));
        put_states_on(Some(id), &mut entries);
    }
    Ok(entries)
}
