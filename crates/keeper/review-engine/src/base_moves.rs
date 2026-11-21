//! Threads on the old side when the base moves (ADR-013 point 4).
//!
//! A thread on a removed line sits on the version the change is counted from: its anchor's
//! `tree` is that commit, its side old. While the base stands it does not move. When the
//! commit the changes of a workspace are counted from changes (the user set another base,
//! or the base branch, or the fork point, moved), [`crate::Reviews::base_moved`] carries
//! each open thread on the old side placed on the old commit to the new one, by the rule
//! the new side's threads follow ([`crate::carry_file`], one file at a time): with the
//! file's text in `from` and in `to`, read through the state store, which keeps both
//! commits (`StateStore::content_in`, which copies a commit it lacks from the worktree).
//!
//! What moves, and what does not:
//!
//! | Thread | What happens |
//! | --- | --- |
//! | On the old side, open, placed on `from` (its `now` is that commit, or with no `now` its anchor's `tree` is) | carried to `to`: its new placement is kept with `Store::set_places`, on the old side, with the notice the follower gives |
//! | On the old side, resolved | stays where it is |
//! | On the old side, placed on another commit | stays where it is: it is not on the base that moved |
//! | On the new side | not touched: the new side's threads follow the states |
//! | With an absolute path (a file outside the worktree) | not touched: it is never carried |
//!
//! The placements are kept in one call of the Store, under the engine's turn of writing.
//! When at least one thread moved, the call gives their ids, and the Keeper tells
//! `ReviewsChanged` of the workspace. A `from` equal to `to` moves nothing.

use yaar_recorder::{CommitId, Worktree};

use crate::engine::{EngineError, Reviews};
use crate::{ThreadId, WorkspaceId};

impl Reviews {
    /// The commit the changes of `workspace` are counted from moved from `from` to `to`:
    /// carries its threads on the old side, as the module doc says, and gives the ids of
    /// those that moved.
    pub fn base_moved(
        &self,
        workspace: &WorkspaceId,
        worktree: &Worktree,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let _ = (workspace, worktree, from, to);
        todo!("carry the old side's threads, as the module doc says")
    }
}
