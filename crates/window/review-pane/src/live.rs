//! The live diff of Flow: the change of one turn of the workspace on screen, as it grows.
//!
//! Built by the review pane's second package of slice 5 (row RP-04). Comments written on
//! its lines are `flow_comment.rs`'s.
//!
//! Which turn. The turn the live diff shows is the newest turn of the timeline: the newest
//! that started, while one runs; else the one the timeline was filled with. The version it
//! counts from is that turn's start as the timeline's `GetTurn` answer gave it
//! (`Timeline::start_of`): until that answer is taken the live diff asks nothing; where it
//! gave none, the commit checked out (`crate::tree::checked_out` of the history, once the
//! history is here, `ReviewPane::live_history`). When a later answer gives the turn
//! another start (a turn that started while the state of the turn before was being
//! recorded), the live diff starts over from it.
//!
//! What it is the diff of. While the turn runs: from that version to the files on disk,
//! read with `Request::CompareVersions { from, to: None }` for the list and
//! `Request::ReadVersions { old: from, new: None }` for each file, a few at a time
//! ([`crate::FILES_ON_THEIR_WAY`]), each diffed off the thread that draws as Changes does.
//! Once the turn ended with a state: from that version to that state, the same two
//! requests with `to` and `new` the state. With no turn known: no rows, and the line
//! `flow-empty`.
//!
//! When it asks. Only while it is on screen: Flow on screen in its second form
//! (`ReviewPane::view` is Flow and `ReviewPane::flow_form` the live diff). While it is not,
//! the facts below only mark what it holds as stale, and nothing is asked. When it comes
//! on screen, or when its turn or the version it counts from changes while it is, it asks
//! for the list once, then for the files. [`ReviewPane::live_look`] is where it looks: the
//! timeline calls it after each change the live diff reads (a turn added or read, the
//! view, the form, a workspace on screen), and the live diff's own hooks below end with it.
//!
//! What makes it grow. While the turn runs, the list is asked again at each `Fact::Step`
//! of that turn that is an edit or a file written whole, and at each `Fact::ChangesChanged`
//! of the workspace on screen, which is how a file a shell command wrote reaches it; a read
//! step asks nothing. At the `Fact::TurnEnded` of that turn with a state, the list is asked
//! again to that state. One question for the list is on its way at a time; a reason to
//! ask again that comes meanwhile asks once more when the answer is here. When the list is
//! here, a file new to it is read, and so is every file a step named since the last list,
//! and every file of the list when a `ChangesChanged` came since; a file keeps what is
//! shown of it until its new versions are here, so that nothing blinks.
//!
//! Following. When the rows are made again while follow is `On` and Flow is at a step
//! (`ReviewPane::at`) whose file the live diff now holds, the list is scrolled so that the
//! heading of that file is in view: a step that came before its file was in the list is
//! followed once it is. While follow is paused or off the scroll stays where it is.
//!
//! Its rows are [`crate::PaneRow`]s, as Changes' are, in the pane's layout, with the
//! threads of the review on screen on their lines (`crate::threads`), made again whenever
//! Changes' are (`ReviewPane::rebuild` calls [`ReviewPane::live_rebuild`]). The reviewer is
//! on one line of it, moved by `j`, `k` and a click.

use std::ops::Range;
use std::path::Path;

use gpui_kit::*;
use yaar_link::{ChangedFile, Fact};

use crate::{FlowAt, LiveChange, PaneRow, Position, ReviewPane};

/// What the live diff keeps. The fields are the package's to choose.
pub(crate) struct Live {}

impl Live {
    /// A live diff that shows nothing yet.
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        let _ = cx;
        Self {}
    }

    /// The change shown.
    pub(crate) fn change(&self) -> Option<&LiveChange> {
        todo!("the second package of the review pane: the change of the live diff")
    }

    /// Its rows.
    pub(crate) fn rows(&self) -> &[PaneRow] {
        todo!("the second package of the review pane: the rows of the live diff")
    }

    /// The line the reviewer is on.
    pub(crate) fn position(&self) -> Option<&Position> {
        todo!("the second package of the review pane: the reviewer's line")
    }

    /// The text of a line of a file it holds.
    pub(crate) fn text(&self, position: &Position) -> Option<String> {
        let _ = position;
        todo!("the second package of the review pane: the text of a line")
    }

    /// The file of the list at `path`, as the last `CompareVersions` answer named it: its
    /// status, and for a renamed file the path it had in the version counted from. What a
    /// comment's anchor is built from.
    pub(crate) fn file(&self, path: &Path) -> Option<&ChangedFile> {
        let _ = path;
        todo!("the second package of the review pane: a file of the list")
    }

    /// The step named in the line above the files, while it has no row.
    pub(crate) fn activity(&self) -> Option<&FlowAt> {
        todo!("the second package of the review pane: the step with no row")
    }

    /// The rows in view, as last laid out.
    pub(crate) fn in_view(&self) -> Range<usize> {
        todo!("the second package of the review pane: the rows in view")
    }
}

impl ReviewPane {
    /// The live diff looks at what it shows and whether it is on screen, as the module
    /// says, and asks for the list when it is on screen and what it holds is stale, of
    /// another turn, or counted from another version. Called by the timeline after each
    /// change the live diff reads, and last by the live diff's own hooks.
    ///
    /// Filled by the second package of the review pane; the contract's body does nothing.
    pub(crate) fn live_look(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The workspace on screen may be another: the live diff is emptied when it is.
    ///
    /// Filled by the second package of the review pane; the contract's body does nothing.
    pub(crate) fn live_workspace(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The agents model changed: a turn that started in the workspace on screen becomes
    /// the turn shown.
    ///
    /// Filled by the second package of the review pane; the contract's body does nothing.
    pub(crate) fn live_agents(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The history was answered: what the change of a turn with no start counts from may
    /// be known now.
    ///
    /// Filled by the second package of the review pane; the contract's body does nothing.
    pub(crate) fn live_history(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// A fact of the Keeper, for the live diff: `Step`, `ChangesChanged` and `TurnEnded`
    /// of the workspace on screen. Called after the timeline heard it.
    ///
    /// Filled by the second package of the review pane; the contract's body does nothing.
    pub(crate) fn live_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        let _ = (fact, cx);
    }

    /// Makes the rows of the live diff again from what it holds, in the pane's layout and
    /// with the threads of the review on screen, keeping the reviewer's line.
    ///
    /// Filled by the second package of the review pane; the contract's body does nothing.
    pub(crate) fn live_rebuild(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// What [`crate::LineDown`] and [`crate::LineUp`] do in the live diff, as they do in
    /// Changes.
    pub(crate) fn live_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let _ = (by, cx);
        todo!("the second package of the review pane: moving through the live diff")
    }

    /// The index in the live diff's rows of the heading of the file the step `at` names;
    /// none for a command, or a file the live diff does not hold.
    pub(crate) fn live_row_of(&self, at: &FlowAt) -> Option<usize> {
        let _ = at;
        todo!("the second package of the review pane: the row of a step")
    }

    /// Scrolls the live diff so that row `row` is in view, as follow wants it.
    pub(crate) fn scroll_live_to(&mut self, row: usize) {
        let _ = row;
        todo!("the second package of the review pane: scrolling the live diff")
    }
}
