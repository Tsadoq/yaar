//! Attaching a review to the user's next message, detaching it, and the session a CLI
//! started for a review takes it. Built by the engine's package "attaching and sending" of
//! slice 6.

use yaar_store::{AttachedRecord, ReviewId, ReviewRecord, SessionRecord};

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
        let kept = match &attachment {
            Attachment::Review => AttachedRecord::Review,
            Attachment::Thread(thread) => AttachedRecord::Thread(thread.clone()),
        };
        let _turn = self.turn();
        let mut current = self.review(review)?;
        let offer = current.attach_with(attachment, sessions)?;
        self.store.set_attached(review, Some(&kept))?;
        Ok(offer)
    }

    /// Takes the attachment of `review` away, and keeps that; every comment it would have
    /// carried stays an unsent draft. Nothing attached is no error.
    pub fn detach(&self, review: &ReviewId) -> Result<(), EngineError> {
        let _turn = self.turn();
        // A review that is not kept is refused, not detached.
        self.review(review)?;
        self.store.set_attached(review, None)?;
        Ok(())
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
        // The turn is let go before the general rule takes it: it is not reentrant.
        if self.takes_for(session, review)? {
            return Ok(review.clone());
        }
        self.session_started(session)
    }

    // Whether `session` takes `review`, the review its terminal was opened for, and has
    // taken it: kept, under the turn, before this returns.
    fn takes_for(&self, session: &SessionRecord, review: &ReviewId) -> Result<bool, EngineError> {
        let _turn = self.turn();
        let Some(bound) = self.store.review(review)? else {
            return Ok(false);
        };
        let reviews = self.store.reviews_of(&session.workspace)?;
        // A session holds one review: the one it has already is the one it keeps.
        let holds_one = reviews
            .iter()
            .any(|held| held.agent.as_deref() == Some(session.id.as_str()));
        if bound.workspace != session.workspace || holds_one {
            return Ok(false);
        }
        let sessions = self.store.sessions_of(&session.workspace)?;
        if !free_to_take(&bound, &sessions, session) {
            return Ok(false);
        }
        self.store.set_review_agent(review, Some(&session.id))?;
        Ok(true)
    }
}

// Whether `review` may be taken by a session of the CLI `taker` runs: it belongs to no
// agent, or to a session of that CLI that does not run. A session the Store does not list
// does not run, and its CLI is not known to differ: the offer made for such a review was
// that of every CLI, and taking it is what accepting the offer means.
fn free_to_take(review: &ReviewRecord, sessions: &[SessionRecord], taker: &SessionRecord) -> bool {
    let Some(holder) = review.agent.as_deref() else {
        return true;
    };
    sessions
        .iter()
        .find(|held| held.id.as_str() == holder)
        .is_none_or(|held| !held.running && held.agent == taker.agent)
}
