//! Which uncommitted changes a review lists: those of its session and those no agent made.

use std::path::PathBuf;

use crate::review::Review;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credited {
    pub path: PathBuf,
    pub by: Vec<String>,
}

/// The uncommitted changes `review` lists, in the order of `changes`: every change no agent
/// made, and those credited to the review's own session (also when credited to another as
/// well). The review of no agent lists all.
pub fn uncommitted_of<'a>(review: &Review, changes: &'a [Credited]) -> Vec<&'a Credited> {
    let Some(session) = review.agent() else {
        return changes.iter().collect();
    };
    changes
        .iter()
        .filter(|change| change.by.is_empty() || change.by.iter().any(|by| by == session))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yaar_store::{Anchor, CommentId, ReviewId, Side, ThreadId, WorkspaceId};

    fn change(path: &str, by: &[&str]) -> Credited {
        Credited {
            path: PathBuf::from(path),
            by: by.iter().map(|session| (*session).to_owned()).collect(),
        }
    }

    fn review_of(id: &str, session: Option<&str>) -> Review {
        Review::new(
            ReviewId::new(id),
            WorkspaceId::new("w"),
            session.map(str::to_owned),
        )
    }

    fn paths<'a>(listed: &[&'a Credited]) -> Vec<&'a str> {
        listed
            .iter()
            .map(|change| change.path.to_str().expect("a path of text"))
            .collect()
    }

    // x by A, y by B, n by no agent, xy by both.
    fn changes() -> Vec<Credited> {
        vec![
            change("x", &["A"]),
            change("y", &["B"]),
            change("n", &[]),
            change("xy", &["A", "B"]),
        ]
    }

    #[test]
    fn a_change_no_agent_made_is_listed_in_every_review_beside_those_of_its_own_session() {
        let changes = changes();
        let (a, b) = (review_of("ra", Some("A")), review_of("rb", Some("B")));
        assert_eq!(paths(&uncommitted_of(&a, &changes)), ["x", "n", "xy"]);
        assert_eq!(paths(&uncommitted_of(&b, &changes)), ["y", "n", "xy"]);

        let none = review_of("r0", None);
        assert_eq!(
            paths(&uncommitted_of(&none, &changes)),
            ["x", "y", "n", "xy"]
        );
    }

    #[test]
    fn a_change_is_listed_only_while_it_is_uncommitted() {
        // `n` is committed, so it is no longer among the changes.
        let after: Vec<Credited> = changes()
            .into_iter()
            .filter(|c| c.path.as_os_str() != "n")
            .collect();
        for review in [
            review_of("ra", Some("A")),
            review_of("rb", Some("B")),
            review_of("r0", None),
        ] {
            assert!(
                !paths(&uncommitted_of(&review, &after)).contains(&"n"),
                "{:?} still lists n",
                review.agent()
            );
        }
        assert!(uncommitted_of(&review_of("ra", Some("A")), &[]).is_empty());
    }

    #[test]
    fn a_session_with_no_change_of_its_own_lists_the_changes_no_agent_made_only() {
        let changes = changes();
        let c = review_of("rc", Some("C"));
        assert_eq!(paths(&uncommitted_of(&c, &changes)), ["n"]);
    }

    #[test]
    fn a_comment_on_a_change_no_agent_made_belongs_to_the_review_it_was_written_in() {
        let changes = changes();
        let (mut a, b) = (review_of("ra", Some("A")), review_of("rb", Some("B")));
        assert!(paths(&uncommitted_of(&a, &changes)).contains(&"n"));
        assert!(paths(&uncommitted_of(&b, &changes)).contains(&"n"));
        a.start_thread(
            ThreadId::new("t1"),
            CommentId::new("c1"),
            Anchor {
                tree: "S1".to_owned(),
                path: PathBuf::from("n"),
                side: Side::New,
                first: 1,
                last: 1,
                text: "line".to_owned(),
            },
            "why",
        )
        .expect("start a thread");

        assert!(a.thread(&ThreadId::new("t1")).is_some());
        assert!(a.comment(&CommentId::new("c1")).is_some());
        assert!(b.thread(&ThreadId::new("t1")).is_none());
        assert!(b.threads().is_empty());
    }
}
