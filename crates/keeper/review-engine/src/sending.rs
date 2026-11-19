//! A review that rides with a message of the user: what it carries, the one general send
//! that hands it out and starts its round, and the state recorded for that round after it.
//! Built by the engine's package "attaching and sending" of slice 6.

use yaar_recorder::{StateId, Worktree};
use yaar_store::{ReviewId, SentBy, SessionId};

use crate::engine::{EngineError, Reviews};
use crate::review::Sent;
use crate::text::ReviewText;

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

    /// The user submitted a message that carries `review`'s attachment: under the engine's
    /// turn, renders what is attached after `instruction`, gives it to `hand_out` with the
    /// number the round will have, and, once `hand_out` succeeded, sends those comments in
    /// that round with no state yet, sent by `sent_by`, takes the attachment away and keeps
    /// all of it in one transaction of the Store. None, with nothing handed out and nothing
    /// changed, when nothing is attached or it carries nothing; refused with
    /// [`EngineError::HandOut`], nothing sent, when `hand_out` failed. No state is
    /// recorded here: [`Reviews::record_round`] records it, off the agent's way.
    pub fn send_attached<T>(
        &self,
        review: &ReviewId,
        sent_by: SentBy,
        instruction: &str,
        hand_out: impl FnOnce(&ReviewText, u32) -> std::io::Result<T>,
    ) -> Result<Option<(T, Sent)>, EngineError> {
        let _ = (review, sent_by, instruction, hand_out);
        todo!()
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
}
