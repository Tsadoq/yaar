//! The engine over the Store and the Recorder: what the Keeper calls.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yaar_recorder::{ChangedPath, Git, GitError, RecordError, StateId, StateStore, Worktree};
use yaar_store::{CommentId, ReviewId, Side, Store, StoreError, ThreadId, WorkspaceId};

use crate::review::{Review, ReviewError};

/// Where a new thread sits, as the Window names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAnchor {
    /// The version the lines are read in: the full id of a commit, a recorded state's or
    /// one of the real repository's, never the id of a git tree object. None for lines of
    /// the files on disk, for which the engine records the state of the worktree first
    /// and puts the thread on it.
    pub tree: Option<String>,
    /// The path of the file in that version, relative to the folder of the worktree: on
    /// the old side of a renamed file its old path, on the new side its path now.
    pub path: PathBuf,
    /// The side of the diff the lines were on.
    pub side: Side,
    /// The first line, counted from 1.
    pub first: u32,
    /// The last line, counted from 1.
    pub last: u32,
    /// The text of the lines as the reviewer saw them, joined by line feeds.
    pub text: String,
}

/// What a draft that is saved belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOn {
    /// It is the first comment of a new thread that sits here.
    NewThread(NewAnchor),
    /// It is a comment of a thread that exists: an edit of a draft, or a reply.
    Thread,
}

/// One save of a draft, as typed: the whole text, with the ids the Window drew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSave {
    /// The thread: new when `on` starts one.
    pub thread: ThreadId,
    /// The comment.
    pub comment: CommentId,
    /// What it belongs to.
    pub on: DraftOn,
    /// The whole text.
    pub text: String,
}

/// What a save of a draft did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// It made the comment, and with it the thread when it started one: the reviews
    /// changed for every Window.
    Made,
    /// It changed the text of a draft that was kept.
    Text,
}

/// A review copied as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The text, for the clipboard.
    pub text: String,
    /// The number of the round the copied comments were sent in.
    pub round: u32,
}

/// The reviews of one workspace, and its files marked viewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceView {
    /// Its reviews: at least one.
    pub reviews: Vec<Review>,
    /// The files marked viewed whose content has not changed since, ordered by path;
    /// empty when no worktree was given.
    pub viewed: Vec<PathBuf>,
}

/// Why the engine did not do what was asked.
#[derive(Debug)]
pub enum EngineError {
    /// The review refused the change; the error says why.
    Refused(ReviewError),
    /// The Store holds no review by this id.
    UnknownReview(ReviewId),
    /// A path leads out of the worktree.
    OutsideWorktree(PathBuf),
    /// A new thread on the files on disk, or a send, needs the worktree, and there is none:
    /// it is gone.
    NoWorktree,
    /// The Store failed.
    Store(StoreError),
    /// A state could not be recorded.
    Record(RecordError),
    /// git failed while a state was read.
    Git(GitError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(e) => e.fmt(f),
            Self::UnknownReview(review) => write!(f, "there is no review {review}"),
            Self::OutsideWorktree(path) => write!(f, "{} is outside the worktree", path.display()),
            Self::NoWorktree => write!(f, "the worktree is gone"),
            Self::Store(e) => e.fmt(f),
            Self::Record(e) => e.fmt(f),
            Self::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<ReviewError> for EngineError {
    fn from(e: ReviewError) -> Self {
        Self::Refused(e)
    }
}

impl From<StoreError> for EngineError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<RecordError> for EngineError {
    fn from(e: RecordError) -> Self {
        Self::Record(e)
    }
}

impl From<GitError> for EngineError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// The reviews of every workspace, kept in the Store, with the states of each worktree
/// recorded in a store of its own under one folder.
///
/// It may be used from several threads at once. Each call that changes something reads
/// it from the Store, applies the rules of [`Review`], and keeps the change through the
/// Store's calls; a change the rules refuse changes nothing. Those reads and writes are
/// made one call at a time, [`Reviews::of_workspace`] included, since it may keep a review
/// and take marks away: two calls never read the same thing and both write it back. A
/// state is recorded before that turn is taken, never during it, so that a recording,
/// which can take a second or more, holds up no other call.
///
/// The workspace of a call that names a review is the review's own, as the Store holds
/// it. The state store of a workspace is `<states>/<workspace id>.git`, opened at its
/// first use and kept.
#[derive(Debug)]
pub struct Reviews {
    store: Store,
    git: Git,
    states: PathBuf,
    // The state store of each workspace, once opened.
    opened: Mutex<BTreeMap<WorkspaceId, Arc<StateStore>>>,
    // Held by each call while it reads what it changes and keeps the change.
    writing: Mutex<()>,
}

impl Reviews {
    /// The reviews kept in `store`, with the states of each worktree recorded through
    /// `git` in a folder under `states`, which lies in the app's folder.
    pub fn new(store: Store, git: Git, states: PathBuf) -> Reviews {
        Reviews {
            store,
            git,
            states,
            opened: Mutex::new(BTreeMap::new()),
            writing: Mutex::new(()),
        }
    }

    /// The reviews of a workspace, whole, and its files marked viewed. A workspace with no
    /// review of an agent session gets its review of no agent here, kept at once, when it
    /// has none: one, also when several calls ask at once. The marks of files whose content changed since are taken away and kept so
    /// first; with no `worktree`, as for a worktree that is gone, no file is viewed.
    pub fn of_workspace(
        &self,
        workspace: &WorkspaceId,
        worktree: Option<&Worktree>,
    ) -> Result<WorkspaceView, EngineError> {
        let _ = (workspace, worktree);
        todo!()
    }

    /// One review, whole, as kept.
    pub fn review(&self, review: &ReviewId) -> Result<Review, EngineError> {
        let _ = review;
        todo!()
    }

    /// Keeps one save of a draft of `review`.
    ///
    /// A comment the review holds has its text changed, [`Saved::Text`]: the save made at
    /// each keystroke, which needs no worktree, records nothing and does not wait for the
    /// drive. A sent comment is refused, as the rules of [`Review`] say.
    ///
    /// A comment the review does not hold is made: with [`DraftOn::Thread`] a reply; with
    /// [`DraftOn::NewThread`] the first comment of a new thread, whose anchor, when it
    /// names no tree, is put on the state of `worktree` recorded now (refused with
    /// [`EngineError::NoWorktree`] when none is given), and when it names a commit has that
    /// commit kept in the state store first when a worktree is given. That is a change
    /// every Window is told of, [`Saved::Made`], and it waits for the drive. A save that
    /// finds, once its state is recorded, that a save of the same comment made the thread
    /// meanwhile changes the text instead.
    pub fn save_draft(
        &self,
        worktree: Option<&Worktree>,
        review: &ReviewId,
        save: &DraftSave,
    ) -> Result<Saved, EngineError> {
        let _ = (worktree, review, save);
        todo!()
    }

    /// Deletes a draft of a review, and its thread when nothing is left of it.
    pub fn delete_draft(&self, review: &ReviewId, comment: &CommentId) -> Result<(), EngineError> {
        let _ = (review, comment);
        todo!()
    }

    /// Copies a review as text and sends it, as a submit does: the text holds every unsent
    /// comment on an unresolved thread, after [`crate::COPY_INSTRUCTION`]; the state of
    /// `worktree`, the worktree of the review's workspace, is recorded; the copied comments
    /// are sent in a new round, recorded at that state, in one transaction of the Store.
    /// Refused, with nothing recorded, when there is nothing to send.
    pub fn copy_as_text(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
    ) -> Result<Copied, EngineError> {
        let _ = (worktree, review);
        todo!()
    }

    /// Marks the change reviewed: records the state of `worktree`, the worktree of the
    /// review's workspace, as it is now and makes it the reviewed state of the review,
    /// from which the next review starts. No commit is made. Gives the state.
    pub fn mark_reviewed(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
    ) -> Result<StateId, EngineError> {
        let _ = (worktree, review);
        todo!()
    }

    /// What counts as changed since the last review: the paths that differ between the
    /// reviewed state and the state of `worktree`, the worktree of the review's workspace,
    /// recorded now. None before the first mark. A commit, made through the app or not,
    /// changes nothing of it, since it changes no file.
    pub fn changed_since_review(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
    ) -> Result<Option<Vec<ChangedPath>>, EngineError> {
        let _ = (worktree, review);
        todo!()
    }

    /// Marks a file of a workspace viewed for its content now, or takes the mark away.
    /// `path` is relative to the folder of the worktree; one that leads out of it is
    /// refused.
    pub fn set_viewed(
        &self,
        workspace: &WorkspaceId,
        worktree: &Worktree,
        path: &Path,
        viewed: bool,
    ) -> Result<(), EngineError> {
        let _ = (workspace, worktree, path, viewed);
        todo!()
    }

    /// The state store of a workspace, where its states are recorded: what reads a state
    /// back.
    pub fn states_of(&self, workspace: &WorkspaceId) -> Result<Arc<StateStore>, EngineError> {
        let _ = (
            workspace,
            &self.store,
            &self.git,
            &self.states,
            &self.opened,
            &self.writing,
        );
        todo!()
    }
}
