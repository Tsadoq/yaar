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
//! before, and on the name under the pointer, found by the layout the line's text was drawn
//! with. The text of a line is drawn by an element as wide as the text, under the selector
//! `review-text-<row>-<side>:<text>` (`review-file-text-<line>:<text>` in the file view). The
//! name is drawn with a mark under the selector `review-name`, whose bounds are those of the
//! name: an underline of an invisible copy of the line laid over it.
//! [`crate::ReviewPane::name_at`] reads it.
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
//! way `review-hover-waiting`, and `review-hover-nothing` where the Navigator reported
//! nothing. An answer found by text match carries `review-hover-guessed` and the words
//! [`GUESSED`], under `review-hover-guessed-words:<words>`, above the text (FR-068). It goes
//! when the reviewer moves to another line or name, and with Escape; a hover asked for the
//! name the mouse rested on goes when the reviewer moves, but not when the mouse does.
//!
//! A definition with one location opens it in the file view ([`crate::file_view`]); with
//! several, or by text match, they are listed as references are. References are listed
//! ([`ReferencesShown`]) under the rows, under the selector `review-references`, each
//! under `review-reference-<index>` with its path relative to the worktree where it lies
//! in it, its line and its text, under `review-reference-words-<index>:<path>:<line>
//! <text>`, the chosen one marked `review-reference-chosen`, the list titled under
//! `review-references-title:<words>` and marked `review-references-guessed`, with [`GUESSED`]
//! under `review-references-guessed-words:<words>`, when found by text match; while the
//! answer is on its way `review-references-waiting`. At most eight entries are drawn, the
//! chosen one among them. The list takes the focus when its answer is here, and its keys work
//! in its own context [`crate::REFERENCES_CONTEXT`]: `j`, `down`, `k`, `up` move the chosen
//! one, `enter` opens it in the file view, `escape` closes the list and gives the focus
//! back to the rows. A click on an entry opens it. The list closes when the file view
//! opens, and the focus is the rows'. With no location, the notice says that none was
//! found.
//!
//! The pane tells `Navigation::old_side_shown` the old side of the change shown, the
//! workspace's and its version, whenever the change shown or its old side changes, and
//! again when a Navigator is connected after the change was shown.
//!
//! # Where the keys act
//!
//! On the lines of Changes and on the lines of the file view. In Flow, whose lines are
//! another list, the name keys say so in the notice and ask nothing.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::Side;
use yaar_frame::NavError;
use yaar_link::WorkspaceId;
use yaar_navigator::{Answered, FileKind, Location, Place, Tree as NavTree};

use crate::draw::ROW_HEIGHT;
use crate::names;
use crate::threads::path_in;
use crate::tree::Source;
use crate::{FileContent, FileShown, Position, REFERENCES_CONTEXT, ReviewPane, View};

/// How long the mouse rests on a name before its hover is asked for.
pub const HOVER_DELAY: Duration = Duration::from_millis(400);

/// The words above an answer found by text match.
pub const GUESSED: &str = "Found by text match while no language server is ready: it may be wrong";

/// How many entries of the list of references are drawn at once, the chosen one among them.
const ENTRIES_IN_VIEW: usize = 8;

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

/// Where the reviewer is: a line of the diff, or a line of the file view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cursor {
    /// A line of a changed file.
    Diff(Position),
    /// A line of the file view, from 1.
    File {
        /// The file.
        path: PathBuf,
        /// The line.
        line: u32,
    },
}

/// A name under the mouse, or under the reviewer's cursor, as a place of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Spot {
    /// A row of the diff, on one of its sides.
    Diff {
        /// The row, by its index in the rows of the pane.
        index: usize,
        /// The side of the row.
        side: Side,
    },
    /// A line of the file view, from 1.
    File(u32),
}

/// The name the mouse rests on, and the wait for it to have rested long enough.
struct Resting {
    spot: Spot,
    /// Where the name starts in the line.
    start: usize,
    /// Dropped with the rest: the timer then never fires.
    _timer: Task<()>,
}

/// The hover shown and the ask for it, which a new ask or a move of the reviewer drops: a
/// dropped ask that is still on its way tells the Navigator it is cancelled.
struct Hovering {
    shown: HoverShown,
    _asking: Task<()>,
}

/// The list shown and the ask for it, if one is on its way.
struct Listing {
    shown: ReferencesShown,
    _asking: Option<Task<()>>,
}

/// The place left in the diff when a file view opened from it.
pub(crate) struct Left {
    /// The view that was on screen.
    pub(crate) view: View,
    /// The name the reviewer was on.
    pub(crate) name: Option<(Cursor, usize)>,
}

/// What this module keeps in the pane: the name the reviewer is on, the asks on their way,
/// the hover and the list shown, the file view and the place left in the diff.
#[derive(Default)]
pub(crate) struct Navigate {
    /// The name the reviewer chose on a line, by its index among the names of the line.
    /// It holds for that line only: on another it is the first.
    pub(crate) name: Option<(Cursor, usize)>,
    /// What the reviewer was on when the hover was last checked against it.
    synced: Option<(Cursor, usize)>,
    hover: Option<Hovering>,
    references: Option<Listing>,
    /// The ask for a definition that is on its way.
    definition: Option<Task<()>>,
    /// The ask for a file that is on its way.
    pub(crate) opening: Option<Task<()>>,
    resting: Option<Resting>,
    /// The focus of the list of references, made when the first list is shown.
    list_focus: Option<FocusHandle>,
    /// What `Navigation::old_side_shown` was last told, when it was told.
    told_old_side: Option<(WorkspaceId, Option<String>)>,
    /// Whether the pane observes the Frame's model of code intelligence yet.
    observing: bool,
    /// The file view.
    pub(crate) file: Option<FileShown>,
    /// Where each line of the file of the file view starts and ends, line ends not
    /// included.
    pub(crate) file_lines: Vec<Range<usize>>,
    /// The scroll of the file view's lines.
    pub(crate) file_scroll: UniformListScrollHandle,
    /// The place left in the diff, while a file view is open.
    pub(crate) left: Option<Left>,
}

/// The reviewer's line, its names, and the one the reviewer is on.
struct Current {
    cursor: Cursor,
    text: String,
    names: Vec<Range<usize>>,
    index: usize,
}

impl Current {
    /// The range of the name the reviewer is on, in `text`.
    fn range(&self) -> Option<Range<usize>> {
        self.names.get(self.index).cloned()
    }
}

impl ReviewPane {
    /// The name the reviewer is on, as its line holds it.
    pub fn name_at(&self) -> Option<String> {
        let current = self.current()?;
        Some(current.text[current.range()?].to_owned())
    }

    /// The place the Navigator is asked about for the name the reviewer is on, as the
    /// module doc's table says; none where the reviewer is on no name.
    pub fn place_at(&self, position: &Position, cx: &App) -> Option<Place> {
        let cursor = Cursor::Diff(position.clone());
        let text = self.line_text(&cursor)?;
        let names = names::names_of(&text);
        let range = match &self.navigate.name {
            Some((at, index)) if *at == cursor => names.get(*index)?.clone(),
            _ => names.first()?.clone(),
        };
        self.place_for(&cursor, &text, range, cx).ok()
    }

    /// Takes the hover away; whether there was one.
    pub(crate) fn drop_hover(&mut self) -> bool {
        self.navigate.hover.take().is_some()
    }

    /// The range, in the text of the line the reviewer is on, of the name the reviewer is
    /// on: where the mark of the name is drawn.
    pub(crate) fn name_range(&self) -> Option<Range<usize>> {
        self.current()?.range()
    }

    /// The hover shown.
    pub fn hover(&self) -> Option<&HoverShown> {
        self.navigate.hover.as_ref().map(|hovering| &hovering.shown)
    }

    /// The list of references or definitions shown.
    pub fn references(&self) -> Option<&ReferencesShown> {
        self.navigate
            .references
            .as_ref()
            .map(|listing| &listing.shown)
    }

    /// Where the reviewer is: the line of the file view while one is open, else the line
    /// of the diff of Changes; none in Flow and where there is no line.
    fn cursor(&self) -> Option<Cursor> {
        if let Some(file) = &self.navigate.file {
            return Some(Cursor::File {
                path: file.file.path.clone(),
                line: file.line,
            });
        }
        if self.view != View::Changes {
            return None;
        }
        self.position.clone().map(Cursor::Diff)
    }

    /// The text of the line the cursor is on, with no line end.
    fn line_text(&self, cursor: &Cursor) -> Option<String> {
        match cursor {
            Cursor::Diff(position) => self.text(position),
            Cursor::File { line, .. } => self.file_line(*line).map(str::to_owned),
        }
    }

    /// The line the reviewer is on, its names, and which of them the reviewer is on.
    fn current(&self) -> Option<Current> {
        let cursor = self.cursor()?;
        let text = self.line_text(&cursor)?;
        let names = names::names_of(&text);
        let index = match &self.navigate.name {
            Some((at, index)) if *at == cursor => (*index).min(names.len().saturating_sub(1)),
            _ => 0,
        };
        Some(Current {
            cursor,
            text,
            names,
            index,
        })
    }

    /// Drops what was shown for a name the reviewer left: when the line or the name of the
    /// reviewer is not what the hover was last checked against, the hover goes, and the ask
    /// for it, if it is on its way, is cancelled. Every action that shows something first
    /// calls this, and so does every frame, which is where a move the actions of the lines
    /// make is seen.
    pub(crate) fn follow_cursor(&mut self) {
        let now = self
            .current()
            .map(|current| (current.cursor, current.index));
        if now != self.navigate.synced {
            self.navigate.synced = now;
            self.navigate.hover = None;
            self.navigate.resting = None;
        }
    }

    /// The place of the name at `range` of `text`, the text of the line the cursor is on.
    fn place_for(
        &self,
        cursor: &Cursor,
        text: &str,
        range: Range<usize>,
        cx: &App,
    ) -> Result<Place, String> {
        let name = text[range.clone()].to_owned();
        let column = range.start as u32;
        let shown: Vec<PathBuf> = self
            .files
            .iter()
            .map(|shown| shown.file.path.clone())
            .collect();
        match cursor {
            Cursor::Diff(position) => {
                let workspace = self.listed_workspace().ok_or("No workspace is on screen")?;
                let index = *self
                    .by_path
                    .get(&position.path)
                    .ok_or("The line is of a file the pane does not hold")?;
                let path = path_in(&self.files[index].file, position.side).to_owned();
                let tree = match position.side {
                    Side::Old => NavTree::Version(
                        self.old_version()
                            .ok_or("There is no version the old lines could be counted from")?,
                    ),
                    Side::New => match &self.listed {
                        Some(Source::Versions {
                            new: Some(version), ..
                        }) => NavTree::Past {
                            version: version.clone(),
                            text: self.new_text(index).ok_or("The file is not text")?,
                        },
                        _ => NavTree::Worktree,
                    },
                };
                Ok(Place {
                    workspace,
                    tree,
                    path,
                    line: position.line + 1,
                    column,
                    name,
                    shown,
                })
            }
            Cursor::File { line, .. } => {
                let file = &self.navigate.file.as_ref().ok_or("No file is shown")?.file;
                let (workspace, tree, path) = match &file.kind {
                    FileKind::Worktree {
                        workspace,
                        relative,
                    } => (workspace.clone(), NavTree::Worktree, relative.clone()),
                    FileKind::Version {
                        workspace,
                        version,
                        relative,
                    } => (
                        workspace.clone(),
                        NavTree::Version(version.clone()),
                        relative.clone(),
                    ),
                    // A file outside is asked about by its absolute path, in the files on
                    // disk of the workspace on screen.
                    FileKind::Outside => (
                        self.models
                            .changes
                            .read(cx)
                            .workspace()
                            .cloned()
                            .ok_or("No workspace is on screen")?,
                        NavTree::Worktree,
                        file.path.clone(),
                    ),
                };
                Ok(Place {
                    workspace,
                    tree,
                    path,
                    line: *line,
                    column,
                    name,
                    shown,
                })
            }
        }
    }

    /// The workspace the files shown belong to.
    fn listed_workspace(&self) -> Option<WorkspaceId> {
        match &self.listed {
            Some(Source::Versions { .. }) => self.tree.of.clone(),
            _ => self
                .changes
                .as_ref()
                .map(|changes| changes.workspace.clone()),
        }
    }

    /// The version the old side of the files shown is read in.
    pub(crate) fn old_version(&self) -> Option<String> {
        match &self.listed {
            Some(Source::Versions { old, .. }) => old.clone(),
            _ => self
                .changes
                .as_ref()
                .and_then(|changes| changes.counted_from.clone()),
        }
    }

    /// The whole text of the new side of the file at `index` of the files, as the pane
    /// holds it: its lines, each followed by a line feed, but for a last line that the
    /// file ends without one.
    fn new_text(&self, index: usize) -> Option<Arc<str>> {
        let FileContent::Diff(shown) = &self.files[index].content else {
            return None;
        };
        let mut text = String::new();
        for line in 0..shown.diff.line_count(Side::New) {
            text.push_str(&String::from_utf8_lossy(shown.diff.line(Side::New, line)));
            text.push('\n');
        }
        if !shown.diff.ends_with_line_feed(Side::New) {
            text.pop();
        }
        Some(text.into())
    }

    /// Whether a name key may act: not in Flow, whose lines are another list. Says why not.
    fn names_here(&mut self, cx: &mut Context<Self>) -> bool {
        if self.navigate.file.is_none() && self.view == View::Flow {
            self.say("Names are asked on the lines of Changes", cx);
            return false;
        }
        true
    }

    /// Moves the name the reviewer is on by `by` names, staying at the first and the last.
    fn step_name(&mut self, by: isize, cx: &mut Context<Self>) {
        self.acted(cx);
        if !self.names_here(cx) {
            return;
        }
        let Some(current) = self.current() else {
            return;
        };
        if current.names.is_empty() {
            return;
        }
        let last = current.names.len() - 1;
        let index = current.index.saturating_add_signed(by).min(last);
        self.navigate.name = Some((current.cursor, index));
        self.follow_cursor();
        cx.notify();
    }

    /// What [`crate::NextName`] does.
    pub(crate) fn next_name(
        &mut self,
        _: &crate::NextName,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_name(1, cx);
    }

    /// What [`crate::PreviousName`] does.
    pub(crate) fn previous_name(
        &mut self,
        _: &crate::PreviousName,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_name(-1, cx);
    }

    /// The place of the name the reviewer is on, ready to ask: or the sentence that says,
    /// in the notice, why there is none.
    fn place_to_ask(&mut self, cx: &mut Context<Self>) -> Option<Place> {
        if !self.names_here(cx) {
            return None;
        }
        if !self.models.navigation.read(cx).is_connected() {
            self.say(NavError::NotConnected.to_string(), cx);
            return None;
        }
        self.follow_cursor();
        let Some(current) = self.current() else {
            self.say("The reviewer is on no line", cx);
            return None;
        };
        let Some(range) = current.range() else {
            self.say("There is no name on this line", cx);
            return None;
        };
        match self.place_for(&current.cursor, &current.text, range, cx) {
            Ok(place) => Some(place),
            Err(why) => {
                self.say(why, cx);
                None
            }
        }
    }

    /// What [`crate::Hover`] does.
    pub(crate) fn hover_name(&mut self, _: &crate::Hover, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        if let Some(place) = self.place_to_ask(cx) {
            self.ask_hover(place, cx);
        }
    }

    /// What [`crate::GoToDefinition`] does.
    pub(crate) fn go_to_definition(
        &mut self,
        _: &crate::GoToDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        if let Some(place) = self.place_to_ask(cx) {
            self.ask_definition(place, window, cx);
        }
    }

    /// What [`crate::FindReferences`] does.
    pub(crate) fn find_references(
        &mut self,
        _: &crate::FindReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(place) = self.place_to_ask(cx) else {
            return;
        };
        let asking = self.models.navigation.update(cx, |navigation, cx| {
            navigation.references(place.clone(), cx)
        });
        let task = cx.spawn_in(window, async move |pane, cx| {
            let answer = asking.await;
            pane.update_in(cx, |pane, window, cx| {
                pane.references_answered(answer, window, cx)
            })
            .ok();
        });
        self.navigate.references = Some(Listing {
            shown: ReferencesShown {
                place,
                locations: Vec::new(),
                guessed: false,
                waiting: true,
                chosen: 0,
                definitions: false,
            },
            _asking: Some(task),
        });
        cx.notify();
    }

    /// Asks the Navigator for the hover at `place`, in place of the one before, whose ask is
    /// cancelled when it is still on its way.
    fn ask_hover(&mut self, place: Place, cx: &mut Context<Self>) {
        let asking = self
            .models
            .navigation
            .update(cx, |navigation, cx| navigation.hover(place.clone(), cx));
        let task = cx.spawn(async move |pane, cx| {
            let answer = asking.await;
            pane.update(cx, |pane, cx| pane.hover_answered(answer, cx))
                .ok();
        });
        self.navigate.hover = Some(Hovering {
            shown: HoverShown {
                place,
                text: None,
                guessed: false,
                waiting: true,
            },
            _asking: task,
        });
        cx.notify();
    }

    /// The answer to the hover asked last.
    fn hover_answered(
        &mut self,
        answer: Result<Answered<Option<String>>, NavError>,
        cx: &mut Context<Self>,
    ) {
        match (answer, &mut self.navigate.hover) {
            (Ok(answered), Some(hovering)) => {
                hovering.shown.text = answered.value;
                hovering.shown.guessed = answered.guessed;
                hovering.shown.waiting = false;
                cx.notify();
            }
            (Err(error), _) => {
                self.navigate.hover = None;
                self.say(error.to_string(), cx);
            }
            (Ok(_), None) => {}
        }
    }

    /// Asks the Navigator where the name at `place` is defined, and opens the one place it
    /// answers, or lists the places.
    pub(crate) fn ask_definition(
        &mut self,
        place: Place,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let asking = self.models.navigation.update(cx, |navigation, cx| {
            navigation.definition(place.clone(), cx)
        });
        self.navigate.definition = Some(cx.spawn_in(window, async move |pane, cx| {
            let answer = asking.await;
            pane.update_in(cx, |pane, window, cx| {
                pane.definition_answered(&place, answer, window, cx)
            })
            .ok();
        }));
    }

    /// The answer to the definition asked last: one place sure of itself opens in the file
    /// view; several, or one found by text match, are listed.
    fn definition_answered(
        &mut self,
        place: &Place,
        answer: Result<Answered<Vec<Location>>, NavError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Answered {
            value: locations,
            guessed,
        } = match answer {
            Ok(answered) => answered,
            Err(error) => return self.say(error.to_string(), cx),
        };
        match (locations.as_slice(), guessed) {
            ([], _) => self.say(format!("No definition of {} was found", place.name), cx),
            ([only], false) => self.open_location(only.clone(), window, cx),
            _ => {
                self.navigate.references = Some(Listing {
                    shown: ReferencesShown {
                        place: place.clone(),
                        locations,
                        guessed,
                        waiting: false,
                        chosen: 0,
                        definitions: true,
                    },
                    _asking: None,
                });
                self.take_focus_for_list(window, cx);
                cx.notify();
            }
        }
    }

    /// The answer to the references asked last.
    fn references_answered(
        &mut self,
        answer: Result<Answered<Vec<Location>>, NavError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(listing) = &mut self.navigate.references else {
            return;
        };
        let name = listing.shown.place.name.clone();
        match answer {
            Err(error) => {
                self.navigate.references = None;
                self.say(error.to_string(), cx);
            }
            Ok(Answered { value, .. }) if value.is_empty() => {
                self.navigate.references = None;
                self.say(format!("No references to {name} were found"), cx);
            }
            Ok(answered) => {
                listing.shown.locations = answered.value;
                listing.shown.guessed = answered.guessed;
                listing.shown.waiting = false;
                self.take_focus_for_list(window, cx);
                cx.notify();
            }
        }
    }

    /// Gives the focus to the list of references, whose keys then work.
    fn take_focus_for_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .navigate
            .list_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        focus.focus(window, cx);
    }

    /// Closes the list of references, and gives the focus back to the rows.
    pub(crate) fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigate.references.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    /// What [`crate::ReferenceDown`] does.
    pub(crate) fn reference_down(
        &mut self,
        _: &crate::ReferenceDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.choose(1, cx);
    }

    /// What [`crate::ReferenceUp`] does.
    pub(crate) fn reference_up(
        &mut self,
        _: &crate::ReferenceUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.choose(-1, cx);
    }

    /// Moves the chosen entry by `by`, staying at the first and the last.
    fn choose(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(listing) = &mut self.navigate.references else {
            return;
        };
        let shown = &mut listing.shown;
        let last = shown.locations.len().saturating_sub(1);
        shown.chosen = shown.chosen.saturating_add_signed(by).min(last);
        cx.notify();
    }

    /// What [`crate::OpenReference`] does.
    pub(crate) fn open_reference(
        &mut self,
        _: &crate::OpenReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(shown) = self.references() else {
            return;
        };
        let chosen = shown.chosen;
        self.open_entry(chosen, window, cx);
    }

    /// Opens the entry `index` of the list, which a click or `enter` chose.
    fn open_entry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(listing) = &mut self.navigate.references else {
            return;
        };
        let Some(location) = listing.shown.locations.get(index).cloned() else {
            return;
        };
        listing.shown.chosen = index;
        self.open_location(location, window, cx);
    }

    /// What [`crate::CloseReferences`] does.
    pub(crate) fn close_references(
        &mut self,
        _: &crate::CloseReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.close_list(window, cx);
    }

    /// A click on the text of a row of the diff: the reviewer comes to that line and to the
    /// name under the pointer, if there is one; with the secondary key held, the pointer's
    /// name is gone to the definition of.
    pub(crate) fn clicked_row(
        &mut self,
        index: usize,
        side: Side,
        layout: &TextLayout,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to(index, side, cx);
        let spot = Spot::Diff { index, side };
        self.clicked(spot, layout, event, window, cx);
    }

    /// The part of a click on a line that the diff and the file view share.
    pub(crate) fn clicked(
        &mut self,
        spot: Spot,
        layout: &TextLayout,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some((cursor, text)) = self.spot_line(&spot) else {
            return;
        };
        // The cursor is where the click put it: a line the row has none of leaves it be.
        if self.cursor().as_ref() != Some(&cursor) {
            return;
        }
        let under = name_under(&text, layout, event.position);
        if let Some(index) = under {
            self.navigate.name = Some((cursor, index));
        }
        self.follow_cursor();
        cx.notify();
        if let (true, Some(index)) = (event.modifiers.secondary(cx), under) {
            let range = names::names_of(&text)[index].clone();
            let cursor = self.cursor().expect("the click put the reviewer on a line");
            match self.place_for(&cursor, &text, range, cx) {
                Ok(place) => self.ask_definition(place, window, cx),
                Err(why) => self.say(why, cx),
            }
        }
    }

    /// The cursor a spot stands for, and the text of its line.
    fn spot_line(&self, spot: &Spot) -> Option<(Cursor, String)> {
        let cursor = match spot {
            Spot::Diff { index, side } => {
                let crate::PaneRow::Diff { path, row } = self.rows.get(*index)? else {
                    return None;
                };
                Cursor::Diff(Position {
                    path: path.clone(),
                    side: *side,
                    line: row.line(*side)?,
                })
            }
            Spot::File(line) => Cursor::File {
                path: self.navigate.file.as_ref()?.file.path.clone(),
                line: *line,
            },
        };
        let text = self.line_text(&cursor)?;
        Some((cursor, text))
    }

    /// The mouse moved over the text of a line: when it rests on a name for
    /// [`HOVER_DELAY`], that name's hover is asked. The wait is the executor's, so a test
    /// moves its clock. A move on to another name, or to a place that holds none, ends the
    /// wait; a wait that ends with the pointer gone from the name asks nothing.
    pub(crate) fn moved_over(
        &mut self,
        spot: Spot,
        layout: &TextLayout,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, text)) = self.spot_line(&spot) else {
            self.navigate.resting = None;
            return;
        };
        let Some(index) = name_under(&text, layout, event.position) else {
            self.navigate.resting = None;
            return;
        };
        let range = names::names_of(&text)[index].clone();
        let resting = &self.navigate.resting;
        if resting
            .as_ref()
            .is_some_and(|rest| rest.spot == spot && rest.start == range.start)
        {
            return;
        }
        let name = text[range.clone()].to_owned();
        let timer = cx.spawn_in(window, {
            let (spot, name, layout) = (spot.clone(), name.clone(), layout.clone());
            async move |pane, cx| {
                cx.background_executor().timer(HOVER_DELAY).await;
                pane.update_in(cx, |pane, window, cx| {
                    pane.rested(&spot, &name, range.start, &layout, window, cx)
                })
                .ok();
            }
        });
        self.navigate.resting = Some(Resting {
            spot,
            start: range.start,
            _timer: timer,
        });
    }

    /// The wheel scrolled: the mouse's rest ends. The pointer has not moved but the text
    /// under it has, and a rest that went on would ask the hover of a name that is gone.
    pub(crate) fn scrolled(&mut self) {
        self.navigate.resting = None;
    }

    /// The mouse rested on a name long enough: asks its hover, if the pointer is still on
    /// the name and the line still holds it where it was.
    fn rested(
        &mut self,
        spot: &Spot,
        name: &str,
        start: usize,
        layout: &TextLayout,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate.resting = None;
        self.follow_cursor();
        let Some((cursor, text)) = self.spot_line(spot) else {
            return;
        };
        let range = start..start + name.len();
        if text.get(range.clone()) != Some(name)
            || name_under(&text, layout, window.mouse_position()) != names::name_at(&text, start)
        {
            return;
        }
        match self.place_for(&cursor, &text, range, cx) {
            Ok(place) => self.ask_hover(place, cx),
            Err(why) => self.say(why, cx),
        }
    }

    /// The hover, drawn under the rows; none while there is none.
    pub(crate) fn render_hover(&self, cx: &Context<Self>) -> Option<Div> {
        let hovering = self.navigate.hover.as_ref()?;
        let shown = &hovering.shown;
        let theme = cx.theme();
        let words = |selector: String, words: String| {
            div().debug_selector(move || selector.clone()).child(words)
        };
        let frame = div()
            .debug_selector(|| "review-hover".to_owned())
            .flex_none()
            .max_h(px(160.))
            .overflow_hidden()
            .px_2()
            .py_1()
            .flex()
            .flex_col()
            .gap_1()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .text_color(theme.secondary_foreground);
        if shown.waiting {
            return Some(
                frame.child(
                    words(
                        "review-hover-waiting".to_owned(),
                        format!("Asking about {}", shown.place.name),
                    )
                    .text_color(theme.muted_foreground),
                ),
            );
        }
        let mark = shown.guessed.then(|| {
            div()
                .debug_selector(|| "review-hover-guessed".to_owned())
                .text_color(theme.warning)
                .child(words(
                    format!("review-hover-guessed-words:{GUESSED}"),
                    GUESSED.to_owned(),
                ))
        });
        let text = match &shown.text {
            Some(text) => words(format!("review-hover-text:{text}"), text.clone()),
            None => words(
                "review-hover-nothing".to_owned(),
                format!("Nothing to show for {}", shown.place.name),
            )
            .text_color(theme.muted_foreground),
        };
        Some(frame.children(mark).child(text))
    }

    /// The list of references, drawn under the rows; none while there is none.
    pub(crate) fn render_references(&self, cx: &Context<Self>) -> Option<Div> {
        let listing = self.navigate.references.as_ref()?;
        let shown = &listing.shown;
        let theme = cx.theme();
        let title = if shown.definitions {
            format!("Definitions of {}", shown.place.name)
        } else {
            format!("References to {}", shown.place.name)
        };
        let frame = div()
            .debug_selector(|| "review-references".to_owned())
            .flex_none()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .text_color(theme.secondary_foreground)
            .key_context(REFERENCES_CONTEXT)
            .on_action(cx.listener(Self::reference_down))
            .on_action(cx.listener(Self::reference_up))
            .on_action(cx.listener(Self::open_reference))
            .on_action(cx.listener(Self::close_references));
        let frame = match &self.navigate.list_focus {
            Some(focus) => frame.track_focus(focus),
            None => frame,
        };
        let line = || {
            div()
                .flex_none()
                .h(ROW_HEIGHT)
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .overflow_hidden()
                .whitespace_nowrap()
        };
        let head = line().text_color(theme.muted_foreground).child(
            div()
                .debug_selector({
                    let title = title.clone();
                    move || format!("review-references-title:{title}")
                })
                .child(title),
        );
        if shown.waiting {
            return Some(
                frame.child(head).child(
                    line().child(
                        div()
                            .debug_selector(|| "review-references-waiting".to_owned())
                            .text_color(theme.muted_foreground)
                            .child("Looking"),
                    ),
                ),
            );
        }
        let mark = shown.guessed.then(|| {
            line()
                .debug_selector(|| "review-references-guessed".to_owned())
                .text_color(theme.warning)
                .child(
                    div()
                        .debug_selector(|| format!("review-references-guessed-words:{GUESSED}"))
                        .child(GUESSED),
                )
        });
        let folder = self.worktree_folder(&shown.place.workspace, cx);
        let first = shown.chosen.saturating_sub(ENTRIES_IN_VIEW - 1);
        let entries = shown
            .locations
            .iter()
            .enumerate()
            .skip(first)
            .take(ENTRIES_IN_VIEW)
            .map(|(index, location)| {
                let path = match &folder {
                    Some(folder) => location.path.strip_prefix(folder).unwrap_or(&location.path),
                    None => &location.path,
                };
                let words = format!(
                    "{}:{} {}",
                    path.display(),
                    location.line,
                    location.text.trim()
                );
                let chosen = index == shown.chosen;
                line()
                    .debug_selector(move || format!("review-reference-{index}"))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, _: &MouseDownEvent, window, cx| {
                            pane.acted(cx);
                            pane.open_entry(index, window, cx);
                        }),
                    )
                    .child(if chosen {
                        div()
                            .debug_selector(|| "review-reference-chosen".to_owned())
                            .flex_none()
                            .w(px(3.))
                            .h_full()
                            .bg(theme.primary)
                    } else {
                        div().flex_none().w(px(3.)).h_full()
                    })
                    .child(
                        div()
                            .debug_selector({
                                let words = words.clone();
                                move || format!("review-reference-words-{index}:{words}")
                            })
                            .child(words),
                    )
            })
            .collect::<Vec<_>>();
        Some(frame.child(head).children(mark).children(entries))
    }

    /// The folder of the worktree of `workspace`, as the list of workspaces holds it.
    fn worktree_folder(&self, workspace: &WorkspaceId, cx: &App) -> Option<PathBuf> {
        self.models
            .workspaces
            .read(cx)
            .workspace(workspace)
            .map(|workspace| workspace.path.clone())
    }

    /// Tells `Navigation::old_side_shown` the old side of the change shown, when it is not
    /// what was told last. Called at every rebuild of the rows, and when the Navigator is
    /// connected or ends, which the pane observes from the first call on.
    pub(crate) fn tell_old_side(&mut self, cx: &mut Context<Self>) {
        if !self.navigate.observing {
            self.navigate.observing = true;
            cx.observe(&self.models.navigation, |pane, _, cx| {
                pane.tell_old_side(cx)
            })
            .detach();
        }
        if !self.models.navigation.read(cx).is_connected() {
            // A Navigator that comes next has been told nothing.
            self.navigate.told_old_side = None;
            return;
        }
        let (Some(workspace), Some(_)) = (self.listed_workspace(), &self.listed) else {
            return;
        };
        let version = self.old_version();
        let now = (workspace, version);
        if self.navigate.told_old_side.as_ref() == Some(&now) {
            return;
        }
        self.navigate.told_old_side = Some(now.clone());
        let (workspace, version) = now;
        self.models.navigation.update(cx, |navigation, _| {
            navigation.old_side_shown(&workspace, version)
        });
    }
}

/// The index, among the names of `text`, of the one under `position`, a point of the
/// window, by the layout the text was drawn with; none where the pointer is on none. The
/// layout gives the character the pointer is over, so a name holds the pointer only on its
/// own characters.
fn name_under(text: &str, layout: &TextLayout, position: Point<Pixels>) -> Option<usize> {
    let at = layout.index_for_position(position).ok()?;
    names::name_at(text, at)
}
