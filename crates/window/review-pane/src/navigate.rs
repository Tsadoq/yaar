//! Code intelligence in the diff: the name the reviewer is on, what the Navigator reports
//! for it, its definition, its references, and the place the reviewer goes back to
//! (FR-051, FR-052, FR-054, FR-055, FR-068).
//!
//! # The name the reviewer is on
//!
//! On the line the reviewer is on, one name is the reviewer's: the first name of the line
//! when the reviewer comes to the line ([`crate::names`]), the next or the one before with
//! [`crate::NextName`] (`l`) and [`crate::PreviousName`] (`h`), and the one clicked when
//! the reviewer clicks on a name of a line: the click puts the reviewer on that line, as
//! before, and on the name under the pointer, found from the pointer's distance into the
//! line's text, the font being of fixed width. It is drawn with a mark under the selector
//! `review-name`, whose bounds are those of the name. [`crate::ReviewPane::name_at`] reads
//! it.
//!
//! # Asking
//!
//! [`crate::Hover`] (`shift-k`), and the mouse resting on a name for [`HOVER_DELAY`],
//! ask `Navigation::hover` for that name; [`crate::GoToDefinition`] (`g d`), and a click
//! with the secondary key held (Cmd on macOS, Ctrl on Linux), `Navigation::definition`;
//! [`crate::FindReferences`] (`g r`) `Navigation::references`. The place asked
//! ([`crate::ReviewPane::place_at`]) is the reviewer's line and name in the version that
//! line belongs to:
//!
//! | The line | `Place::tree` | `Place::path` |
//! | --- | --- | --- |
//! | a removed line, in the unified view or on the old half of the split view | `Tree::Version(<the version the old side is read in>)` | the old path |
//! | an added or unchanged line of a change that ends at the files on disk | `Tree::Worktree` | the path now |
//! | an added or unchanged line of a change that ends at a version (a past turn, a commit, a range) | `Tree::Past { version, text }`, the text of the whole new side as the pane holds it | the path now |
//!
//! The version the old side is read in is the one the pane read the old side of the file
//! in: `Changes::counted_from` for the comparison, the version before the first entry of a
//! selection, a turn's start, the `from` of what changed since a review (FR-055). Its line
//! is counted from 1. `Place::shown` is the paths of the files of the change shown.
//!
//! An ask replaces the one before of its kind: the [`yaar_frame::Asking`] of the one before
//! is dropped, which cancels it. Moving to another line drops the hover and its ask.
//!
//! # What is shown
//!
//! The hover ([`HoverShown`]) is drawn under the rows, above the notice, under the selector
//! `review-hover`, its text under `review-hover-text:<text>`; while its answer is on its
//! way `review-hover-waiting`. An answer found by text match carries
//! `review-hover-guessed` and the words [`GUESSED`] above the text (FR-068). It goes when
//! the reviewer moves to another line or name, and with Escape.
//!
//! A definition with one location opens it in the file view ([`crate::file_view`]); with
//! several, or by text match, they are listed as references are. References are listed
//! ([`ReferencesShown`]) under the rows, under the selector `review-references`, each
//! under `review-reference-<index>` with its path relative to the worktree where it lies
//! in it, its line and its text, the chosen one marked `review-reference-chosen`, the list
//! marked `review-references-guessed` with [`GUESSED`] when found by text match. The list
//! takes the focus, and its keys work in its own context [`crate::REFERENCES_CONTEXT`]:
//! `j`, `down`, `k`, `up` move the chosen one, `enter` opens it in the file view, `escape`
//! closes the list and gives the focus back to the rows. A click on an entry opens it.
//! With no location, the notice says that none was found.
//!
//! The pane tells `Navigation::old_side_shown` the old side of the change shown, the
//! workspace's and its version, whenever the change shown or its old side changes.

use std::time::Duration;

use gpui_kit::*;
use yaar_navigator::{Location, Place};

use crate::{Position, ReviewPane};

/// How long the mouse rests on a name before its hover is asked for.
pub const HOVER_DELAY: Duration = Duration::from_millis(400);

/// The words above an answer found by text match.
pub const GUESSED: &str = "Found by text match while no language server is ready: it may be wrong";

/// The hover the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverShown {
    /// Where it was asked.
    pub place: Place,
    /// What the Navigator reported; none while it is on its way, and where it reported
    /// nothing.
    pub text: Option<String>,
    /// Whether it was found by text match.
    pub guessed: bool,
    /// Whether the answer is still on its way.
    pub waiting: bool,
}

/// The list of references, or of definitions, the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencesShown {
    /// Where it was asked.
    pub place: Place,
    /// The locations; empty while they are on their way.
    pub locations: Vec<Location>,
    /// Whether they were found by text match.
    pub guessed: bool,
    /// Whether they are still on their way.
    pub waiting: bool,
    /// The chosen one, by its index.
    pub chosen: usize,
    /// Whether these are definitions, listed because there were several.
    pub definitions: bool,
}

/// What this module keeps in the pane: the name the reviewer is on, the asks on their way,
/// the hover and the list shown.
#[derive(Default)]
pub(crate) struct Navigate {}

impl ReviewPane {
    /// The name the reviewer is on, as its line holds it.
    pub fn name_at(&self) -> Option<String> {
        todo!()
    }

    /// The place the Navigator is asked about for the name the reviewer is on, as the
    /// module doc's table says; none where the reviewer is on no name.
    pub fn place_at(&self, position: &Position, cx: &App) -> Option<Place> {
        let _ = (position, cx);
        todo!()
    }

    /// The hover shown.
    pub fn hover(&self) -> Option<&HoverShown> {
        todo!()
    }

    /// The list of references or definitions shown.
    pub fn references(&self) -> Option<&ReferencesShown> {
        todo!()
    }

    /// What [`crate::NextName`] does.
    pub(crate) fn next_name(
        &mut self,
        _: &crate::NextName,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::PreviousName`] does.
    pub(crate) fn previous_name(
        &mut self,
        _: &crate::PreviousName,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::Hover`] does.
    pub(crate) fn hover_name(&mut self, _: &crate::Hover, _: &mut Window, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// What [`crate::GoToDefinition`] does.
    pub(crate) fn go_to_definition(
        &mut self,
        _: &crate::GoToDefinition,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::FindReferences`] does.
    pub(crate) fn find_references(
        &mut self,
        _: &crate::FindReferences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::ReferenceDown`] does.
    pub(crate) fn reference_down(
        &mut self,
        _: &crate::ReferenceDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::ReferenceUp`] does.
    pub(crate) fn reference_up(
        &mut self,
        _: &crate::ReferenceUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::OpenReference`] does.
    pub(crate) fn open_reference(
        &mut self,
        _: &crate::OpenReference,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// What [`crate::CloseReferences`] does.
    pub(crate) fn close_references(
        &mut self,
        _: &crate::CloseReferences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = cx;
    }

    /// The hover, drawn under the rows; none while there is none.
    pub(crate) fn render_hover(&self, cx: &Context<Self>) -> Option<Div> {
        let _ = cx;
        None
    }

    /// The list of references, drawn under the rows; none while there is none.
    pub(crate) fn render_references(&self, cx: &Context<Self>) -> Option<Div> {
        let _ = cx;
        None
    }

    /// Tells `Navigation::old_side_shown` the old side of the change shown, when it is not
    /// what was told last. Called at every rebuild of the rows.
    pub(crate) fn tell_old_side(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }
}
