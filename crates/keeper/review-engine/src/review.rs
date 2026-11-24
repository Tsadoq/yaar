mod answers;
mod attaching;
mod rounds;

pub use answers::ThreadState;
pub use attaching::{Offer, SessionNow};

use std::fmt;

use yaar_store::{
    Anchor, Author, Comment, CommentId, ReviewId, ReviewRecord, Round, SentBy, SessionId, Thread,
    ThreadId, WorkspaceId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Every unsent comment on an unresolved thread at the moment of the send, drafts written after
    /// the attaching included.
    Review,
    Thread(ThreadId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every unsent comment on an unresolved thread, whatever is attached: what copying a review
    /// as text takes.
    Everything,
    /// Everything when the review is attached, one thread when that alone is, else nothing.
    Attached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsent<'a> {
    pub thread: &'a Thread,
    pub comments: Vec<&'a Comment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub round: Round,
    pub comments: Vec<CommentId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewError {
    UnknownThread(ThreadId),
    UnknownComment(CommentId),
    NotADraft(CommentId),
    TakenId(String),
    NothingSent(ThreadId),
    DraftPending {
        thread: ThreadId,
        draft: CommentId,
    },
    /// The first line is 0, the last is before the first, or the text holds another number of lines
    /// than the range.
    BadAnchor,
    NothingToSend,
    OtherSession(SessionId),
    UnknownRound(u32),
    RoundHasState(u32),
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
            Self::RoundHasState(round) => write!(f, "round {round} has its state already"),
        }
    }
}

impl std::error::Error for ReviewError {}

/// One review: threads written for one agent session in one worktree, or for no agent. A comment is
/// a draft until a send carries it; then it refuses edit and delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    record: ReviewRecord,
    attached: Option<Attachment>,
}

impl Review {
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

    pub fn from_record(record: ReviewRecord) -> Review {
        let attached = record.attached.as_ref().map(|attached| match attached {
            yaar_store::AttachedRecord::Review => Attachment::Review,
            yaar_store::AttachedRecord::Thread(thread) => Attachment::Thread(thread.clone()),
        });
        Review { record, attached }
    }

    pub fn record(&self) -> &ReviewRecord {
        &self.record
    }

    pub fn id(&self) -> &ReviewId {
        &self.record.id
    }

    pub fn workspace(&self) -> &WorkspaceId {
        &self.record.workspace
    }

    pub fn agent(&self) -> Option<&str> {
        self.record.agent.as_deref()
    }

    pub fn threads(&self) -> &[Thread] {
        &self.record.threads
    }

    pub fn thread(&self, thread: &ThreadId) -> Option<&Thread> {
        self.record.threads.iter().find(|t| &t.id == thread)
    }

    pub fn comment(&self, comment: &CommentId) -> Option<(&Thread, &Comment)> {
        self.record.threads.iter().find_map(|thread| {
            thread
                .comments
                .iter()
                .find(|c| &c.id == comment)
                .map(|found| (thread, found))
        })
    }

    pub fn rounds(&self) -> &[Round] {
        &self.record.rounds
    }

    pub fn reviewed(&self) -> Option<&str> {
        self.record.reviewed.as_deref()
    }

    pub fn attached(&self) -> Option<&Attachment> {
        self.attached.as_ref()
    }

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
            if self.attached == Some(Attachment::Thread(thread)) {
                self.attached = None;
            }
        }
        Ok(())
    }

    /// A thread attached alone stays attached and carries nothing.
    pub fn resolve(&mut self, thread: &ThreadId) -> Result<(), ReviewError> {
        self.thread_mut(thread)?.resolved = true;
        Ok(())
    }

    pub fn reopen(&mut self, thread: &ThreadId) -> Result<(), ReviewError> {
        self.thread_mut(thread)?.resolved = false;
        Ok(())
    }

    pub fn attach(&mut self, attachment: Attachment) -> Result<(), ReviewError> {
        if let Attachment::Thread(thread) = &attachment
            && self.thread(thread).is_none()
        {
            return Err(ReviewError::UnknownThread(thread.clone()));
        }
        self.attached = Some(attachment);
        Ok(())
    }

    pub fn detach(&mut self) {
        self.attached = None;
    }

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

    /// Refused with [`ReviewError::NothingToSend`], changing nothing, when there is nothing to
    /// carry. A send of what was attached detaches it.
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

    pub fn mark_reviewed(&mut self, state: &str) {
        self.record.reviewed = Some(state.to_string());
    }

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

    #[test]
    fn a_draft_is_tied_to_a_path_a_side_lines_and_their_text_in_a_tree_whatever_the_line() {
        let mut review = review();
        let given = [
            anchor("S1", "src/a.rs", Side::New, 12, 12, "let added = 1;"),
            anchor("C0", "src/a.rs", Side::Old, 7, 7, "let removed = 2;"),
            anchor(
                "S1",
                "src/untouched.rs",
                Side::New,
                40,
                42,
                "fn a() {}\n\nfn b() {}",
            ),
            anchor("S2", "src/b.rs", Side::New, 5, 5, "let drafted = 3;"),
            anchor("S2", "src/b.rs", Side::New, 5, 5, "let drafted = 3;"),
        ];
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
            assert_eq!(review.thread(&thread.id), Some(thread));

            assert!(!thread.resolved && !thread.addressed);
            assert_eq!(thread.comments.len(), 1);
            let first = &thread.comments[0];
            assert_eq!(first.author, Author::Reviewer);
            assert_eq!(first.text, format!("comment {}", n + 1));
            assert_eq!(first.round, None);
        }

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
            anchor("S1", "a.rs", Side::New, 0, 0, "x"),
            anchor("S1", "a.rs", Side::New, 5, 4, "x"),
            anchor("S1", "a.rs", Side::New, 5, 6, "x"),
            anchor("S1", "a.rs", Side::New, 5, 5, "x\ny"),
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

    #[test]
    fn a_comment_is_a_draft_until_sent_and_a_sent_one_refuses_edit_and_delete() {
        let mut review = review();
        start(&mut review, 1, "first");
        start(&mut review, 2, "second");

        assert_eq!(round_of(&review, 101), None);

        review
            .edit(&comment_id(101), "first, edited")
            .expect("edit");
        let (_, comment) = review.comment(&comment_id(101)).expect("found");
        assert_eq!(comment.text, "first, edited");
        review.delete(&comment_id(102)).expect("delete");
        assert!(review.thread(&thread_id(2)).is_none());
        assert_eq!(review.threads().len(), 1);

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

        let before = review.clone();
        assert_eq!(
            review.reply(&thread_id(1), comment_id(3), "y"),
            Err(ReviewError::DraftPending {
                thread: thread_id(1),
                draft: comment_id(2),
            })
        );
        assert_eq!(review, before);

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

        let before = review.clone();
        assert_eq!(
            review.send(Scope::Everything, "S2", SentBy::Copied),
            Err(ReviewError::NothingToSend)
        );
        assert_eq!(review, before);

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
        assert!(review.unsent(Scope::Attached).is_empty());
    }

    #[test]
    fn what_is_attached_is_read_at_the_send_and_one_thread_attached_alone_takes_no_new_draft() {
        let mut review = review();
        start(&mut review, 1, "one");
        start(&mut review, 2, "two");
        review.attach(Attachment::Review).expect("attach");
        assert_eq!(review.attached(), Some(&Attachment::Review));

        review.edit(&comment_id(101), "one, edited").expect("edit");
        assert_eq!(
            texts(&review.unsent(Scope::Attached)),
            ["one, edited", "two"]
        );

        start(&mut review, 3, "three");
        assert_eq!(
            texts(&review.unsent(Scope::Attached)),
            ["one, edited", "two", "three"]
        );

        review.resolve(&thread_id(2)).expect("resolve");
        assert_eq!(
            texts(&review.unsent(Scope::Attached)),
            ["one, edited", "three"]
        );

        let sent = review
            .send(Scope::Attached, "S1", SentBy::Copied)
            .expect("send");
        assert_eq!(sent.round.number, 1);
        assert_eq!(sent.comments, vec![comment_id(101), comment_id(103)]);
        assert_eq!(round_of(&review, 101), Some(1));
        assert_eq!(round_of(&review, 103), Some(1));
        assert_eq!(round_of(&review, 102), None);
        assert_eq!(review.attached(), None);

        review
            .reply(&thread_id(1), comment_id(50), "a reply")
            .expect("reply");
        review
            .attach(Attachment::Thread(thread_id(1)))
            .expect("attach");
        start(&mut review, 4, "four");
        assert_eq!(texts(&review.unsent(Scope::Attached)), ["a reply"]);
        assert_eq!(
            texts(&review.unsent(Scope::Everything)),
            ["a reply", "four"]
        );

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
