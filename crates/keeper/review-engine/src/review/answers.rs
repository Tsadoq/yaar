//! The agent's replies, the states of a thread, and resolving every addressed thread at
//! once. Built by the engine's package "replies and the states of a thread" of slice 6.

use yaar_store::{Author, Comment, CommentId, SessionId, ThreadId};

use super::{Review, ReviewError};

/// The state of a thread, as the review pane lists it: the first of these that holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadState {
    /// The reviewer resolved it.
    Resolved,
    /// It holds a comment of the reviewer that was not sent.
    Draft,
    /// The agent's last reply marked it addressed.
    Addressed,
    /// The agent replied last, without the addressed mark: it waits for the reviewer.
    WaitsForReviewer,
    /// Its comments were sent, and the agent has not replied since.
    Sent,
}

impl Review {
    /// The state of a thread, by [`ThreadState`]'s order; none for a thread the review
    /// does not hold.
    pub fn state_of(&self, thread: &ThreadId) -> Option<ThreadState> {
        let held = self.thread(thread)?;
        Some(if held.resolved {
            ThreadState::Resolved
        } else if held.comments.iter().any(|c| c.round.is_none()) {
            ThreadState::Draft
        } else if held.addressed {
            ThreadState::Addressed
        } else if held
            .comments
            .last()
            .is_some_and(|c| c.author == Author::Agent)
        {
            ThreadState::WaitsForReviewer
        } else {
            ThreadState::Sent
        })
    }

    /// Keeps a reply of the agent of `session` in a thread: a comment of the agent, sent
    /// in the newest round, and the thread addressed or not as the reply says. A reply
    /// without the mark takes an earlier mark away: the thread waits for the reviewer. A
    /// reply never resolves a thread, and leaves a resolved one resolved. Refused with
    /// [`ReviewError::OtherSession`] when the review does not belong to `session`,
    /// [`ReviewError::UnknownThread`] for a thread the review does not hold, a thread of
    /// another session's review among them, [`ReviewError::NothingSent`] when no round was
    /// sent, and [`ReviewError::TakenId`]; nothing changes then.
    pub fn agent_reply(
        &mut self,
        session: &SessionId,
        thread: &ThreadId,
        comment: CommentId,
        text: &str,
        addressed: bool,
    ) -> Result<(), ReviewError> {
        if self.agent() != Some(session.as_str()) {
            return Err(ReviewError::OtherSession(session.clone()));
        }
        if self.thread(thread).is_none() {
            return Err(ReviewError::UnknownThread(thread.clone()));
        }
        let Some(round) = self.record.rounds.last().map(|round| round.number) else {
            return Err(ReviewError::NothingSent(thread.clone()));
        };
        self.check_free(comment.as_str())?;
        let held = self.thread_mut(thread)?;
        held.comments.push(Comment {
            id: comment,
            author: Author::Agent,
            text: text.to_string(),
            round: Some(round),
        });
        held.addressed = addressed;
        Ok(())
    }

    /// Resolves every thread in the state [`ThreadState::Addressed`], and gives them, in
    /// the order of the threads: the threads the agent marked addressed, not resolved, and
    /// with no draft of the reviewer's, which the pane lists as a draft and which a
    /// resolve would take out of the next send. The reviewer's action: nothing else
    /// resolves.
    pub fn resolve_addressed(&mut self) -> Vec<ThreadId> {
        let addressed: Vec<ThreadId> = self
            .threads()
            .iter()
            .filter(|thread| self.state_of(&thread.id) == Some(ThreadState::Addressed))
            .map(|thread| thread.id.clone())
            .collect();
        for thread in &addressed {
            self.thread_mut(thread)
                .expect("the thread was found above")
                .resolved = true;
        }
        addressed
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use yaar_store::{Anchor, Author, ReviewId, SentBy, Side, WorkspaceId};

    use super::*;
    use crate::review::Scope;

    fn id(n: u32) -> String {
        format!("{n:032x}")
    }

    fn thread(n: u32) -> ThreadId {
        ThreadId::new(id(n))
    }

    fn comment(n: u32) -> CommentId {
        CommentId::new(id(n))
    }

    fn session(name: &str) -> SessionId {
        SessionId::new(name)
    }

    // A review of session `agent` with threads 1 to `count`, each holding one comment of the
    // reviewer.
    fn review_of(agent: &str, count: u32) -> Review {
        let mut review = Review::new(
            ReviewId::new(id(1000)),
            WorkspaceId::new("w"),
            Some(agent.to_string()),
        );
        for n in 1..=count {
            review
                .start_thread(
                    thread(n),
                    comment(n + 100),
                    Anchor {
                        tree: "S1".to_string(),
                        path: PathBuf::from("src/a.rs"),
                        side: Side::New,
                        first: n,
                        last: n,
                        text: "let x = 1;".to_string(),
                    },
                    "why?",
                )
                .expect("start the thread");
        }
        review
    }

    // The review of `agent` with `count` threads, all sent in round 1.
    fn sent(agent: &str, count: u32) -> Review {
        let mut review = review_of(agent, count);
        review
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send");
        review
    }

    fn reply(review: &mut Review, n: u32, as_comment: u32, addressed: bool) {
        review
            .agent_reply(
                &session("A"),
                &thread(n),
                comment(as_comment),
                "done",
                addressed,
            )
            .expect("the agent replies");
    }

    fn state(review: &Review, n: u32) -> ThreadState {
        review.state_of(&thread(n)).expect("the thread is held")
    }

    fn held(review: &Review, n: u32) -> &yaar_store::Thread {
        review.thread(&thread(n)).expect("the thread is held")
    }

    // REPLY-03
    #[test]
    fn a_reply_without_the_mark_waits_for_the_reviewer_and_with_it_is_addressed_and_only_the_reviewer_resolves()
     {
        let mut review = sent("A", 2);

        // A reply without the addressed mark leaves the thread open, waiting.
        reply(&mut review, 1, 201, false);
        assert_eq!(state(&review, 1), ThreadState::WaitsForReviewer);
        assert!(!held(&review, 1).resolved);
        assert!(!held(&review, 1).addressed);

        // A reply with it makes the thread addressed and not resolved.
        reply(&mut review, 2, 202, true);
        assert_eq!(state(&review, 2), ThreadState::Addressed);
        assert!(!held(&review, 2).resolved);
        assert!(held(&review, 2).addressed);

        // The reply is the agent's comment, sent in the round the review is in.
        let (_, kept) = review.comment(&comment(201)).expect("the reply is kept");
        assert_eq!(kept.author, Author::Agent);
        assert_eq!(kept.round, Some(1));
        assert_eq!(kept.text, "done");

        // No reply resolves: the second reply without the mark takes the mark away.
        reply(&mut review, 2, 203, false);
        assert_eq!(state(&review, 2), ThreadState::WaitsForReviewer);
        assert!(!held(&review, 2).addressed);
        assert!(!held(&review, 2).resolved);

        // The reviewer's action resolves.
        review.resolve(&thread(2)).expect("resolve");
        assert_eq!(state(&review, 2), ThreadState::Resolved);

        // A reply with the mark to a resolved thread leaves it resolved, and so does one
        // without.
        reply(&mut review, 2, 204, true);
        assert_eq!(state(&review, 2), ThreadState::Resolved);
        reply(&mut review, 2, 205, false);
        assert_eq!(state(&review, 2), ThreadState::Resolved);
        assert_eq!(state(&review, 1), ThreadState::WaitsForReviewer);
    }

    // REPLY-04
    #[test]
    fn a_reply_that_names_a_thread_of_another_sessions_review_is_refused_and_changes_nothing() {
        let mut mine = sent("A", 1);
        // The other review holds a thread of its own, number 2.
        let mut other = Review::new(
            ReviewId::new(id(1001)),
            WorkspaceId::new("w"),
            Some("B".to_string()),
        );
        other
            .start_thread(
                thread(2),
                comment(102),
                Anchor {
                    tree: "S1".to_string(),
                    path: PathBuf::from("src/b.rs"),
                    side: Side::New,
                    first: 1,
                    last: 1,
                    text: "let y = 2;".to_string(),
                },
                "and this?",
            )
            .expect("start the thread");
        other
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send");
        let (mine_before, other_before) = (mine.clone(), other.clone());

        // A's reply to B's thread: the review of A does not hold it.
        assert_eq!(
            mine.agent_reply(&session("A"), &thread(2), comment(301), "no", true),
            Err(ReviewError::UnknownThread(thread(2)))
        );
        // B's reply to A's thread: the review is not B's.
        assert_eq!(
            mine.agent_reply(&session("B"), &thread(1), comment(302), "no", true),
            Err(ReviewError::OtherSession(session("B")))
        );
        assert_eq!(
            other.agent_reply(&session("A"), &thread(1), comment(303), "no", true),
            Err(ReviewError::OtherSession(session("A")))
        );
        assert_eq!(mine, mine_before);
        assert_eq!(other, other_before);
    }

    #[test]
    fn a_review_of_no_agent_takes_no_reply() {
        let mut review = Review::new(ReviewId::new(id(1000)), WorkspaceId::new("w"), None);
        assert_eq!(
            review.agent_reply(&session("A"), &thread(1), comment(301), "x", true),
            Err(ReviewError::OtherSession(session("A")))
        );
    }

    #[test]
    fn a_reply_is_refused_before_any_send_and_for_a_taken_id_and_changes_nothing() {
        let mut unsent = review_of("A", 1);
        let before = unsent.clone();
        assert_eq!(
            unsent.agent_reply(&session("A"), &thread(1), comment(301), "x", true),
            Err(ReviewError::NothingSent(thread(1)))
        );
        assert_eq!(unsent, before);

        let mut review = sent("A", 1);
        let before = review.clone();
        // The id of the reviewer's own comment, and the id of the thread.
        for taken in [comment(101), comment(1)] {
            let id = taken.to_string();
            assert_eq!(
                review.agent_reply(&session("A"), &thread(1), taken, "x", true),
                Err(ReviewError::TakenId(id))
            );
        }
        assert_eq!(review, before);
    }

    #[test]
    fn a_reply_goes_in_the_newest_round() {
        let mut review = sent("A", 1);
        review
            .reply(&thread(1), comment(150), "and again")
            .expect("a follow-up");
        review
            .send(Scope::Everything, "S2", SentBy::Delivered)
            .expect("send the follow-up");
        reply(&mut review, 1, 201, true);
        let (_, kept) = review.comment(&comment(201)).expect("the reply is kept");
        assert_eq!(kept.round, Some(2));
    }

    #[test]
    fn a_thread_is_in_the_first_state_that_holds_and_a_thread_not_held_has_none() {
        let mut review = sent("A", 1);
        assert_eq!(state(&review, 1), ThreadState::Sent);

        reply(&mut review, 1, 201, true);
        // An unsent comment of the reviewer comes before the agent's mark.
        review
            .reply(&thread(1), comment(150), "one more thing")
            .expect("a follow-up");
        assert_eq!(state(&review, 1), ThreadState::Draft);

        review.resolve(&thread(1)).expect("resolve");
        assert_eq!(state(&review, 1), ThreadState::Resolved);
        assert_eq!(review.state_of(&thread(9)), None);
    }

    #[test]
    fn a_thread_that_holds_only_a_draft_is_a_draft() {
        let review = review_of("A", 1);
        assert_eq!(state(&review, 1), ThreadState::Draft);
    }

    // REV-13
    #[test]
    fn a_resolved_thread_can_be_reopened_and_one_action_resolves_every_addressed_thread() {
        let mut review = sent("A", 6);

        // T1 is resolved and reopened: it is as it was before, sent.
        review.resolve(&thread(1)).expect("resolve");
        assert_eq!(state(&review, 1), ThreadState::Resolved);
        review.reopen(&thread(1)).expect("reopen");
        assert!(!held(&review, 1).resolved);
        assert_eq!(state(&review, 1), ThreadState::Sent);

        // T2 and T3 addressed, T4 waits for the reviewer, T5 sent, T6 resolved.
        reply(&mut review, 2, 202, true);
        reply(&mut review, 3, 203, true);
        reply(&mut review, 4, 204, false);
        review.resolve(&thread(6)).expect("resolve");

        assert_eq!(review.resolve_addressed(), vec![thread(2), thread(3)]);
        assert_eq!(state(&review, 2), ThreadState::Resolved);
        assert_eq!(state(&review, 3), ThreadState::Resolved);
        assert_eq!(state(&review, 4), ThreadState::WaitsForReviewer);
        assert_eq!(state(&review, 5), ThreadState::Sent);
        assert_eq!(state(&review, 6), ThreadState::Resolved);

        // A second call finds none.
        let before = review.clone();
        assert_eq!(review.resolve_addressed(), Vec::<ThreadId>::new());
        assert_eq!(review, before);
    }

    #[test]
    fn a_thread_reopened_after_it_was_addressed_is_addressed_again() {
        let mut review = sent("A", 1);
        reply(&mut review, 1, 201, true);
        review.resolve(&thread(1)).expect("resolve");
        review.reopen(&thread(1)).expect("reopen");
        assert_eq!(state(&review, 1), ThreadState::Addressed);
    }

    #[test]
    fn resolving_every_addressed_thread_leaves_one_with_a_draft_of_the_reviewer_alone() {
        let mut review = sent("A", 2);
        reply(&mut review, 1, 201, true);
        reply(&mut review, 2, 202, true);
        // The reviewer started a follow-up in thread 2: it shows as a draft, and resolving
        // it would take the draft out of the next send.
        review
            .reply(&thread(2), comment(150), "not yet")
            .expect("a follow-up");
        assert_eq!(review.resolve_addressed(), vec![thread(1)]);
        assert_eq!(state(&review, 2), ThreadState::Draft);
    }

    #[test]
    fn an_unknown_thread_cannot_be_reopened() {
        let mut review = sent("A", 1);
        assert_eq!(
            review.reopen(&thread(9)),
            Err(ReviewError::UnknownThread(thread(9)))
        );
    }
}
