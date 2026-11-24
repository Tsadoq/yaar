use yaar_recorder::{ChangedPath, Worktree};
use yaar_store::{ReviewId, SessionId};

use crate::engine::{EngineError, Reviews};
use crate::review::{Review, ReviewError};

impl Reviews {
    /// The newest round of the session's review counts as edited from now on. Gives the review and
    /// round when one changed.
    pub fn agent_edited(
        &self,
        session: &SessionId,
    ) -> Result<Option<(ReviewId, u32)>, EngineError> {
        let _turn = self.turn();
        let Some(kept) = self.store.session(session)? else {
            return Ok(None);
        };
        let Some(record) = self
            .store
            .reviews_of(&kept.workspace)?
            .into_iter()
            .find(|review| review.agent.as_deref() == Some(session.as_str()))
        else {
            return Ok(None);
        };
        let mut review = Review::from_record(record);
        let Some(round) = review.agent_edited() else {
            return Ok(None);
        };
        self.store.set_round_edited(review.id(), round)?;
        Ok(Some((review.id().clone(), round)))
    }

    /// Refused with [`crate::ReviewError::UnknownRound`] for an unknown round or one with no state
    /// yet. Records a state, so the threads follow the code to it.
    pub fn changed_since_round(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
        round: u32,
    ) -> Result<Vec<ChangedPath>, EngineError> {
        let current = self.review(review)?;
        let sent = current
            .rounds()
            .iter()
            .find(|held| held.number == round)
            .and_then(|held| held.state.clone())
            .ok_or(ReviewError::UnknownRound(round))?;
        let states = self.states_of(current.workspace())?;
        let now = self.record_state(current.workspace(), worktree)?;
        Ok(states.changed_paths(&sent, now.id.as_str())?)
    }
}
