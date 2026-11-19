//! The one general send of a review, whatever way it leaves, and its two uses: what is
//! attached, riding with a message of the user, whose round's state is recorded after it;
//! and the copy as text. Built by the engine's package "attaching and sending" of slice 6.
//!
//! A way of leaving is a `yaar_agent::Delivery`, a delivery module: the engine renders the
//! text after its instruction, hands the text to it, and keeps the send once it succeeded.
//! The engine names no way: Claude Code's hook, pi's extension and a copy are each handed
//! the same text, after their own instruction.

use yaar_agent::{CopyAsText, Delivery};
use yaar_recorder::{StateId, Worktree};
use yaar_store::{ReviewId, SentBy, SessionId};

use crate::engine::{Copied, EngineError, Reviews};
use crate::review::{ReviewError, Scope, Sent};
use crate::text::{ReviewText, as_text};

impl Reviews {
    /// The review of `session`: the one the Store holds whose agent is that session; none
    /// for a session with no review, as one whose review the next session took over.
    pub fn review_of_session(&self, session: &SessionId) -> Result<Option<ReviewId>, EngineError> {
        let _ = session;
        todo!()
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
        let _ = (review, instruction);
        todo!()
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
        let _ = (review, scope, sent_by, state, delivery);
        todo!()
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
        let _ = (worktree, review, round);
        todo!()
    }

    /// Copies a review as text and sends it, as a submit does: the text holds every unsent
    /// comment on an unresolved thread, after `yaar_agent::COPY_INSTRUCTION`; the state of
    /// `worktree`, the worktree of the review's workspace, is recorded; the copied comments
    /// are sent in a new round, recorded at that state, in one transaction of the Store.
    /// Refused, with nothing recorded, when there is nothing to send. The package that
    /// builds [`Reviews::send`] makes this the state recorded and then that send, of
    /// [`Scope::Everything`] through `yaar_agent::CopyAsText`; what it does stays as it is.
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

        let _turn = self.turn();
        let mut current = self.review(review)?;
        let mut copy = CopyAsText;
        let rendered = as_text(&current, Scope::Everything, copy.instruction());
        if rendered.comments.is_empty() {
            return Err(ReviewError::NothingToSend.into());
        }
        let sent = current.send(Scope::Everything, state.id.as_str(), SentBy::Copied)?;
        let handed = copy
            .hand_out(&yaar_agent::Rendered {
                text: &rendered.text,
                review: review.as_str(),
                round: sent.round.number,
                threads: rendered.threads.len(),
            })
            .map_err(EngineError::HandOut)?;
        self.store.put_round(review, &sent.round, &sent.comments)?;
        Ok(Copied {
            text: String::from_utf8(handed).expect("a copy hands back the text it was given"),
            round: sent.round.number,
        })
    }
}
