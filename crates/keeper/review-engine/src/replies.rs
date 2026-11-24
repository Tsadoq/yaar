//! The agent's replies to threads, and resolving and reopening them, which only the
//! reviewer does.

use yaar_store::{CommentId, ReviewId, SessionId, ThreadId};

use crate::engine::{EngineError, Reviews, random_id};
use crate::review::Review;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadReply {
    pub thread: ThreadId,
    /// The reply's text, kept byte for byte.
    pub text: String,
    pub addressed: bool,
}

impl Reviews {
    /// Keeps a reply of the agent of `session` in that session's review, with a comment id the
    /// engine draws, in one transaction. Refused with nothing kept as
    /// [`crate::Review::agent_reply`] refuses, or with [`EngineError::NoReviewOfSession`].
    pub fn agent_replied(
        &self,
        session: &SessionId,
        reply: &ThreadReply,
    ) -> Result<(ReviewId, CommentId), EngineError> {
        let _turn = self.turn();
        let kept = self
            .store
            .session(session)?
            .ok_or_else(|| EngineError::NoReviewOfSession(session.clone()))?;
        let record = self
            .store
            .reviews_of(&kept.workspace)?
            .into_iter()
            .find(|review| review.agent.as_deref() == Some(session.as_str()))
            .ok_or_else(|| EngineError::NoReviewOfSession(session.clone()))?;
        let mut review = Review::from_record(record);
        let comment = CommentId::new(random_id()?);
        review.agent_reply(
            session,
            &reply.thread,
            comment.clone(),
            &reply.text,
            reply.addressed,
        )?;
        let (_, made) = review.comment(&comment).expect("the reply was made");
        self.store.put_reply(&reply.thread, made, reply.addressed)?;
        Ok((review.id().clone(), comment))
    }

    pub fn resolve(&self, review: &ReviewId, thread: &ThreadId) -> Result<(), EngineError> {
        let _turn = self.turn();
        self.review(review)?.resolve(thread)?;
        self.store.set_resolved(thread, true)?;
        Ok(())
    }

    pub fn reopen(&self, review: &ReviewId, thread: &ThreadId) -> Result<(), EngineError> {
        let _turn = self.turn();
        self.review(review)?.reopen(thread)?;
        self.store.set_resolved(thread, false)?;
        Ok(())
    }

    /// In one transaction; gives the threads resolved.
    pub fn resolve_addressed(&self, review: &ReviewId) -> Result<Vec<ThreadId>, EngineError> {
        let _turn = self.turn();
        let resolved = self.review(review)?.resolve_addressed();
        if !resolved.is_empty() {
            self.store.resolve_threads(&resolved)?;
        }
        Ok(resolved)
    }
}
