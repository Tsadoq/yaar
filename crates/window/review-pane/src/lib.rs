//! Review pane (`W-REVIEW`).
//!
//! Owns: Flow and Changes, unified and split diff, threads and comment boxes, file views, follow.
//!
//! The pane shows one of two views, [`View`]: Changes, the change of the worktree the
//! reviewer selects, and Flow, what the agent does as it works. Flow has two forms,
//! `yaar_link::FlowForm`: the timeline of the agent's steps, each read, edit, new file and
//! command in order ([`TimelineEntry`]), and the live diff of the running turn
//! ([`LiveChange`]). While follow is on ([`Following`]), Flow moves to each step of the
//! agent as it comes ([`FlowAt`]); a scroll, a click or a key in the pane pauses it.
//!
//! Surface:
//! - [`entry`], the pane as the Window program registers it: the one line that program
//!   needs. It holds [`pane`], which builds the pane for the Frame, and [`controls`];
//! - [`ReviewPane`], the view, and the model of what it shows: its [`PaneRow`]s, its
//!   [`Layout`], the [`Position`] of the line the reviewer is on, the reviews of the
//!   workspace on screen, and the [`CommentBox`] while one is open;
//! - [`controls`], the actions of the pane with their keys: [`LineDown`], [`LineUp`],
//!   [`ToggleLayout`], and the review actions [`Comment`], [`EditDraft`], [`Reply`],
//!   [`DeleteDraft`], [`NextThread`], [`PreviousThread`], [`CopyReview`],
//!   [`ToggleViewed`], [`MarkReviewed`] and [`Commit`]; and those of Changes and of whose
//!   review is shown: [`FocusHistory`], [`HistoryDown`], [`HistoryUp`], [`MarkChange`],
//!   [`ShowChange`], [`FocusRows`], [`ToggleKeepSelection`], [`TakeOffer`] and
//!   [`NextReview`], which the pane's package of slice 4 gives their keys;
//! - [`ChangeShown`], the change of the worktree the pane shows the files of, and what a
//!   turn end offers;
//! - [`View`], [`TimelineEntry`], [`FlowAt`], [`Following`] and [`LiveChange`], what Flow
//!   shows, and the actions of Flow and follow: [`ToggleView`], [`ToggleFlowForm`],
//!   [`ToggleFollow`] and [`ResumeFollow`]; [`TakeOffer`] takes, in Flow, what a turn end
//!   offered to open in Changes.
//!
//! Changes. Above the files the pane shows the tree of the worktree's history, which it
//! asks the Keeper for: its commits and the turns of its agents, oldest first, and last
//! the uncommitted changes the review shown lists, those no agent made marked. The reviewer selects one entry, or
//! a range of them, by clicks or with the keys tuicr gives its commit selector, and the
//! files below are the change from the version before the first to the version of the
//! last. Until the reviewer selects one, the pane shows the changes of the workspace as
//! slice 1 did. When a turn ends in the workspace on screen the pane selects what changed
//! since the reviewer's last review, or, set to keep the selection, keeps it and offers
//! that change instead.
//!
//! Whose review. With one agent session in the worktree the pane names it above the
//! review; with several it names each, and the reviewer chooses whose review it shows.
//!
//! The pane shows the changes of the workspace on screen: it observes
//! `yaar_frame::WorkspaceChanges`, asks through it for the two versions of each changed
//! file, a few files at a time, and diffs them with `yaar-diff` off the thread that
//! draws. Nothing in it changes a file of the worktree, and it offers no way to, but one:
//! the commit control, which asks the Keeper for an ordinary git commit of what is
//! uncommitted.
//!
//! It shows the reviews of the workspace on screen too, which it asks the Keeper for, and
//! their threads on the lines they sit on. A comment is written in a comment box, which
//! saves its text at every change, while the reviewer types, and never waits for the
//! answer: what is typed is the Keeper's to keep from the first keystroke.
//!
//! It depends on the Frame and on no other pane.

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

use std::cell::RefCell;
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
    ThreadId, TurnEnd, TurnId, WorkspaceId, WorkspaceReviews,
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
        /// Moves the reviewer one line down.
        LineDown,
        /// Moves the reviewer one line up.
        LineUp,
        /// Switches the diff between the unified and the split view.
        ToggleLayout,
        /// Opens a comment box on the line the reviewer is on, for the first comment of a
        /// new thread there.
        Comment,
        /// Opens the comment box on the reviewer's draft in the thread on the line the
        /// reviewer is on, to edit it.
        EditDraft,
        /// Opens a comment box for a reply in the thread on the line the reviewer is on,
        /// whose last comment was sent.
        Reply,
        /// Deletes the reviewer's draft in the thread on the line the reviewer is on.
        DeleteDraft,
        /// Puts the reviewer on the line of the next thread, in the order of the rows.
        NextThread,
        /// Puts the reviewer on the line of the previous thread.
        PreviousThread,
        /// Copies the review as text to the clipboard, which sends its comments.
        CopyReview,
        /// Marks the file the reviewer is in viewed, or takes the mark away.
        ToggleViewed,
        /// Marks the change reviewed.
        MarkReviewed,
        /// Asks for a commit message, and commits every uncommitted change with it.
        Commit,
        /// Moves the focus from the rows of the diff to the tree of Changes.
        FocusHistory,
        /// Moves the focus from the tree of Changes back to the rows of the diff.
        FocusRows,
        /// Moves the cursor of the tree of Changes one entry down.
        HistoryDown,
        /// Moves the cursor of the tree of Changes one entry up.
        HistoryUp,
        /// Marks the entry under the cursor of the tree for the range to show, or takes the
        /// mark away.
        MarkChange,
        /// Shows the change of the marked entries of the tree, from the first to the last,
        /// or of the entry under the cursor when none is marked.
        ShowChange,
        /// Sets whether a turn that ends keeps the selection of Changes and offers what
        /// changed since the last review, or selects it.
        ToggleKeepSelection,
        /// Shows the change a turn end offered.
        TakeOffer,
        /// Shows the review of the next agent session of the worktree.
        NextReview,
        /// Shows Flow when Changes is on screen, and Changes when Flow is.
        ToggleView,
        /// Switches Flow between its two forms, the timeline and the live diff, and keeps
        /// the form chosen for the workspace on screen. Only while Flow is on screen: in
        /// Changes it does nothing and sends nothing.
        ToggleFlowForm,
        /// Turns follow off when it is on, paused or not, and on when it is off.
        ToggleFollow,
        /// Resumes follow where it was paused: Flow moves to the newest step. Only while
        /// Flow is on screen: in Changes it does nothing.
        ResumeFollow,
        /// Attaches the review shown to the user's next message to its agent: every unsent
        /// comment on an unresolved thread, those written later included.
        AttachReview,
        /// Attaches the thread on the line the reviewer is on alone to the user's next
        /// message to the agent: a new draft elsewhere does not join it.
        AttachThread,
        /// Takes away what is attached of the review shown.
        DetachReview,
        /// Resolves the thread on the line the reviewer is on.
        ResolveThread,
        /// Reopens the resolved thread on the line the reviewer is on.
        ReopenThread,
        /// Resolves every thread of the review shown that the agent marked addressed.
        ResolveAddressed,
        /// Shows the line a moved thread on the line the reviewer is on was written on,
        /// or hides it again.
        ShowOriginal,
        /// Shows in Changes what changed since the comments of a round were sent: the
        /// first press the newest round with a recorded state, each press after it the
        /// round before, and after the first round the newest again.
        SinceRound,
        /// Takes the offer to start an agent with the review attached: starts the chosen
        /// CLI offered, the first until Next Offered Agent chooses another.
        StartAgent,
        /// Makes the next CLI of the offer to start an agent the chosen one, which Start
        /// Agent starts; after the last, the first.
        NextOfferedAgent,
        /// Puts the reviewer on the next name of the line.
        NextName,
        /// Puts the reviewer on the name before, on the line.
        PreviousName,
        /// Shows what the language server reports for the name the reviewer is on.
        Hover,
        /// Opens the definition of the name the reviewer is on in the file view, or lists
        /// its definitions where there are several.
        GoToDefinition,
        /// Lists the references to the name the reviewer is on.
        FindReferences,
        /// Closes the file view and brings the reviewer back to the place in the diff they
        /// left.
        GoBack,
        /// Chooses the next entry of the list of references.
        ReferenceDown,
        /// Chooses the entry before, in the list of references.
        ReferenceUp,
        /// Opens the chosen entry of the list of references in the file view.
        OpenReference,
        /// Closes the list of references.
        CloseReferences,
        /// Accepts the offer to download the missing language server of the code on screen.
        DownloadServer,
        /// Asks for the update of the language server of the code on screen.
        UpdateServer,
        /// Answers yes to the question before a language server inside a repository runs.
        AllowServers,
        /// Answers no to it.
        RefuseServers,
    ]
);

/// The key context of the pane: its keys work while it has the focus.
pub const KEY_CONTEXT: &str = "ReviewPane";

/// The key context of the tree of Changes, inside the pane: its keys work while the tree
/// has the focus.
pub const HISTORY_CONTEXT: &str = "ReviewHistory";

/// The key context of the list of references, inside the pane: its keys work while the
/// list has the focus.
pub const REFERENCES_CONTEXT: &str = "ReviewReferences";

/// A change of the worktree the pane shows the files of, or that a turn end offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeShown {
    /// The changes of the workspace as `yaar_frame::WorkspaceChanges` holds them: against
    /// its base, as slice 1 showed them. What the pane shows until the reviewer selects in
    /// the tree, and after a workspace comes on screen.
    Comparison,
    /// The entries `first` to `last`, both included, of the history's entries: from the
    /// version before `first`, the entry above it or the history's start, to the version
    /// of `last`. One entry is `first == last`.
    Entries {
        /// The first entry, by its index in `History::entries`.
        first: usize,
        /// The last entry.
        last: usize,
    },
    /// The uncommitted changes the review shown lists, as the Keeper answers
    /// `GetUncommitted` for that review: from the commit checked out to the files on
    /// disk, with those no agent made marked.
    Uncommitted,
    /// From one version to another, as what changed since the last review is: from the
    /// state the reviewer last marked reviewed, or the history's start before the first
    /// mark, to the state recorded at a turn's end.
    Since {
        /// The version it is counted from, by its full id; none where nothing is
        /// committed.
        from: Option<String>,
        /// The state it leads to.
        to: String,
    },
}

/// Which of its two views the pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// The change of the worktree the reviewer selects in the tree: what the pane shows
    /// when it opens.
    #[default]
    Changes,
    /// What the agent does as it works, in the form chosen last in the workspace on
    /// screen.
    Flow,
}

/// A step of an agent in a turn, as Flow names the one it is at: the turn, and the
/// number of the step in it, from 1.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlowAt {
    /// The turn.
    pub turn: TurnId,
    /// The number of the step in the turn.
    pub number: u32,
}

/// One entry of Flow's timeline, in the order drawn: the turns of the workspace on screen
/// in the order they started, each as its heading, then its steps in the order of their
/// numbers, then its end once it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineEntry {
    /// The heading of a turn.
    Turn {
        /// The turn.
        turn: TurnId,
        /// Its session.
        session: SessionId,
        /// The session as the Window names it, `Claude Code 1`.
        title: String,
    },
    /// A step of a turn: a read, an edit, a file written whole or a command.
    Step {
        /// The turn.
        turn: TurnId,
        /// Its number in the turn, from 1.
        number: u32,
        /// What the agent did.
        step: Step,
    },
    /// The end of a turn.
    End {
        /// The turn.
        turn: TurnId,
        /// How it ended, when the pane knows it: a turn read with `GetTurn` says, a
        /// `TurnEnded` fact does not.
        how: Option<TurnEnd>,
    },
}

/// Follow, as the reviewer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Following {
    /// Flow moves to each step of the agent as it comes: what the pane starts with.
    #[default]
    On,
    /// Follow is on, and paused by a scroll, a click or a key in the pane while a turn
    /// ran; `missed` steps came since. An offer to resume is drawn.
    Paused {
        /// The steps that came since follow paused.
        missed: u32,
    },
    /// The reviewer turned follow off.
    Off,
}

/// The change the live diff shows: of one turn of the workspace on screen, from the state
/// it counts from to the files on disk while it runs, and to the state recorded at its end
/// once it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveChange {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The turn.
    pub turn: TurnId,
    /// The version the change counts from: the turn's `start`, or, where the turn has
    /// none, the commit checked out, as the history gives it; none where nothing is
    /// committed.
    pub from: Option<String>,
    /// The state recorded at the turn's end; none while it runs, and when the recording
    /// failed: the files on disk.
    pub to: Option<String>,
}

/// The layout a diff is shown in until the reviewer switches it.
pub const FIRST_LAYOUT: Layout = Layout::Unified;

/// The name of the pane in the Frame.
pub const NAME: &str = "review";

/// How many files of the changes the pane has asked the Keeper for and not yet got, at
/// the most. The other files wait their turn, in the order of the list.
///
/// A question is written to the Keeper's socket by the thread that draws, and a socket
/// takes only so much before a write has to wait for the Keeper to read: 8,192 bytes by
/// default on macOS, which is less than the questions for the files of a large change.
/// Asked for all at once, those files would hold the window still while the Keeper
/// worked through them. A few at a time keep the Keeper as busy, and let the files
/// after a large one arrive while that one is still being read and diffed.
pub const FILES_ON_THEIR_WAY: usize = 4;

/// The pane as the Window program registers it: its name, its keys and its builder.
pub fn entry() -> PaneEntry {
    PaneEntry {
        name: NAME,
        controls,
        build: pane,
    }
}

/// Builds the Review pane for a window, under the name [`NAME`].
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

/// The actions of the pane with their keys, those of the rows in [`KEY_CONTEXT`] and those
/// of the tree of Changes in [`HISTORY_CONTEXT`]: `j` and `down` for [`LineDown`], `k`
/// and `up` for [`LineUp`], `s` for [`ToggleLayout`]. While the tree has the focus the
/// keys of the rows do not work, and its own keys are the same letters moving the cursor
/// of the tree instead: `j` and `down` for [`HistoryDown`], `k` and `up` for
/// [`HistoryUp`], `space` for [`MarkChange`], `enter` for [`ShowChange`], and `tab`, which
/// takes the focus into the tree from the rows ([`FocusHistory`]) and back ([`FocusRows`]).
///
/// The review actions follow vim and tuicr wherever either has a key for the action, and
/// take a key neither uses for another action of this pane where neither has one:
///
/// | Action | Key | Where the key comes from | Sends |
/// | --- | --- | --- | --- |
/// | [`Comment`] | `c` | tuicr, add a line comment | nothing; the box saves as it is typed |
/// | [`EditDraft`] | `i` | tuicr, edit the comment at the cursor; vim, insert | nothing; the box saves |
/// | [`Reply`] | `o` | neither; vim's `o` opens a line below | nothing; the box saves |
/// | [`DeleteDraft`] | `d d` | tuicr and vim, `dd` | `DeleteDraft` |
/// | [`NextThread`], [`PreviousThread`] | `m`, `shift-m` | tuicr, next and previous comment | nothing |
/// | [`CopyReview`] | `y` | tuicr, copy the review to the clipboard; vim, yank | `CopyReview` |
/// | [`ToggleViewed`] | `r` | tuicr, toggle a file reviewed | `SetViewed` |
/// | [`MarkReviewed`] | `shift-r` | neither; tuicr's `R` marks a hunk reviewed | `MarkReviewed` |
/// | [`Commit`] | `shift-c` | neither; git's own tools use `C` for a commit | `Commit`, after a line of text |
/// | [`FocusHistory`], [`FocusRows`] | `tab` | tuicr, Tab cycles the focus | nothing |
/// | [`HistoryDown`], [`HistoryUp`] | `j` and `down`, `k` and `up` | tuicr, the commit selector | nothing |
/// | [`MarkChange`] | `space` | tuicr, the commit selector marks a commit | nothing |
/// | [`ShowChange`] | `enter` | tuicr, the commit selector confirms | nothing; the files come by `CompareVersions` and `ReadVersions` |
/// | [`ToggleKeepSelection`] | `shift-t` | neither | nothing |
/// | [`TakeOffer`] | `shift-o` | neither | nothing; the files come by `CompareVersions` and `ReadVersions` |
/// | [`NextReview`] | `shift-a` | neither | nothing; the uncommitted changes of the review come by `GetUncommitted` |
/// | [`ToggleView`] | `f` | neither | nothing; Flow asks `GetTurn` for what it has not heard |
/// | [`ToggleFollow`] | `shift-f` | neither | nothing |
/// | [`ToggleFlowForm`] | `shift-l` | neither | `SetSetting`, the form of the workspace on screen, in Flow only |
/// | [`ResumeFollow`] | `shift-g` | vim, `G` goes to the last line; tuicr, `G` to the last file | nothing |
///
/// In Flow, `j`, `k` and the arrows move through the timeline's entries, or through the
/// lines of the live diff; `c` writes a comment on the line of the live diff the reviewer
/// is on; `shift-o` opens in Changes what a turn end offered. `f` and `shift-f` work in
/// both views. `shift-l` and `shift-g` work only while Flow is on screen: in Changes they,
/// and their actions, do nothing at all, so that no form is switched and no follow resumed
/// out of the reviewer's sight, and nothing is sent.
///
/// In the comment box the keys type text; `enter` closes the box and `shift-enter` starts
/// a new line, as in tuicr. `escape` closes it too and keeps the draft, where tuicr's
/// throws the edit away: the draft was saved as it was typed, and `d d` deletes it.
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
        control(
            "Keep Selection",
            || Box::new(ToggleKeepSelection),
            "shift-t",
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
        // A key vim gives a line down too, as `j` and Down do: every plain and shifted letter
        // is vim's or the pane's, Ctrl-K starts the app's chords on Linux, and an Option
        // letter is a dead key on macOS, so no free key is nearer.
        control(
            "Next Offered Agent",
            || Box::new(NextOfferedAgent),
            "ctrl-n",
        ),
        // Code intelligence: vim's `h` and `l` move by a character, here by a name; `K`
        // shows what the word under the cursor is (vim's keyword lookup); `gd` goes to a
        // definition; Neovim gives the references `grr`, shortened here as vim's `gr`
        // replaces text, which a read-only view never does. Back is Escape: vim's Ctrl-O
        // is Open Folder on Linux.
        control("Next Name", || Box::new(NextName), "l"),
        control("Previous Name", || Box::new(PreviousName), "h"),
        control("Hover", || Box::new(Hover), "shift-k"),
        control("Go to Definition", || Box::new(GoToDefinition), "g d"),
        control("Find References", || Box::new(FindReferences), "g r"),
        control("Back to the Diff", || Box::new(GoBack), "escape"),
        in_references("Next Reference", || Box::new(ReferenceDown), "j"),
        in_references("Next Reference", || Box::new(ReferenceDown), "down"),
        in_references("Previous Reference", || Box::new(ReferenceUp), "k"),
        in_references("Previous Reference", || Box::new(ReferenceUp), "up"),
        in_references("Open Reference", || Box::new(OpenReference), "enter"),
        in_references("Close References", || Box::new(CloseReferences), "escape"),
        // The line of the language server: neither vim nor tuicr has these actions.
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

/// The comment box, while one is open: what is typed in it and where it goes.
///
/// It is drawn in the pane under the selector `review-comment-box`, below the rows and
/// outside the element that holds the pane's key context, so that the letters typed in it
/// are text and not keys of the pane. It has the focus while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentBox {
    /// The review the draft belongs to.
    pub review: ReviewId,
    /// The thread: a new one when `anchor` is some.
    pub thread: ThreadId,
    /// The draft, by the id the pane drew for it, or the id it has when it is edited.
    pub comment: CommentId,
    /// Where the new thread sits; none for a draft in a thread that exists, an edit or a
    /// reply. Its lines are counted from 1, as the Link counts them.
    pub anchor: Option<Anchor>,
    /// The line the box was opened on.
    pub at: Position,
    /// The text in the box now.
    pub text: String,
}

/// One line of one version of one changed file: where the reviewer is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Position {
    /// The path of the file, as the changes list it.
    pub path: PathBuf,
    /// The version the line belongs to. A line that did not change is on both sides and
    /// is named by its new one.
    pub side: Side,
    /// The number of the line in that version, counted from 0.
    pub line: u32,
}

/// Why the lines of a changed file are not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unshown {
    /// The Keeper has not sent the file yet, or the diff is still being computed.
    Loading,
    /// The file is not text.
    Binary,
    /// The file is too large to send.
    TooLarge,
    /// The Keeper refused to send it.
    Refused,
}

/// One row of the pane, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneRow {
    /// The line that says what the changes are compared with: the first row, once the
    /// changes are here. When there is nothing to compare with, it says why and offers
    /// to set a base.
    Comparison {
        /// What the changes are compared with, as the Keeper told it: the row says
        /// this.
        comparison: Comparison,
        /// Whether the row offers to set a base: with
        /// `yaar_link::Comparison::NothingToCompare`, and with no other comparison.
        offers_base: bool,
    },
    /// A commit of the comparison. The commits follow the comparison line, newest first,
    /// before the first file.
    Commit {
        /// Its full id.
        id: String,
        /// The first line of its message.
        summary: String,
    },
    /// The heading of a changed file. The rows of its diff follow.
    File {
        /// Its path.
        path: PathBuf,
        /// What happened to it.
        status: FileStatus,
    },
    /// A row of the diff of the file above, in the layout of the pane.
    Diff {
        /// The path of the file.
        path: PathBuf,
        /// The row.
        row: Row,
    },
    /// Stands for the diff of the file above while it cannot be shown.
    Unshown {
        /// The path of the file.
        path: PathBuf,
        /// Why.
        why: Unshown,
    },
    /// A thread of the review, after the row of its diff that shows its last line, or
    /// among the threads elsewhere. The row of each of its comments follows. It is drawn
    /// under the selector `review-thread-<thread id>`. Not a stop of [`LineDown`] and
    /// [`LineUp`].
    Thread {
        /// The thread.
        thread: ThreadId,
    },
    /// One comment of the thread above, in its order. Its text is drawn under the
    /// selector `review-comment-<comment id>:draft` while it is a draft, and
    /// `review-comment-<comment id>:round-<n>` once it was sent in round `n`.
    Comment {
        /// The thread it belongs to.
        thread: ThreadId,
        /// The comment.
        comment: CommentId,
    },
    /// The heading of the threads whose line no row shows, after the last file: a thread
    /// is never hidden. Drawn under the selector `review-threads-elsewhere`. The threads
    /// of a workspace whose worktree is gone are all there.
    ThreadsElsewhere,
    /// A file the turn of Flow's live diff read and did not change: one row, its path
    /// marked as read, with no diff below it and no line the reviewer can be on. Only the
    /// live diff holds such rows, after every other row of it, one a file, in the order
    /// of the file's first read in the turn. Changes holds none.
    Read {
        /// The path, as the read step names it: relative to the worktree's folder when
        /// the file lies inside it, absolute otherwise.
        path: PathBuf,
    },
}

/// A changed file as the pane holds it.
struct ShownFile {
    file: ChangedFile,
    content: FileContent,
    /// Where its heading and the rows under it lie in the rows of the pane.
    rows: Range<usize>,
    /// The threads of the review that the rows of this file show.
    placed: Vec<ThreadId>,
}

/// What the pane holds of a changed file.
enum FileContent {
    /// Nothing it could show lines of.
    Unshown(Unshown),
    /// The difference between its two versions.
    Diff(Box<ShownDiff>),
}

/// A question for the two versions of one file that is on its way to the Keeper or back.
struct Asked {
    /// The place of the file in the files of the pane.
    index: usize,
    /// The answer, when it comes.
    sides: Task<Result<FileSides, AskError>>,
}

/// The diff of one file, with what was worked out of it for drawing.
struct ShownDiff {
    diff: FileDiff,
    /// The changed words of each hunk that was drawn so far, by the index of the hunk.
    /// A hunk is asked for its words when it first comes into view, since the words of
    /// a large hunk take longer to find than its lines.
    emphasis: RefCell<HashMap<usize, Rc<Emphasis>>>,
}

/// The Review pane: a view, and the model of what it shows.
///
/// The reviewer is on one line while a diff is shown: at first the first changed line of
/// the first file. [`LineDown`] and [`LineUp`] move to the next and the previous line of
/// the diff, across files, and a click on a line moves there. The pane scrolls so that
/// the line stays in view; the wheel scrolls without moving the reviewer.
///
/// The files arrive one after the other, and not always in the order of the list. While
/// a file above the first diff that is shown is still on its way, that file may yet hold
/// the first changed line: until it is here the reviewer is on no line, and the two
/// actions do nothing, unless a click has put the reviewer on a line.
///
/// The lines of a diff have one order, whatever the layout: the order of the unified
/// view, where the removed lines of a change come before the lines added in their place.
/// In the split view [`LineDown`] therefore goes down the old side of a change and then
/// down its new side.
///
/// [`ToggleLayout`] changes how the rows are laid out and nothing else: the reviewer
/// stays on the same line of the same version of the same file, and that line stays in
/// view.
///
/// For tests, which cannot read what is drawn, each row that is laid out carries a
/// selector that `debug_bounds` finds: `review-row-<index>`, with its index in
/// [`ReviewPane::rows`]. In the split view the two halves of a row of a diff carry
/// `review-row-<index>-old` and `review-row-<index>-new` as well. The text of a line is
/// drawn by an element inside its row that carries `review-text-<index>-old:<text>` or
/// `review-text-<index>-new:<text>`, where the text is what the row hands to the text
/// system for that line: the line, or of a line of more than 1,024 bytes its start.
/// The tree of Changes above the rows carries `review-history-<index>` on the entry at
/// `index` of the history and `review-history-uncommitted` on the last, and the heading of a
/// file that no agent made, in the uncommitted changes, carries `review-no-agent-<index of its
/// row>`; the heading of a file that a session other than the review's own is credited with
/// too carries `review-shared-<index of its row>`.
/// The line above the tree that names the agent sessions whose reviews there are carries
/// `review-session` for the one session there is, or `review-session-<index>` on each of
/// several, which a click shows the review of; the line that says the pane keeps the
/// selection at a turn end carries `review-keeps-selection`, what a turn end offered
/// `review-offer`, and its control `review-offer-take`.
/// The control that offers to set a base carries `review-offer-base`. The heading of a
/// file marked viewed carries `review-viewed-<index>`, with the index of its row, and the
/// comparison line carries `review-reviewed` while the review has a reviewed state.
///
/// When the Keeper refuses to read the changes again, the pane keeps the rows it has
/// and says above them, in a line that carries `review-refused`, that they could not be
/// read again and why. The line goes with the next answer.
pub struct ReviewPane {
    models: Models,
    focus: FocusHandle,
    layout: Layout,
    rows: Vec<PaneRow>,
    position: Option<Position>,
    /// The index in `rows` of the row that shows `position`.
    position_row: Option<usize>,
    /// The changes the rows are made of: the last the Keeper answered with.
    changes: Option<Changes>,
    /// The files of `changes`, in their order.
    files: Vec<ShownFile>,
    /// Where each file is in `files`.
    by_path: HashMap<PathBuf, usize>,
    /// The reading of the files of `changes`: at most [`FILES_ON_THEIR_WAY`] tasks,
    /// each with one question on its way, which it follows with the question for the
    /// next file in `unread` when the answer is here. New changes take their place,
    /// which drops the answers to the old questions.
    reading: Vec<Task<()>>,
    /// The files of `changes` that were not asked for yet, by their place in `files`.
    unread: Range<usize>,
    scroll: UniformListScrollHandle,
    /// The rows the list last laid out, as the list itself told it.
    in_view: Rc<RefCell<Range<usize>>>,
    /// The workspace the reviews belong to: the one on screen.
    reviews_of: Option<WorkspaceId>,
    /// What the pane shows of them: the last answer of the Keeper with every save the
    /// pane sent since that answer was asked for put into it again.
    reviews: Option<WorkspaceReviews>,
    /// The drafts the pane saved since the `GetReviews` whose answer it shows was
    /// asked, one for each draft, in the order they were first saved.
    saves: Vec<Save>,
    /// How many saves the pane sent, ever.
    saved: u64,
    /// The last question for the reviews. A new one takes its place, which drops the
    /// answer to the old one: of two answers, only the one asked last is shown.
    reading_reviews: Option<Task<()>>,
    /// The comment box, while one is open.
    open_box: Option<OpenBox>,
    /// The field for a commit message, while one is open.
    commit_field: Option<CommitField>,
    /// What is said under the rows, until the reviewer acts again.
    notice: Option<String>,
    /// The tree of Changes above the files, and which change the files are of.
    tree: Tree,
    /// Where the versions of the files in `files` come from; none while there are no
    /// files.
    listed: Option<Source>,
    /// Whether a turn end keeps the selection and offers what changed instead of
    /// selecting it. It outlasts the workspaces: it is the reviewer's setting.
    keep: bool,
    /// The review the reviewer, or a turn end, chose to show; none until one was chosen
    /// in the workspace on screen.
    chosen: Option<ReviewId>,
    /// The sessions of the workspace on screen that a review belongs to, as the agents
    /// model held them when it last changed.
    sessions: Vec<Session>,
    /// Which view is on screen. Changed by [`ToggleView`], by a message of the user to the
    /// agent while follow is on, and by taking a turn end's offer in Flow.
    view: View,
    /// The form Flow shows: the one chosen last in the workspace on screen, as
    /// `yaar_frame::SettingList` holds it, until the reviewer switches it.
    form: FlowForm,
    /// The step Flow is at, where follow last moved it; none before the first step.
    at: Option<FlowAt>,
    /// Follow, as the reviewer sees it. Flow moves with the steps only while it is
    /// [`Following::On`].
    following: Following,
    /// The timeline of Flow, and what Flow keeps of the turns of the workspace on screen.
    flow: flow::Timeline,
    /// The live diff of Flow.
    live: live::Live,
    /// What follow keeps beside [`Following`]: the turns it has seen start, and what a
    /// turn end offered in Flow.
    follow: follow::Follow,
    /// What attaching keeps: the offer to start an agent the last attach answered with.
    attaching: attaching::Attaching,
    /// What the threads that moved keep: whose original line is shown.
    moved: moved::Moved,
    /// What code intelligence keeps: the name the reviewer is on, the asks on their way, the
    /// hover, the list of references, the file view and the place left in the diff.
    navigate: navigate::Navigate,
}

impl ReviewPane {
    /// A pane that shows the changes of the workspace on screen of `models`, in
    /// [`FIRST_LAYOUT`].
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
            pane.sessions_changed(cx);
            pane.flow_agents(cx);
            pane.live_agents(cx);
            pane.follow_agents(cx);
        })
        .detach();
        // The answers on the servers inside a repository are settings too: the line of the
        // language server is drawn again for each, whenever the Navigator told its states.
        cx.observe(&models.settings, |pane, _, cx| {
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
            scroll: UniformListScrollHandle::new(),
            in_view: Rc::new(RefCell::new(0..0)),
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
            keep: false,
            chosen: None,
            sessions: Vec::new(),
            view: View::Changes,
            form: FlowForm::Timeline,
            at: None,
            following: Following::On,
            flow: flow::Timeline::new(cx),
            live: live::Live::new(cx),
            follow: follow::Follow::new(cx),
            attaching: attaching::Attaching::default(),
            moved: moved::Moved::default(),
            navigate: navigate::Navigate::default(),
        };
        pane.sessions_changed(cx);
        pane.follow_history(window, cx);
        pane.follow(cx);
        pane.follow_reviews(window, cx);
        // A pane built while a workspace, its agents and its settings are already in the
        // models starts from them, as it would from a change of each.
        pane.flow_workspace(cx);
        pane.live_workspace(cx);
        pane.follow_workspace(cx);
        pane.flow_agents(cx);
        pane.live_agents(cx);
        pane.follow_agents(cx);
        pane.flow_settings(cx);
        pane
    }

    /// The layout the diffs are shown in.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The rows of the pane, top to bottom, in the layout it has now: the comparison
    /// line, its commits, then each changed file with the rows of its diff. These are
    /// the rows the pane draws from, not a second account of them.
    pub fn rows(&self) -> &[PaneRow] {
        &self.rows
    }

    /// The text of one line of one version of a changed file, without its line end;
    /// none while the pane does not hold that file. Bytes that are not UTF-8 are shown
    /// as the replacement character.
    pub fn text(&self, position: &Position) -> Option<String> {
        let shown = self.diff_of(&position.path)?;
        (position.line < shown.diff.line_count(position.side))
            .then(|| shown.text(position.side, position.line).into_owned())
    }

    /// The line the reviewer is on. None while no diff is shown, and while a file above
    /// the first diff that is shown is still on its way and no click has chosen a line.
    pub fn position(&self) -> Option<&Position> {
        self.position.as_ref()
    }

    /// The index in [`ReviewPane::rows`] of the row that shows the line the reviewer is
    /// on.
    pub fn position_row(&self) -> Option<usize> {
        self.position_row
    }

    /// The indices in [`ReviewPane::rows`] of the rows in view, as last laid out.
    pub fn rows_in_view(&self) -> Range<usize> {
        self.in_view.borrow().clone()
    }

    /// The reviews of the workspace on screen, with its files marked viewed, as the Keeper
    /// last answered; none before its first answer and while no workspace is on screen.
    ///
    /// What the reviewer saved since that answer was asked for is in them: a save of a
    /// draft is told to nobody, so the pane puts each text it saved into its copy, the
    /// draft of a new thread included, and an answer of the Keeper is shown with the
    /// saves sent after it was asked for put over it again. They hold every text typed,
    /// also after the comment box was closed.
    pub fn reviews(&self) -> Option<&WorkspaceReviews> {
        self.reviews.as_ref()
    }

    /// Every thread of the review shown, in the order of its threads, with its state as
    /// the Keeper told it: draft, sent, addressed, waiting for the reviewer, resolved.
    /// Each thread's row draws its state under the selector
    /// `review-thread-state-<thread id>:<state>`, the state written `draft`, `sent`,
    /// `addressed`, `waiting` or `resolved`.
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

    /// The review the pane shows and acts on: the one the reviewer chose, or a turn end
    /// chose for them, among [`ReviewPane::reviews`]; until one is chosen, the review of
    /// no agent while there is one, else that of the first session that runs, else the
    /// first review.
    pub fn review(&self) -> Option<&Review> {
        whose::shown(self.reviews.as_ref(), self.chosen.as_ref(), &self.sessions)
    }

    /// The history of the workspace on screen, as the Keeper last answered; none before
    /// its first answer and while no workspace is on screen.
    pub fn history(&self) -> Option<&History> {
        self.tree.history.as_ref()
    }

    /// The change whose files the pane shows.
    pub fn change_shown(&self) -> ChangeShown {
        self.tree.shown.clone()
    }

    /// The entry of the tree under the cursor, by its index in the history's entries, with
    /// one more than the last for the uncommitted changes; none before the tree has the
    /// focus once.
    pub fn history_cursor(&self) -> Option<usize> {
        self.tree.cursor
    }

    /// The entries of the tree marked for a range, by their indices, in order.
    pub fn marked_changes(&self) -> Vec<usize> {
        self.tree.marked.clone()
    }

    /// Whether a turn that ends keeps the selection of Changes and offers what changed
    /// since the last review, rather than selecting it. Off until the reviewer sets it; it
    /// lasts as long as the window, until the settings keep it.
    pub fn keeps_selection(&self) -> bool {
        self.keep
    }

    /// What a turn end offered while the pane kept the selection, drawn under the selector
    /// `review-offer`, with its control under `review-offer-take`; none when nothing is
    /// offered. Taking it, or selecting anything, takes it away.
    pub fn offer(&self) -> Option<&ChangeShown> {
        self.tree.offer.as_ref().map(|offer| &offer.change)
    }

    /// The agent sessions of the worktree on screen that a review belongs to, in the
    /// order they started, as `yaar_frame::AgentList` held them when the pane last drew:
    /// a copy, read without the app's context. With one, the pane
    /// names it above the review, under the selector `review-session`; with several, it
    /// names each under `review-session-<index>`, which a click shows the review of.
    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.clone()
    }

    /// The session whose review the pane shows; none for the review of no agent.
    pub fn review_session(&self) -> Option<&SessionId> {
        self.review()?.agent.as_ref()
    }

    /// Whether the change of `path` that the pane lists under uncommitted changes was made
    /// by no agent, as the credits of the uncommitted list say; such a file's heading
    /// carries a mark under the selector `review-no-agent-<index of its row>`.
    pub fn made_by_no_agent(&self, path: &Path) -> bool {
        self.tree.uncommitted.as_ref().is_some_and(|listed| {
            listed
                .iter()
                .any(|credited| credited.file.path == path && credited.by.is_empty())
        })
    }

    /// The uncommitted changes the review shown lists, as the Keeper last answered
    /// `GetUncommitted` for it, ordered by path, each with who made it; none before its
    /// first answer. The pane asks again when the review shown changes, at
    /// `ChangesChanged` and at `TurnEnded` of the workspace on screen.
    pub fn uncommitted(&self) -> Option<&[CreditedFile]> {
        self.tree.uncommitted.as_deref()
    }

    /// Which view is on screen: Changes until the reviewer, or a message to the agent
    /// while follow is on, shows Flow. The pane draws the view's name under
    /// `review-view-flow` and `review-view-changes` in the bar at its top, `review-bar`,
    /// which a click on either switches.
    pub fn view(&self) -> View {
        self.view
    }

    /// The form Flow shows, now or when it next comes on screen: the one chosen last in
    /// the workspace on screen, the timeline where none was chosen.
    pub fn flow_form(&self) -> FlowForm {
        self.form
    }

    /// The timeline of Flow: the turns of the workspace on screen with their steps, as
    /// [`TimelineEntry`] says. Entry `i` is drawn under `flow-entry-<i>` while the
    /// timeline is on screen.
    pub fn timeline(&self) -> &[TimelineEntry] {
        self.flow.entries()
    }

    /// The entry of the timeline the reviewer is on, by its index in
    /// [`ReviewPane::timeline`]: where `j`, `k` or a click put the reviewer, or where
    /// follow moved Flow; none before either.
    pub fn timeline_cursor(&self) -> Option<usize> {
        self.flow.cursor()
    }

    /// The step Flow is at: the newest step while follow is on, where follow paused, or
    /// where the reviewer resumed it; none before the first step.
    pub fn flow_at(&self) -> Option<&FlowAt> {
        self.at.as_ref()
    }

    /// The index of the row of [`ReviewPane::flow_at`] in the rows of the form on screen:
    /// its entry in [`ReviewPane::timeline`], or in [`ReviewPane::live_rows`] the heading of
    /// the file the step names, or, for a read of a file the turn did not change, that
    /// file's [`PaneRow::Read`]; none when that form has no row for it, as the live diff
    /// for a command, or for an edit or a write whose file its list does not hold yet,
    /// which it names in the line above its files, [`ReviewPane::activity`].
    pub fn flow_at_row(&self) -> Option<usize> {
        let at = self.at.as_ref()?;
        match self.form {
            FlowForm::Timeline => self.flow.row_of(at),
            FlowForm::LiveDiff => self.live_row_of(at),
        }
    }

    /// The indices of the rows of the form of Flow last laid out that are in view: of
    /// [`ReviewPane::timeline`] or of [`ReviewPane::live_rows`].
    pub fn flow_rows_in_view(&self) -> Range<usize> {
        match self.form {
            FlowForm::Timeline => self.flow.in_view(),
            FlowForm::LiveDiff => self.live.in_view(),
        }
    }

    /// Follow, as the reviewer sees it. Drawn as the control `flow-follow` in the bar, and,
    /// while paused, as the line `flow-follow-paused`, which says how many steps came
    /// since, with the control `flow-follow-resume`.
    pub fn following(&self) -> Following {
        self.following
    }

    /// What a turn end offered while Flow was on screen: the change since the reviewer's
    /// last review, to open in Changes. Drawn under `flow-offer`, with the control
    /// `flow-offer-take`; [`TakeOffer`] takes it too. None when nothing is offered.
    pub fn flow_offer(&self) -> Option<&ChangeShown> {
        self.follow.offer()
    }

    /// The change the live diff shows; none before a turn of the workspace on screen is
    /// known.
    pub fn live_change(&self) -> Option<&LiveChange> {
        self.live.change()
    }

    /// The rows of the live diff, top to bottom, as [`ReviewPane::rows`] are for Changes:
    /// each changed file's heading, then the rows of its diff and the threads on them;
    /// last, each file the turn read and did not change, as a [`PaneRow::Read`], in the
    /// order of its first read. Row `i` is drawn under `flow-row-<i>`, the text of a line
    /// in it under `flow-text-<i>-old:<text>` or `flow-text-<i>-new:<text>`, and the mark
    /// of a file read under `flow-read-<i>`.
    pub fn live_rows(&self) -> &[PaneRow] {
        self.live.rows()
    }

    /// The line of the live diff the reviewer is on, where `j`, `k` or a click put the
    /// reviewer; the line a comment written in Flow sits on.
    pub fn live_position(&self) -> Option<&Position> {
        self.live.position()
    }

    /// The text of one line of one version of a file of the live diff, as
    /// [`ReviewPane::text`] gives it for Changes.
    pub fn live_text(&self, position: &Position) -> Option<String> {
        self.live.text(position)
    }

    /// The step the live diff names in the line above its files, `flow-activity`, while it
    /// has no row for it: a command, or an edit or a write whose file the live diff's list
    /// does not hold yet. A read always has a row.
    pub fn activity(&self) -> Option<&FlowAt> {
        let shown = self.live.change()?;
        self.at
            .as_ref()
            .filter(|at| at.turn == shown.turn && self.live_row_of(at).is_none())
    }

    /// The comment box, while one is open.
    pub fn comment_box(&self) -> Option<CommentBox> {
        self.open_box.as_ref().map(|open| open.view.clone())
    }

    /// Whether the field for a commit message is open. It is drawn under the selector
    /// `review-commit-field`; `enter` commits with its line, `escape` closes it.
    pub fn commit_field_open(&self) -> bool {
        self.commit_field.is_some()
    }

    /// Why the last review action the reviewer took was not done, as the pane says it in
    /// the line under the selector `review-notice`: a refusal of the Keeper, as for a copy
    /// with nothing to send or a commit git refused. None once the reviewer acts again.
    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }

    /// Takes up what the shared model holds, each time the model says it changed.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let model = self.models.changes.read(cx);
        let Some(changes) = model.changes() else {
            // No workspace is on screen, or another one came and has no answer yet.
            self.changes = None;
            // The files of a recorded turn that is shown in the meantime are not the
            // comparison's to take away.
            if self.tree.shown == ChangeShown::Comparison {
                self.files.clear();
                self.by_path.clear();
                self.reading.clear();
                self.unread = 0..0;
                self.listed = None;
                self.position = None;
                self.rebuild(cx);
            }
            return;
        };
        // The model keeps the last answer while it asks again, and after a refusal:
        // either way nothing new came, and what is shown stays. A refusal is said above
        // the rows when they are drawn, and they are drawn again because the model,
        // which was read to draw them, changed.
        let nothing_new = model.is_loading() || model.refusal().is_some();
        if nothing_new && self.changes.as_ref() == Some(changes) {
            return;
        }
        let changes = changes.clone();
        // The reviewer's selection stays when the comparison is read again; only what
        // the pane picked itself for want of changes gives way to them.
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

    /// Shows an answer of the Keeper for the comparison, and starts to ask for the two
    /// versions of each file of it.
    ///
    /// An answer for the workspace that is shown already is the same changes read
    /// again, as when the window comes back to the front: each file keeps what is shown
    /// of it until its new versions are here, so the diff does not blink.
    pub(crate) fn show(&mut self, changes: Changes, cx: &mut Context<Self>) {
        let same_workspace = self
            .changes
            .as_ref()
            .is_some_and(|shown| shown.workspace == changes.workspace);
        if !same_workspace {
            self.listed = None;
        }
        let files = changes.files.clone();
        self.changes = Some(changes);
        self.show_files(files, Source::Comparison, cx);
    }

    /// Shows these files, read as `source` says, and starts to ask for the two versions of
    /// each. When the files before were read the same way, each keeps what is shown of it
    /// until its new versions are here.
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
        // The files are asked for from the top of the list, and only a few at a time:
        // the rest as the answers come.
        self.unread = 0..self.files.len();
        self.reading.clear();
        self.reading = (0..FILES_ON_THEIR_WAY)
            .map_while(|_| self.read(cx))
            .collect();
    }

    /// Asks for the first file that was not asked for yet. Each time the answer is
    /// here, the task it gives diffs that file, puts it in its place and asks for the
    /// next one, so it has one question on its way at a time. None when every file was
    /// asked for.
    fn read(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let mut asked = self.ask(cx)?;
        Some(cx.spawn(async move |pane, cx| {
            loop {
                let content = match asked.sides.await.map(versions) {
                    Ok(Ok((old, new))) => {
                        // The diff of a large file takes long enough to be seen: it is
                        // made off the thread that draws.
                        let diff = cx
                            .background_spawn(async move { FileDiff::new(&old, &new) })
                            .await;
                        FileContent::Diff(Box::new(ShownDiff {
                            diff,
                            emphasis: RefCell::default(),
                        }))
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
                    // Every file was asked for, or the pane is gone.
                    Ok(None) | Err(_) => break,
                }
            }
        }))
    }

    /// Asks the Keeper for the two versions of the first file that was not asked for
    /// yet, if there is one.
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

    /// Asks the Keeper for the two versions of a file of a change between two versions of
    /// the workspace: the old one in `old`, none where nothing came before, and the new
    /// one in `new`, none for the file on disk.
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
        // A renamed file is read by the path it had in the old version.
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

    /// Puts what came for the file at `index` in the place of what was shown of it.
    fn loaded(&mut self, index: usize, content: FileContent, cx: &mut Context<Self>) {
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
        // The file may show a thread that was among the threads elsewhere until now.
        self.rows
            .truncate(self.files.last().map_or(0, |last| last.rows.end));
        self.rows.extend(threads::elsewhere(&self.files, review));
        // What the reviewer reads stays where it is on screen when a file above it
        // arrives with more rows, or with fewer, than it had.
        if was.end <= self.in_view.borrow().start {
            self.shift_view(now as f32 - was.end as f32);
        }
        self.settle(cx);
    }

    /// Makes the rows again from what the pane holds.
    pub(crate) fn rebuild(&mut self, cx: &mut Context<Self>) {
        let review = whose::shown(self.reviews.as_ref(), self.chosen.as_ref(), &self.sessions);
        let shown = self.new_side_shown();
        let mut rows = Vec::new();
        // Above the files of the comparison goes what it is a comparison with; the files
        // of an entry of the tree are told by the tree.
        if let (Some(changes), ChangeShown::Comparison) = (&self.changes, &self.tree.shown) {
            rows.push(PaneRow::Comparison {
                comparison: changes.comparison.clone(),
                offers_base: matches!(changes.comparison, Comparison::NothingToCompare { .. }),
            });
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
        self.rows = rows;
        self.settle(cx);
        self.tell_old_side(cx);
        // What the rows of Changes are made again for, the reviews or the layout, the rows
        // of the live diff are made again for too.
        self.live_rebuild(cx);
    }

    /// After the rows changed: finds the row of the line the reviewer is on, and puts
    /// the reviewer on the first changed line when there is no such row.
    fn settle(&mut self, cx: &mut Context<Self>) {
        self.position_row = self.position.as_ref().and_then(|at| self.locate(at));
        if self.position_row.is_none() {
            self.position = self.first_change();
            self.position_row = self.position.as_ref().and_then(|at| self.locate(at));
        }
        if self.rows.is_empty() {
            // No list is laid out for no rows, so nothing else would say so.
            *self.in_view.borrow_mut() = 0..0;
        }
        cx.notify();
    }

    /// The diff the pane holds for the file at `path`.
    fn diff_of(&self, path: &Path) -> Option<&ShownDiff> {
        match &self.files[*self.by_path.get(path)?].content {
            FileContent::Diff(shown) => Some(shown),
            FileContent::Unshown(_) => None,
        }
    }

    /// The index of the row that shows the line at `position`.
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

    /// The first changed line of the first file that has one. None while a file before
    /// it is still on its way: the first file may yet be that one.
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

    /// What [`LineDown`] does.
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

    /// What [`LineUp`] does.
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

    /// What [`LineDown`] and [`LineUp`] do in Flow: move the reviewer through the entries
    /// of the timeline, or through the lines of the live diff, by `by`.
    fn flow_step(&mut self, by: isize, cx: &mut Context<Self>) {
        match self.form {
            FlowForm::Timeline => self.timeline_step(by, cx),
            FlowForm::LiveDiff => self.live_step(by, cx),
        }
    }

    /// What [`ToggleLayout`] does.
    pub(crate) fn toggle_layout(
        &mut self,
        _: &ToggleLayout,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let was = self.position_row;
        self.layout = match self.layout {
            Layout::Unified => Layout::Split,
            Layout::Split => Layout::Unified,
        };
        self.rebuild(cx);
        // The line of the reviewer has another row now. The view goes with it, so the
        // line is drawn at the height it was drawn at; and should it have been out of
        // view before, it is brought in.
        if let (Some(was), Some(now)) = (was, self.position_row) {
            self.shift_view(now as f32 - was as f32);
            self.scroll.scroll_to_item(now, ScrollStrategy::Nearest);
        }
    }

    /// Moves the reviewer by `by` lines in the order of the lines, and stays at the
    /// first and at the last.
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
        self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
    }

    /// Puts the reviewer on a line of the row at `index`: what a click on it does.
    pub(crate) fn go_to(&mut self, index: usize, side: Side, cx: &mut Context<Self>) {
        // A click is told by the index its row had when it was drawn. The rows may have
        // changed since, and then there may be no such row, or no line in it.
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
        // What was shown for the line the reviewer left goes now, not when the next frame
        // is drawn.
        self.follow_cursor();
        cx.notify();
    }

    /// Scrolls back to the first row: other changes are read from their start.
    fn to_top(&self) {
        let handle = self.scroll.0.borrow().base_handle.clone();
        handle.set_offset(point(px(0.), px(0.)));
    }

    /// Scrolls by this many rows without the reviewer moving, so that a row that moved
    /// by as many in the list is drawn where it was.
    fn shift_view(&self, rows: f32) {
        let handle = self.scroll.0.borrow().base_handle.clone();
        let mut offset = handle.offset();
        // The offset of a list scrolled down is negative; the list itself keeps it from
        // passing its end.
        offset.y = (offset.y - draw::ROW_HEIGHT * rows).min(px(0.));
        handle.set_offset(offset);
    }
}

/// The two versions of a file as bytes, or why its lines cannot be shown. A version that
/// does not exist has no bytes: the diff is then of added lines only, or of removed
/// ones.
fn versions(sides: FileSides) -> Result<(Vec<u8>, Vec<u8>), Unshown> {
    let bytes = |content| match content {
        Content::Absent => Ok(Vec::new()),
        Content::Text(bytes) => Ok(bytes),
        Content::Binary { .. } => Err(Unshown::Binary),
        Content::TooLarge { .. } => Err(Unshown::TooLarge),
    };
    Ok((bytes(sides.old)?, bytes(sides.new)?))
}

/// The heading of a file and the rows under it, with the threads of the review that sit
/// on them, and the threads it placed.
fn file_rows(
    file: &ShownFile,
    layout: Layout,
    review: Option<&Review>,
) -> (Vec<PaneRow>, Vec<ThreadId>) {
    file_rows_in(file, layout, review, None)
}

/// As [`file_rows`], for a diff whose new side is the version `shown`, none for the files
/// on disk: a thread that moved is placed by it (`threads::place_of`).
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

/// The lines a row shows that the reviewer can be on, each as [`Position`] names it: an
/// unchanged line once, by its new side; in the split view the removed line of a row
/// before the added one.
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

/// Every line the reviewer can be on, in the one order the lines have, each by the index
/// of the row that shows it and its side.
///
/// The order is that of the unified view: the removed lines of a change, then the lines
/// added in their place. The split view shows the two beside each other, row by row, so
/// there the old sides of the rows of one change come first and their new sides after.
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
    // The rows of threads and comments are in no run and hold no line: a thread in the
    // middle of a change does not split it in two.
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
        // The rows of one change in the split view; one row in every other case.
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
                // A plain key that worked anywhere would be taken from a program in a
                // terminal, and from every field a user types into. The keys of the tree
                // work in the tree, which is beside the rows and not inside them.
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
                    // The keys of the list of references work in the list.
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
                ("shift-t", ToggleKeepSelection.name(), None, Asks::Nothing),
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

    /// The source of a key that tuicr binds.
    const TUICR: &str = "tuicr 0.29.0 docs/KEYBINDINGS.md";

    #[test]
    fn for_each_action_that_vim_or_tuicr_has_a_key_for_the_default_key_is_that_key() {
        // Every action of the pane that vim or tuicr has a key for, with that key and
        // whose it is. An action may be here twice, once for each key it has.
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
            // The commit selector of tuicr: Tab cycles the focus, j and k and the arrows
            // move, space marks a commit and enter shows the marked ones.
            (FocusHistory.name(), "tab", vec![TUICR]),
            (FocusRows.name(), "tab", vec![TUICR]),
            (HistoryDown.name(), "j", vec!["vim", TUICR]),
            (HistoryDown.name(), "down", vec![TUICR]),
            (HistoryUp.name(), "k", vec!["vim", TUICR]),
            (HistoryUp.name(), "up", vec![TUICR]),
            (MarkChange.name(), "space", vec![TUICR]),
            (ShowChange.name(), "enter", vec![TUICR]),
            // Resuming follow goes to the newest step, the last of Flow: vim's `G` goes to
            // the last line, tuicr's to the last file.
            (ResumeFollow.name(), "shift-g", vec!["vim", TUICR]),
            // Code intelligence: vim's `l` and `h` step right and left, here a name at a
            // time. Why these keys and not vim's word motion: the pane has no place inside a
            // line but a name, so the step is from name to name, and vim's `w` is taken (it
            // is Original Line). `K` looks up the word under the cursor; `gd` goes to its
            // definition.
            (NextName.name(), "l", vec!["vim"]),
            (PreviousName.name(), "h", vec!["vim"]),
            (Hover.name(), "shift-k", vec!["vim"]),
            (GoToDefinition.name(), "g d", vec!["vim"]),
            // The list of references moves as tuicr's lists do, and opens as its commit
            // selector confirms.
            (ReferenceDown.name(), "j", vec!["vim", TUICR]),
            (ReferenceDown.name(), "down", vec![TUICR]),
            (ReferenceUp.name(), "k", vec!["vim", TUICR]),
            (ReferenceUp.name(), "up", vec![TUICR]),
            (OpenReference.name(), "enter", vec![TUICR]),
        ];
        // The actions that neither has a key for, and why.
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
            // Back to the Diff is Escape, tuicr's key for leaving a mode. Vim jumps back with
            // `ctrl-o`, and has `ctrl-t` for the tag stack; on Linux both belong to the
            // app: `ctrl-o` is the Frame's Open Folder and `ctrl-t` the terminal pane's New
            // Terminal, and a control has one key, the same on both systems.
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
        // An action added later has to be in one of the two lists, or this fails.
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
        // A key listed here for an action that has no control, or a reason for one that
        // has a key of vim's or tuicr's, would be a mistake in the lists.
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
