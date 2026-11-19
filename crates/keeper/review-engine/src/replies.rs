//! The agent's replies to threads, and what only the reviewer does: resolving a thread,
//! reopening one, and resolving every addressed thread at once. Built by the engine's
//! package "replies and the states of a thread" of slice 6.

use yaar_store::{CommentId, ReviewId, SessionId, ThreadId};

use crate::engine::{EngineError, Reviews, random_id};
use crate::review::Review;

/// A reply of the agent to one thread, as its reply tool gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadReply {
    /// The thread, by its id.
    pub thread: ThreadId,
    /// The reply, byte for byte.
    pub text: String,
    /// Whether the agent says it addressed the thread.
    pub addressed: bool,
}

impl Reviews {
    /// Keeps a reply of the agent of `session` in the review of that session, by
    /// [`crate::Review::agent_reply`], with a comment id the engine draws, in one
    /// transaction of the Store; gives the review and the comment. Refused, with nothing
    /// kept, as `agent_reply` refuses, a thread of another session's review among them,
    /// and with [`EngineError::NoReviewOfSession`] for a session that has no review.
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
        // The review of the session is found as `turn_ended` finds it.
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

    /// Resolves a thread of `review`, and keeps it.
    pub fn resolve(&self, review: &ReviewId, thread: &ThreadId) -> Result<(), EngineError> {
        let _turn = self.turn();
        self.review(review)?.resolve(thread)?;
        self.store.set_resolved(thread, true)?;
        Ok(())
    }

    /// Reopens a resolved thread of `review`, and keeps it.
    pub fn reopen(&self, review: &ReviewId, thread: &ThreadId) -> Result<(), EngineError> {
        let _turn = self.turn();
        self.review(review)?.reopen(thread)?;
        self.store.set_resolved(thread, false)?;
        Ok(())
    }

    /// Resolves every addressed thread of `review` that is not resolved, by
    /// [`crate::Review::resolve_addressed`], in one transaction of the Store; gives them.
    pub fn resolve_addressed(&self, review: &ReviewId) -> Result<Vec<ThreadId>, EngineError> {
        let _turn = self.turn();
        let resolved = self.review(review)?.resolve_addressed();
        if !resolved.is_empty() {
            self.store.resolve_threads(&resolved)?;
        }
        Ok(resolved)
    }
}
