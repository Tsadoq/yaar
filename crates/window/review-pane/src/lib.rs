//! Review pane: Changes (the selected change, with the history tree above its files) and Flow (the
//! agent's steps as a timeline or the live diff of the running turn). The pane changes no file of
//! the worktree except through the commit control.

mod actions;
mod attaching;
mod comments;
mod draw;
pub mod file_view;
mod flow;
mod flow_comment;
mod flow_draw;
mod follow;
mod live;
mod live_draw;
mod moved;
mod names;
pub mod navigate;
mod rounds;
mod saves;
pub mod servers;
mod shared;
mod states;
mod steady;
mod threads;
mod tree;
mod turns;
mod whose;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::*;
use yaar_diff::{CONTEXT_LINES, Emphasis, FileDiff, Row, Side};
use yaar_frame::{AskError, Asks, BuildPane, Control, Models, Pane, PaneEntry, Place, Sends};
use yaar_link::{
    Anchor, ChangedFile, Changes, CommentId, Comparison, Content, CreditedFile, Fact, FileSides,
    FileStatus, FlowForm, History, RequestKind, Review, ReviewId, Session, SessionId, Step,
    ThreadId, TurnEnd, TurnEndSelects, TurnId, WorkspaceId, WorkspaceReviews,
};

use crate::actions::CommitField;
use crate::comments::OpenBox;
use crate::saves::Save;
use crate::tree::{Source, Tree};

pub use file_view::FileShown;
pub use navigate::{HoverShown, ReferencesShown};
pub use rounds::RoundShown;
pub use yaar_diff::Layout;

actions!(
    review,
    [
        LineDown,
        LineUp,
        ToggleLayout,
        Comment,
        EditDraft,
        Reply,
        DeleteDraft,
        NextThread,
        PreviousThread,
        /// Copying also sends the review's comments.
        CopyReview,
        ToggleViewed,
        MarkReviewed,
        Commit,
        FocusHistory,
        FocusRows,
        HistoryDown,
        HistoryUp,
        MarkChange,
        ShowChange,
        ToggleKeepSelection,
        TakeOffer,
        NextReview,
        ToggleView,
        ToggleFlowForm,
        ToggleFollow,
        ResumeFollow,
        /// Attaches the review to the user's next message to its agent: every unsent comment on an
        /// unresolved thread, those written later included.
        AttachReview,
        AttachThread,
        DetachReview,
        ResolveThread,
        ReopenThread,
        ResolveAddressed,
        ShowOriginal,
        /// Shows in Changes what changed since a round's comments were sent: first the newest round
        /// with a recorded state, then each round before it, then the newest again.
        SinceRound,
        StartAgent,
        NextOfferedAgent,
        NextName,
        PreviousName,
        Hover,
        GoToDefinition,
        FindReferences,
        GoBack,
        ReferenceDown,
        ReferenceUp,
        OpenReference,
        CloseReferences,
        DownloadServer,
        UpdateServer,
        AllowServers,
        RefuseServers,
    ]
);

pub const KEY_CONTEXT: &str = "ReviewPane";

pub const HISTORY_CONTEXT: &str = "ReviewHistory";

pub const REFERENCES_CONTEXT: &str = "ReviewReferences";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeShown {
    /// The workspace's changes against its base, as `yaar_frame::WorkspaceChanges` holds them;
    /// shown until the reviewer selects in the tree.
    Comparison,
    /// Entries `first` to `last` of `History::entries`, by index, both included: from the version
    /// before `first` (the entry above it, or the history's start) to the version of `last`.
    Entries { first: usize, last: usize },
    /// As the Keeper answers `GetUncommitted`: from the commit checked out to the files on disk.
    Uncommitted,
    /// From one recorded state to a later one: after a turn's end, from the state last marked
    /// reviewed (or the history's start) to the turn's state; after [`SinceRound`], from a
    /// round's state to the newest recorded one.
    Since { from: Option<String>, to: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Changes,
    Flow,
}

/// A step of a turn, as Flow names the one it is at: the turn, and the step's number in it,
/// from 1.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlowAt {
    pub turn: TurnId,
    pub number: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineEntry {
    Turn {
        turn: TurnId,
        session: SessionId,
        title: String,
    },
    Step {
        turn: TurnId,
        number: u32,
        step: Step,
    },
    End {
        turn: TurnId,
        how: Option<TurnEnd>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Following {
    #[default]
    On,
    Paused {
        missed: u32,
    },
    Off,
}

/// The change of the turn the live diff shows: from the version it counts from to the files on
/// disk while the turn runs, and to the state recorded at its end once it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveChange {
    pub workspace: WorkspaceId,
    pub turn: TurnId,
    /// The turn's `start`, or where it has none the commit checked out; none where nothing is
    /// committed.
    pub from: Option<String>,
    /// None while it runs, and when the recording failed: the files on disk.
    pub to: Option<String>,
}

/// The file Flow's live view is currently following an agent read into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveReadShown {
    pub path: PathBuf,
    pub lines: Option<std::ops::RangeInclusive<u32>>,
    pub line_count: usize,
}

pub const FIRST_LAYOUT: Layout = Layout::Unified;

pub const NAME: &str = "review";

pub const EXPIRED_TURN: &str = "a turn (its recording has expired)";

/// At most this many files are asked for and not yet answered. Questions are written to the
/// Keeper's socket on the drawing thread, and a full socket (8,192 bytes by default on macOS)
/// blocks the write: asking for all at once would hold the window still.
pub const FILES_ON_THEIR_WAY: usize = 4;

pub fn entry() -> PaneEntry {
    PaneEntry {
        name: NAME,
        controls,
        build: pane,
    }
}

pub fn pane() -> BuildPane {
    Box::new(|models, window, cx| {
        let view = cx.new(|cx| ReviewPane::new(models, window, cx));
        let focus = view.read(cx).focus.clone();
        Pane {
            name: NAME,
            view: view.into(),
            focus,
        }
    })
}

/// Keys follow vim and tuicr where either has one. The tree's keys, in [`HISTORY_CONTEXT`], are the
/// same letters moving its cursor; `shift-l` and `shift-g` work only in Flow. In the comment box
/// `escape` keeps the draft, unlike tuicr.
pub fn controls() -> Vec<Control> {
    fn control(
        title: &'static str,
        action: fn() -> Box<dyn Action>,
        keystroke: &'static str,
    ) -> Control {
        Control {
            title,
            action,
            keystroke,
            place: Place::In(KEY_CONTEXT),
            sends: None,
            asks: Asks::Nothing,
        }
    }
    fn in_tree(
        title: &'static str,
        action: fn() -> Box<dyn Action>,
        keystroke: &'static str,
    ) -> Control {
        Control {
            place: Place::In(HISTORY_CONTEXT),
            ..control(title, action, keystroke)
        }
    }
    fn in_references(
        title: &'static str,
        action: fn() -> Box<dyn Action>,
        keystroke: &'static str,
    ) -> Control {
        Control {
            place: Place::In(REFERENCES_CONTEXT),
            ..control(title, action, keystroke)
        }
    }
    fn sends(control: Control, kind: RequestKind) -> Control {
        Control {
            sends: Some(Sends::Request(kind)),
            ..control
        }
    }
    vec![
        control("Line Down", || Box::new(LineDown), "j"),
        control("Line Down", || Box::new(LineDown), "down"),
        control("Line Up", || Box::new(LineUp), "k"),
        control("Line Up", || Box::new(LineUp), "up"),
        control("Unified or Split Diff", || Box::new(ToggleLayout), "s"),
        control("Comment", || Box::new(Comment), "c"),
        control("Edit Draft", || Box::new(EditDraft), "i"),
        control("Reply", || Box::new(Reply), "o"),
        sends(
            control("Delete Draft", || Box::new(DeleteDraft), "d d"),
            RequestKind::DeleteDraft,
        ),
        control("Next Thread", || Box::new(NextThread), "m"),
        control("Previous Thread", || Box::new(PreviousThread), "shift-m"),
        sends(
            control("Copy Review", || Box::new(CopyReview), "y"),
            RequestKind::CopyReview,
        ),
        sends(
            control("File Viewed", || Box::new(ToggleViewed), "r"),
            RequestKind::SetViewed,
        ),
        sends(
            control("Mark Reviewed", || Box::new(MarkReviewed), "shift-r"),
            RequestKind::MarkReviewed,
        ),
        Control {
            asks: Asks::Text,
            ..sends(
                control("Commit…", || Box::new(Commit), "shift-c"),
                RequestKind::Commit,
            )
        },
        control("History", || Box::new(FocusHistory), "tab"),
        in_tree("Rows", || Box::new(FocusRows), "tab"),
        in_tree("History Down", || Box::new(HistoryDown), "j"),
        in_tree("History Down", || Box::new(HistoryDown), "down"),
        in_tree("History Up", || Box::new(HistoryUp), "k"),
        in_tree("History Up", || Box::new(HistoryUp), "up"),
        in_tree("Mark Change", || Box::new(MarkChange), "space"),
        in_tree("Show Change", || Box::new(ShowChange), "enter"),
        sends(
            control(
                "Keep Selection",
                || Box::new(ToggleKeepSelection),
                "shift-t",
            ),
            RequestKind::SetSetting,
        ),
        control("Take Offer", || Box::new(TakeOffer), "shift-o"),
        control("Next Review", || Box::new(NextReview), "shift-a"),
        control("Flow or Changes", || Box::new(ToggleView), "f"),
        control("Follow", || Box::new(ToggleFollow), "shift-f"),
        sends(
            control(
                "Timeline or Live Diff",
                || Box::new(ToggleFlowForm),
                "shift-l",
            ),
            RequestKind::SetSetting,
        ),
        control("Resume Follow", || Box::new(ResumeFollow), "shift-g"),
        sends(
            control("Attach Review", || Box::new(AttachReview), "a"),
            RequestKind::AttachReview,
        ),
        sends(
            control("Attach Thread", || Box::new(AttachThread), "t"),
            RequestKind::AttachReview,
        ),
        sends(
            control("Detach Review", || Box::new(DetachReview), "shift-d"),
            RequestKind::DetachReview,
        ),
        sends(
            control("Resolve Thread", || Box::new(ResolveThread), "x"),
            RequestKind::ResolveThread,
        ),
        sends(
            control("Reopen Thread", || Box::new(ReopenThread), "u"),
            RequestKind::ReopenThread,
        ),
        sends(
            control(
                "Resolve Addressed",
                || Box::new(ResolveAddressed),
                "shift-x",
            ),
            RequestKind::ResolveAddressed,
        ),
        control("Original Line", || Box::new(ShowOriginal), "w"),
        control("Since Round", || Box::new(SinceRound), "shift-s"),
        control("Start Agent", || Box::new(StartAgent), "shift-n"),
        // Ctrl-N, a line down in vim as `j` is: every plain and shifted letter is vim's or the
        // pane's, Ctrl-K starts the app's chords on Linux, and an Option letter is a dead key on
        // macOS.
        control(
            "Next Offered Agent",
            || Box::new(NextOfferedAgent),
            "ctrl-n",
        ),
        control("Next Name", || Box::new(NextName), "l"),
        control("Previous Name", || Box::new(PreviousName), "h"),
        control("Hover", || Box::new(Hover), "shift-k"),
        control("Go to Definition", || Box::new(GoToDefinition), "g d"),
        // `g r`: Neovim's `grr` shortened, as vim's `gr` replaces text, which this read-only view
        // never does. Back is Escape: vim's Ctrl-O is Open Folder on Linux.
        control("Find References", || Box::new(FindReferences), "g r"),
        control("Back to the Diff", || Box::new(GoBack), "escape"),
        in_references("Next Reference", || Box::new(ReferenceDown), "j"),
        in_references("Next Reference", || Box::new(ReferenceDown), "down"),
        in_references("Previous Reference", || Box::new(ReferenceUp), "k"),
        in_references("Previous Reference", || Box::new(ReferenceUp), "up"),
        in_references("Open Reference", || Box::new(OpenReference), "enter"),
        in_references("Close References", || Box::new(CloseReferences), "escape"),
        control(
            "Download Language Server",
            || Box::new(DownloadServer),
            "shift-i",
        ),
        control(
            "Update Language Server",
            || Box::new(UpdateServer),
            "shift-u",
        ),
        sends(
            control(
                "Run Language Servers of the Repository",
                || Box::new(AllowServers),
                "shift-y",
            ),
            RequestKind::SetSetting,
        ),
        sends(
            control(
                "Keep Language Servers of the Repository Out",
                || Box::new(RefuseServers),
                "shift-q",
            ),
            RequestKind::SetSetting,
        ),
    ]
}

/// Drawn outside the element that holds the key context, so typed letters are text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentBox {
    pub review: ReviewId,
    pub thread: ThreadId,
    pub comment: CommentId,
    /// None for a draft in an existing thread. Lines are counted from 1, as the Link counts them.
    pub anchor: Option<Anchor>,
    pub at: Position,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Position {
    pub path: PathBuf,
    pub side: Side,
    /// Counted from 0.
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unshown {
    Loading,
    Binary,
    TooLarge,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneRow {
    Commit {
        id: String,
        summary: String,
    },
    File {
        path: PathBuf,
        status: FileStatus,
    },
    Diff {
        path: PathBuf,
        row: Row,
    },
    Unshown {
        path: PathBuf,
        why: Unshown,
    },
    Thread {
        thread: ThreadId,
    },
    Comment {
        thread: ThreadId,
        comment: CommentId,
    },
    /// The heading of the threads whose line no row shows, after the last file, so that no thread
    /// is hidden; a workspace whose worktree is gone has all its threads there.
    ThreadsElsewhere,
    /// A file the live diff's turn read and did not change: no diff, no line to be on. Only the
    /// live diff holds such rows, after every other row, in order of first read.
    Read {
        path: PathBuf,
    },
}

struct ShownFile {
    file: ChangedFile,
    content: FileContent,
    rows: Range<usize>,
    placed: Vec<ThreadId>,
}

enum FileContent {
    Unshown(Unshown),
    Diff(Box<ShownDiff>),
}

struct Asked {
    index: usize,
    sides: Task<Result<FileSides, AskError>>,
}

struct ShownDiff {
    diff: FileDiff,
    emphasis: RefCell<HashMap<usize, Rc<Emphasis>>>,
    max_line_chars: usize,
}

impl ShownDiff {
    pub(crate) fn from_parts(diff: FileDiff, max_line_chars: usize) -> Self {
        Self {
            diff,
            emphasis: RefCell::default(),
            max_line_chars,
        }
    }
}

pub(crate) fn compute_diff(old: &[u8], new: &[u8]) -> (FileDiff, usize) {
    let diff = FileDiff::new(old, new);
    let max_chars = max_line_chars(&diff);
    (diff, max_chars)
}

fn max_line_chars(diff: &FileDiff) -> usize {
    let mut max_chars = 0usize;
    for side in [Side::Old, Side::New] {
        for line in 0..diff.line_count(side) {
            let text = String::from_utf8_lossy(diff.line(side, line));
            let end = text.floor_char_boundary(draw::DRAWN_BYTES);
            max_chars = max_chars.max(text[..end].chars().count());
        }
    }
    max_chars
}

/// The reviewer is on one line while a diff is shown. Lines have one order whatever the layout,
/// that of the unified view: removed lines of a change before those added; in the split view
/// [`LineDown`] goes down the old side, then the new.
pub struct ReviewPane {
    models: Models,
    focus: FocusHandle,
    layout: Layout,
    rows: Vec<PaneRow>,
    position: Option<Position>,
    position_row: Option<usize>,
    changes: Option<Changes>,
    files: Vec<ShownFile>,
    by_path: HashMap<PathBuf, usize>,
    reading: Vec<Task<()>>,
    unread: Range<usize>,
    list: ListState,
    list_width: Pixels,
    h_scroll: Pixels,
    reviews_of: Option<WorkspaceId>,
    reviews: Option<WorkspaceReviews>,
    saves: Vec<Save>,
    saved: u64,
    reading_reviews: Option<Task<()>>,
    open_box: Option<OpenBox>,
    commit_field: Option<CommitField>,
    notice: Option<String>,
    tree: Tree,
    listed: Option<Source>,
    keep: bool,
    chosen: Option<ReviewId>,
    sessions: Vec<Session>,
    session_picker_open: bool,
    show_empty_past_sessions: bool,
    session_picker_bounds: Rc<Cell<Bounds<Pixels>>>,
    view: View,
    form: FlowForm,
    at: Option<FlowAt>,
    following: Following,
    flow: flow::Timeline,
    live: live::Live,
    follow: follow::Follow,
    attaching: attaching::Attaching,
    moved: moved::Moved,
    navigate: navigate::Navigate,
    first_diff_marked: bool,
}

impl ReviewPane {
    pub fn new(models: &Models, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_in(&models.changes, window, |pane, _, window, cx| {
            pane.follow_history(window, cx);
            pane.follow(cx);
            pane.follow_reviews(window, cx);
            pane.sessions_changed(cx);
            pane.flow_workspace(cx);
            pane.live_workspace(cx);
            pane.follow_workspace(cx);
        })
        .detach();
        cx.observe(&models.agents, |pane, _, cx| {
            pane.cli_chosen(cx);
            pane.sessions_changed(cx);
            pane.flow_agents(cx);
            pane.live_agents(cx);
            pane.follow_agents(cx);
        })
        .detach();
        cx.observe(&models.settings, |pane, _, cx| {
            pane.turn_end_setting(cx);
            pane.flow_settings(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&models.navigation, |pane, _, cx| pane.servers_changed(cx))
            .detach();
        cx.subscribe(&models.link, |pane, _, fact: &Fact, cx| pane.told(fact, cx))
            .detach();
        let mut pane = Self {
            models: models.clone(),
            focus: cx.focus_handle(),
            layout: FIRST_LAYOUT,
            rows: Vec::new(),
            position: None,
            position_row: None,
            changes: None,
            files: Vec::new(),
            by_path: HashMap::new(),
            reading: Vec::new(),
            unread: 0..0,
            list: ListState::new(0, ListAlignment::Top, px(200.)),
            list_width: px(0.),
            h_scroll: px(0.),
            reviews_of: None,
            reviews: None,
            saves: Vec::new(),
            saved: 0,
            reading_reviews: None,
            open_box: None,
            commit_field: None,
            notice: None,
            tree: Tree::new(cx),
            listed: None,
            keep: models.settings.read(cx).turn_end() == TurnEndSelects::KeepSelection,
            chosen: None,
            sessions: Vec::new(),
            session_picker_open: false,
            show_empty_past_sessions: false,
            session_picker_bounds: Rc::default(),
            view: View::Changes,
            form: FlowForm::LiveDiff,
            at: None,
            following: Following::On,
            flow: flow::Timeline::new(),
            live: live::Live::new(),
            follow: follow::Follow::new(cx),
            attaching: attaching::Attaching::new(models.agents.read(cx).chosen_cli().cloned()),
            moved: moved::Moved::default(),
            navigate: navigate::Navigate::default(),
            first_diff_marked: false,
        };
        pane.sessions_changed(cx);
        pane.follow_history(window, cx);
        pane.follow(cx);
        pane.follow_reviews(window, cx);
        pane.flow_workspace(cx);
        pane.live_workspace(cx);
        pane.follow_workspace(cx);
        pane.flow_agents(cx);
        pane.live_agents(cx);
        pane.follow_agents(cx);
        pane.flow_settings(cx);
        pane
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn h_scroll(&self) -> Pixels {
        self.h_scroll
    }

    pub(crate) fn max_h_scroll(&self) -> Pixels {
        let max_chars = self
            .files
            .iter()
            .filter_map(|held| match &held.content {
                FileContent::Diff(shown) => Some(shown.max_line_chars),
                FileContent::Unshown(_) => None,
            })
            .max()
            .unwrap_or(0);
        if max_chars == 0 {
            return px(0.);
        }
        let viewport = if self.list_width > px(0.) {
            self.list_width
        } else {
            self.list.viewport_bounds().size.width
        };
        if viewport <= px(0.) {
            return px(0.);
        }
        let code_width = match self.layout {
            Layout::Unified => (viewport - px(125.)).max(px(0.)),
            Layout::Split => (viewport / 2. - px(74.)).max(px(0.)),
        };
        let content_width = px(max_chars as f32 * 8.0) + yaar_ui::metrics::SPACE_6;
        (content_width - code_width).max(px(0.))
    }

    pub(crate) fn scroll_horizontal(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(delta) => (delta.x, delta.y),
            ScrollDelta::Lines(lines) => (px(lines.x * 24.), px(lines.y * 24.)),
        };
        let delta_x = if dx != px(0.) {
            dx
        } else if event.modifiers.shift && dy != px(0.) {
            dy
        } else {
            return;
        };
        let max = self.max_h_scroll();
        let next = (self.h_scroll - delta_x).clamp(px(0.), max);
        if next != self.h_scroll {
            self.h_scroll = next;
            self.scrolled();
            cx.notify();
        }
    }

    pub fn rows(&self) -> &[PaneRow] {
        &self.rows
    }

    /// The comparison shown in the review header while Changes displays the workspace's changes
    /// against its base, and whether the header offers to set a base.
    pub fn comparison(&self) -> Option<(&Comparison, bool)> {
        if self.tree.shown != ChangeShown::Comparison {
            return None;
        }
        let changes = self.changes.as_ref()?;
        Some((
            &changes.comparison,
            matches!(changes.comparison, Comparison::NothingToCompare { .. }),
        ))
    }

    pub fn text(&self, position: &Position) -> Option<String> {
        let shown = self.diff_of(&position.path)?;
        (position.line < shown.diff.line_count(position.side))
            .then(|| shown.text(position.side, position.line).into_owned())
    }

    /// None while no diff is shown, and while a file above the first diff is on its way and no
    /// click chose a line.
    pub fn position(&self) -> Option<&Position> {
        self.position.as_ref()
    }

    pub fn position_row(&self) -> Option<usize> {
        self.position_row
    }

    pub fn rows_in_view(&self) -> Range<usize> {
        let len = self.rows.len();
        if len == 0 {
            return 0..0;
        }
        let viewport = self.list.viewport_bounds();
        if viewport.size.height <= px(0.) {
            return 0..0;
        }
        let mut start = self.list.logical_scroll_top().item_ix.min(len);
        while start < len
            && self
                .list
                .bounds_for_item(start)
                .is_some_and(|bounds| bounds.bottom() <= viewport.top())
        {
            start += 1;
        }
        let mut end = start;
        while end < len {
            match self.list.bounds_for_item(end) {
                Some(bounds) if bounds.top() < viewport.bottom() => end += 1,
                _ => break,
            }
        }
        start..end
    }

    /// A save of a draft is told to nobody, so the pane puts each saved text into its copy and
    /// keeps them after the box closes.
    pub fn reviews(&self) -> Option<&WorkspaceReviews> {
        self.reviews.as_ref()
    }

    pub fn thread_states(&self) -> Vec<(ThreadId, yaar_link::ThreadState)> {
        self.review()
            .map(|review| {
                review
                    .threads
                    .iter()
                    .map(|thread| (thread.id.clone(), thread.state))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Until one is chosen: the review of no agent if there is one, else the first running
    /// session's, else the first.
    pub fn review(&self) -> Option<&Review> {
        whose::shown(self.reviews.as_ref(), self.chosen.as_ref(), &self.sessions)
    }

    pub fn history(&self) -> Option<&History> {
        self.tree.history.as_ref()
    }

    pub fn change_shown(&self) -> ChangeShown {
        self.tree.shown.clone()
    }

    /// One more than the last entry stands for the uncommitted changes; none before the tree has
    /// had the focus.
    pub fn history_cursor(&self) -> Option<usize> {
        self.tree.cursor
    }

    pub fn marked_changes(&self) -> Vec<usize> {
        self.tree.marked.clone()
    }

    pub fn keeps_selection(&self) -> bool {
        self.keep
    }

    pub fn offer(&self) -> Option<&ChangeShown> {
        self.tree.offer.as_ref().map(|offer| &offer.change)
    }

    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.clone()
    }

    pub fn review_session(&self) -> Option<&SessionId> {
        self.review()?.agent.as_ref()
    }

    pub fn made_by_no_agent(&self, path: &Path) -> bool {
        self.tree.uncommitted.as_ref().is_some_and(|listed| {
            listed
                .iter()
                .any(|credited| credited.file.path == path && credited.by.is_empty())
        })
    }

    pub fn uncommitted(&self) -> Option<&[CreditedFile]> {
        self.tree.uncommitted.as_deref()
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn flow_form(&self) -> FlowForm {
        self.form
    }

    pub fn timeline(&self) -> &[TimelineEntry] {
        self.flow.entries()
    }

    pub fn timeline_cursor(&self) -> Option<usize> {
        self.flow.cursor()
    }

    pub fn flow_at(&self) -> Option<&FlowAt> {
        self.at.as_ref()
    }

    /// The row of `flow_at` in the form on screen; none when that form has none, as the live diff
    /// for a command, or for an edit or write whose file its list does not hold yet.
    pub fn flow_at_row(&self) -> Option<usize> {
        let at = self.at.as_ref()?;
        match self.form {
            FlowForm::Timeline => self.flow.row_of(at),
            FlowForm::LiveDiff => self.live_row_of(at),
        }
    }

    pub fn flow_rows_in_view(&self) -> Range<usize> {
        match self.form {
            FlowForm::Timeline => self.flow.in_view(),
            FlowForm::LiveDiff => self.live.in_view(),
        }
    }

    pub fn following(&self) -> Following {
        self.following
    }

    pub fn flow_offer(&self) -> Option<&ChangeShown> {
        self.follow.offer()
    }

    pub fn live_change(&self) -> Option<&LiveChange> {
        self.live.change()
    }

    pub fn live_rows(&self) -> &[PaneRow] {
        self.live.rows()
    }

    pub fn live_position(&self) -> Option<&Position> {
        self.live.position()
    }

    pub fn live_text(&self, position: &Position) -> Option<String> {
        self.live.text(position)
    }

    pub fn live_reading(&self) -> Option<LiveReadShown> {
        self.live.reading().map(|reading| LiveReadShown {
            path: reading.path.clone(),
            lines: reading.lines.clone(),
            line_count: reading.line_ranges.len(),
        })
    }

    pub fn live_reading_in_view(&self) -> Range<usize> {
        self.live
            .reading()
            .map_or(0..0, |reading| reading.in_view.borrow().clone())
    }

    /// The step the live diff names above its files, having no row for it: a command, or an edit
    /// or write whose file the live list does not hold yet.
    pub fn activity(&self) -> Option<&FlowAt> {
        let shown = self.live.change()?;
        self.at
            .as_ref()
            .filter(|at| at.turn == shown.turn && self.live_row_of(at).is_none())
    }

    pub fn comment_box(&self) -> Option<CommentBox> {
        self.open_box.as_ref().map(|open| open.view.clone())
    }

    pub fn commit_field_open(&self) -> bool {
        self.commit_field.is_some()
    }

    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let model = self.models.changes.read(cx);
        let Some(changes) = model.changes() else {
            self.changes = None;
            // The files of a recorded turn shown in the meantime are not the comparison's to take
            // away.
            if self.tree.shown == ChangeShown::Comparison {
                self.files.clear();
                self.by_path.clear();
                self.reading.clear();
                self.unread = 0..0;
                self.listed = None;
                self.position = None;
                self.h_scroll = px(0.);
                self.rebuild(cx);
            }
            return;
        };
        // The model keeps the last answer while it asks again and after a refusal: nothing new
        // came.
        let nothing_new = model.is_loading() || model.refusal().is_some();
        if nothing_new && self.changes.as_ref() == Some(changes) {
            return;
        }
        let changes = changes.clone();
        // The reviewer's selection stays when the comparison is read again; only what the pane
        // picked itself gives way.
        if self.tree.first_picture {
            self.tree.shown = ChangeShown::Comparison;
            self.tree.first_picture = false;
            self.tree.wanted = Source::Comparison;
            self.tree.listing = None;
        }
        if self.tree.shown == ChangeShown::Comparison {
            self.show(changes, cx);
        } else {
            self.changes = Some(changes);
        }
    }

    pub(crate) fn show(&mut self, changes: Changes, cx: &mut Context<Self>) {
        let same_workspace = self
            .changes
            .as_ref()
            .is_some_and(|shown| shown.workspace == changes.workspace);
        if !same_workspace {
            self.listed = None;
            self.h_scroll = px(0.);
        }
        let files = changes.files.clone();
        self.changes = Some(changes);
        self.show_files(files, Source::Comparison, cx);
    }

    pub(crate) fn show_files(
        &mut self,
        files: Vec<ChangedFile>,
        source: Source,
        cx: &mut Context<Self>,
    ) {
        let mut held: HashMap<PathBuf, FileContent> = HashMap::new();
        if self.listed.as_ref() == Some(&source) {
            held.extend(
                self.files
                    .drain(..)
                    .map(|shown| (shown.file.path, shown.content)),
            );
        } else {
            self.position = None;
            self.h_scroll = px(0.);
            self.to_top();
        }
        self.by_path = files
            .iter()
            .enumerate()
            .map(|(index, file)| (file.path.clone(), index))
            .collect();
        self.files = files
            .into_iter()
            .map(|file| ShownFile {
                content: held
                    .remove(&file.path)
                    .unwrap_or(FileContent::Unshown(Unshown::Loading)),
                file,
                rows: 0..0,
                placed: Vec::new(),
            })
            .collect();
        self.listed = Some(source);
        self.rebuild(cx);
        self.unread = 0..self.files.len();
        self.reading.clear();
        self.reading = (0..FILES_ON_THEIR_WAY)
            .map_while(|_| self.read(cx))
            .collect();
    }

    fn read(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let mut asked = self.ask(cx)?;
        Some(cx.spawn(async move |pane, cx| {
            loop {
                let content = match asked.sides.await.map(versions) {
                    Ok(Ok((old, new))) => {
                        // The diff of a large file takes long enough to be seen: it is made off the
                        // drawing thread.
                        let (diff, max_chars) = cx
                            .background_spawn(async move { compute_diff(&old, &new) })
                            .await;
                        FileContent::Diff(Box::new(ShownDiff::from_parts(diff, max_chars)))
                    }
                    Ok(Err(why)) => FileContent::Unshown(why),
                    Err(_) => FileContent::Unshown(Unshown::Refused),
                };
                let next = pane.update(cx, |pane, cx| {
                    pane.loaded(asked.index, content, cx);
                    pane.ask(cx)
                });
                match next {
                    Ok(Some(next)) => asked = next,
                    Ok(None) | Err(_) => break,
                }
            }
        }))
    }

    fn ask(&mut self, cx: &mut Context<Self>) -> Option<Asked> {
        let index = self.unread.next()?;
        let file = &self.files[index].file;
        let sides = match &self.listed {
            Some(Source::Versions { old, new }) => {
                self.read_versions(file.clone(), old.clone(), new.clone(), cx)
            }
            Some(Source::Comparison) | None => self
                .models
                .changes
                .update(cx, |changes, cx| changes.file(file, cx)),
        };
        Some(Asked { index, sides })
    }

    fn read_versions(
        &self,
        file: ChangedFile,
        old: Option<String>,
        new: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<Result<FileSides, AskError>> {
        let Some(workspace) = self.tree.of.clone() else {
            return Task::ready(Err(AskError::Unexpected));
        };
        let old_path = match &file.status {
            FileStatus::Renamed { from } => from.clone(),
            _ => file.path.clone(),
        };
        let request = yaar_link::Request::ReadVersions {
            workspace,
            old,
            old_path,
            new,
            path: file.path,
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        cx.spawn(async move |_, _| match asked.await {
            Ok(yaar_link::Reply::File(sides)) => Ok(sides),
            Ok(_) => Err(AskError::Unexpected),
            Err(error) => Err(error),
        })
    }

    fn loaded(&mut self, index: usize, content: FileContent, cx: &mut Context<Self>) {
        let top = self.list.logical_scroll_top();
        let review = whose::shown(self.reviews.as_ref(), self.chosen.as_ref(), &self.sessions);
        let shown = self.new_side_shown();
        let file = &mut self.files[index];
        file.content = content;
        let (rows, placed) = file_rows_in(file, self.layout, review, shown.as_deref());
        file.placed = placed;
        let was = file.rows.clone();
        file.rows = was.start..was.start + rows.len();
        let now = file.rows.end;
        drop(self.rows.splice(was.clone(), rows));
        for below in &mut self.files[index + 1..] {
            below.rows = below.rows.start - was.end + now..below.rows.end - was.end + now;
        }
        self.rows
            .truncate(self.files.last().map_or(0, |last| last.rows.end));
        self.rows.extend(threads::elsewhere(&self.files, review));
        let next_top = if was.end <= top.item_ix {
            ListOffset {
                item_ix: top.item_ix - was.end + now,
                offset_in_item: top.offset_in_item,
            }
        } else {
            top
        };
        self.sync_list(Some(next_top));
        self.settle(cx);
    }

    pub(crate) fn rebuild(&mut self, cx: &mut Context<Self>) {
        let top = self.list.logical_scroll_top();
        let review = whose::shown(self.reviews.as_ref(), self.chosen.as_ref(), &self.sessions);
        let shown = self.new_side_shown();
        let mut rows = Vec::new();
        if let (Some(changes), ChangeShown::Comparison) = (&self.changes, &self.tree.shown) {
            rows.extend(changes.commits.iter().map(|commit| PaneRow::Commit {
                id: commit.id.clone(),
                summary: commit.summary.clone(),
            }));
        }
        for file in &mut self.files {
            let start = rows.len();
            let (held, placed) = file_rows_in(file, self.layout, review, shown.as_deref());
            rows.extend(held);
            file.placed = placed;
            file.rows = start..rows.len();
        }
        rows.extend(threads::elsewhere(&self.files, review));
        let unchanged = self.rows == rows;
        self.rows = rows;
        if unchanged {
            self.list.remeasure();
        } else {
            let clamped = ListOffset {
                item_ix: top.item_ix.min(self.rows.len()),
                offset_in_item: if top.item_ix < self.rows.len() {
                    top.offset_in_item
                } else {
                    px(0.)
                },
            };
            self.sync_list(Some(clamped));
        }
        self.settle(cx);
        self.tell_old_side(cx);
        self.live_rebuild(cx);
    }

    fn settle(&mut self, cx: &mut Context<Self>) {
        self.position_row = self.position.as_ref().and_then(|at| self.locate(at));
        if self.position_row.is_none() {
            self.position = self.first_change();
            self.position_row = self.position.as_ref().and_then(|at| self.locate(at));
        }
        cx.notify();
    }

    fn diff_of(&self, path: &Path) -> Option<&ShownDiff> {
        match &self.files[*self.by_path.get(path)?].content {
            FileContent::Diff(shown) => Some(shown),
            FileContent::Unshown(_) => None,
        }
    }

    fn locate(&self, position: &Position) -> Option<usize> {
        let rows = self.files[*self.by_path.get(&position.path)?].rows.clone();
        let start = rows.start;
        self.rows[rows]
            .iter()
            .position(|row| {
                stops_of(row).any(|(side, line)| (side, line) == (position.side, position.line))
            })
            .map(|index| start + index)
    }

    fn first_change(&self) -> Option<Position> {
        for file in &self.files {
            match &file.content {
                FileContent::Unshown(Unshown::Loading) => return None,
                FileContent::Unshown(_) => {}
                FileContent::Diff(_) => {
                    let changed = self.rows[file.rows.clone()]
                        .iter()
                        .find_map(|row| match row {
                            PaneRow::Diff {
                                row: Row::Context { .. },
                                ..
                            } => None,
                            row => stops_of(row).next(),
                        });
                    if let Some((side, line)) = changed {
                        return Some(Position {
                            path: file.file.path.clone(),
                            side,
                            line,
                        });
                    }
                }
            }
        }
        None
    }

    pub(crate) fn line_down(&mut self, _: &LineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        if self.file_view_step(1, cx) {
            return;
        }
        if self.view == View::Flow {
            return self.flow_step(1, cx);
        }
        self.step(1, cx);
    }

    pub(crate) fn line_up(&mut self, _: &LineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        if self.file_view_step(-1, cx) {
            return;
        }
        if self.view == View::Flow {
            return self.flow_step(-1, cx);
        }
        self.step(-1, cx);
    }

    fn flow_step(&mut self, by: isize, cx: &mut Context<Self>) {
        match self.form {
            FlowForm::Timeline => self.timeline_step(by, cx),
            FlowForm::LiveDiff => self.live_step(by, cx),
        }
    }

    pub(crate) fn toggle_layout(
        &mut self,
        _: &ToggleLayout,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let was_delta = self
            .position_row
            .and_then(|row| self.row_offset_from_top(row));
        self.layout = match self.layout {
            Layout::Unified => Layout::Split,
            Layout::Split => Layout::Unified,
        };
        self.h_scroll = px(0.);
        self.rebuild(cx);
        if let Some(now) = self.position_row {
            match was_delta {
                Some(delta) => self.scroll_row_to_offset(now, delta),
                None => self.list.scroll_to_reveal_item(now),
            }
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let (Some(position), Some(row)) = (&self.position, self.position_row) else {
            return;
        };
        let stops = stops(&self.rows);
        let Some(now) = stops.iter().position(|stop| *stop == (row, position.side)) else {
            return;
        };
        let Some(&(row, side)) = now.checked_add_signed(by).and_then(|next| stops.get(next)) else {
            return;
        };
        self.go_to(row, side, cx);
        self.list.scroll_to_reveal_item(row);
    }

    pub(crate) fn go_to(&mut self, index: usize, side: Side, cx: &mut Context<Self>) {
        let Some(PaneRow::Diff { path, row }) = self.rows.get(index) else {
            return;
        };
        let Some(line) = row.line(side) else {
            return;
        };
        self.position = Some(Position {
            path: path.clone(),
            side,
            line,
        });
        self.position_row = Some(index);
        self.follow_cursor();
        cx.notify();
    }

    pub(crate) fn jump_to_file(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.list.scroll_to(ListOffset {
            item_ix: row.min(self.rows.len()),
            offset_in_item: px(0.),
        });
        if let Some((diff_row, side)) = self
            .rows
            .iter()
            .enumerate()
            .skip(row + 1)
            .take_while(|(_, held)| !matches!(held, PaneRow::File { .. }))
            .find_map(|(index, held)| stops_of(held).next().map(|(side, _)| (index, side)))
        {
            self.go_to(diff_row, side, cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn toggle_viewed_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.acted(cx);
        let (Some(workspace), Some(reviews)) = (self.reviews_of.clone(), &self.reviews) else {
            return self.say("The reviews are not here yet", cx);
        };
        let viewed = !reviews.viewed.contains(&path);
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                yaar_link::Request::SetViewed {
                    workspace,
                    path,
                    viewed,
                },
                cx,
            )
        });
        self.report(asked, cx);
    }

    pub(crate) fn resolve_or_reopen_thread(
        &mut self,
        thread: ThreadId,
        resolve: bool,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        let request = if resolve {
            yaar_link::Request::ResolveThread { review, thread }
        } else {
            yaar_link::Request::ReopenThread { review, thread }
        };
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(request, cx));
        self.report(asked, cx);
    }

    pub(crate) fn attach_only_thread(&mut self, thread: ThreadId, cx: &mut Context<Self>) {
        self.acted(cx);
        self.attach(Some(thread), cx);
    }

    fn to_top(&self) {
        self.list.scroll_to(ListOffset::default());
    }

    pub(crate) fn sync_list(&self, anchor: Option<ListOffset>) {
        self.list
            .reset_with_uniform_height(self.rows.len(), draw::ROW_HEIGHT);
        if let Some(offset) = anchor {
            self.list.scroll_to(offset);
        }
    }

    pub(crate) fn row_offset_from_top(&self, index: usize) -> Option<Pixels> {
        let viewport = self.list.viewport_bounds();
        if viewport.size.height <= px(0.) {
            return None;
        }
        let bounds = self.list.bounds_for_item(index)?;
        (bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom())
            .then_some(bounds.top() - viewport.top())
    }

    pub(crate) fn row_measured_height(&self, index: usize) -> Option<Pixels> {
        Some(self.list.bounds_for_item(index)?.size.height)
    }

    pub(crate) fn scroll_row_to_offset_with_heights(
        &self,
        row: usize,
        delta_from_top: Pixels,
        known: &[(PaneRow, Pixels)],
    ) {
        if delta_from_top <= px(0.) {
            self.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: -delta_from_top,
            });
            return;
        }
        let mut remaining = delta_from_top;
        let mut item_ix = row;
        while item_ix > 0 && remaining > px(0.) {
            let prev = item_ix - 1;
            let height = known
                .iter()
                .find(|(held, _)| *held == self.rows[prev])
                .map(|(_, height)| *height)
                .unwrap_or_else(|| match self.rows[prev] {
                    PaneRow::File { .. } => draw::FILE_HEADER_HEIGHT,
                    PaneRow::Commit { .. } => draw::COMMIT_ROW_HEIGHT,
                    PaneRow::Unshown { .. } => draw::UNSHOWN_ROW_HEIGHT,
                    _ => draw::ROW_HEIGHT,
                });
            if remaining <= height {
                self.list.scroll_to(ListOffset {
                    item_ix: prev,
                    offset_in_item: height - remaining,
                });
                return;
            }
            remaining -= height;
            item_ix = prev;
        }
        self.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
    }

    pub(crate) fn scroll_row_to_offset(&self, row: usize, delta_from_top: Pixels) {
        self.scroll_row_to_offset_with_heights(row, delta_from_top, &[]);
    }
}

fn versions(sides: FileSides) -> Result<(Vec<u8>, Vec<u8>), Unshown> {
    let bytes = |content| match content {
        Content::Absent => Ok(Vec::new()),
        Content::Text(bytes) => Ok(bytes),
        Content::Binary { .. } => Err(Unshown::Binary),
        Content::TooLarge { .. } => Err(Unshown::TooLarge),
    };
    Ok((bytes(sides.old)?, bytes(sides.new)?))
}

fn file_rows(
    file: &ShownFile,
    layout: Layout,
    review: Option<&Review>,
) -> (Vec<PaneRow>, Vec<ThreadId>) {
    file_rows_in(file, layout, review, None)
}

fn file_rows_in(
    file: &ShownFile,
    layout: Layout,
    review: Option<&Review>,
    shown: Option<&str>,
) -> (Vec<PaneRow>, Vec<ThreadId>) {
    let path = &file.file.path;
    let mut rows = vec![PaneRow::File {
        path: path.clone(),
        status: file.file.status.clone(),
    }];
    match &file.content {
        FileContent::Unshown(why) => rows.push(PaneRow::Unshown {
            path: path.clone(),
            why: *why,
        }),
        FileContent::Diff(shown) => rows.extend(
            shown
                .diff
                .rows(layout, CONTEXT_LINES)
                .into_iter()
                .map(|row| PaneRow::Diff {
                    path: path.clone(),
                    row,
                }),
        ),
    }
    threads::with_threads(&file.file, rows, review, shown)
}

fn stops_of(row: &PaneRow) -> impl Iterator<Item = (Side, u32)> {
    let (old, new) = match row {
        PaneRow::Diff { row, .. } => match *row {
            Row::Context { new, .. } => (None, Some(new)),
            Row::Skipped { .. } => (None, None),
            Row::Removed { .. } | Row::Added { .. } | Row::Changed { .. } => {
                (row.line(Side::Old), row.line(Side::New))
            }
        },
        _ => (None, None),
    };
    old.map(|line| (Side::Old, line))
        .into_iter()
        .chain(new.map(|line| (Side::New, line)))
}

fn stops(rows: &[PaneRow]) -> Vec<(usize, Side)> {
    let changed = |row: &PaneRow| {
        matches!(
            row,
            PaneRow::Diff {
                row: Row::Changed { .. },
                ..
            }
        )
    };
    let lines: Vec<usize> = (0..rows.len())
        .filter(|&index| {
            !matches!(
                rows[index],
                PaneRow::Thread { .. } | PaneRow::Comment { .. } | PaneRow::ThreadsElsewhere
            )
        })
        .collect();
    let mut stops = Vec::new();
    let mut next = 0;
    while next < lines.len() {
        let run = lines[next..]
            .iter()
            .take_while(|&&index| changed(&rows[index]))
            .count()
            .max(1);
        let run = &lines[next..next + run];
        for side in [Side::Old, Side::New] {
            for &index in run {
                if stops_of(&rows[index]).any(|(stop, _)| stop == side) {
                    stops.push((index, side));
                }
            }
        }
        next += run.len();
    }
    stops
}

#[cfg(test)]
mod tests {
    use gpui_kit::Action;
    use yaar_frame::{Asks, Place, Sends};
    use yaar_link::RequestKind;

    use super::{
        AllowServers, AttachReview, AttachThread, CloseReferences, DetachReview, DownloadServer,
        FindReferences, GoBack, GoToDefinition, Hover, NextName, NextOfferedAgent, OpenReference,
        PreviousName, REFERENCES_CONTEXT, ReferenceDown, ReferenceUp, RefuseServers, ReopenThread,
        ResolveAddressed, ResolveThread, ShowOriginal, SinceRound, StartAgent, UpdateServer,
    };
    use super::{
        Comment, Commit, CopyReview, DeleteDraft, EditDraft, FocusHistory, FocusRows,
        HISTORY_CONTEXT, HistoryDown, HistoryUp, KEY_CONTEXT, LineDown, LineUp, MarkChange,
        MarkReviewed, NextReview, NextThread, PreviousThread, Reply, ResumeFollow, ShowChange,
        TakeOffer, ToggleFlowForm, ToggleFollow, ToggleKeepSelection, ToggleLayout, ToggleView,
        ToggleViewed, controls,
    };

    #[test]
    fn the_keys_of_the_pane_work_in_the_pane_only_and_the_review_actions_send_what_they_name() {
        let table: Vec<(&str, &str, Option<Sends>, Asks)> = controls()
            .iter()
            .map(|control| {
                let context = if [
                    FocusRows.name(),
                    HistoryDown.name(),
                    HistoryUp.name(),
                    MarkChange.name(),
                    ShowChange.name(),
                ]
                .contains(&(control.action)().name())
                {
                    HISTORY_CONTEXT
                } else if [
                    ReferenceDown.name(),
                    ReferenceUp.name(),
                    OpenReference.name(),
                    CloseReferences.name(),
                ]
                .contains(&(control.action)().name())
                {
                    REFERENCES_CONTEXT
                } else {
                    KEY_CONTEXT
                };
                assert_eq!(control.place, Place::In(context), "{}", control.title);
                assert_eq!(control.context(), Some(context), "{}", control.title);
                (
                    control.keystroke,
                    (control.action)().name(),
                    control.sends,
                    control.asks,
                )
            })
            .collect();
        let request = |kind| Some(Sends::Request(kind));
        assert_eq!(
            table,
            [
                ("j", LineDown.name(), None, Asks::Nothing),
                ("down", LineDown.name(), None, Asks::Nothing),
                ("k", LineUp.name(), None, Asks::Nothing),
                ("up", LineUp.name(), None, Asks::Nothing),
                ("s", ToggleLayout.name(), None, Asks::Nothing),
                ("c", Comment.name(), None, Asks::Nothing),
                ("i", EditDraft.name(), None, Asks::Nothing),
                ("o", Reply.name(), None, Asks::Nothing),
                (
                    "d d",
                    DeleteDraft.name(),
                    request(RequestKind::DeleteDraft),
                    Asks::Nothing
                ),
                ("m", NextThread.name(), None, Asks::Nothing),
                ("shift-m", PreviousThread.name(), None, Asks::Nothing),
                (
                    "y",
                    CopyReview.name(),
                    request(RequestKind::CopyReview),
                    Asks::Nothing
                ),
                (
                    "r",
                    ToggleViewed.name(),
                    request(RequestKind::SetViewed),
                    Asks::Nothing
                ),
                (
                    "shift-r",
                    MarkReviewed.name(),
                    request(RequestKind::MarkReviewed),
                    Asks::Nothing
                ),
                (
                    "shift-c",
                    Commit.name(),
                    request(RequestKind::Commit),
                    Asks::Text
                ),
                ("tab", FocusHistory.name(), None, Asks::Nothing),
                ("tab", FocusRows.name(), None, Asks::Nothing),
                ("j", HistoryDown.name(), None, Asks::Nothing),
                ("down", HistoryDown.name(), None, Asks::Nothing),
                ("k", HistoryUp.name(), None, Asks::Nothing),
                ("up", HistoryUp.name(), None, Asks::Nothing),
                ("space", MarkChange.name(), None, Asks::Nothing),
                ("enter", ShowChange.name(), None, Asks::Nothing),
                (
                    "shift-t",
                    ToggleKeepSelection.name(),
                    request(RequestKind::SetSetting),
                    Asks::Nothing
                ),
                ("shift-o", TakeOffer.name(), None, Asks::Nothing),
                ("shift-a", NextReview.name(), None, Asks::Nothing),
                ("f", ToggleView.name(), None, Asks::Nothing),
                ("shift-f", ToggleFollow.name(), None, Asks::Nothing),
                (
                    "shift-l",
                    ToggleFlowForm.name(),
                    request(RequestKind::SetSetting),
                    Asks::Nothing
                ),
                ("shift-g", ResumeFollow.name(), None, Asks::Nothing),
                (
                    "a",
                    AttachReview.name(),
                    request(RequestKind::AttachReview),
                    Asks::Nothing
                ),
                (
                    "t",
                    AttachThread.name(),
                    request(RequestKind::AttachReview),
                    Asks::Nothing
                ),
                (
                    "shift-d",
                    DetachReview.name(),
                    request(RequestKind::DetachReview),
                    Asks::Nothing
                ),
                (
                    "x",
                    ResolveThread.name(),
                    request(RequestKind::ResolveThread),
                    Asks::Nothing
                ),
                (
                    "u",
                    ReopenThread.name(),
                    request(RequestKind::ReopenThread),
                    Asks::Nothing
                ),
                (
                    "shift-x",
                    ResolveAddressed.name(),
                    request(RequestKind::ResolveAddressed),
                    Asks::Nothing
                ),
                ("w", ShowOriginal.name(), None, Asks::Nothing),
                ("shift-s", SinceRound.name(), None, Asks::Nothing),
                ("shift-n", StartAgent.name(), None, Asks::Nothing),
                ("ctrl-n", NextOfferedAgent.name(), None, Asks::Nothing),
                ("l", NextName.name(), None, Asks::Nothing),
                ("h", PreviousName.name(), None, Asks::Nothing),
                ("shift-k", Hover.name(), None, Asks::Nothing),
                ("g d", GoToDefinition.name(), None, Asks::Nothing),
                ("g r", FindReferences.name(), None, Asks::Nothing),
                ("escape", GoBack.name(), None, Asks::Nothing),
                ("j", ReferenceDown.name(), None, Asks::Nothing),
                ("down", ReferenceDown.name(), None, Asks::Nothing),
                ("k", ReferenceUp.name(), None, Asks::Nothing),
                ("up", ReferenceUp.name(), None, Asks::Nothing),
                ("enter", OpenReference.name(), None, Asks::Nothing),
                ("escape", CloseReferences.name(), None, Asks::Nothing),
                ("shift-i", DownloadServer.name(), None, Asks::Nothing),
                ("shift-u", UpdateServer.name(), None, Asks::Nothing),
                (
                    "shift-y",
                    AllowServers.name(),
                    request(RequestKind::SetSetting),
                    Asks::Nothing
                ),
                (
                    "shift-q",
                    RefuseServers.name(),
                    request(RequestKind::SetSetting),
                    Asks::Nothing
                ),
            ]
        );
    }

    const TUICR: &str = "tuicr 0.29.0 docs/KEYBINDINGS.md";

    #[test]
    fn for_each_action_that_vim_or_tuicr_has_a_key_for_the_default_key_is_that_key() {
        let keys: Vec<(&str, &str, Vec<&str>)> = vec![
            (LineDown.name(), "j", vec!["vim", TUICR]),
            (LineDown.name(), "down", vec![TUICR]),
            (LineUp.name(), "k", vec!["vim", TUICR]),
            (LineUp.name(), "up", vec![TUICR]),
            (Comment.name(), "c", vec![TUICR]),
            (EditDraft.name(), "i", vec![TUICR]),
            (DeleteDraft.name(), "d d", vec![TUICR, "vim"]),
            (NextThread.name(), "m", vec![TUICR]),
            (PreviousThread.name(), "shift-m", vec![TUICR]),
            (CopyReview.name(), "y", vec![TUICR]),
            (ToggleViewed.name(), "r", vec![TUICR]),
            (FocusHistory.name(), "tab", vec![TUICR]),
            (FocusRows.name(), "tab", vec![TUICR]),
            (HistoryDown.name(), "j", vec!["vim", TUICR]),
            (HistoryDown.name(), "down", vec![TUICR]),
            (HistoryUp.name(), "k", vec!["vim", TUICR]),
            (HistoryUp.name(), "up", vec![TUICR]),
            (MarkChange.name(), "space", vec![TUICR]),
            (ShowChange.name(), "enter", vec![TUICR]),
            (ResumeFollow.name(), "shift-g", vec!["vim", TUICR]),
            (NextName.name(), "l", vec!["vim"]),
            (PreviousName.name(), "h", vec!["vim"]),
            (Hover.name(), "shift-k", vec!["vim"]),
            (GoToDefinition.name(), "g d", vec!["vim"]),
            (ReferenceDown.name(), "j", vec!["vim", TUICR]),
            (ReferenceDown.name(), "down", vec![TUICR]),
            (ReferenceUp.name(), "k", vec!["vim", TUICR]),
            (ReferenceUp.name(), "up", vec![TUICR]),
            (OpenReference.name(), "enter", vec![TUICR]),
        ];
        let others: Vec<(&str, &str)> = vec![
            (
                ToggleLayout.name(),
                "tuicr has the command `:diff`, vim no such thing",
            ),
            (
                Reply.name(),
                "neither has one; vim's `o` opens a line below",
            ),
            (
                MarkReviewed.name(),
                "tuicr's `R` marks a hunk reviewed, not the change",
            ),
            (
                Commit.name(),
                "neither has one; tuicr's `C` adds a file comment",
            ),
            (
                ToggleKeepSelection.name(),
                "neither has one; no turn ends under the selection in either",
            ),
            (
                TakeOffer.name(),
                "neither has one; neither offers a change to show",
            ),
            (
                NextReview.name(),
                "neither has one; neither shows the review of another agent session",
            ),
            (
                ToggleView.name(),
                "neither has one; neither shows what an agent does as it works",
            ),
            (
                ToggleFollow.name(),
                "neither has one; neither follows an agent",
            ),
            (
                ToggleFlowForm.name(),
                "neither has one; neither has a timeline of steps or a live diff",
            ),
            (
                AttachReview.name(),
                "neither has one; neither sends a review with a message to an agent",
            ),
            (
                AttachThread.name(),
                "neither has one; neither sends one thread alone",
            ),
            (
                DetachReview.name(),
                "neither has one; vim's `D` deletes to the end of a line",
            ),
            (
                ResolveThread.name(),
                "neither has one; tuicr resolves remote threads only through `:comments`",
            ),
            (
                ReopenThread.name(),
                "neither has one; vim's `u` undoes the last change, the nearest action",
            ),
            (
                ResolveAddressed.name(),
                "neither has one; neither has an agent that marks a thread addressed",
            ),
            (
                ShowOriginal.name(),
                "neither has one; neither keeps the line a comment was written on",
            ),
            (
                SinceRound.name(),
                "neither has one; neither sends rounds of a review",
            ),
            (
                StartAgent.name(),
                "neither has one; neither starts an agent",
            ),
            (
                NextOfferedAgent.name(),
                "neither has one; vim's `ctrl-n` is a second key for a line down, which `j` has",
            ),
            (
                FindReferences.name(),
                "neither has one; Neovim's `grr`, shortened, as vim's `gr` replaces text",
            ),
            (
                GoBack.name(),
                "vim's `ctrl-o` jumps back, but it opens a folder on Linux; tuicr's Escape leaves",
            ),
            (
                CloseReferences.name(),
                "neither has one; tuicr's Escape leaves a mode",
            ),
            (
                DownloadServer.name(),
                "neither has one; neither downloads a language server",
            ),
            (
                UpdateServer.name(),
                "neither has one; neither updates a language server",
            ),
            (
                AllowServers.name(),
                "neither has one; neither asks before a server inside a repository runs",
            ),
            (
                RefuseServers.name(),
                "neither has one; neither asks before a server inside a repository runs",
            ),
        ];
        let held = controls();
        for (action, key, source) in &keys {
            assert!(!source.is_empty(), "{action} has a key of nobody's");
            assert!(
                held.iter().any(
                    |control| (control.action)().name() == *action && control.keystroke == *key
                ),
                "{action} has no control with the key {key} that {source:?} give it"
            );
        }
        for control in &held {
            let name = (control.action)().name();
            assert!(
                keys.iter().any(|(action, ..)| *action == name)
                    || others
                        .iter()
                        .any(|(action, why)| *action == name && !why.is_empty()),
                "{name} is in neither list: say which key of vim or tuicr it takes, or why neither has one"
            );
        }
        for (action, ..) in &others {
            assert!(
                keys.iter().all(|(listed, ..)| listed != action),
                "{action} is in both lists"
            );
            assert!(
                held.iter()
                    .any(|control| (control.action)().name() == *action),
                "{action} has no control"
            );
        }
    }
}
