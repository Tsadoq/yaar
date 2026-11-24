use yaar_agent::{CopyAsText, Delivery, Rendered};
use yaar_recorder::{StateId, Worktree};
use yaar_store::{ReviewId, SentBy, SessionId};

use crate::engine::{Copied, EngineError, Reviews};
use crate::review::{ReviewError, Scope, Sent};
use crate::text::{ReviewText, as_text};

impl Reviews {
    /// None for a session with no review, e.g. one whose review the next session took over.
    pub fn review_of_session(&self, session: &SessionId) -> Result<Option<ReviewId>, EngineError> {
        // Each read is one call of the Store and no turn is taken: a caller that holds the turn may
        // ask.
        let Some(kept) = self.store.session(session)? else {
            return Ok(None);
        };
        Ok(self
            .store
            .reviews_of(&kept.workspace)?
            .into_iter()
            .find(|review| review.agent.as_deref() == Some(session.as_str()))
            .map(|review| review.id))
    }

    /// None when nothing is attached or it carries nothing. Byte for byte the text
    /// [`Reviews::send_attached`] hands out, if the review does not change between the two.
    pub fn attached_text(
        &self,
        review: &ReviewId,
        instruction: &str,
    ) -> Result<Option<ReviewText>, EngineError> {
        let rendered = as_text(&self.review(review)?, Scope::Attached, instruction);
        Ok((!rendered.comments.is_empty()).then_some(rendered))
    }

    /// Only `Attached` may be sent with no state (given later by [`Reviews::record_round`]).
    /// Nothing is kept unless the hand-out succeeded: [`EngineError::HandOut`] means nothing was
    /// sent; None when the scope carries nothing.
    pub fn send(
        &self,
        review: &ReviewId,
        scope: Scope,
        sent_by: SentBy,
        state: Option<&StateId>,
        delivery: &mut dyn Delivery,
    ) -> Result<Option<(Vec<u8>, Sent)>, EngineError> {
        if state.is_none() && scope != Scope::Attached {
            return Err(ReviewError::NothingToSend.into());
        }
        let _turn = self.turn();
        let mut current = self.review(review)?;
        let rendered = as_text(&current, scope, delivery.instruction());
        if rendered.comments.is_empty() {
            return Ok(None);
        }
        // Made on this copy first, so the round has its number to hand out.
        let sent = match state {
            Some(state) => current.send(scope, state.as_str(), sent_by)?,
            None => current.send_attached(sent_by)?,
        };
        let handed = delivery
            .hand_out(&Rendered {
                text: &rendered.text,
                review: review.as_str(),
                round: sent.round.number,
                threads: rendered.threads.len(),
            })
            .map_err(EngineError::HandOut)?;
        match scope {
            Scope::Attached => {
                self.store
                    .put_round_detached(review, &sent.round, &sent.comments)?
            }
            Scope::Everything => self.store.put_round(review, &sent.round, &sent.comments)?,
        }
        Ok(Some((handed, sent)))
    }

    /// Sends with no state; [`Reviews::record_round`] records it, off the agent's way.
    pub fn send_attached(
        &self,
        review: &ReviewId,
        sent_by: SentBy,
        delivery: &mut dyn Delivery,
    ) -> Result<Option<(Vec<u8>, Sent)>, EngineError> {
        self.send(review, Scope::Attached, sent_by, None, delivery)
    }

    /// Gives the state; the threads follow the code to it.
    pub fn record_round(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
        round: u32,
    ) -> Result<StateId, EngineError> {
        // A round that cannot be given a state records none.
        let before = self.review(review)?;
        match before.rounds().iter().find(|held| held.number == round) {
            None => return Err(ReviewError::UnknownRound(round).into()),
            Some(held) if held.state.is_some() => {
                return Err(ReviewError::RoundHasState(round).into());
            }
            Some(_) => {}
        }
        let state = self.record_state(before.workspace(), worktree)?.id;

        let _turn = self.turn();
        let mut current = self.review(review)?;
        current.set_round_state(round, state.as_str())?;
        self.store.set_round_state(review, round, state.as_str())?;
        Ok(state)
    }

    /// Records the state of `worktree`, then sends the unsent comments as a new round at that
    /// state, in one transaction. Nothing is recorded when there is nothing to send.
    pub fn copy_as_text(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
    ) -> Result<Copied, EngineError> {
        let before = self.review(review)?;
        if before.unsent(Scope::Everything).is_empty() {
            return Err(ReviewError::NothingToSend.into());
        }
        let state = self.record_state(before.workspace(), worktree)?;
        let (handed, sent) = self
            .send(
                review,
                Scope::Everything,
                SentBy::Copied,
                Some(&state.id),
                &mut CopyAsText,
            )?
            .ok_or(ReviewError::NothingToSend)?;
        Ok(Copied {
            text: String::from_utf8(handed).expect("a copy hands back the text it was given"),
            round: sent.round.number,
        })
    }
}
