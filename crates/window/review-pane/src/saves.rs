//! A save of a draft is told to nobody, so the pane puts each one into the reviews it shows itself.

use std::io::{self, Read};

use yaar_link::{Anchor, Author, Comment, CommentId, ReviewId, Thread, ThreadId, WorkspaceReviews};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Save {
    /// Counts the saves sent, itself included: tells whether an answer of the Keeper already holds
    /// it.
    pub(crate) seq: u64,
    pub(crate) review: ReviewId,
    pub(crate) thread: ThreadId,
    pub(crate) comment: CommentId,
    /// Set for the first comment of a new thread only.
    pub(crate) anchor: Option<Anchor>,
    pub(crate) text: String,
}

/// True when a thread or comment was added (the rows change); a new text alone returns false.
pub(crate) fn apply(reviews: &mut WorkspaceReviews, save: &Save) -> bool {
    let Some(review) = reviews.reviews.iter_mut().find(|r| r.id == save.review) else {
        return false;
    };
    if let Some(thread) = review.threads.iter_mut().find(|t| t.id == save.thread) {
        match thread.comments.iter_mut().find(|c| c.id == save.comment) {
            Some(comment) => {
                if comment.round.is_none() {
                    comment.text.clone_from(&save.text);
                }
                false
            }
            None => {
                thread.comments.push(draft(save));
                true
            }
        }
    } else if let Some(anchor) = &save.anchor {
        review.threads.push(Thread {
            id: save.thread.clone(),
            anchor: anchor.clone(),
            resolved: false,
            addressed: false,
            comments: vec![draft(save)],
            state: yaar_link::ThreadState::Draft,
            now: None,
        });
        true
    } else {
        false
    }
}

fn draft(save: &Save) -> Comment {
    Comment {
        id: save.comment.clone(),
        author: Author::Reviewer,
        text: save.text.clone(),
        round: None,
    }
}

pub(crate) fn new_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use yaar_link::{Lines, Review, Side, WorkspaceId};

    use super::*;

    fn review_id(digit: char) -> ReviewId {
        ReviewId::new(digit.to_string().repeat(32))
    }
    fn thread_id(digit: char) -> ThreadId {
        ThreadId::new(digit.to_string().repeat(32))
    }
    fn comment_id(digit: char) -> CommentId {
        CommentId::new(digit.to_string().repeat(32))
    }

    fn anchor() -> Anchor {
        Anchor {
            tree: None,
            path: "src/a.rs".into(),
            side: Side::New,
            lines: Lines { first: 3, last: 3 },
            text: "let a = 1;".to_owned(),
        }
    }

    fn reviews() -> WorkspaceReviews {
        WorkspaceReviews {
            workspace: WorkspaceId::new("w".repeat(32)),
            reviews: vec![Review {
                id: review_id('r'),
                agent: None,
                reviewed: None,
                attached: None,
                threads: Vec::new(),
                rounds: Vec::new(),
            }],
            viewed: Vec::new(),
        }
    }

    fn save(seq: u64, thread: char, comment: char, anchor: Option<Anchor>, text: &str) -> Save {
        Save {
            seq,
            review: review_id('r'),
            thread: thread_id(thread),
            comment: comment_id(comment),
            anchor,
            text: text.to_owned(),
        }
    }

    fn first_comment_texts(reviews: &WorkspaceReviews) -> Vec<Vec<&str>> {
        reviews.reviews[0]
            .threads
            .iter()
            .map(|thread| thread.comments.iter().map(|c| c.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn the_first_save_of_a_new_thread_adds_it_and_the_next_changes_only_its_text() {
        let mut held = reviews();
        assert!(apply(&mut held, &save(1, 't', 'c', Some(anchor()), "h")));
        let thread = &held.reviews[0].threads[0];
        assert_eq!(thread.id, thread_id('t'));
        assert_eq!(thread.anchor, anchor());
        assert!(!thread.resolved && !thread.addressed);
        assert_eq!(
            thread.comments,
            [Comment {
                id: comment_id('c'),
                author: Author::Reviewer,
                text: "h".to_owned(),
                round: None
            }]
        );
        assert!(!apply(&mut held, &save(2, 't', 'c', Some(anchor()), "he")));
        assert_eq!(first_comment_texts(&held), [["he"]]);
    }

    #[test]
    fn a_save_of_a_comment_the_answer_holds_already_is_not_added_twice() {
        let mut held = reviews();
        assert!(apply(&mut held, &save(1, 't', 'c', Some(anchor()), "h")));
        assert!(!apply(&mut held, &save(1, 't', 'c', Some(anchor()), "h")));
        assert_eq!(held.reviews[0].threads.len(), 1);
        assert_eq!(held.reviews[0].threads[0].comments.len(), 1);
    }

    #[test]
    fn a_reply_goes_to_the_end_of_its_thread_and_a_sent_comment_keeps_its_text() {
        let mut held = reviews();
        apply(&mut held, &save(1, 't', 'c', Some(anchor()), "first"));
        held.reviews[0].threads[0].comments[0].round = Some(1);
        assert!(apply(&mut held, &save(2, 't', 'd', None, "reply")));
        assert_eq!(first_comment_texts(&held), [["first", "reply"]]);
        assert!(!apply(
            &mut held,
            &save(3, 't', 'c', Some(anchor()), "late")
        ));
        assert_eq!(first_comment_texts(&held), [["first", "reply"]]);
    }

    #[test]
    fn a_save_of_what_the_reviews_do_not_hold_is_left_out() {
        let mut held = reviews();
        let mut elsewhere = save(1, 't', 'c', Some(anchor()), "h");
        elsewhere.review = review_id('x');
        assert!(!apply(&mut held, &elsewhere));
        assert!(!apply(&mut held, &save(2, 'u', 'd', None, "reply")));
        assert!(held.reviews[0].threads.is_empty());
    }

    #[test]
    fn an_id_is_thirty_two_lowercase_hexadecimal_digits_and_never_the_same_twice() {
        let (one, two) = (new_id().expect("an id"), new_id().expect("an id"));
        for id in [&one, &two] {
            assert_eq!(id.len(), 32);
            assert!(id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        }
        assert_ne!(one, two);
    }
}
