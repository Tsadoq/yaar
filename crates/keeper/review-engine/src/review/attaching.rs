//! Attaching a review, with the offer to start its agent, and a send whose state is
//! recorded after it. Built by the engine's package "attaching and sending" of slice 6.

use yaar_store::{SentBy, SessionId};

use super::{Attachment, Review, ReviewError, Scope, Sent};

/// One agent session of the review's workspace as it is now, as the Keeper knows it: what
/// attaching needs to know to offer to start an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionNow {
    /// The session, by the app's id for it.
    pub id: SessionId,
    /// The Adapter of its CLI, by its name: `claude`.
    pub cli: String,
    /// Whether it runs.
    pub running: bool,
    /// Whether it waits at a permission prompt.
    pub waits_for_permission: bool,
}

/// What attaching a review offers to start, so that the review rides with the first
/// message of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// Nothing: the agent the review belongs to runs, and its next message carries it.
    Nothing,
    /// The CLI of the agent the review belongs to, which does not run, whether or not
    /// another agent runs in the workspace.
    Start {
        /// The Adapter of that CLI, by its name.
        cli: String,
    },
    /// Any CLI the app supports: the review belongs to no agent.
    AnyCli,
}

impl Review {
    /// Attaches the review, or one thread alone, as [`Review::attach`] does, whatever the
    /// state of its agent, a permission prompt included, and says what to offer: see
    /// [`Offer`]. `sessions` are the sessions of the review's workspace, those that ended
    /// among them; the review's own is found among them by its id. A review whose session
    /// is not among them, whose CLI is then unknown, offers [`Offer::AnyCli`], as the
    /// review of no agent does. Refused with
    /// [`ReviewError::NothingToSend`], and nothing attached, when what would be attached
    /// holds no unsent comment on an unresolved thread; with
    /// [`ReviewError::UnknownThread`] for a thread the review does not hold.
    pub fn attach_with(
        &mut self,
        attachment: Attachment,
        sessions: &[SessionNow],
    ) -> Result<Offer, ReviewError> {
        // The refusal leaves what was attached as it was.
        let before = self.attached.clone();
        self.attach(attachment)?;
        if self.unsent(Scope::Attached).is_empty() {
            self.attached = before;
            return Err(ReviewError::NothingToSend);
        }
        Ok(self.offer(sessions))
    }

    /// Sends what is attached, as a message of the user carries it: what
    /// [`Review::unsent`] gives for [`Scope::Attached`] starts the next round, sent by
    /// `sent_by`, with no state yet, which [`Review::set_round_state`] gives it later; the
    /// attachment goes, and every thread one of whose comments it sends loses its addressed
    /// mark: the agent has a follow-up to answer. Refused with [`ReviewError::NothingToSend`], and nothing changed,
    /// when nothing is attached or what is attached carries nothing: the attachment then
    /// stays.
    pub fn send_attached(&mut self, sent_by: SentBy) -> Result<Sent, ReviewError> {
        // The general send names a state; this round has none yet, so the placeholder is
        // taken out again before anything else reads the round.
        let mut sent = self.send(Scope::Attached, "", sent_by)?;
        sent.round.state = None;
        if let Some(round) = self.record.rounds.last_mut() {
            round.state = None;
        }
        // The agent has a follow-up to answer in every thread a comment was sent in.
        for thread in &mut self.record.threads {
            if thread
                .comments
                .iter()
                .any(|c| sent.comments.contains(&c.id))
            {
                thread.addressed = false;
            }
        }
        Ok(sent)
    }

    /// Gives a round that was started with no state the state recorded for it. Refused
    /// with [`ReviewError::UnknownRound`] for a round the review does not hold, and with
    /// [`ReviewError::RoundHasState`] for one that has a state already.
    pub fn set_round_state(&mut self, round: u32, state: &str) -> Result<(), ReviewError> {
        let held = self
            .record
            .rounds
            .iter_mut()
            .find(|held| held.number == round)
            .ok_or(ReviewError::UnknownRound(round))?;
        if held.state.is_some() {
            return Err(ReviewError::RoundHasState(round));
        }
        held.state = Some(state.to_owned());
        Ok(())
    }

    // What attaching offers, given the sessions of the workspace as they are now.
    fn offer(&self, sessions: &[SessionNow]) -> Offer {
        let own = self
            .agent()
            .and_then(|agent| sessions.iter().find(|session| session.id.as_str() == agent));
        match own {
            None => Offer::AnyCli,
            Some(session) if session.running => Offer::Nothing,
            Some(session) => Offer::Start {
                cli: session.cli.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use yaar_store::{Anchor, CommentId, ReviewId, ThreadId, WorkspaceId};

    fn id(n: u32) -> String {
        format!("{n:032x}")
    }

    fn thread(n: u32) -> ThreadId {
        ThreadId::new(id(n))
    }

    fn comment(n: u32) -> CommentId {
        CommentId::new(id(n))
    }

    fn review_of(agent: Option<&str>) -> Review {
        Review::new(
            ReviewId::new(id(1000)),
            WorkspaceId::new("w"),
            agent.map(str::to_owned),
        )
    }

    // Starts thread `n` with its first comment `n + 100`, a draft.
    fn start(review: &mut Review, n: u32) {
        review
            .start_thread(
                thread(n),
                comment(n + 100),
                Anchor {
                    tree: "S0".to_owned(),
                    path: PathBuf::from("a.rs"),
                    side: yaar_store::Side::New,
                    first: n,
                    last: n,
                    text: "let x = 1;".to_owned(),
                },
                &format!("comment {n}"),
            )
            .expect("start the thread");
    }

    // A reply of the reviewer in thread `n`, with the comment id `n + 200`.
    fn reply(review: &mut Review, n: u32) {
        review
            .reply(&thread(n), comment(n + 200), &format!("reply {n}"))
            .expect("reply");
    }

    fn session(id: &str, cli: &str, running: bool, waits_for_permission: bool) -> SessionNow {
        SessionNow {
            id: SessionId::new(id),
            cli: cli.to_owned(),
            running,
            waits_for_permission,
        }
    }

    // The review of session A with T1 a draft, T2 sent and then a draft reply, T3 resolved
    // with a draft, T4 sent with no draft left.
    fn review_with_four_threads() -> Review {
        let mut review = review_of(Some("A"));
        start(&mut review, 2);
        start(&mut review, 4);
        review
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send T2 and T4");
        reply(&mut review, 2);
        start(&mut review, 1);
        start(&mut review, 3);
        review.resolve(&thread(3)).expect("resolve T3");
        review
    }

    // The unsent comments of a scope, as (thread, comment) pairs in order.
    fn carried(review: &Review, scope: Scope) -> Vec<(ThreadId, CommentId)> {
        review
            .unsent(scope)
            .iter()
            .flat_map(|unsent| {
                unsent
                    .comments
                    .iter()
                    .map(|c| (unsent.thread.id.clone(), c.id.clone()))
            })
            .collect()
    }

    fn rounds_of_comments(review: &Review) -> Vec<(CommentId, Option<u32>)> {
        review
            .threads()
            .iter()
            .flat_map(|thread| thread.comments.iter().map(|c| (c.id.clone(), c.round)))
            .collect()
    }

    // REV-06
    #[test]
    fn attaching_a_review_attaches_every_unsent_comment_on_an_unresolved_thread_and_after_a_detach_each_is_a_draft_and_the_next_message_carries_nothing()
     {
        let mut review = review_with_four_threads();
        let running = [session("A", "claude", true, false)];

        // Attaching attaches every unsent comment on an unresolved thread: T1's draft and
        // T2's reply, nothing of T3 (resolved) or T4 (nothing unsent).
        assert_eq!(
            review.attach_with(Attachment::Review, &running),
            Ok(Offer::Nothing)
        );
        assert_eq!(review.attached(), Some(&Attachment::Review));
        assert_eq!(
            carried(&review, Scope::Attached),
            vec![(thread(2), comment(202)), (thread(1), comment(101))]
        );

        // After a detach each is still an unsent draft: no comment changed its round.
        let before = rounds_of_comments(&review);
        let drafts = carried(&review, Scope::Everything);
        review.detach();
        assert_eq!(review.attached(), None);
        assert_eq!(rounds_of_comments(&review), before);
        assert_eq!(carried(&review, Scope::Everything), drafts);
        assert_eq!(round_of(&review, 101), None);
        assert_eq!(round_of(&review, 202), None);

        // The user's next message carries no review.
        let unchanged = review.clone();
        assert_eq!(
            review.send_attached(SentBy::Delivered),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, unchanged);
    }

    fn round_of(review: &Review, comment_number: u32) -> Option<u32> {
        review
            .comment(&comment(comment_number))
            .expect("held")
            .1
            .round
    }

    // REV-10
    #[test]
    fn attaching_a_review_whose_agent_does_not_run_offers_that_cli_even_while_another_agent_runs_and_the_review_of_no_agent_offers_any()
     {
        // A ended, B runs in the same workspace.
        let sessions = [
            session("A", "claude", false, false),
            session("B", "claude", true, false),
        ];
        let mut review = review_with_four_threads();
        assert_eq!(
            review.attach_with(Attachment::Review, &sessions),
            Ok(Offer::Start {
                cli: "claude".to_owned()
            })
        );
        assert_eq!(review.attached(), Some(&Attachment::Review));

        // The CLI offered is the one of the review's own session, not of the one that runs.
        let sessions = [
            session("A", "pi", false, false),
            session("B", "claude", true, false),
        ];
        assert_eq!(
            review.attach_with(Attachment::Review, &sessions),
            Ok(Offer::Start {
                cli: "pi".to_owned()
            })
        );

        // With A running: nothing to offer.
        let sessions = [
            session("A", "claude", true, false),
            session("B", "claude", true, false),
        ];
        assert_eq!(
            review.attach_with(Attachment::Review, &sessions),
            Ok(Offer::Nothing)
        );

        // The review of no agent, with another agent running: any CLI.
        let mut of_no_agent = review_of(None);
        start(&mut of_no_agent, 1);
        assert_eq!(
            of_no_agent.attach_with(Attachment::Review, &sessions[1..]),
            Ok(Offer::AnyCli)
        );
        assert_eq!(of_no_agent.attached(), Some(&Attachment::Review));

        // A review whose session is not among the sessions given: its CLI is unknown.
        let mut lost = review_of(Some("gone"));
        start(&mut lost, 1);
        assert_eq!(
            lost.attach_with(Attachment::Review, &sessions),
            Ok(Offer::AnyCli)
        );
        assert_eq!(lost.attach_with(Attachment::Review, &[]), Ok(Offer::AnyCli));
    }

    // REV-11
    #[test]
    fn a_review_can_be_attached_while_its_agent_waits_at_a_permission_prompt_and_stays_attached_until_a_message_of_the_user_carries_it()
     {
        let mut review = review_of(Some("A"));
        start(&mut review, 2);
        start(&mut review, 3);
        review
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send T2 and T3");
        reply(&mut review, 2);
        reply(&mut review, 3);
        start(&mut review, 1);

        // A runs and waits at a permission prompt: nothing to offer, and it is attached.
        let waiting = [session("A", "claude", true, true)];
        assert_eq!(
            review.attach_with(Attachment::Review, &waiting),
            Ok(Offer::Nothing)
        );
        assert_eq!(review.attached(), Some(&Attachment::Review));

        // The reviewer goes on: a draft edited, a thread resolved, a new thread.
        review
            .edit(&comment(101), "comment 1, edited")
            .expect("edit");
        review.resolve(&thread(3)).expect("resolve T3");
        start(&mut review, 4);
        assert_eq!(review.attached(), Some(&Attachment::Review));

        // The user's next message carries T1's and T4's drafts and T2's reply, nothing of
        // T3, and the attachment goes with it.
        let sent = review.send_attached(SentBy::Delivered).expect("send");
        assert_eq!(
            sent.comments,
            vec![comment(202), comment(101), comment(104)]
        );
        assert_eq!(review.attached(), None);
        assert_eq!(round_of(&review, 203), None);
        assert_eq!(round_of(&review, 101), Some(2));
    }

    #[test]
    fn what_cannot_be_attached_is_refused_and_leaves_what_was_attached_as_it_was() {
        let sessions = [session("A", "claude", true, false)];
        let mut review = review_with_four_threads();
        review
            .attach_with(Attachment::Thread(thread(1)), &sessions)
            .expect("attach T1");
        let before = review.clone();

        // A thread the review does not hold.
        assert_eq!(
            review.attach_with(Attachment::Thread(thread(9)), &sessions),
            Err(ReviewError::UnknownThread(thread(9)))
        );
        // Threads that carry nothing: T3 is resolved, T4 holds no unsent comment.
        for held in [3, 4] {
            assert_eq!(
                review.attach_with(Attachment::Thread(thread(held)), &sessions),
                Err(ReviewError::NothingToSend)
            );
        }
        assert_eq!(review, before);

        // A review with nothing unsent at all.
        let mut empty = review_of(Some("A"));
        assert_eq!(
            empty.attach_with(Attachment::Review, &sessions),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(empty.attached(), None);
    }

    #[test]
    fn attaching_replaces_what_was_attached_and_one_thread_alone_carries_its_comments_only() {
        let sessions = [session("A", "claude", true, false)];
        let mut review = review_with_four_threads();
        assert_eq!(
            review.attach_with(Attachment::Thread(thread(2)), &sessions),
            Ok(Offer::Nothing)
        );
        assert_eq!(
            carried(&review, Scope::Attached),
            vec![(thread(2), comment(202))]
        );

        review
            .attach_with(Attachment::Review, &sessions)
            .expect("attach the review");
        assert_eq!(review.attached(), Some(&Attachment::Review));
        assert_eq!(carried(&review, Scope::Attached).len(), 2);
    }

    #[test]
    fn a_send_of_what_is_attached_starts_a_round_with_no_state_and_takes_the_addressed_marks_of_the_threads_it_carries_away()
     {
        let mut review = review_with_four_threads();
        // The agent addressed T2 and T4; the reviewer follows up on T2 only.
        let mut record = review.record().clone();
        for held in &mut record.threads {
            held.addressed = held.id == thread(2) || held.id == thread(4);
        }
        review = Review::from_record(record);
        review
            .attach_with(Attachment::Review, &[session("A", "claude", true, false)])
            .expect("attach");

        let sent = review.send_attached(SentBy::Delivered).expect("send");
        assert_eq!(sent.round.number, 2);
        assert_eq!(sent.round.state, None);
        assert_eq!(sent.round.sent_by, SentBy::Delivered);
        assert!(!sent.round.edited);
        assert_eq!(review.rounds().last(), Some(&sent.round));
        assert_eq!(sent.comments, vec![comment(202), comment(101)]);
        assert_eq!(review.attached(), None);

        // T2 carried a follow-up: the agent has to answer it again. T4 carried nothing.
        assert!(!review.thread(&thread(2)).expect("T2").addressed);
        assert!(review.thread(&thread(4)).expect("T4").addressed);
    }

    #[test]
    fn a_message_whose_attachment_carries_nothing_sends_nothing_and_the_attachment_stays() {
        let mut review = review_with_four_threads();
        review
            .attach_with(
                Attachment::Thread(thread(1)),
                &[session("A", "claude", true, false)],
            )
            .expect("attach T1");
        // The reviewer resolves the thread meanwhile: it leaves the attachment's carrying.
        review.resolve(&thread(1)).expect("resolve");
        let before = review.clone();

        assert_eq!(
            review.send_attached(SentBy::Delivered),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, before);
        assert_eq!(review.attached(), Some(&Attachment::Thread(thread(1))));
    }

    #[test]
    fn a_round_sent_with_no_state_is_given_one_once() {
        let mut review = review_with_four_threads();
        review
            .attach_with(Attachment::Review, &[session("A", "claude", true, false)])
            .expect("attach");
        review.send_attached(SentBy::Delivered).expect("send");

        // Round 1 was sent with a state, round 2 with none.
        assert_eq!(
            review.set_round_state(1, "S9"),
            Err(ReviewError::RoundHasState(1))
        );
        assert_eq!(
            review.set_round_state(3, "S9"),
            Err(ReviewError::UnknownRound(3))
        );
        assert_eq!(review.rounds()[1].state, None);

        review.set_round_state(2, "S2").expect("give the state");
        assert_eq!(review.rounds()[1].state.as_deref(), Some("S2"));
        assert_eq!(review.rounds()[0].state.as_deref(), Some("S1"));
        assert_eq!(
            review.set_round_state(2, "S3"),
            Err(ReviewError::RoundHasState(2))
        );
        assert_eq!(review.rounds()[1].state.as_deref(), Some("S2"));
    }
}
