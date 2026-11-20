//! The one general send of a review, whatever way it leaves, and its two uses: what is
//! attached, riding with a message of the user, whose round's state is recorded after it;
//! and the copy as text. Built by the engine's package "attaching and sending" of slice 6.
//!
//! A way of leaving is a `yaar_agent::Delivery`, a delivery module: the engine renders the
//! text after its instruction, hands the text to it, and keeps the send once it succeeded.
//! The engine names no way: Claude Code's hook, pi's extension and a copy are each handed
//! the same text, after their own instruction.

use yaar_agent::{CopyAsText, Delivery, Rendered};
use yaar_recorder::{StateId, Worktree};
use yaar_store::{ReviewId, SentBy, SessionId};

use crate::engine::{Copied, EngineError, Reviews};
use crate::review::{ReviewError, Scope, Sent};
use crate::text::{ReviewText, as_text};

impl Reviews {
    /// The review of `session`: the one the Store holds whose agent is that session; none
    /// for a session with no review, as one whose review the next session took over.
    pub fn review_of_session(&self, session: &SessionId) -> Result<Option<ReviewId>, EngineError> {
        // Read as one call of the Store each, and no turn is taken: a caller that holds the
        // turn may ask.
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

    /// What the user's next message would carry now: what is attached, rendered by
    /// [`crate::as_text`] after `instruction`, the instruction of the delivery module of
    /// the agent's CLI; none when nothing is attached or what is attached carries nothing.
    /// The text is the one [`Reviews::send_attached`] hands out, byte for byte, as long as
    /// the review does not change between the two.
    pub fn attached_text(
        &self,
        review: &ReviewId,
        instruction: &str,
    ) -> Result<Option<ReviewText>, EngineError> {
        let rendered = as_text(&self.review(review)?, Scope::Attached, instruction);
        Ok((!rendered.comments.is_empty()).then_some(rendered))
    }

    /// The one send: under the engine's turn, renders what `scope` of `review` carries after
    /// `delivery`'s instruction ([`crate::as_text`]); hands it to `delivery` as a
    /// `yaar_agent::Rendered` with the number the round will have and the count of its
    /// threads; and, once that succeeded, sends those comments in that round, sent by
    /// `sent_by`, keeping it all in one transaction of the Store: the round, its comments,
    /// the addressed marks it takes away, and, for [`Scope::Attached`], the attachment it
    /// takes away (`Store::put_round_detached`; `Store::put_round` for another scope). The
    /// round is kept at `state`, or, with none, with no state, which
    /// [`Reviews::record_round`] gives it after; only [`Scope::Attached`] is sent with none
    /// ([`crate::Review::send_attached`]), and another scope with none is refused with
    /// [`ReviewError::NothingToSend`]. Gives the bytes `delivery` gave and the send. None,
    /// with nothing handed out and nothing changed, when the scope carries nothing; refused
    /// with [`EngineError::HandOut`], nothing sent, when `delivery` failed.
    pub fn send(
        &self,
        review: &ReviewId,
        scope: Scope,
        sent_by: SentBy,
        state: Option<&StateId>,
        delivery: &mut dyn Delivery,
    ) -> Result<Option<(Vec<u8>, Sent)>, EngineError> {
        // A send of everything is always at a state; only what is attached is sent before
        // its state is recorded.
        if state.is_none() && scope != Scope::Attached {
            return Err(ReviewError::NothingToSend.into());
        }
        let _turn = self.turn();
        let mut current = self.review(review)?;
        let rendered = as_text(&current, scope, delivery.instruction());
        if rendered.comments.is_empty() {
            return Ok(None);
        }
        // The send is made on this copy first, so that the round has its number to hand out;
        // nothing is kept before the hand-out succeeded.
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

    /// The user submitted a message that carries `review`'s attachment: [`Reviews::send`]
    /// of [`Scope::Attached`] with no state, through `delivery`, the delivery of the CLI of
    /// the review's agent for that message. No state is recorded here:
    /// [`Reviews::record_round`] records it, off the agent's way.
    pub fn send_attached(
        &self,
        review: &ReviewId,
        sent_by: SentBy,
        delivery: &mut dyn Delivery,
    ) -> Result<Option<(Vec<u8>, Sent)>, EngineError> {
        self.send(review, Scope::Attached, sent_by, None, delivery)
    }

    /// Records the state of `worktree`, the worktree of `review`'s workspace, as the state
    /// of round `round`, which was sent with none, and keeps it; gives the state. The
    /// threads of the workspace are carried to it as to any state the engine takes.
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
        // The recording can take a second: no turn is held while it runs.
        let state = self.record_state(before.workspace(), worktree)?.id;

        let _turn = self.turn();
        let mut current = self.review(review)?;
        current.set_round_state(round, state.as_str())?;
        self.store.set_round_state(review, round, state.as_str())?;
        Ok(state)
    }

    /// Copies a review as text and sends it, as a submit does: the text holds every unsent
    /// comment on an unresolved thread, after `yaar_agent::COPY_INSTRUCTION`; the state of
    /// `worktree`, the worktree of the review's workspace, is recorded; the copied comments
    /// are sent in a new round, recorded at that state, in one transaction of the Store.
    /// Refused, with nothing recorded, when there is nothing to send. It is the state
    /// recorded and then [`Reviews::send`] of [`Scope::Everything`] through
    /// `yaar_agent::CopyAsText`.
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
