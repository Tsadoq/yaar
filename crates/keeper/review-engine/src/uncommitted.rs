//! Which uncommitted changes a review lists: those of its session and those no agent
//! made.

use std::path::PathBuf;

use crate::review::Review;

/// One uncommitted change of a worktree with who made it, as Workspaces credits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credited {
    /// The path, relative to the folder of the worktree.
    pub path: PathBuf,
    /// The ids of the sessions it is credited to; empty for a change no agent made.
    pub by: Vec<String>,
}

/// The uncommitted changes `review` lists under uncommitted changes, in the order of
/// `changes`: every change no agent made, in every review of the worktree, while it is
/// uncommitted; and those credited to the review's own session, also one credited to
/// another session as well, which then shows in both. The review of no agent lists them
/// all. The Keeper answers `GetUncommitted` with it: the review pane lists what this
/// gives, and nothing else.
pub fn uncommitted_of<'a>(review: &Review, changes: &'a [Credited]) -> Vec<&'a Credited> {
    let Some(session) = review.agent() else {
        // The review of no agent shows the whole worktree.
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

    // x by A, y by B, n by no agent, xy by both: in the order the Keeper counts them.
    fn changes() -> Vec<Credited> {
        vec![
            change("x", &["A"]),
            change("y", &["B"]),
            change("n", &[]),
            change("xy", &["A", "B"]),
        ]
    }

    // REV-20
    #[test]
    fn a_change_no_agent_made_is_listed_in_every_review_beside_those_of_its_own_session() {
        let changes = changes();
        let (a, b) = (review_of("ra", Some("A")), review_of("rb", Some("B")));
        assert_eq!(paths(&uncommitted_of(&a, &changes)), ["x", "n", "xy"]);
        assert_eq!(paths(&uncommitted_of(&b, &changes)), ["y", "n", "xy"]);

        // The review of no agent shows the whole worktree.
        let none = review_of("r0", None);
        assert_eq!(
            paths(&uncommitted_of(&none, &changes)),
            ["x", "y", "n", "xy"]
        );
    }

    // REV-20
    #[test]
    fn a_change_is_listed_only_while_it_is_uncommitted() {
        // The list is made from the changes the worktree has now: once `n` is committed
        // it is not among them, and no review lists it.
        let after: Vec<Credited> = changes().into_iter().filter(|c| c.path.as_os_str() != "n").collect();
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

    // REV-20
    #[test]
    fn a_session_with_no_change_of_its_own_lists_the_changes_no_agent_made_only() {
        let changes = changes();
        let c = review_of("rc", Some("C"));
        assert_eq!(paths(&uncommitted_of(&c, &changes)), ["n"]);
    }

    // REV-20
    #[test]
    fn a_comment_on_a_change_no_agent_made_belongs_to_the_review_it_was_written_in() {
        let changes = changes();
        let (mut a, b) = (review_of("ra", Some("A")), review_of("rb", Some("B")));
        // `n` is in both lists; the reviewer of A comments on it.
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
