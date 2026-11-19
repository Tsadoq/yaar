//! A review rendered as text: what is copied by hand, and what a delivery module wraps in
//! its own words in a later slice.

use yaar_store::{CommentId, ThreadId};

use crate::review::{Review, Scope};

/// The instruction that goes first in a review copied as text. An agent the app cannot
/// deliver to has no reply tool, so it is asked to answer each thread in its own answer,
/// naming the thread by its id.
pub const COPY_INSTRUCTION: &str = "This is a code review. Address each thread below. \
In your answer, reply to each thread on its own, starting with its id as it is written \
after \"Thread\", and say whether you addressed it.";

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
    let _ = (review, scope, instruction);
    todo!()
}
