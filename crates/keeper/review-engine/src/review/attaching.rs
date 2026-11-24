use yaar_store::{SentBy, SessionId};

use super::{Attachment, Review, ReviewError, Scope, Sent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionNow {
    pub id: SessionId,
    pub cli: String,
    pub running: bool,
    pub waits_for_permission: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// The agent the review belongs to runs: its next message carries the review.
    Nothing,
    /// The CLI of the agent the review belongs to, which does not run, whether or not another agent
    /// runs in the workspace.
    Start { cli: String },
    /// Any CLI the app supports: the review belongs to no agent, or its session is not among
    /// those given.
    AnyCli,
}

impl Review {
    /// `sessions` are the workspace's, ended ones included; a review whose session is not among
    /// them offers [`Offer::AnyCli`]. Refused, attaching nothing, with `NothingToSend` or
    /// `UnknownThread`.
    pub fn attach_with(
        &mut self,
        attachment: Attachment,
        sessions: &[SessionNow],
    ) -> Result<Offer, ReviewError> {
        let before = self.attached.clone();
        self.attach(attachment)?;
        if self.unsent(Scope::Attached).is_empty() {
            self.attached = before;
            return Err(ReviewError::NothingToSend);
        }
        Ok(self.offer(sessions))
    }

    /// Starts the next round with no state yet ([`Review::set_round_state`]), detaches, and removes
    /// the addressed mark of every thread it sends in.
    pub fn send_attached(&mut self, sent_by: SentBy) -> Result<Sent, ReviewError> {
        // The general send names a state; this round has none yet, so the placeholder is removed
        // before anything else reads the round.
        let mut sent = self.send(Scope::Attached, "", sent_by)?;
        sent.round.state = None;
        if let Some(round) = self.record.rounds.last_mut() {
            round.state = None;
        }
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

    #[test]
    fn attaching_a_review_attaches_every_unsent_comment_on_an_unresolved_thread_and_after_a_detach_each_is_a_draft_and_the_next_message_carries_nothing()
     {
        let mut review = review_with_four_threads();
        let running = [session("A", "claude", true, false)];

        assert_eq!(
            review.attach_with(Attachment::Review, &running),
            Ok(Offer::Nothing)
        );
        assert_eq!(review.attached(), Some(&Attachment::Review));
        assert_eq!(
            carried(&review, Scope::Attached),
            vec![(thread(2), comment(202)), (thread(1), comment(101))]
        );

        let before = rounds_of_comments(&review);
        let drafts = carried(&review, Scope::Everything);
        review.detach();
        assert_eq!(review.attached(), None);
        assert_eq!(rounds_of_comments(&review), before);
        assert_eq!(carried(&review, Scope::Everything), drafts);
        assert_eq!(round_of(&review, 101), None);
        assert_eq!(round_of(&review, 202), None);

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

    #[test]
    fn attaching_a_review_whose_agent_does_not_run_offers_that_cli_even_while_another_agent_runs_and_the_review_of_no_agent_offers_any()
     {
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

        let sessions = [
            session("A", "claude", true, false),
            session("B", "claude", true, false),
        ];
        assert_eq!(
            review.attach_with(Attachment::Review, &sessions),
            Ok(Offer::Nothing)
        );

        let mut of_no_agent = review_of(None);
        start(&mut of_no_agent, 1);
        assert_eq!(
            of_no_agent.attach_with(Attachment::Review, &sessions[1..]),
            Ok(Offer::AnyCli)
        );
        assert_eq!(of_no_agent.attached(), Some(&Attachment::Review));

        let mut lost = review_of(Some("gone"));
        start(&mut lost, 1);
        assert_eq!(
            lost.attach_with(Attachment::Review, &sessions),
            Ok(Offer::AnyCli)
        );
        assert_eq!(lost.attach_with(Attachment::Review, &[]), Ok(Offer::AnyCli));
    }

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

        let waiting = [session("A", "claude", true, true)];
        assert_eq!(
            review.attach_with(Attachment::Review, &waiting),
            Ok(Offer::Nothing)
        );
        assert_eq!(review.attached(), Some(&Attachment::Review));

        review
            .edit(&comment(101), "comment 1, edited")
            .expect("edit");
        review.resolve(&thread(3)).expect("resolve T3");
        start(&mut review, 4);
        assert_eq!(review.attached(), Some(&Attachment::Review));

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

        assert_eq!(
            review.attach_with(Attachment::Thread(thread(9)), &sessions),
            Err(ReviewError::UnknownThread(thread(9)))
        );
        for held in [3, 4] {
            assert_eq!(
                review.attach_with(Attachment::Thread(thread(held)), &sessions),
                Err(ReviewError::NothingToSend)
            );
        }
        assert_eq!(review, before);

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
