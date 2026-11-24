//! Whether a round shows as sent: until its agent's first edit after the send.

use super::Review;

impl Review {
    /// The newest round, when not yet edited, is edited from now on, however it was sent. Gives
    /// its number; none when nothing changed.
    pub fn agent_edited(&mut self) -> Option<u32> {
        let newest = self.record.rounds.last_mut()?;
        if newest.edited {
            return None;
        }
        newest.edited = true;
        Some(newest.number)
    }

    /// False for a round the review does not hold. An edit is kept on the newest round only, so
    /// a round shows as sent while no round from it on is edited.
    pub fn shows_sent(&self, round: u32) -> bool {
        let rounds = &self.record.rounds;
        let Some(at) = rounds.iter().position(|held| held.number == round) else {
            return false;
        };
        rounds[at..].iter().all(|held| !held.edited)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use yaar_store::{Anchor, CommentId, ReviewId, SentBy, Side, ThreadId, WorkspaceId};

    use super::super::{Review, Scope};

    fn review() -> Review {
        Review::new(ReviewId::new("r"), WorkspaceId::new("w"), None)
    }

    fn draft(review: &mut Review, n: u32) {
        let anchor = Anchor {
            tree: "S0".to_string(),
            path: PathBuf::from("a.rs"),
            side: Side::New,
            first: 1,
            last: 1,
            text: "let x = 1;".to_string(),
        };
        review
            .start_thread(
                ThreadId::new(format!("t{n}")),
                CommentId::new(format!("c{n}")),
                anchor,
                "a comment",
            )
            .expect("start the thread");
    }

    #[test]
    fn a_round_shows_as_sent_until_the_first_edit_its_agent_reports_and_the_edit_is_reported_once()
    {
        let mut review = review();
        assert!(!review.shows_sent(1), "no round is held yet");
        assert_eq!(review.agent_edited(), None, "no round, no edit to count");

        draft(&mut review, 1);
        review
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send");
        assert!(review.shows_sent(1));

        assert_eq!(review.agent_edited(), Some(1));
        assert!(!review.shows_sent(1));
        assert!(review.rounds()[0].edited);

        let kept = review.clone();
        assert_eq!(review.agent_edited(), None);
        assert_eq!(review, kept);
    }

    #[test]
    fn a_second_round_shows_as_sent_whatever_the_first_did_and_an_edit_ends_only_the_newest() {
        let mut review = review();
        draft(&mut review, 1);
        review
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send");
        review.agent_edited();

        draft(&mut review, 2);
        review
            .send(Scope::Everything, "S2", SentBy::Delivered)
            .expect("send");
        assert!(review.shows_sent(2));
        assert!(!review.shows_sent(1));

        assert_eq!(review.agent_edited(), Some(2));
        assert!(!review.shows_sent(2));
        assert!(!review.shows_sent(1));
        assert!(!review.shows_sent(3), "a round the review does not hold");
        assert!(!review.shows_sent(0));
    }

    #[test]
    fn a_round_the_agent_never_edited_after_still_shows_as_sent_when_a_newer_one_is_sent_and_ends_at_the_next_edit()
     {
        let mut review = review();
        draft(&mut review, 1);
        review
            .send(Scope::Everything, "S1", SentBy::Delivered)
            .expect("send");
        draft(&mut review, 2);
        review
            .send(Scope::Everything, "S2", SentBy::Delivered)
            .expect("send");

        assert!(review.shows_sent(1));
        assert!(review.shows_sent(2));

        assert_eq!(review.agent_edited(), Some(2));
        assert!(!review.shows_sent(1));
        assert!(!review.shows_sent(2));
    }

    #[test]
    fn a_round_sent_as_a_copy_ends_at_the_agents_edit_as_one_delivered_does() {
        let mut review = review();
        draft(&mut review, 1);
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        assert!(review.shows_sent(1));
        assert_eq!(review.agent_edited(), Some(1));
        assert!(!review.shows_sent(1));
    }
}
