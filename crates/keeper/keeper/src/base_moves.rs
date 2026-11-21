//! The base of a workspace moved: the Keeper has the Review engine carry the threads on the
//! old side to the commit the changes are now counted from (`Reviews::base_moved`).
//!
//! The commit the changes of a workspace are counted from is `Changes::counted_from`. It
//! moves when the user sets another base (`SetBase`), and when the base branch or the
//! branch of the workspace moves, which the Keeper sees as a new `counted_from` when it
//! answers `GetChanges`, also when it moved while the Keeper was not running (an overnight
//! pull). The last `counted_from` the Keeper answered or found is noted in the workspace's
//! state store (`StateStore::note_counted_from`, read back with
//! `StateStore::counted_from`, through `Reviews::states_of`), so that the note outlives a
//! restart of the Keeper.
//!
//! | Where | What is done |
//! | --- | --- |
//! | `SetBase`, before the base is set | [`before_set`] reads `counted_from` now, from `Workspaces::changes` |
//! | `SetBase`, after the base is set | [`after_set`] reads it again; when it differs from `before`, `Reviews::base_moved(before, after)`, notes `after`, and puts `ReviewsChanged` of the workspace into the facts when a thread moved |
//! | `GetChanges`, answered | [`changes_seen`] compares the answer's `counted_from` with the one noted; when both are there and differ, the same as above; when none was noted, it only notes |
//!
//! Two `GetChanges` of two Windows at once may both carry from the same noted commit: the
//! second carries nothing, since the first moved every thread placed on that commit.
//! A workspace with no `counted_from` before or after carries nothing. A failure of the
//! carry is written to the standard error in one line, starting `yaar-keeper:`, and does not
//! change the reply.

use yaar_link::{Fact, WorkspaceId};
use yaar_recorder::CommitId;

use crate::wiring::Kept;

/// `SetBase` of `workspace` is about to be answered: what its changes are counted from now.
pub(crate) fn before_set(kept: &Kept, workspace: &WorkspaceId) -> Option<CommitId> {
    // Until the carry is built nothing is read, and nothing is carried.
    let _ = (kept, workspace);
    None
}

/// `SetBase` of `workspace` was kept: carries the old side's threads from `before`, as the
/// module doc says.
pub(crate) fn after_set(
    kept: &Kept,
    workspace: &WorkspaceId,
    before: Option<CommitId>,
    facts: &mut Vec<Fact>,
) {
    let _ = (kept, workspace, before, facts);
}

/// `GetChanges` of `workspace` is answered with `counted_from`: carries the old side's
/// threads when it moved since it was last noted, as the module doc says.
pub(crate) fn changes_seen(
    kept: &Kept,
    workspace: &WorkspaceId,
    counted_from: Option<&str>,
    facts: &mut Vec<Fact>,
) {
    let _ = (kept, workspace, counted_from, facts);
}
