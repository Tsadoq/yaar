//! The commits and recorded states of a worktree, in the order they happened.

use std::collections::HashMap;

use crate::states::{RecordError, RecordedState, StateStore};
use crate::worktree::{Commit, CommitId, Worktree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Commit(Commit),
    State(RecordedState),
}

/// Commits of the checked-out branch after `since` (first parents, git's order, never date), oldest
/// first, each followed by the states taken on it. States taken on `since` come first; states on
/// unlisted commits are dropped.
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
        entries.extend(
            taken_on
                .remove(&commit)
                .into_iter()
                .flatten()
                .map(Entry::State),
        );
    };
    // States taken on `since`, or, from the root, before anything was committed.
    put_states_on(since.cloned(), &mut entries);
    for commit in commits {
        let id = commit.id.clone();
        entries.push(Entry::Commit(commit));
        put_states_on(Some(id), &mut entries);
    }
    Ok(entries)
}
