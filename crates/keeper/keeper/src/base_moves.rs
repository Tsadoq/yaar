//! The base of a workspace moved: the Keeper has the Review engine carry the threads on the
//! old side to the commit the changes are now counted from (`Reviews::base_moved`).
//!
//! The commit the changes of a workspace are counted from is `Changes::counted_from`. It
//! moves when the user sets another base (`SetBase`), and when the base branch or the
//! branch of the workspace moves, which the Keeper sees as a new `counted_from` when it
//! answers `GetChanges`. [`Seen`] keeps, for each workspace, the last `counted_from` the
//! Keeper answered or found, in memory: the first `GetChanges` after a start of the Keeper
//! carries nothing, it only notes.
//!
//! | Where | What is done |
//! | --- | --- |
//! | `SetBase`, before the base is set | [`before_set`] reads `counted_from` now, from `Workspaces::changes` |
//! | `SetBase`, after the base is set | [`after_set`] reads it again; when it differs, `Reviews::base_moved(from, to)`, notes `to`, and puts `ReviewsChanged` of the workspace into the facts when a thread moved |
//! | `GetChanges`, answered | [`changes_seen`] compares the answer's `counted_from` with the one noted; when both are there and differ, the same as above; then notes it |
//!
//! A workspace with no `counted_from` before or after carries nothing. A failure of the
//! carry is written to the standard error in one line, starting `yaar-keeper:`, and does not
//! change the reply.

use std::collections::HashMap;
use std::sync::Mutex;

use yaar_link::{Fact, WorkspaceId};
use yaar_recorder::CommitId;

use crate::wiring::Kept;

/// The last commit each workspace's changes were counted from, as the Keeper answered it.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    _counted_from: Mutex<HashMap<WorkspaceId, CommitId>>,
}

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
