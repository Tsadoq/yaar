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
//! | `SetBase`, after the base is set | [`after_set`] reads it again and carries the threads placed on the commit noted and on `before` (each one that is not `after`) to `after` with `Reviews::base_moved`; notes `after` when every carry succeeded, also when nothing differed; puts `ReviewsChanged` of the workspace into the facts when a thread was re-placed, whether its lines moved or stayed |
//! | `GetChanges`, answered | [`changes_seen`] compares the answer's `counted_from` with the one noted; when both are there and differ, it carries from the noted commit to the answer's, notes it when the carry succeeded, and puts `ReviewsChanged` into the facts when a thread was re-placed; when none was noted, it only notes |
//!
//! `SetBase` carries from the commit noted as well as from `before` when they differ: a base
//! that moved since the last `GetChanges` left threads on the noted commit, and the ones
//! written since on `before`.
//!
//! Two `GetChanges` of two Windows at once may both carry from the same noted commit: the
//! second carries nothing, since the first moved every thread placed on that commit.
//! A `GetChanges` that read its `counted_from` before another Window's `SetBase` took
//! effect, and is answered after it, finds the newer commit noted and carries the threads
//! back to the older one; the next answer carries them forward again. Two Windows at once
//! are left for after the first release, and this with them.
//! A workspace with no `counted_from` before or after carries nothing, and one with no
//! worktree carries nothing. A failure of the carry, or of reading or writing the note, is
//! written to the standard error in one line, starting `yaar-keeper:`, and does not change
//! the reply; after a failed carry the note is left as it was, so that the next
//! `GetChanges` tries again.

use yaar_link::{Fact, WorkspaceId};
use yaar_recorder::CommitId;

use crate::wiring::Kept;

/// `SetBase` of `workspace` is about to be answered: what its changes are counted from now.
pub(crate) fn before_set(kept: &Kept, workspace: &WorkspaceId) -> Option<CommitId> {
    counted_from(kept, workspace)
}

/// `SetBase` of `workspace` was kept: carries the old side's threads from `before`, as the
/// module doc says.
pub(crate) fn after_set(
    kept: &Kept,
    workspace: &WorkspaceId,
    before: Option<CommitId>,
    facts: &mut Vec<Fact>,
) {
    let Some(after) = counted_from(kept, workspace) else {
        return;
    };
    let mut from: Vec<CommitId> = noted(kept, workspace).into_iter().collect();
    from.extend(before.filter(|before| !from.contains(before)));
    carry(kept, workspace, &from, &after, facts);
}

/// `GetChanges` of `workspace` is answered with `counted_from`: carries the old side's
/// threads when it moved since it was last noted, as the module doc says.
pub(crate) fn changes_seen(
    kept: &Kept,
    workspace: &WorkspaceId,
    counted_from: Option<&str>,
    facts: &mut Vec<Fact>,
) {
    let Some(now) = counted_from else {
        return;
    };
    let now = CommitId::new(now);
    match noted(kept, workspace) {
        Some(was) if was == now => {}
        Some(was) => carry(kept, workspace, &[was], &now, facts),
        None => note(kept, workspace, &now),
    }
}

/// The commit the changes of `workspace` are counted from now, by one `changes`.
fn counted_from(kept: &Kept, workspace: &WorkspaceId) -> Option<CommitId> {
    let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
    match kept.workspaces.changes(&id) {
        Ok(changes) => changes.counted_from,
        // The request that follows says why it cannot be answered.
        Err(_) => None,
    }
}

/// The commit last noted for `workspace`, none when nothing was or it cannot be read.
fn noted(kept: &Kept, workspace: &WorkspaceId) -> Option<CommitId> {
    let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
    let read = kept
        .reviews
        .states_of(&id)
        .map_err(|e| e.to_string())
        .and_then(|states| states.counted_from().map_err(|e| e.to_string()));
    match read {
        Ok(found) => found,
        Err(e) => {
            eprintln!("yaar-keeper: the commit {workspace} is counted from could not be read: {e}");
            None
        }
    }
}

/// Notes `commit` as the one the changes of `workspace` are counted from.
fn note(kept: &Kept, workspace: &WorkspaceId, commit: &CommitId) {
    let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
    let wrote = kept
        .reviews
        .states_of(&id)
        .map_err(|e| e.to_string())
        .and_then(|states| states.note_counted_from(commit).map_err(|e| e.to_string()));
    if let Err(e) = wrote {
        eprintln!("yaar-keeper: the commit {workspace} is counted from could not be noted: {e}");
    }
}

/// Carries the threads of `workspace` on the old side from each commit of `from` that is
/// not `to` to `to`, notes `to` when every carry succeeded, and tells `ReviewsChanged` when
/// a thread moved.
fn carry(
    kept: &Kept,
    workspace: &WorkspaceId,
    from: &[CommitId],
    to: &CommitId,
    facts: &mut Vec<Fact>,
) {
    let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
    let worktree = match kept.workspaces.worktree(&id) {
        Ok(Some(worktree)) => worktree,
        // A workspace whose worktree is gone has no commit to read the base in.
        Ok(None) => return,
        Err(e) => {
            eprintln!("yaar-keeper: the threads of {workspace} could not follow the base: {e}");
            return;
        }
    };
    let mut moved = false;
    let mut failed = false;
    for from in from.iter().filter(|from| *from != to) {
        match kept.reviews.base_moved(&id, &worktree, from, to) {
            Ok(threads) => moved |= !threads.is_empty(),
            Err(e) => {
                failed = true;
                eprintln!(
                    "yaar-keeper: the threads of {workspace} could not follow the base to {to}: {e}"
                );
            }
        }
    }
    if !failed {
        note(kept, workspace, to);
    }
    if moved {
        facts.push(Fact::ReviewsChanged {
            workspace: workspace.clone(),
        });
    }
}
