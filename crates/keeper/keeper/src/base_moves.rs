//! Carries review threads on the old side to the commit the changes are counted from when it moves.
//! The last commit seen is noted in the state store; a failed carry leaves it, so `GetChanges`
//! retries.

use yaar_link::{Fact, WorkspaceId};
use yaar_recorder::CommitId;

use crate::wiring::Kept;

pub(crate) fn before_set(kept: &Kept, workspace: &WorkspaceId) -> Option<CommitId> {
    counted_from(kept, workspace)
}

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
        // A `GetChanges` that read its commit before another Window's `SetBase` and is answered
        // after it carries the threads back to the older commit; the next answer carries them
        // forward.
        Some(was) => carry(kept, workspace, &[was], &now, facts),
        None => note(kept, workspace, &now),
    }
}

fn counted_from(kept: &Kept, workspace: &WorkspaceId) -> Option<CommitId> {
    let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
    match kept.workspaces.changes(&id) {
        Ok(changes) => changes.counted_from,
        // The request that follows says why it cannot be answered.
        Err(_) => None,
    }
}

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
        Ok(None) => return,
        Err(e) => {
            eprintln!("yaar-keeper: the threads of {workspace} could not follow the base: {e}");
            return;
        }
    };
    let mut moved = false;
    let mut failed = false;
    for from in from.iter().filter(|from| *from != to) {
        // A carry that failed on one file still kept the other files' threads, which a Window is
        // told of; the note stays as it was.
        let (threads, failure) = match kept.reviews.base_moved_in_part(&id, &worktree, from, to) {
            Ok(done) => done,
            Err(e) => (Vec::new(), Some(e)),
        };
        moved |= !threads.is_empty();
        if let Some(e) = failure {
            failed = true;
            eprintln!(
                "yaar-keeper: the threads of {workspace} could not follow the base to {to}: {e}"
            );
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
