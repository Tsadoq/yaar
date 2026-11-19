//! Attaching a review to the user's next message, detaching it, and the session a CLI
//! started for a review takes it. Built by the engine's package "attaching and sending" of
//! slice 6.

use yaar_store::{ReviewId, SessionRecord};

use crate::engine::{EngineError, Reviews};
use crate::review::{Attachment, Offer, SessionNow};

impl Reviews {
    /// Attaches `review`, or one of its threads alone, by [`crate::Review::attach_with`],
    /// and keeps what is attached in the Store under the engine's turn; gives what to
    /// offer. `sessions` are the sessions of the review's workspace as the Keeper knows
    /// them now. Refused as `attach_with` refuses, and with [`EngineError::UnknownReview`];
    /// nothing is kept then.
    pub fn attach(
        &self,
        review: &ReviewId,
        attachment: Attachment,
        sessions: &[SessionNow],
    ) -> Result<Offer, EngineError> {
        let _ = (review, attachment, sessions);
        todo!()
    }

    /// Takes the attachment of `review` away, and keeps that; every comment it would have
    /// carried stays an unsent draft. Nothing attached is no error.
    pub fn detach(&self, review: &ReviewId) -> Result<(), EngineError> {
        let _ = review;
        todo!()
    }

    /// `session` starts in a terminal the app opened for `review`, the launcher of a CLI
    /// offered when the review was attached: the session takes that review, as the first
    /// rule of a session that starts, before those of [`crate::takeover`], as long as the
    /// review belongs to no agent or to a session of the same CLI that does not run; else
    /// it is as [`Reviews::session_started`]. Kept under the engine's turn before this
    /// returns; gives the session's review, whose attachment stays as it was.
    pub fn session_started_for(
        &self,
        session: &SessionRecord,
        review: &ReviewId,
    ) -> Result<ReviewId, EngineError> {
        let _ = (session, review);
        todo!()
    }
}
