use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yaar_recorder::{
    ChangedPath, Clock, CommitId, Git, GitError, RecordError, RecordedState, StateId, StateStore,
    SystemClock, Worktree,
};
use yaar_store::{
    Anchor, CommentId, ReviewId, SessionId, SessionRecord, Side, Store, StoreError, ThreadId,
    WorkspaceId,
};

use crate::review::{Review, ReviewError};
use crate::takeover::{Takeover, takeover};
use crate::viewed::{ViewedFiles, fingerprint};
use crate::workspace::with_review_of_no_agent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAnchor {
    /// Full id of a commit, never a git tree id. None for the files on disk: the engine records the
    /// worktree's state first. Ignored for an absolute path.
    pub tree: Option<String>,
    pub path: PathBuf,
    pub side: Side,
    pub first: u32,
    pub last: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOn {
    NewThread(NewAnchor),
    Thread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSave {
    pub thread: ThreadId,
    pub comment: CommentId,
    pub on: DraftOn,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    Made,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    pub text: String,
    pub round: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceView {
    pub reviews: Vec<Review>,
    pub viewed: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum EngineError {
    Refused(ReviewError),
    UnknownReview(ReviewId),
    OutsideWorktree(PathBuf),
    NoWorktree,
    Store(StoreError),
    Record(RecordError),
    Git(GitError),
    Random(std::io::Error),
    NoReviewOfSession(SessionId),
    /// Nothing was sent.
    HandOut(std::io::Error),
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
            Self::HandOut(e) => write!(f, "the review could not be handed out: {e}"),
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

/// The reviews of every workspace, kept in the Store; the states of each worktree live in
/// `<states>/<workspace id>.git`. Each change is read and written under one turn; a state is
/// recorded before the turn is taken, since a recording can take a second.
#[derive(Debug)]
pub struct Reviews {
    pub(crate) store: Store,
    git: Git,
    pub(crate) states: PathBuf,
    pub(crate) opened: Mutex<BTreeMap<WorkspaceId, Arc<StateStore>>>,
    writing: Mutex<()>,
    watch: Option<Arc<dyn StateWatch>>,
    pub(crate) clock: Arc<dyn Clock>,
}

/// Tells whether a worktree's files changed since its newest state; hears of every state taken.
pub trait StateWatch: fmt::Debug + Send + Sync {
    /// True when it cannot tell.
    fn changed_since_state(&self, workspace: &WorkspaceId) -> bool;

    fn state_taken(&self, workspace: &WorkspaceId, state: &StateId);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEnded {
    pub state: StateId,
    pub since: Option<String>,
    /// From `since`, or from the caller's start before the first mark; None with neither.
    pub changed: Option<Vec<ChangedPath>>,
}

impl Reviews {
    pub fn new(store: Store, git: Git, states: PathBuf) -> Reviews {
        Reviews {
            store,
            git,
            states,
            opened: Mutex::new(BTreeMap::new()),
            writing: Mutex::new(()),
            watch: None,
            clock: Arc::new(SystemClock),
        }
    }

    /// Call before the first state store is opened: the clock stamps every store opened afterwards.
    pub fn with_clock(self, clock: Arc<dyn Clock>) -> Reviews {
        let mut engine = self;
        engine.clock = clock;
        engine
    }

    pub fn watching(self, watch: Arc<dyn StateWatch>) -> Reviews {
        let mut engine = self;
        engine.watch = Some(watch);
        engine
    }

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

    /// Refused with [`EngineError::NoReviewOfSession`] for a session with no review, recording
    /// nothing.
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
        let state = self.record_state(&workspace, worktree)?.id;

        // Read after the recording, so a mark made while it ran counts.
        let since = self.review(&review)?.reviewed().map(str::to_owned);
        let states = self.states_of(&workspace)?;
        let changed = match (&since, start) {
            (Some(reviewed), _) => Some(states.changed_paths(reviewed, state.as_str())?),
            // Copied into the state store and compared in one call, so no clean-up can take the
            // copy away between the two.
            (None, Some(start)) => {
                Some(states.compare(Some(worktree), start.as_str(), Some(state.as_str()))?)
            }
            (None, None) => None,
        };
        Ok(TurnEnded {
            state,
            since,
            changed,
        })
    }

    /// A workspace with no review at all gets its review of no agent here, kept at once, so calls
    /// that race make only one. Viewed marks of files changed since marking are cleared; with no
    /// `worktree` no file is viewed.
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
            let cleared = marks.settle(|path| worktree.content_on_disk(path).ok().flatten());
            for path in cleared {
                self.store.set_viewed(workspace, &path, None)?;
            }
            viewed = marks.viewed();
        }
        Ok(WorkspaceView { reviews, viewed })
    }

    pub fn review(&self, review: &ReviewId) -> Result<Review, EngineError> {
        self.store
            .review(review)?
            .map(Review::from_record)
            .ok_or_else(|| EngineError::UnknownReview(review.clone()))
    }

    /// Saves a draft: a comment the review holds has its text changed ([`Saved::Text`], the
    /// keystroke save: records nothing, does not wait for the drive), else it is made
    /// ([`Saved::Made`]). A new thread with no tree sits on the state of `worktree` recorded now.
    /// An absolute anchor path has no version and is kept on the new side.
    pub fn save_draft(
        &self,
        worktree: Option<&Worktree>,
        review: &ReviewId,
        save: &DraftSave,
    ) -> Result<Saved, EngineError> {
        // Found without the turn: recording a state takes long.
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
                    let side = if is_outside(&at.path) {
                        Side::New
                    } else {
                        at.side
                    };
                    let anchor = Anchor {
                        tree: version,
                        path: at.path.clone(),
                        side,
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

    pub fn delete_draft(&self, review: &ReviewId, comment: &CommentId) -> Result<(), EngineError> {
        let _turn = self.turn();
        let mut current = self.review(review)?;
        current.delete(comment)?;
        self.store.delete_draft(comment)?;
        Ok(())
    }

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

    /// None before the first mark; a commit changes no file, so changes nothing here.
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

    pub fn states_of(&self, workspace: &WorkspaceId) -> Result<Arc<StateStore>, EngineError> {
        let mut opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = opened.get(workspace) {
            return Ok(Arc::clone(found));
        }
        let folder = self.states.join(format!("{workspace}.git"));
        let store = Arc::new(StateStore::open_with(
            &self.git,
            &folder,
            Arc::clone(&self.clock),
        )?);
        opened.insert(workspace.clone(), Arc::clone(&store));
        Ok(store)
    }

    // Every state the engine takes goes through here, so the watch hears of it and threads are
    // carried to it.
    pub(crate) fn record_state(
        &self,
        workspace: &WorkspaceId,
        worktree: &Worktree,
    ) -> Result<RecordedState, EngineError> {
        let state = self.states_of(workspace)?.record(worktree)?;
        if let Some(watch) = &self.watch {
            watch.state_taken(workspace, &state.id);
        }
        self.followed(workspace, &state.id);
        Ok(state)
    }

    pub(crate) fn turn(&self) -> MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // The version a new thread's lines are read in: the named commit kept in the state store (it
    // reads with the repository gone), the state of the files on disk recorded now, or empty for a
    // file outside the worktree.
    fn version_of(
        &self,
        worktree: Option<&Worktree>,
        workspace: &WorkspaceId,
        at: &NewAnchor,
    ) -> Result<String, EngineError> {
        if is_outside(&at.path) {
            return Ok(String::new());
        }
        match (&at.tree, worktree) {
            (None, None) => Err(EngineError::NoWorktree),
            (None, Some(worktree)) => {
                // The watcher reports a write a moment after it and a Window may have read the new
                // lines by then, so the state must hold them (`holds_lines`); files nobody changed
                // since the newest state are that state.
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

/// A file outside the worktree or one git ignores: no version, never carried through a diff.
pub(crate) fn is_outside(path: &Path) -> bool {
    path.is_absolute()
}

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

pub(crate) fn random_id() -> Result<String, EngineError> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut from| from.read_exact(&mut bytes))
        .map_err(EngineError::Random)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
