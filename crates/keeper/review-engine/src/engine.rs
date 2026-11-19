//! The engine over the Store and the Recorder: what the Keeper calls.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yaar_recorder::{
    ChangedPath, CommitId, Git, GitError, RecordError, RecordedState, StateId, StateStore, Worktree,
};
use yaar_store::{
    Anchor, CommentId, ReviewId, SentBy, SessionId, SessionRecord, Side, Store, StoreError,
    ThreadId, WorkspaceId,
};

use crate::review::{Review, ReviewError, Scope};
use crate::takeover::{Takeover, takeover};
use crate::text::{COPY_INSTRUCTION, as_text};
use crate::viewed::{ViewedFiles, fingerprint};
use crate::workspace::with_review_of_no_agent;

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
    /// The system gave no random number for the id of a new review.
    Random(std::io::Error),
    /// The Store holds no review of this session: it holds no such session, or the
    /// session was never told to the engine as started.
    NoReviewOfSession(SessionId),
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
            Self::Random(e) => write!(f, "no random number for an id: {e}"),
            Self::NoReviewOfSession(session) => {
                write!(f, "there is no review of the session {session}")
            }
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
    // What tells whether files changed since the newest state, and hears of each state
    // taken; none records at every new thread, as slice 3 did.
    watch: Option<Arc<dyn StateWatch>>,
}

/// What tells the engine whether the files of a worktree changed since its newest state,
/// so that a new thread on unchanged files sits on that state and records none; and hears
/// of every state the engine takes, so that what changed is counted from it.
pub trait StateWatch: fmt::Debug + Send + Sync {
    /// Whether the files of `workspace` may have changed since its newest state: true when
    /// it cannot tell.
    fn changed_since_state(&self, workspace: &WorkspaceId) -> bool;

    /// A state of `workspace` was taken now.
    fn state_taken(&self, workspace: &WorkspaceId, state: &StateId);
}

/// What a turn that ended leaves: the state recorded at its end, and what changed since
/// the last review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEnded {
    /// The state recorded at the end of the turn.
    pub state: StateId,
    /// The state the reviewer last marked reviewed in the session's review; none before
    /// the first mark.
    pub since: Option<String>,
    /// What changed since the last review: the paths that differ from `since` to `state`,
    /// or, before the first mark, from the start the caller gave to `state`; none with no
    /// start either, as where nothing was committed.
    pub changed: Option<Vec<ChangedPath>>,
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
            watch: None,
        }
    }

    /// The same engine, told by `watch` whether files changed since the newest state: a
    /// new thread on the files on disk then records a state only when they did, and every
    /// state the engine takes is told to `watch`.
    pub fn watching(self, watch: Arc<dyn StateWatch>) -> Reviews {
        let mut engine = self;
        engine.watch = Some(watch);
        engine
    }

    /// `session` starts: it keeps its review, takes one over, or gets a new one, by
    /// [`crate::takeover`] over the reviews and the sessions the Store holds for its
    /// workspace, and that is kept, under the engine's turn, before this returns. Gives
    /// the session's review. The session itself is the caller's to keep in the Store
    /// before.
    pub fn session_started(&self, session: &SessionRecord) -> Result<ReviewId, EngineError> {
        let _turn = self.turn();
        let reviews: Vec<Review> = self
            .store
            .reviews_of(&session.workspace)?
            .into_iter()
            .map(Review::from_record)
            .collect();
        let sessions = self.store.sessions_of(&session.workspace)?;
        match takeover(&reviews, &sessions, session) {
            Takeover::Keeps(review) => Ok(review),
            Takeover::TakesOver(review) => {
                self.store.set_review_agent(&review, Some(&session.id))?;
                Ok(review)
            }
            Takeover::New => {
                let review = Review::new(
                    ReviewId::new(random_id()?),
                    session.workspace.clone(),
                    Some(session.id.as_str().to_owned()),
                );
                self.store.put_review(review.record())?;
                Ok(review.id().clone())
            }
        }
    }

    /// A turn of `session` ended in `worktree`: records the state of the worktree, and
    /// gives it with what changed since the last review of the session's review, counted
    /// from `start` before the first mark. The turn's record is the caller's to keep.
    /// Refused with [`EngineError::NoReviewOfSession`] for a session that has no review,
    /// and then nothing is recorded.
    pub fn turn_ended(
        &self,
        worktree: &Worktree,
        session: &SessionId,
        start: Option<&CommitId>,
    ) -> Result<TurnEnded, EngineError> {
        let (workspace, review) = {
            let _turn = self.turn();
            let kept = self
                .store
                .session(session)?
                .ok_or_else(|| EngineError::NoReviewOfSession(session.clone()))?;
            let review = self
                .store
                .reviews_of(&kept.workspace)?
                .into_iter()
                .find(|review| review.agent.as_deref() == Some(session.as_str()))
                .ok_or_else(|| EngineError::NoReviewOfSession(session.clone()))?;
            (kept.workspace, review.id)
        };
        // The recording can take a second: no turn is held while it runs.
        let state = self.record_state(&workspace, worktree)?.id;

        // What the review marked reviewed is read after the recording, so that a mark made
        // while it ran counts.
        let since = self.review(&review)?.reviewed().map(str::to_owned);
        let states = self.states_of(&workspace)?;
        let from = match (&since, start) {
            (Some(reviewed), _) => Some(reviewed.clone()),
            (None, Some(start)) => {
                // The commit of the real repository is copied into the state store, so
                // that it can be compared with a state.
                states.keep_commit(worktree, start)?;
                Some(start.as_str().to_owned())
            }
            (None, None) => None,
        };
        let changed = match from {
            Some(from) => Some(states.changed_paths(&from, state.as_str())?),
            None => None,
        };
        Ok(TurnEnded {
            state,
            since,
            changed,
        })
    }

    /// The reviews of a workspace, whole, and its files marked viewed. A workspace with no
    /// review of an agent session gets its review of no agent here, kept at once, when it
    /// has none: one, also when several calls ask at once. Marks of files whose content
    /// changed since they were marked are taken away, and that is kept, before the view is
    /// given; with no `worktree`, as for a worktree that is gone, no file is viewed.
    pub fn of_workspace(
        &self,
        workspace: &WorkspaceId,
        worktree: Option<&Worktree>,
    ) -> Result<WorkspaceView, EngineError> {
        let _turn = self.turn();
        let mut reviews: Vec<Review> = self
            .store
            .reviews_of(workspace)?
            .into_iter()
            .map(Review::from_record)
            .collect();
        let fresh = ReviewId::new(random_id()?);
        let known = reviews.len();
        reviews = with_review_of_no_agent(workspace, reviews, || fresh);
        if reviews.len() > known {
            // The review is kept before it is given, so that its id is the same at the
            // next reading.
            let added = reviews.last().expect("a review was added");
            self.store.put_review(added.record())?;
        }

        let mut viewed = Vec::new();
        if let Some(worktree) = worktree {
            let mut marks = ViewedFiles::from_marks(
                self.store
                    .viewed(workspace)?
                    .into_iter()
                    .map(|record| (record.path, record.content)),
            );
            // A file that cannot be read counts as no file.
            let cleared = marks.settle(|path| worktree.content_on_disk(path).ok().flatten());
            for path in cleared {
                self.store.set_viewed(workspace, &path, None)?;
            }
            viewed = marks.viewed();
        }
        Ok(WorkspaceView { reviews, viewed })
    }

    /// One review, whole, as kept.
    pub fn review(&self, review: &ReviewId) -> Result<Review, EngineError> {
        self.store
            .review(review)?
            .map(Review::from_record)
            .ok_or_else(|| EngineError::UnknownReview(review.clone()))
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
        // The version of a new thread is found without the turn, since recording a state
        // takes long; it is only looked for once the turn has shown that it is needed.
        let mut version: Option<String> = None;
        loop {
            let turn = self.turn();
            let mut current = self.review(review)?;
            if current.comment(&save.comment).is_some() {
                current.edit(&save.comment, &save.text)?;
                self.store.save_draft(&save.comment, &save.text)?;
                return Ok(Saved::Text);
            }
            match &save.on {
                DraftOn::Thread => {
                    current.reply(&save.thread, save.comment.clone(), &save.text)?;
                    let (_, made) = current.comment(&save.comment).expect("the reply was made");
                    self.store.put_comment(&save.thread, made)?;
                    return Ok(Saved::Made);
                }
                DraftOn::NewThread(at) => {
                    let Some(version) = version.take() else {
                        drop(turn);
                        version = Some(self.version_of(worktree, current.workspace(), at)?);
                        continue;
                    };
                    let anchor = Anchor {
                        tree: version,
                        path: at.path.clone(),
                        side: at.side,
                        first: at.first,
                        last: at.last,
                        text: at.text.clone(),
                    };
                    current.start_thread(
                        save.thread.clone(),
                        save.comment.clone(),
                        anchor,
                        &save.text,
                    )?;
                    let made = current.thread(&save.thread).expect("the thread was made");
                    self.store.put_thread(review, made)?;
                    return Ok(Saved::Made);
                }
            }
        }
    }

    /// Deletes a draft of a review, and its thread when nothing is left of it.
    pub fn delete_draft(&self, review: &ReviewId, comment: &CommentId) -> Result<(), EngineError> {
        let _turn = self.turn();
        let mut current = self.review(review)?;
        current.delete(comment)?;
        self.store.delete_draft(comment)?;
        Ok(())
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
        let before = self.review(review)?;
        if before.unsent(Scope::Everything).is_empty() {
            return Err(ReviewError::NothingToSend.into());
        }
        let state = self.record_state(before.workspace(), worktree)?;

        let _turn = self.turn();
        let mut current = self.review(review)?;
        let rendered = as_text(&current, Scope::Everything, COPY_INSTRUCTION);
        if rendered.comments.is_empty() {
            return Err(ReviewError::NothingToSend.into());
        }
        let sent = current.send(Scope::Everything, state.id.as_str(), SentBy::Copied)?;
        self.store.put_round(review, &sent.round, &sent.comments)?;
        Ok(Copied {
            text: rendered.text,
            round: sent.round.number,
        })
    }

    /// Marks the change reviewed: records the state of `worktree`, the worktree of the
    /// review's workspace, as it is now and makes it the reviewed state of the review,
    /// from which the next review starts. No commit is made. Gives the state.
    pub fn mark_reviewed(
        &self,
        worktree: &Worktree,
        review: &ReviewId,
    ) -> Result<StateId, EngineError> {
        let before = self.review(review)?;
        let state = self.record_state(before.workspace(), worktree)?;

        let _turn = self.turn();
        let mut current = self.review(review)?;
        current.mark_reviewed(state.id.as_str());
        self.store.set_reviewed(review, state.id.as_str())?;
        Ok(state.id)
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
        let current = self.review(review)?;
        let Some(reviewed) = current.reviewed() else {
            return Ok(None);
        };
        let states = self.states_of(current.workspace())?;
        let now = self.record_state(current.workspace(), worktree)?;
        Ok(Some(states.changed_paths(reviewed, now.id.as_str())?))
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
        let leads_out = path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)));
        if leads_out {
            return Err(EngineError::OutsideWorktree(path.to_owned()));
        }
        let _turn = self.turn();
        if viewed {
            let content = worktree.content_on_disk(path)?;
            let mark = fingerprint(content.as_deref());
            self.store.set_viewed(workspace, path, Some(&mark))?;
        } else {
            self.store.set_viewed(workspace, path, None)?;
        }
        Ok(())
    }

    /// The state store of a workspace, where its states are recorded: what reads a state
    /// back.
    pub fn states_of(&self, workspace: &WorkspaceId) -> Result<Arc<StateStore>, EngineError> {
        let mut opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = opened.get(workspace) {
            return Ok(Arc::clone(found));
        }
        let folder = self.states.join(format!("{workspace}.git"));
        let store = Arc::new(StateStore::open(&self.git, &folder)?);
        opened.insert(workspace.clone(), Arc::clone(&store));
        Ok(store)
    }

    // Records the state of `worktree` in the state store of `workspace` and tells the watch
    // of it: every state the engine takes goes through here.
    fn record_state(
        &self,
        workspace: &WorkspaceId,
        worktree: &Worktree,
    ) -> Result<RecordedState, EngineError> {
        let state = self.states_of(workspace)?.record(worktree)?;
        if let Some(watch) = &self.watch {
            watch.state_taken(workspace, &state.id);
        }
        Ok(state)
    }

    // The turn: held while a call reads what it changes and keeps the change.
    fn turn(&self) -> MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // The version a new thread's lines are read in: the commit the window named, kept in
    // the state store so that it reads with the repository gone, or the state of the
    // files on disk, recorded now.
    fn version_of(
        &self,
        worktree: Option<&Worktree>,
        workspace: &WorkspaceId,
        at: &NewAnchor,
    ) -> Result<String, EngineError> {
        match (&at.tree, worktree) {
            (None, None) => Err(EngineError::NoWorktree),
            (None, Some(worktree)) => {
                // Files nobody changed since the newest state are that state: a thread
                // goes on it, and nothing is recorded, unless there is no state yet. The
                // watcher tells of a write a moment after it, and a Window may have read
                // the new lines by then, so the state must hold the lines too.
                let unchanged = self
                    .watch
                    .as_ref()
                    .is_some_and(|watch| !watch.changed_since_state(workspace));
                if unchanged {
                    let states = self.states_of(workspace)?;
                    if let Some(newest) = states.states()?.pop()
                        && let Some(content) = states.content(newest.id.as_str(), &at.path)?
                        && holds_lines(&content, at)
                    {
                        return Ok(newest.id.as_str().to_owned());
                    }
                }
                Ok(self
                    .record_state(workspace, worktree)?
                    .id
                    .as_str()
                    .to_owned())
            }
            (Some(commit), Some(worktree)) => {
                self.states_of(workspace)?
                    .keep_commit(worktree, &CommitId::new(commit))?;
                Ok(commit.clone())
            }
            (Some(commit), None) => Ok(commit.clone()),
        }
    }
}

// Whether the file `content` has, on the lines the anchor names, the text the anchor holds.
fn holds_lines(content: &[u8], at: &NewAnchor) -> bool {
    let Ok(content) = std::str::from_utf8(content) else {
        return false;
    };
    let (Some(first), Some(last)) = (
        at.first
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok()),
        usize::try_from(at.last).ok(),
    ) else {
        return false;
    };
    // A line end after the last line starts another, empty one, as the anchor counts it.
    let lines: Vec<&str> = content.split('\n').collect();
    lines
        .get(first..last)
        .is_some_and(|held| held.join("\n") == at.text)
}

// 32 lowercase hexadecimal digits from the random numbers of the system.
fn random_id() -> Result<String, EngineError> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut from| from.read_exact(&mut bytes))
        .map_err(EngineError::Random)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
