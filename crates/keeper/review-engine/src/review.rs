//! One review and its rules, with no file and no process: what can be done to its threads
//! and comments, what is attached, and what a send carries.
//!
//! The rules slice 6 adds are in child modules, one for each package that builds them:
//! attaching with the offer to start an agent, and a send whose state is recorded later
//! (`attaching.rs`); the agent's replies and the states of a thread (`answers.rs`); and
//! the round its agent has edited since (`rounds.rs`).

mod answers;
mod attaching;
mod rounds;

pub use answers::ThreadState;
pub use attaching::{Offer, SessionNow};

use std::fmt;

use yaar_store::{
    Anchor, Author, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy, Thread, ThreadId,
    WorkspaceId,
};

/// What of a review goes with the user's next message to its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// The review: every unsent comment on an unresolved thread at the moment of the send,
    /// drafts written after the attaching included, and a thread resolved meanwhile not.
    Review,
    /// One thread alone: its unsent comments, while it is unresolved. A draft on another
    /// thread does not join.
    Thread(ThreadId),
}

/// Which comments a send, or the text of a review, takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every unsent comment on an unresolved thread, whatever is attached: what copying a
    /// review as text takes.
    Everything,
    /// What is attached: everything as [`Scope::Everything`] when the review is attached,
    /// the one thread when one thread is attached alone, and nothing when nothing is.
    Attached,
}

/// The unsent comments of one thread that a send would carry, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsent<'a> {
    /// The thread.
    pub thread: &'a Thread,
    /// Its unsent comments, oldest first: at least one.
    pub comments: Vec<&'a Comment>,
}

/// What a send did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// The round it started.
    pub round: Round,
    /// The comments it sent, now carrying that round, in the order of [`Review::unsent`].
    pub comments: Vec<CommentId>,
}

/// Why a review refused a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewError {
    /// The review holds no thread by this id.
    UnknownThread(ThreadId),
    /// The review holds no comment by this id.
    UnknownComment(CommentId),
    /// The comment was sent: it carries its round and cannot be edited or deleted.
    NotADraft(CommentId),
    /// The id is taken by a thread or a comment of the review.
    TakenId(String),
    /// A reply was asked for in a thread with no sent comment: there is nothing to follow
    /// up on, and its draft is edited instead.
    NothingSent(ThreadId),
    /// A reply was asked for in a thread that holds a draft already: that draft is edited
    /// instead.
    DraftPending {
        /// The thread.
        thread: ThreadId,
        /// Its draft.
        draft: CommentId,
    },
    /// The anchor is not a range of lines: the first line is 0, the last is before the
    /// first, or its text holds another number of lines than the range.
    BadAnchor,
    /// A send was asked for with nothing to carry.
    NothingToSend,
    /// A reply of an agent came from a session the review does not belong to.
    OtherSession(String),
    /// The round is not one of the review's, or it has a state already.
    UnknownRound(u32),
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownThread(thread) => write!(f, "there is no thread {thread}"),
            Self::UnknownComment(comment) => write!(f, "there is no comment {comment}"),
            Self::NotADraft(comment) => {
                write!(
                    f,
                    "the comment {comment} was sent and can no longer be changed"
                )
            }
            Self::TakenId(id) => write!(f, "the id {id} is taken"),
            Self::NothingSent(thread) => {
                write!(f, "the thread {thread} has no sent comment to reply to")
            }
            Self::DraftPending { thread, draft } => {
                write!(f, "the thread {thread} holds the draft {draft} already")
            }
            Self::BadAnchor => write!(f, "the lines of the comment are not a range of lines"),
            Self::NothingToSend => write!(f, "there is nothing to send"),
            Self::OtherSession(session) => {
                write!(f, "the review does not belong to the session {session}")
            }
            Self::UnknownRound(round) => write!(f, "there is no round {round} to change"),
        }
    }
}

impl std::error::Error for ReviewError {}

/// One review: threads written for one agent session in one worktree, or for no agent.
///
/// A comment is a draft until a send carries it; a draft can be edited and deleted. A send
/// starts a round: every comment it carries gets the round's number and refuses to be
/// edited or deleted from then on. A follow-up on a sent comment is a new draft, a reply,
/// in the same thread. A resolved thread is not sent, an unsent comment on it included.
/// A review has no votes and no labels.
///
/// The ids of threads and comments are given by the caller, which draws them; an id the
/// review holds already is refused. Every change is checked before anything is changed: a
/// change that is refused leaves the review as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    record: ReviewRecord,
    attached: Option<Attachment>,
}

impl Review {
    /// A new review with no thread, no round and no reviewed state.
    pub fn new(id: ReviewId, workspace: WorkspaceId, agent: Option<String>) -> Review {
        Review::from_record(ReviewRecord {
            id,
            workspace,
            agent,
            reviewed: None,
            threads: Vec::new(),
            rounds: Vec::new(),
            attached: None,
        })
    }

    /// The review the Store kept, with what the Store keeps attached. What [`Review::attach`]
    /// and [`Review::detach`] change later is the review's, until the caller keeps it.
    pub fn from_record(record: ReviewRecord) -> Review {
        let attached = record.attached.as_ref().map(|attached| match attached {
            yaar_store::AttachedRecord::Review => Attachment::Review,
            yaar_store::AttachedRecord::Thread(thread) => Attachment::Thread(thread.clone()),
        });
        Review { record, attached }
    }

    /// The review as the Store keeps it.
    pub fn record(&self) -> &ReviewRecord {
        &self.record
    }

    /// Its id.
    pub fn id(&self) -> &ReviewId {
        &self.record.id
    }

    /// The workspace it belongs to.
    pub fn workspace(&self) -> &WorkspaceId {
        &self.record.workspace
    }

    /// The agent session it belongs to; none for the review of no agent.
    pub fn agent(&self) -> Option<&str> {
        self.record.agent.as_deref()
    }

    /// Its threads, in the order they were started.
    pub fn threads(&self) -> &[Thread] {
        &self.record.threads
    }

    /// One thread.
    pub fn thread(&self, thread: &ThreadId) -> Option<&Thread> {
        self.record.threads.iter().find(|t| &t.id == thread)
    }

    /// One comment, with the thread it is in.
    pub fn comment(&self, comment: &CommentId) -> Option<(&Thread, &Comment)> {
        self.record.threads.iter().find_map(|thread| {
            thread
                .comments
                .iter()
                .find(|c| &c.id == comment)
                .map(|found| (thread, found))
        })
    }

    /// Its rounds, the first first.
    pub fn rounds(&self) -> &[Round] {
        &self.record.rounds
    }

    /// The state last marked reviewed, by its full id; none before the first mark.
    pub fn reviewed(&self) -> Option<&str> {
        self.record.reviewed.as_deref()
    }

    /// What is attached to the user's next message; none while nothing is.
    pub fn attached(&self) -> Option<&Attachment> {
        self.attached.as_ref()
    }

    /// Starts a thread on `anchor` with a first comment, a draft of the reviewer with
    /// `text`. The anchor is kept as given: the tree, the path, the side, the lines and
    /// their text, whatever line it is and wherever it was written. Refused with
    /// [`ReviewError::BadAnchor`] for lines that are no range, and
    /// [`ReviewError::TakenId`] for an id the review holds.
    pub fn start_thread(
        &mut self,
        thread: ThreadId,
        comment: CommentId,
        anchor: Anchor,
        text: &str,
    ) -> Result<(), ReviewError> {
        // A line end after the last line starts another line, so it counts as one.
        let lines = anchor.text.split('\n').count();
        let is_range = anchor.first >= 1
            && anchor.last >= anchor.first
            && u64::try_from(lines).ok() == Some(u64::from(anchor.last - anchor.first) + 1);
        if !is_range {
            return Err(ReviewError::BadAnchor);
        }
        // One id for both new parts would make the second a taken id once the first is in.
        if thread.as_str() == comment.as_str() {
            return Err(ReviewError::TakenId(thread.to_string()));
        }
        self.check_free(thread.as_str())?;
        self.check_free(comment.as_str())?;
        self.record.threads.push(Thread {
            id: thread,
            anchor,
            resolved: false,
            addressed: false,
            comments: vec![Comment {
                id: comment,
                author: Author::Reviewer,
                text: text.to_string(),
                round: None,
            }],
            now: None,
        });
        Ok(())
    }

    /// Adds a draft reply of the reviewer at the end of a thread that holds a sent comment
    /// and no draft. Refused with [`ReviewError::NothingSent`] in a thread with no sent
    /// comment, [`ReviewError::DraftPending`] in one that holds a draft, and
    /// [`ReviewError::UnknownThread`] and [`ReviewError::TakenId`].
    pub fn reply(
        &mut self,
        thread: &ThreadId,
        comment: CommentId,
        text: &str,
    ) -> Result<(), ReviewError> {
        let held = self
            .thread(thread)
            .ok_or_else(|| ReviewError::UnknownThread(thread.clone()))?;
        if held.comments.iter().all(|c| c.round.is_none()) {
            return Err(ReviewError::NothingSent(thread.clone()));
        }
        if let Some(draft) = held.comments.iter().find(|c| c.round.is_none()) {
            return Err(ReviewError::DraftPending {
                thread: thread.clone(),
                draft: draft.id.clone(),
            });
        }
        self.check_free(comment.as_str())?;
        let held = self
            .record
            .threads
            .iter_mut()
            .find(|t| &t.id == thread)
            .expect("the thread was found above");
        held.comments.push(Comment {
            id: comment,
            author: Author::Reviewer,
            text: text.to_string(),
            round: None,
        });
        Ok(())
    }

    /// Changes the text of a draft. Refused with [`ReviewError::NotADraft`] for a sent
    /// comment, which keeps its text.
    pub fn edit(&mut self, comment: &CommentId, text: &str) -> Result<(), ReviewError> {
        self.check_draft(comment)?;
        let draft = self
            .record
            .threads
            .iter_mut()
            .flat_map(|thread| thread.comments.iter_mut())
            .find(|c| &c.id == comment)
            .expect("the comment was found above");
        draft.text = text.to_string();
        Ok(())
    }

    /// Deletes a draft; a thread left with no comment goes with it. Refused with
    /// [`ReviewError::NotADraft`] for a sent comment, which stays.
    pub fn delete(&mut self, comment: &CommentId) -> Result<(), ReviewError> {
        self.check_draft(comment)?;
        let (thread, _) = self.comment(comment).expect("the comment was found above");
        let thread = thread.id.clone();
        let held = self
            .record
            .threads
            .iter_mut()
            .find(|t| t.id == thread)
            .expect("the thread was found above");
        held.comments.retain(|c| &c.id != comment);
        if held.comments.is_empty() {
            self.record.threads.retain(|t| t.id != thread);
            // A thread that is gone cannot be what the next message carries.
            if self.attached == Some(Attachment::Thread(thread)) {
                self.attached = None;
            }
        }
        Ok(())
    }

    /// Resolves a thread: it is sent no more, an unsent comment on it included, until it
    /// is reopened. A thread attached alone stays attached and carries nothing.
    pub fn resolve(&mut self, thread: &ThreadId) -> Result<(), ReviewError> {
        self.thread_mut(thread)?.resolved = true;
        Ok(())
    }

    /// Reopens a resolved thread.
    pub fn reopen(&mut self, thread: &ThreadId) -> Result<(), ReviewError> {
        self.thread_mut(thread)?.resolved = false;
        Ok(())
    }

    /// Attaches the review, or one of its threads alone, to the user's next message, in
    /// place of what was attached. What the send will carry is read at the send, by
    /// [`Review::unsent`] with [`Scope::Attached`]: an edit changes it, a new draft joins
    /// an attached review, and a thread resolved meanwhile leaves it.
    pub fn attach(&mut self, attachment: Attachment) -> Result<(), ReviewError> {
        if let Attachment::Thread(thread) = &attachment
            && self.thread(thread).is_none()
        {
            return Err(ReviewError::UnknownThread(thread.clone()));
        }
        self.attached = Some(attachment);
        Ok(())
    }

    /// Takes the attachment away. Every comment it would have carried stays an unsent
    /// draft.
    pub fn detach(&mut self) {
        self.attached = None;
    }

    /// The unsent comments a send of `scope` would carry now, thread by thread, in the
    /// order of the threads: every unsent comment on an unresolved thread, or for
    /// [`Scope::Attached`] those of what is attached. A thread with none is left out.
    pub fn unsent(&self, scope: Scope) -> Vec<Unsent<'_>> {
        let only = match scope {
            Scope::Everything => None,
            Scope::Attached => match &self.attached {
                None => return Vec::new(),
                Some(Attachment::Review) => None,
                Some(Attachment::Thread(thread)) => Some(thread),
            },
        };
        self.record
            .threads
            .iter()
            .filter(|thread| !thread.resolved && only.is_none_or(|only| &thread.id == only))
            .filter_map(|thread| {
                let comments: Vec<&Comment> = thread
                    .comments
                    .iter()
                    .filter(|comment| comment.round.is_none())
                    .collect();
                (!comments.is_empty()).then_some(Unsent { thread, comments })
            })
            .collect()
    }

    /// Sends what [`Review::unsent`] gives for `scope`: starts the next round, recorded at
    /// `state` and sent by `sent_by`, and gives each of those comments that round's
    /// number. A send of what was attached detaches it. One thread sent alone is a round
    /// like any other. Refused with [`ReviewError::NothingToSend`], and nothing changed,
    /// when there is nothing to carry.
    pub fn send(
        &mut self,
        scope: Scope,
        state: &str,
        sent_by: SentBy,
    ) -> Result<Sent, ReviewError> {
        let carried: Vec<CommentId> = self
            .unsent(scope)
            .iter()
            .flat_map(|unsent| unsent.comments.iter().map(|comment| comment.id.clone()))
            .collect();
        if carried.is_empty() {
            return Err(ReviewError::NothingToSend);
        }
        let number = u32::try_from(self.record.rounds.len() + 1).expect("rounds fit in a u32");
        for comment in self
            .record
            .threads
            .iter_mut()
            .flat_map(|thread| thread.comments.iter_mut())
            .filter(|comment| carried.contains(&comment.id))
        {
            comment.round = Some(number);
        }
        let round = Round {
            number,
            state: Some(state.to_string()),
            sent_by,
            edited: false,
        };
        self.record.rounds.push(round.clone());
        if scope == Scope::Attached {
            self.attached = None;
        }
        Ok(Sent {
            round,
            comments: carried,
        })
    }

    /// Makes `state` the reviewed state, from which the next review starts.
    pub fn mark_reviewed(&mut self, state: &str) {
        self.record.reviewed = Some(state.to_string());
    }

    // Refuses an id that names a thread or a comment of the review.
    fn check_free(&self, id: &str) -> Result<(), ReviewError> {
        let taken = self.record.threads.iter().any(|thread| {
            thread.id.as_str() == id || thread.comments.iter().any(|c| c.id.as_str() == id)
        });
        if taken {
            Err(ReviewError::TakenId(id.to_string()))
        } else {
            Ok(())
        }
    }

    // Refuses a comment the review does not hold, or one that was sent.
    fn check_draft(&self, comment: &CommentId) -> Result<(), ReviewError> {
        match self.comment(comment) {
            None => Err(ReviewError::UnknownComment(comment.clone())),
            Some((_, found)) if found.round.is_some() => {
                Err(ReviewError::NotADraft(comment.clone()))
            }
            Some(_) => Ok(()),
        }
    }

    fn thread_mut(&mut self, thread: &ThreadId) -> Result<&mut Thread, ReviewError> {
        self.record
            .threads
            .iter_mut()
            .find(|t| &t.id == thread)
            .ok_or_else(|| ReviewError::UnknownThread(thread.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use yaar_store::Side;

    fn id(n: u32) -> String {
        format!("{n:032x}")
    }

    fn thread_id(n: u32) -> ThreadId {
        ThreadId::new(id(n))
    }

    fn comment_id(n: u32) -> CommentId {
        CommentId::new(id(n))
    }

    fn review() -> Review {
        Review::new(ReviewId::new(id(1000)), WorkspaceId::new("w"), None)
    }

    fn anchor(tree: &str, path: &str, side: Side, first: u32, last: u32, text: &str) -> Anchor {
        Anchor {
            tree: tree.to_string(),
            path: PathBuf::from(path),
            side,
            first,
            last,
            text: text.to_string(),
        }
    }

    // Starts thread `n` with comment `n + 100` on a one-line anchor.
    fn start(review: &mut Review, n: u32, text: &str) {
        review
            .start_thread(
                thread_id(n),
                comment_id(n + 100),
                anchor("S1", "src/a.rs", Side::New, n, n, "let x = 1;"),
                text,
            )
            .expect("start the thread");
    }

    fn round_of(review: &Review, comment: u32) -> Option<u32> {
        let (_, found) = review
            .comment(&comment_id(comment))
            .expect("the comment is in the review");
        found.round
    }

    fn texts(unsent: &[Unsent<'_>]) -> Vec<String> {
        unsent
            .iter()
            .flat_map(|thread| thread.comments.iter().map(|comment| comment.text.clone()))
            .collect()
    }

    #[test]
    fn a_new_review_has_no_thread_no_round_and_no_reviewed_state() {
        let review = review();
        assert_eq!(review.id(), &ReviewId::new(id(1000)));
        assert_eq!(review.workspace(), &WorkspaceId::new("w"));
        assert_eq!(review.agent(), None);
        assert!(review.threads().is_empty());
        assert!(review.rounds().is_empty());
        assert_eq!(review.reviewed(), None);
        assert_eq!(review.attached(), None);

        let of_an_agent = Review::new(
            ReviewId::new(id(1001)),
            WorkspaceId::new("w"),
            Some("session".to_string()),
        );
        assert_eq!(of_an_agent.agent(), Some("session"));
    }

    #[test]
    fn a_review_read_back_from_its_record_is_that_record_with_nothing_attached() {
        let mut first = review();
        start(&mut first, 1, "one");
        first
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        first.mark_reviewed("S2");
        first.attach(Attachment::Review).expect("attach");

        let again = Review::from_record(first.record().clone());
        assert_eq!(again.record(), first.record());
        assert_eq!(again.attached(), None);
        assert_eq!(again.reviewed(), Some("S2"));
        assert_eq!(again.rounds().len(), 1);
    }

    // REV-01
    #[test]
    fn a_draft_is_tied_to_a_path_a_side_lines_and_their_text_in_a_tree_whatever_the_line() {
        let mut review = review();
        let given = [
            // An added line.
            anchor("S1", "src/a.rs", Side::New, 12, 12, "let added = 1;"),
            // A removed line, in a commit of the real repository.
            anchor("C0", "src/a.rs", Side::Old, 7, 7, "let removed = 2;"),
            // A line outside the diff, three lines of a file the change leaves alone.
            anchor(
                "S1",
                "src/untouched.rs",
                Side::New,
                40,
                42,
                "fn a() {}\n\nfn b() {}",
            ),
            // The same line written in Flow and written in Changes. The review pane builds
            // both anchors one way: the path in the version of the line's side, the side,
            // the line and its text, and for the new side the state the turn ended with.
            // In Flow the line is one of the turn's live diff, read from the files on
            // disk; the engine records them in state S2 before the anchor reaches the model.
            // In Changes it is a line of the turn's change, to state S2.
            anchor("S2", "src/b.rs", Side::New, 5, 5, "let drafted = 3;"),
            anchor("S2", "src/b.rs", Side::New, 5, 5, "let drafted = 3;"),
        ];
        // This states the premise and shows nothing: both literals are the same. What can
        // fail is below, where each of the two is kept as a thread of its own and comes
        // back field for field.
        assert_eq!(given[3], given[4]);
        for (n, anchor) in given.iter().enumerate() {
            let n = n as u32 + 1;
            review
                .start_thread(
                    thread_id(n),
                    comment_id(n + 100),
                    anchor.clone(),
                    &format!("comment {n}"),
                )
                .expect("start the thread");
        }

        assert_eq!(review.threads().len(), given.len());
        for (n, wanted) in given.iter().enumerate() {
            let thread = &review.threads()[n];
            let kept = &thread.anchor;
            assert_eq!(kept.tree, wanted.tree);
            assert_eq!(kept.path, wanted.path);
            assert_eq!(kept.side, wanted.side);
            assert_eq!(kept.first, wanted.first);
            assert_eq!(kept.last, wanted.last);
            assert_eq!(kept.text, wanted.text);
            // The same threads are found by their id.
            assert_eq!(review.thread(&thread.id), Some(thread));

            // A new thread is open and holds one draft of the reviewer.
            assert!(!thread.resolved && !thread.addressed);
            assert_eq!(thread.comments.len(), 1);
            let first = &thread.comments[0];
            assert_eq!(first.author, Author::Reviewer);
            assert_eq!(first.text, format!("comment {}", n + 1));
            assert_eq!(first.round, None);
        }

        // The two anchors of one line, written in either view, are kept the same.
        let (in_flow, in_changes) = (&review.threads()[3].anchor, &review.threads()[4].anchor);
        assert_eq!(in_flow, in_changes);
        assert_eq!(
            (&in_flow.tree, &in_flow.path, in_flow.side),
            (&"S2".to_owned(), &"src/b.rs".into(), Side::New)
        );
        assert_eq!(
            (in_flow.first, in_flow.last, in_flow.text.as_str()),
            (5, 5, "let drafted = 3;")
        );

        // Edit and delete work the same on each.
        for n in 1..=given.len() as u32 {
            review
                .edit(&comment_id(n + 100), &format!("edited {n}"))
                .expect("edit the draft");
            let (_, comment) = review.comment(&comment_id(n + 100)).expect("found");
            assert_eq!(comment.text, format!("edited {n}"));
        }
        for n in 1..=given.len() as u32 {
            review.delete(&comment_id(n + 100)).expect("delete");
            assert!(review.thread(&thread_id(n)).is_none());
        }
        assert!(review.threads().is_empty());
    }

    #[test]
    fn lines_that_are_no_range_are_refused_and_leave_the_review_as_it_was() {
        let mut review = review();
        start(&mut review, 1, "kept");
        let before = review.clone();

        let bad = [
            // The first line is 0: lines count from 1.
            anchor("S1", "a.rs", Side::New, 0, 0, "x"),
            // The last line is before the first.
            anchor("S1", "a.rs", Side::New, 5, 4, "x"),
            // The text holds fewer lines than the range.
            anchor("S1", "a.rs", Side::New, 5, 6, "x"),
            // The text holds more lines than the range.
            anchor("S1", "a.rs", Side::New, 5, 5, "x\ny"),
            // A line end after the last line is another line.
            anchor("S1", "a.rs", Side::New, 5, 5, "x\n"),
        ];
        for anchor in bad {
            let refused = review.start_thread(thread_id(2), comment_id(102), anchor, "no");
            assert_eq!(refused, Err(ReviewError::BadAnchor));
            assert_eq!(review, before);
        }
    }

    #[test]
    fn an_empty_line_is_a_line_of_its_own() {
        let mut review = review();
        review
            .start_thread(
                thread_id(1),
                comment_id(101),
                anchor("S1", "a.rs", Side::New, 3, 3, ""),
                "on a blank line",
            )
            .expect("a blank line is one line");
        review
            .start_thread(
                thread_id(2),
                comment_id(102),
                anchor("S1", "a.rs", Side::New, 3, 4, "\n"),
                "on two blank lines",
            )
            .expect("two blank lines are two lines");
    }

    #[test]
    fn an_id_the_review_holds_is_taken_whether_it_names_a_thread_or_a_comment() {
        let mut review = review();
        start(&mut review, 1, "one");
        let before = review.clone();
        let a = anchor("S1", "a.rs", Side::New, 1, 1, "x");

        // A thread id again, a comment id again, a comment id used as a thread id, a
        // thread id used as a comment id, and one id for both new parts.
        let attempts = [
            (thread_id(1), comment_id(102)),
            (thread_id(2), comment_id(101)),
            (ThreadId::new(id(101)), comment_id(102)),
            (thread_id(2), CommentId::new(id(1))),
            (ThreadId::new(id(2)), CommentId::new(id(2))),
        ];
        for (thread, comment) in attempts {
            let taken = thread.as_str().to_string();
            let refused = review.start_thread(thread, comment, a.clone(), "no");
            assert!(
                matches!(refused, Err(ReviewError::TakenId(_))),
                "{taken}: {refused:?}"
            );
            assert_eq!(review, before);
        }
    }

    // REV-02
    #[test]
    fn a_comment_is_a_draft_until_sent_and_a_sent_one_refuses_edit_and_delete() {
        let mut review = review();
        start(&mut review, 1, "first");
        start(&mut review, 2, "second");

        // A draft until sent.
        assert_eq!(round_of(&review, 101), None);

        // A draft can be edited and deleted.
        review
            .edit(&comment_id(101), "first, edited")
            .expect("edit");
        let (_, comment) = review.comment(&comment_id(101)).expect("found");
        assert_eq!(comment.text, "first, edited");
        review.delete(&comment_id(102)).expect("delete");
        assert!(review.thread(&thread_id(2)).is_none());
        assert_eq!(review.threads().len(), 1);

        // Once sent it carries the number of its round.
        let sent = review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        assert_eq!(
            sent.round,
            Round {
                number: 1,
                state: Some("S1".to_string()),
                sent_by: SentBy::Copied,
                edited: false,
            }
        );
        assert_eq!(sent.comments, vec![comment_id(101)]);
        assert_eq!(round_of(&review, 101), Some(1));

        start(&mut review, 3, "third");
        let next = review
            .send(Scope::Everything, "S2", SentBy::Copied)
            .expect("send again");
        assert_eq!(next.round.number, 2);
        assert_eq!(round_of(&review, 103), Some(2));
        assert_eq!(round_of(&review, 101), Some(1));
        assert_eq!(review.rounds().len(), 2);

        // And refuses both, leaving the review as it was.
        let before = review.clone();
        assert_eq!(
            review.edit(&comment_id(101), "changed"),
            Err(ReviewError::NotADraft(comment_id(101)))
        );
        assert_eq!(review, before);
        assert_eq!(
            review.delete(&comment_id(101)),
            Err(ReviewError::NotADraft(comment_id(101)))
        );
        assert_eq!(review, before);
    }

    #[test]
    fn a_send_carries_the_state_and_the_sender_it_was_given() {
        let mut review = review();
        start(&mut review, 1, "one");
        let sent = review
            .send(Scope::Everything, "abc", SentBy::Copied)
            .expect("send");
        assert_eq!(review.rounds().first(), Some(&sent.round));
        assert_eq!(review.rounds()[0].state.as_deref(), Some("abc"));
    }

    #[test]
    fn a_change_to_a_comment_or_a_thread_nobody_holds_is_refused() {
        let mut review = review();
        start(&mut review, 1, "one");
        let before = review.clone();

        assert_eq!(
            review.edit(&comment_id(9), "x"),
            Err(ReviewError::UnknownComment(comment_id(9)))
        );
        assert_eq!(
            review.delete(&comment_id(9)),
            Err(ReviewError::UnknownComment(comment_id(9)))
        );
        assert_eq!(
            review.reply(&thread_id(9), comment_id(109), "x"),
            Err(ReviewError::UnknownThread(thread_id(9)))
        );
        assert_eq!(
            review.resolve(&thread_id(9)),
            Err(ReviewError::UnknownThread(thread_id(9)))
        );
        assert_eq!(
            review.reopen(&thread_id(9)),
            Err(ReviewError::UnknownThread(thread_id(9)))
        );
        assert_eq!(
            review.attach(Attachment::Thread(thread_id(9))),
            Err(ReviewError::UnknownThread(thread_id(9)))
        );
        assert_eq!(review, before);
        assert!(review.comment(&comment_id(9)).is_none());
        assert!(review.thread(&thread_id(9)).is_none());
    }

    #[test]
    fn deleting_a_reply_leaves_its_thread_and_the_comment_sent_before() {
        let mut review = review();
        start(&mut review, 1, "one");
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        review
            .reply(&thread_id(1), comment_id(2), "again")
            .expect("reply");

        review.delete(&comment_id(2)).expect("delete the reply");

        let thread = review.thread(&thread_id(1)).expect("the thread stays");
        assert_eq!(thread.comments.len(), 1);
        assert_eq!(thread.comments[0].round, Some(1));
    }

    // REV-03
    #[test]
    fn a_follow_up_on_a_sent_comment_is_a_draft_reply_in_the_same_thread() {
        let mut review = review();
        start(&mut review, 1, "first");
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");

        review
            .reply(&thread_id(1), comment_id(2), "and one more thing")
            .expect("reply");

        assert_eq!(review.threads().len(), 1);
        let thread = review.thread(&thread_id(1)).expect("thread");
        let ids: Vec<&CommentId> = thread.comments.iter().map(|c| &c.id).collect();
        assert_eq!(ids, [&comment_id(101), &comment_id(2)]);
        let reply = &thread.comments[1];
        assert_eq!(reply.author, Author::Reviewer);
        assert_eq!(reply.text, "and one more thing");
        assert_eq!(reply.round, None);
        // It is found in that thread by its id, and it is a draft: it can be edited.
        let (found_in, _) = review.comment(&comment_id(2)).expect("found");
        assert_eq!(found_in.id, thread_id(1));
        review
            .edit(&comment_id(2), "reworded")
            .expect("edit the reply");
    }

    #[test]
    fn a_reply_needs_a_sent_comment_and_no_draft_in_the_thread() {
        let mut review = review();
        start(&mut review, 1, "first");
        let before = review.clone();

        // Nothing sent: the draft is edited instead.
        assert_eq!(
            review.reply(&thread_id(1), comment_id(2), "x"),
            Err(ReviewError::NothingSent(thread_id(1)))
        );
        assert_eq!(review, before);

        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        review
            .reply(&thread_id(1), comment_id(2), "x")
            .expect("reply");

        // A draft reply is there already: it is edited instead.
        let before = review.clone();
        assert_eq!(
            review.reply(&thread_id(1), comment_id(3), "y"),
            Err(ReviewError::DraftPending {
                thread: thread_id(1),
                draft: comment_id(2),
            })
        );
        assert_eq!(review, before);

        // Once the reply is sent it carries round 2 and a reply can follow it.
        review
            .send(Scope::Everything, "S2", SentBy::Copied)
            .expect("send the reply");
        assert_eq!(round_of(&review, 2), Some(2));
        review
            .reply(&thread_id(1), comment_id(3), "y")
            .expect("reply again");
        assert_eq!(
            review.thread(&thread_id(1)).expect("thread").comments.len(),
            3
        );

        // The id of the reply must be free.
        review
            .send(Scope::Everything, "S3", SentBy::Copied)
            .expect("send the second reply");
        let before = review.clone();
        assert!(matches!(
            review.reply(&thread_id(1), comment_id(101), "z"),
            Err(ReviewError::TakenId(_))
        ));
        assert_eq!(review, before);
    }

    #[test]
    fn a_resolved_thread_is_left_out_of_what_is_sent_an_unsent_comment_included() {
        let mut review = review();
        start(&mut review, 1, "kept");
        start(&mut review, 2, "resolved");
        review.resolve(&thread_id(2)).expect("resolve");

        for scope in [Scope::Everything, Scope::Attached] {
            review.attach(Attachment::Review).expect("attach");
            assert_eq!(texts(&review.unsent(scope)), ["kept"]);
        }
        let sent = review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        assert_eq!(sent.comments, vec![comment_id(101)]);
        assert_eq!(round_of(&review, 102), None);

        // Nothing but the resolved thread's draft is left: nothing to send.
        let before = review.clone();
        assert_eq!(
            review.send(Scope::Everything, "S2", SentBy::Copied),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, before);

        // Reopened, its draft is sent again.
        review.reopen(&thread_id(2)).expect("reopen");
        assert_eq!(texts(&review.unsent(Scope::Everything)), ["resolved"]);
    }

    #[test]
    fn an_unsent_comment_is_listed_by_thread_in_order_with_the_threads_without_one_left_out() {
        let mut review = review();
        start(&mut review, 1, "a");
        start(&mut review, 2, "b");
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        start(&mut review, 3, "c");
        review
            .reply(&thread_id(1), comment_id(4), "a2")
            .expect("reply");

        let unsent = review.unsent(Scope::Everything);
        let threads: Vec<&ThreadId> = unsent.iter().map(|u| &u.thread.id).collect();
        assert_eq!(threads, [&thread_id(1), &thread_id(3)]);
        assert_eq!(texts(&unsent), ["a2", "c"]);
        // Nothing is attached, so an attached send carries nothing.
        assert!(review.unsent(Scope::Attached).is_empty());
    }

    // REV-07
    #[test]
    fn what_is_attached_is_read_at_the_send_and_one_thread_attached_alone_takes_no_new_draft() {
        let mut review = review();
        start(&mut review, 1, "one");
        start(&mut review, 2, "two");
        review.attach(Attachment::Review).expect("attach");
        assert_eq!(review.attached(), Some(&Attachment::Review));

        // An edit changes what will be sent.
        review.edit(&comment_id(101), "one, edited").expect("edit");
        assert_eq!(
            texts(&review.unsent(Scope::Attached)),
            ["one, edited", "two"]
        );

        // A new draft joins.
        start(&mut review, 3, "three");
        assert_eq!(
            texts(&review.unsent(Scope::Attached)),
            ["one, edited", "two", "three"]
        );

        // A thread resolved meanwhile leaves.
        review.resolve(&thread_id(2)).expect("resolve");
        assert_eq!(
            texts(&review.unsent(Scope::Attached)),
            ["one, edited", "three"]
        );

        // The send carries exactly those, and nothing is attached after it.
        let sent = review
            .send(Scope::Attached, "S1", SentBy::Copied)
            .expect("send");
        assert_eq!(sent.round.number, 1);
        assert_eq!(sent.comments, vec![comment_id(101), comment_id(103)]);
        assert_eq!(round_of(&review, 101), Some(1));
        assert_eq!(round_of(&review, 103), Some(1));
        assert_eq!(round_of(&review, 102), None);
        assert_eq!(review.attached(), None);

        // One thread attached alone: a new draft does not join.
        review
            .reply(&thread_id(1), comment_id(50), "a reply")
            .expect("reply");
        review
            .attach(Attachment::Thread(thread_id(1)))
            .expect("attach");
        start(&mut review, 4, "four");
        assert_eq!(texts(&review.unsent(Scope::Attached)), ["a reply"]);
        // Everything is still what a copy would take.
        assert_eq!(
            texts(&review.unsent(Scope::Everything)),
            ["a reply", "four"]
        );

        // That send counts as a round like any other.
        let sent = review
            .send(Scope::Attached, "S2", SentBy::Copied)
            .expect("send the thread");
        assert_eq!(sent.round.number, 2);
        assert_eq!(sent.comments, vec![comment_id(50)]);
        assert_eq!(review.rounds().len(), 2);
        assert_eq!(round_of(&review, 50), Some(2));
        assert_eq!(round_of(&review, 104), None);
        assert_eq!(review.attached(), None);
    }

    #[test]
    fn a_thread_attached_alone_that_is_resolved_stays_attached_and_carries_nothing() {
        let mut review = review();
        start(&mut review, 1, "one");
        review
            .attach(Attachment::Thread(thread_id(1)))
            .expect("attach");
        review.resolve(&thread_id(1)).expect("resolve");

        assert_eq!(review.attached(), Some(&Attachment::Thread(thread_id(1))));
        assert!(review.unsent(Scope::Attached).is_empty());
        let before = review.clone();
        assert_eq!(
            review.send(Scope::Attached, "S1", SentBy::Copied),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, before);
    }

    #[test]
    fn a_review_detached_keeps_every_draft_it_would_have_carried() {
        let mut review = review();
        start(&mut review, 1, "one");
        review.attach(Attachment::Review).expect("attach");
        review.detach();

        assert_eq!(review.attached(), None);
        assert!(review.unsent(Scope::Attached).is_empty());
        assert_eq!(texts(&review.unsent(Scope::Everything)), ["one"]);
        assert_eq!(round_of(&review, 101), None);
    }

    #[test]
    fn a_thread_attached_alone_goes_when_its_only_draft_is_deleted() {
        let mut review = review();
        start(&mut review, 1, "one");
        start(&mut review, 2, "two");
        review
            .attach(Attachment::Thread(thread_id(1)))
            .expect("attach");

        review.delete(&comment_id(101)).expect("delete");

        assert_eq!(review.attached(), None);
        assert!(review.unsent(Scope::Attached).is_empty());
        // Deleting a draft of another thread leaves the attachment alone.
        review.attach(Attachment::Review).expect("attach");
        review.delete(&comment_id(102)).expect("delete");
        assert_eq!(review.attached(), Some(&Attachment::Review));
    }

    #[test]
    fn an_attachment_replaces_the_one_before() {
        let mut review = review();
        start(&mut review, 1, "one");
        review.attach(Attachment::Review).expect("attach");
        review
            .attach(Attachment::Thread(thread_id(1)))
            .expect("attach");
        assert_eq!(review.attached(), Some(&Attachment::Thread(thread_id(1))));
    }

    #[test]
    fn a_send_with_nothing_to_carry_is_refused_and_changes_nothing() {
        let mut review = review();
        let empty = review.clone();
        assert_eq!(
            review.send(Scope::Everything, "S1", SentBy::Copied),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, empty);

        start(&mut review, 1, "one");
        review
            .send(Scope::Everything, "S1", SentBy::Copied)
            .expect("send");
        let sent = review.clone();
        assert_eq!(
            review.send(Scope::Everything, "S2", SentBy::Copied),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, sent);
        assert_eq!(review.rounds().len(), 1);
    }

    #[test]
    fn marking_reviewed_keeps_the_latest_state_and_touches_nothing_else() {
        let mut review = review();
        start(&mut review, 1, "one");
        let mut expected = review.record().clone();

        review.mark_reviewed("S1");
        assert_eq!(review.reviewed(), Some("S1"));
        review.mark_reviewed("S2");
        assert_eq!(review.reviewed(), Some("S2"));

        expected.reviewed = Some("S2".to_string());
        assert_eq!(review.record(), &expected);
        assert!(review.rounds().is_empty());
    }
}
