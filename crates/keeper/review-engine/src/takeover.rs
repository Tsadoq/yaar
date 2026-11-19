//! Which review a session that starts takes: the rule of a review per agent session.

use yaar_store::{ReviewId, SessionRecord};

use crate::review::Review;

/// What a session that starts does with the reviews of its workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Takeover {
    /// The session keeps the review it held before: its CLI resumed it.
    Keeps(ReviewId),
    /// The session takes over this review: that of no agent, or one whose session of the
    /// same CLI has ended.
    TakesOver(ReviewId),
    /// The session gets a review of its own, new.
    New,
}

/// What `session`, which starts now in its workspace, does with that workspace's
/// `reviews`, given every session of the workspace as the Store holds them, `session`
/// among them or not.
///
/// In this order:
/// 1. a review that belongs to `session` already, as when its CLI resumes it: it keeps
///    it;
/// 2. a review whose session ended in the terminal `session` starts in and was of the
///    same CLI, as after a clear or a new start of the CLI there: it takes it over;
/// 3. the review of no agent: the first session of a supported CLI takes it over;
/// 4. a review whose session was of the same CLI and has ended, the one that ended last:
///    the next session of that CLI in the worktree takes it over;
/// 5. else a new review: two sessions that run at once in one worktree have a review
///    each.
///
/// A review whose session runs is never taken over, and a review whose session was of
/// another CLI is never taken over by this one.
pub fn takeover(
    reviews: &[Review],
    sessions: &[SessionRecord],
    session: &SessionRecord,
) -> Takeover {
    let session_of = |review: &Review| {
        let holder = review.agent()?;
        sessions.iter().find(|held| held.id.as_str() == holder)
    };
    if let Some(own) = reviews
        .iter()
        .find(|review| review.agent() == Some(session.id.as_str()))
    {
        return Takeover::Keeps(own.id().clone());
    }

    // The reviews whose session ended, and of the same CLI: the only ones another session
    // may take. A session that runs keeps its review, whatever else holds.
    let free: Vec<(&Review, &SessionRecord)> = reviews
        .iter()
        .filter_map(|review| Some((review, session_of(review)?)))
        .filter(|(_, held)| !held.running && held.agent == session.agent)
        .collect();
    // Ended later is a larger number; one with no number ended before any that has one.
    let last_ended = |candidates: Vec<(&Review, &SessionRecord)>| {
        candidates
            .into_iter()
            .max_by_key(|(_, held)| held.ended.unwrap_or(0))
            .map(|(review, _)| Takeover::TakesOver(review.id().clone()))
    };

    if let Some(terminal) = &session.terminal
        && let Some(taken) = last_ended(
            free.iter()
                .filter(|(_, held)| held.terminal.as_ref() == Some(terminal))
                .copied()
                .collect(),
        )
    {
        return taken;
    }
    if let Some(none) = reviews.iter().find(|review| review.agent().is_none()) {
        return Takeover::TakesOver(none.id().clone());
    }
    last_ended(free).unwrap_or(Takeover::New)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yaar_store::{EndedBy, SessionId, WorkspaceId};

    fn w() -> WorkspaceId {
        WorkspaceId::new("w")
    }

    // A session of `agent` in `terminal`: running when `ended` is none, else ended in that
    // place of the order of ends.
    fn session(id: &str, agent: &str, terminal: &str, ended: Option<u64>) -> SessionRecord {
        SessionRecord {
            id: SessionId::new(id),
            workspace: w(),
            agent: agent.to_owned(),
            cli_session: format!("cli-{id}"),
            terminal: Some(terminal.to_owned()),
            number: 1,
            running: ended.is_none(),
            ended,
            ended_by: ended.map(|_| EndedBy::Cli),
        }
    }

    // The review `id`, of the session `of`, or of no agent.
    fn review(id: &str, of: Option<&str>) -> Review {
        Review::new(ReviewId::new(id), w(), of.map(str::to_owned))
    }

    fn id(review: &str) -> ReviewId {
        ReviewId::new(review)
    }

    // REV-17
    #[test]
    fn the_first_session_takes_over_the_review_of_no_agent_and_a_second_that_runs_gets_its_own() {
        let first = session("s1", "claude", "t1", None);
        let none = review("r0", None);
        assert_eq!(
            takeover(&[none], &[first.clone()], &first),
            Takeover::TakesOver(id("r0"))
        );

        // The first one now holds that review and runs: the second finds no review to take
        // over, so it has one of its own. Two sessions in one worktree, a review each.
        let second = session("s2", "claude", "t2", None);
        let held = review("r0", Some("s1"));
        assert_eq!(
            takeover(&[held], &[first, second.clone()], &second),
            Takeover::New
        );
    }

    // REV-17
    #[test]
    fn a_review_stays_open_when_its_session_ends_and_the_next_session_of_that_cli_takes_it_over() {
        // The review of the ended session was not given back to no agent, and nothing
        // else took it: it is there, held by the ended session, for the next one.
        let ended = session("s1", "claude", "t1", Some(1));
        let next = session("s2", "claude", "t2", None);
        let kept = review("r1", Some("s1"));
        assert_eq!(
            takeover(&[kept], &[ended, next.clone()], &next),
            Takeover::TakesOver(id("r1"))
        );
    }

    // REV-17
    #[test]
    fn a_session_of_another_cli_never_takes_over_the_review_of_an_ended_session() {
        let ended = session("s1", "claude", "t1", Some(1));
        let other = session("p1", "pi", "t1", None);
        let kept = review("r1", Some("s1"));
        // Not even from the terminal the ended session ran in.
        assert_eq!(
            takeover(&[kept], &[ended, other.clone()], &other),
            Takeover::New
        );
    }

    // REV-17
    #[test]
    fn of_two_ended_sessions_the_one_that_ended_last_gives_its_review() {
        // Listed with the later end first, so that the order of the list is not the order
        // of the ends.
        let late = session("s2", "claude", "t2", Some(5));
        let early = session("s1", "claude", "t1", Some(2));
        let next = session("s3", "claude", "t3", None);
        let reviews = [review("r2", Some("s2")), review("r1", Some("s1"))];
        assert_eq!(
            takeover(&reviews, &[late, early, next.clone()], &next),
            Takeover::TakesOver(id("r2"))
        );
    }

    // REV-17
    #[test]
    fn a_session_started_in_the_terminal_of_an_ended_one_takes_that_terminals_review() {
        let elsewhere = session("s1", "claude", "t1", Some(7));
        let here = session("s2", "claude", "t2", Some(3));
        let next = session("s3", "claude", "t2", None);
        let reviews = [review("r1", Some("s1")), review("r2", Some("s2"))];
        // s1 ended last, yet the new session started in the terminal s2 ran in.
        assert_eq!(
            takeover(&reviews, &[elsewhere, here, next.clone()], &next),
            Takeover::TakesOver(id("r2"))
        );

        // The terminal comes before the review of no agent too: after a clear in a
        // terminal, the session goes on with the review of the one before it.
        let none = review("r0", None);
        let reviews = [none, review("r2", Some("s2"))];
        let here = session("s2", "claude", "t2", Some(3));
        assert_eq!(
            takeover(&reviews, &[here, next.clone()], &next),
            Takeover::TakesOver(id("r2"))
        );
    }

    // REV-17
    #[test]
    fn an_ended_session_of_another_cli_in_the_same_terminal_gives_no_review() {
        let ended = session("p1", "pi", "t1", Some(1));
        let next = session("s1", "claude", "t1", None);
        assert_eq!(
            takeover(&[review("r1", Some("p1"))], &[ended, next.clone()], &next),
            Takeover::New
        );
    }

    // REV-17
    #[test]
    fn the_review_of_no_agent_goes_before_that_of_an_ended_session_in_another_terminal() {
        let ended = session("s1", "claude", "t1", Some(1));
        let next = session("s2", "claude", "t2", None);
        let reviews = [review("r1", Some("s1")), review("r0", None)];
        assert_eq!(
            takeover(&reviews, &[ended, next.clone()], &next),
            Takeover::TakesOver(id("r0"))
        );
    }

    // REV-17
    #[test]
    fn a_resumed_session_keeps_its_own_review_though_one_of_no_agent_waits() {
        // Resumed: the Store holds it again, running, and its review names it.
        let resumed = session("s1", "claude", "t9", None);
        let reviews = [review("r0", None), review("r1", Some("s1"))];
        assert_eq!(
            takeover(&reviews, &[resumed.clone()], &resumed),
            Takeover::Keeps(id("r1"))
        );

        // Also while another ended session's review is there, from the terminal it
        // resumes in.
        let other = session("s2", "claude", "t9", Some(4));
        let reviews = [review("r2", Some("s2")), review("r1", Some("s1"))];
        assert_eq!(
            takeover(&reviews, &[other, resumed.clone()], &resumed),
            Takeover::Keeps(id("r1"))
        );
    }

    // REV-17
    #[test]
    fn a_review_whose_session_runs_is_not_taken_over_whatever_the_terminal() {
        let running = session("s1", "claude", "t1", None);
        let next = session("s2", "claude", "t1", None);
        assert_eq!(
            takeover(&[review("r1", Some("s1"))], &[running, next.clone()], &next),
            Takeover::New
        );
    }

    // REV-17
    #[test]
    fn a_review_whose_session_the_store_does_not_hold_is_left_alone() {
        let next = session("s2", "claude", "t1", None);
        assert_eq!(
            takeover(&[review("r1", Some("gone"))], &[next.clone()], &next),
            Takeover::New
        );
    }
}
