//! A round shown as sent until its agent's first edit, and what changed since a round's
//! comments were sent. Built by the engine's package "rounds and what changed since" of
//! slice 6.

use yaar_recorder::{ChangedPath, Worktree};
use yaar_store::{ReviewId, SessionId};

use crate::engine::{EngineError, Reviews};

impl Reviews {
    /// The agent of `session` reported an edit, with its edit or its write tool: the newest
    /// round of its review counts as edited from now on, by
    /// [`crate::Review::agent_edited`], and that is kept. Gives the review and the round
    /// when one changed; none when none did, and for a session with no review.
    pub fn agent_edited(
        &self,
        session: &SessionId,
    ) -> Result<Option<(ReviewId, u32)>, EngineError> {
        let _ = session;
        todo!()
    }

    /// What changed since the comments of round `round` of `review` were sent: the paths
    /// that differ between the state recorded at that send and the state of `worktree`,
    /// the worktree of the review's workspace, recorded now, renames paired as git pairs
    /// them. Refused with [`crate::ReviewError::UnknownRound`] for a round the review does
    /// not hold or whose state is not recorded yet.
    pub fn changed_since_round(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
        round: u32,
    ) -> Result<Vec<ChangedPath>, EngineError> {
        let _ = (worktree, review, round);
        todo!()
    }
}
