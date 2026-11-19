//! The live diff of Flow: the change of one turn of the workspace on screen, as it grows,
//! and the comments written on its lines.
//!
//! Built by the review pane's second package of slice 5 (rows RP-04, RP-14).
//!
//! Which turn. The turn the live diff shows is the newest turn of the timeline: the newest
//! that started, while one runs; else the one the timeline was filled with. It reads the
//! version that turn counts from in `yaar_link::Turn::start`, which the timeline's
//! `GetTurn` answer gives (`ReviewPane::flow`, `Timeline::step` and its entries), or, where
//! that is none, the commit checked out (`crate::tree::checked_out` of the history).
//!
//! What it is the diff of. While the turn runs: from that version to the files on disk,
//! read with `Request::CompareVersions { from, to: None }` for the list and
//! `Request::ReadVersions { old: from, new: None }` for each file, a few at a time
//! ([`crate::FILES_ON_THEIR_WAY`]), each diffed off the thread that draws as Changes does.
//! Once the turn ended with a state: from that version to that state, the same two
//! requests with `to` and `new` the state. With no turn known: no rows, and the line
//! `flow-empty`.
//!
//! What makes it grow. While the turn runs, the list is asked again at each `Fact::Step`
//! of that turn that is an edit or a file written whole, and at each `Fact::ChangesChanged`
//! of the workspace on screen, which is how a file a shell command wrote reaches it. One
//! question for the list is on its way at a time; a reason to ask again that comes
//! meanwhile asks once more when the answer is here. When the list is here, a file new to
//! it is read, and so is every file a step named since the last list, and every file of the
//! list when a `ChangesChanged` came since; a file keeps what is shown of it until its new
//! versions are here, so that nothing blinks.
//!
//! Its rows are [`crate::PaneRow`]s, as Changes' are, in the pane's layout, with the
//! threads of the review on screen on their lines (`crate::threads`), made again whenever
//! Changes' are (`ReviewPane::rebuild` calls [`ReviewPane::live_rebuild`]). The reviewer is
//! on one line of it, moved by `j`, `k` and a click; `c` writes a comment there: the box is
//! the pane's one comment box, and the draft's anchor is the line's path in its version,
//! its side, its line and its text, in the version of that side: the new side in `to`, or
//! the files on disk (none), the old side in `from`, exactly as Changes anchors a line of
//! the same version, so that the thread shows on that line in Changes.

use std::ops::Range;

use gpui_kit::*;
use yaar_link::Fact;

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

    /// What [`crate::Comment`] does in Flow: on the line of the live diff the reviewer is
    /// on, the comment box opens for the first comment of a new thread; in the timeline,
    /// the pane says that a comment is written on a line of the live diff.
    pub(crate) fn comment_in_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = (window, cx);
        todo!("the second package of the review pane: a comment written in Flow")
    }
}
