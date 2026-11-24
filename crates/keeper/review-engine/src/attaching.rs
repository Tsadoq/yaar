use yaar_store::{AttachedRecord, ReviewId, ReviewRecord, SessionRecord};

use crate::engine::{EngineError, Reviews};
use crate::review::{Attachment, Offer, SessionNow};

impl Reviews {
    /// Attaches `review`, or one thread of it, and keeps that in the Store. `sessions` are the
    /// workspace's sessions as the Keeper knows them now. Nothing is kept when refused.
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

    /// Every comment the attachment would have carried stays an unsent draft. Nothing attached is
    /// no error.
    pub fn detach(&self, review: &ReviewId) -> Result<(), EngineError> {
        let _turn = self.turn();
        // Refuses a review the Store does not hold instead of detaching nothing.
        self.review(review)?;
        self.store.set_attached(review, None)?;
        Ok(())
    }

    /// A session started in a terminal the app opened for `review` takes it, before the rules of
    /// [`crate::takeover`], if the review has no agent or one of the same CLI that does not run;
    /// else as [`Reviews::session_started`]. The attachment is left as it was.
    pub fn session_started_for(
        &self,
        session: &SessionRecord,
        review: &ReviewId,
    ) -> Result<ReviewId, EngineError> {
        // The turn is released before the general rule takes it: it is not reentrant.
        if self.takes_for(session, review)? {
            return Ok(review.clone());
        }
        self.session_started(session)
    }

    fn takes_for(&self, session: &SessionRecord, review: &ReviewId) -> Result<bool, EngineError> {
        let _turn = self.turn();
        let Some(bound) = self.store.review(review)? else {
            return Ok(false);
        };
        let reviews = self.store.reviews_of(&session.workspace)?;
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

// A session the Store does not list does not run. Its CLI is not known to differ: the offer
// made for such a review was that of every CLI.
fn free_to_take(review: &ReviewRecord, sessions: &[SessionRecord], taker: &SessionRecord) -> bool {
    let Some(holder) = review.agent.as_deref() else {
        return true;
    };
    sessions
        .iter()
        .find(|held| held.id.as_str() == holder)
        .is_none_or(|held| !held.running && held.agent == taker.agent)
}
