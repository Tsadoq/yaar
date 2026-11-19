//! A review rendered as text: the one text every way of delivering hands out, after that
//! way's own instruction (`yaar_agent::Delivery::instruction`).

use yaar_store::{Author, CommentId, Side, ThreadId};

use crate::review::{Review, Scope};

/// A review as text, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewText {
    /// The text.
    pub text: String,
    /// The threads it holds, in the order it holds them.
    pub threads: Vec<ThreadId>,
    /// The unsent comments it holds, in the order it holds them: those a send of the same
    /// scope marks sent.
    pub comments: Vec<CommentId>,
}

/// Renders what a send of `scope` would carry as text, after `instruction`.
///
/// For each thread of [`Review::unsent`], in that order: a heading with the thread's id;
/// the path, the side and the lines it sits on; the code it sits on, its anchor's text,
/// fenced so that no run of backticks in the code can close the fence; the comments sent
/// before, each with its author and round, so that a follow-up reads in its place; and
/// then each unsent comment, in order. Nothing of a resolved thread is in it, and nothing
/// of a thread with no unsent comment. Lines are counted from 1, as the anchor counts
/// them.
pub fn as_text(review: &Review, scope: Scope, instruction: &str) -> ReviewText {
    let mut blocks = vec![instruction.to_string()];
    let mut threads = Vec::new();
    let mut comments = Vec::new();
    for unsent in review.unsent(scope) {
        let thread = unsent.thread;
        let anchor = &thread.anchor;
        let side = match anchor.side {
            Side::Old => "old",
            Side::New => "new",
        };
        let lines = if anchor.first == anchor.last {
            format!("line {} on the {side} side", anchor.first)
        } else {
            format!(
                "lines {} to {}, on the {side} side",
                anchor.first, anchor.last
            )
        };
        let fence = "`".repeat(longest_run_of_backticks(&anchor.text).max(2) + 1);
        let mut block = format!(
            "## Thread {}\n\n{}, {lines}\n\n{fence}\n{}\n{fence}",
            thread.id,
            anchor.path.display(),
            anchor.text
        );
        // What was sent before is context: a follow-up reads in its place.
        for earlier in thread.comments.iter().filter(|c| c.round.is_some()) {
            let by = match earlier.author {
                Author::Reviewer => "the reviewer",
                Author::Agent => "the agent",
            };
            let round = earlier.round.unwrap_or_default();
            block.push_str(&format!(
                "\n\nSent before, round {round}, by {by}:\n{}",
                earlier.text
            ));
        }
        for unsent in &unsent.comments {
            block.push_str(&format!("\n\nComment:\n{}", unsent.text));
            comments.push(unsent.id.clone());
        }
        threads.push(thread.id.clone());
        blocks.push(block);
    }
    let mut text = blocks.join("\n\n");
    text.push('\n');
    ReviewText {
        text,
        threads,
        comments,
    }
}

// The length of the longest run of backticks in `text`.
fn longest_run_of_backticks(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use yaar_agent::COPY_INSTRUCTION;
    use yaar_store::{Anchor, ReviewId, SentBy, Side, WorkspaceId};

    fn thread(n: u32) -> ThreadId {
        ThreadId::new(format!("{n:032x}"))
    }

    fn comment(n: u32) -> CommentId {
        CommentId::new(format!("{n:032x}"))
    }

    fn anchor(path: &str, side: Side, first: u32, last: u32, text: &str) -> Anchor {
        Anchor {
            tree: "S1".to_string(),
            path: PathBuf::from(path),
            side,
            first,
            last,
            text: text.to_string(),
        }
    }

    // Starts thread `n` with its comment `n + 100`.
    fn start(review: &mut Review, n: u32, anchor: Anchor, text: &str) {
        review
            .start_thread(thread(n), comment(n + 100), anchor, text)
            .expect("start the thread");
    }

    fn review() -> Review {
        Review::new(
            ReviewId::new(format!("{:032x}", 1000)),
            WorkspaceId::new("w"),
            None,
        )
    }

    // The review of REV-04: B and D are sent in round 1, then A is a draft, B has a draft
    // reply, C is a draft on a thread resolved, D stays all sent. Each path, line, code
    // and text is a string found nowhere else.
    fn the_review_of_the_row() -> Review {
        let mut review = review();
        start(
            &mut review,
            2,
            anchor("src/bravo.rs", Side::Old, 7, 7, "let bravo_code = 7;"),
            "bravo first comment",
        );
        start(
            &mut review,
            4,
            anchor("src/delta.rs", Side::New, 9, 9, "let delta_code = 9;"),
            "delta only comment",
        );
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send B and D");
        start(
            &mut review,
            1,
            anchor(
                "src/alpha.rs",
                Side::New,
                12,
                14,
                "let alpha_one = 1;\nlet alpha_two = \"```\";\nlet alpha_three = 3;",
            ),
            "alpha draft",
        );
        review
            .reply(&thread(2), comment(202), "bravo follow-up")
            .expect("reply on B");
        start(
            &mut review,
            3,
            anchor("src/charlie.rs", Side::New, 3, 3, "let charlie_code = 3;"),
            "charlie draft",
        );
        review.resolve(&thread(3)).expect("resolve C");
        review
    }

    // REV-04
    #[test]
    fn the_review_as_text_holds_every_unsent_comment_of_an_unresolved_thread_with_its_place_and_code_and_nothing_of_a_resolved_one()
     {
        let review = the_review_of_the_row();
        let rendered = as_text(&review, Scope::Everything, COPY_INSTRUCTION);

        // Every unsent comment on an unresolved thread, and no other: A's draft and B's
        // reply, in the order of the threads (B was started before A).
        assert_eq!(rendered.threads, vec![thread(2), thread(1)]);
        assert_eq!(rendered.comments, vec![comment(202), comment(101)]);
        assert!(rendered.text.starts_with(COPY_INSTRUCTION));
        assert!(rendered.text.contains("alpha draft"));
        assert!(rendered.text.contains("bravo follow-up"));

        // Each with its path, its lines as the format says, and the code it sits on, all
        // between its heading and the next.
        let at = |needle: &str| {
            rendered
                .text
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} is not in:\n{}", rendered.text))
        };
        let (b, a) = (
            at(&format!("## Thread {}", thread(2))),
            at(&format!("## Thread {}", thread(1))),
        );
        assert!(b < a);
        let bravo = &rendered.text[b..a];
        let alpha = &rendered.text[a..];
        assert!(
            bravo.contains("src/bravo.rs, line 7 on the old side"),
            "{bravo}"
        );
        assert!(bravo.contains("let bravo_code = 7;"));
        assert!(bravo.contains("bravo follow-up"));
        assert!(!bravo.contains("alpha"));
        assert!(
            alpha.contains("src/alpha.rs, lines 12 to 14, on the new side"),
            "{alpha}"
        );
        // A's code holds three backticks, so it is fenced with four; B's holds none, so
        // three are enough.
        assert!(
            alpha.contains(
                "\n````\nlet alpha_one = 1;\nlet alpha_two = \"```\";\nlet alpha_three = 3;\n````\n"
            ),
            "{alpha}"
        );
        assert!(
            bravo.contains("\n```\nlet bravo_code = 7;\n```\n"),
            "{bravo}"
        );
        assert!(alpha.contains("alpha draft"));

        // What was sent before reads in its place, as context and not as a comment to send.
        assert!(bravo.contains("Sent before, round 1, by the reviewer:\nbravo first comment"));
        let sent_before = bravo.find("bravo first comment").expect("context");
        let follow_up = bravo
            .find("Comment:\nbravo follow-up")
            .expect("the follow-up");
        assert!(sent_before < follow_up);
        assert!(!alpha.contains("Sent before"));

        // Nothing of the resolved thread C, nor of D, which holds nothing unsent.
        for gone in [
            thread(3).as_str().to_string(),
            comment(103).as_str().to_string(),
            "src/charlie.rs".to_string(),
            "let charlie_code = 3;".to_string(),
            "charlie draft".to_string(),
            thread(4).as_str().to_string(),
            "src/delta.rs".to_string(),
            "let delta_code = 9;".to_string(),
            "delta only comment".to_string(),
        ] {
            assert!(
                !rendered.text.contains(&gone),
                "{gone} is in:\n{}",
                rendered.text
            );
        }
    }

    #[test]
    fn code_with_backticks_is_fenced_by_more_backticks_than_it_holds() {
        let mut review = review();
        start(
            &mut review,
            1,
            anchor("README.md", Side::New, 1, 3, "```rust\nlet x = 1;\n```"),
            "three",
        );
        start(
            &mut review,
            2,
            anchor("NOTES.md", Side::New, 5, 5, "a run of `````, five"),
            "five",
        );
        start(
            &mut review,
            3,
            anchor("plain.md", Side::New, 5, 5, "no ticks `one` here"),
            "none",
        );
        let text = as_text(&review, Scope::Everything, "Go.").text;

        assert!(
            text.contains("\n````\n```rust\nlet x = 1;\n```\n````\n"),
            "{text}"
        );
        assert!(
            text.contains("\n``````\na run of `````, five\n``````\n"),
            "{text}"
        );
        assert!(text.contains("\n```\nno ticks `one` here\n```\n"), "{text}");
    }

    #[test]
    fn a_comment_of_the_agent_sent_before_is_named_as_the_agent_s() {
        let mut review = review();
        start(
            &mut review,
            1,
            anchor("a.rs", Side::New, 1, 1, "x"),
            "reviewer words",
        );
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        review
            .reply(&thread(1), comment(201), "reply two")
            .expect("reply");
        let mut record = review.record().clone();
        record.threads[0].comments.insert(
            1,
            yaar_store::Comment {
                id: comment(300),
                author: yaar_store::Author::Agent,
                text: "agent words".to_string(),
                round: Some(1),
            },
        );
        let review = Review::from_record(record);
        let text = as_text(&review, Scope::Everything, "Go.").text;

        assert!(
            text.contains("Sent before, round 1, by the reviewer:\nreviewer words"),
            "{text}"
        );
        assert!(
            text.contains("Sent before, round 1, by the agent:\nagent words"),
            "{text}"
        );
        assert!(text.contains("Comment:\nreply two"), "{text}");
    }

    #[test]
    fn a_review_with_nothing_to_send_is_the_instruction_alone_and_names_no_thread() {
        let mut review = review();
        start(&mut review, 1, anchor("a.rs", Side::New, 1, 1, "x"), "one");
        review.resolve(&thread(1)).expect("resolve");
        let rendered = as_text(&review, Scope::Everything, "Go.");

        assert_eq!(rendered.text.trim_end(), "Go.");
        assert!(rendered.threads.is_empty() && rendered.comments.is_empty());
    }

    #[test]
    fn the_scope_decides_which_comments_the_text_holds() {
        let mut review = review();
        start(&mut review, 1, anchor("a.rs", Side::New, 1, 1, "x"), "one");
        start(&mut review, 2, anchor("b.rs", Side::New, 1, 1, "y"), "two");

        // Nothing is attached: a send of what is attached carries nothing.
        assert!(as_text(&review, Scope::Attached, "Go.").comments.is_empty());

        review
            .attach(crate::Attachment::Thread(thread(2)))
            .expect("attach");
        let rendered = as_text(&review, Scope::Attached, "Go.");
        assert_eq!(rendered.comments, vec![comment(102)]);
        assert!(!rendered.text.contains("a.rs"));
        assert_eq!(as_text(&review, Scope::Everything, "Go.").comments.len(), 2);
    }
}
