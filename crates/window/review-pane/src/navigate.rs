//! Hover, definition and references of the name the reviewer is on, asked in its line's
//! version: `Tree::Version` on the old side, `Tree::Worktree` on a new side that is the files
//! on disk, else `Tree::Past` with the whole new side. A new ask cancels the last of its kind.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use yaar_diff::Side;
use yaar_frame::NavError;
use yaar_link::WorkspaceId;
use yaar_navigator::{Answered, FileKind, Location, Place, Tree as NavTree};
use yaar_ui::metrics::{SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6};
use yaar_ui::pieces::{Banner, Button, ButtonKind, Chip, Divider, ListRow, Popover, Tone};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::names;
use crate::threads::path_in;
use crate::tree::Source;
use crate::{FileContent, FileShown, Position, REFERENCES_CONTEXT, ReviewPane, View};

pub const HOVER_DELAY: Duration = Duration::from_millis(400);

pub const GUESSED: &str = "Found by text match while no language server is ready: it may be wrong";

const ENTRIES_IN_VIEW: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverShown {
    pub place: Place,
    /// None while the ask is on its way and where the Navigator reported nothing.
    pub text: Option<String>,
    pub guessed: bool,
    pub waiting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencesShown {
    pub place: Place,
    pub locations: Vec<Location>,
    pub guessed: bool,
    pub waiting: bool,
    pub chosen: usize,
    pub definitions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cursor {
    Diff(Position),
    /// A line of the file view, counted from 1.
    File {
        path: PathBuf,
        line: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Spot {
    Diff {
        index: usize,
        side: Side,
    },
    /// A line of the file view, counted from 1.
    File(u32),
}

struct Resting {
    spot: Spot,
    start: usize,
    _timer: Task<()>,
}

/// A dropped ask that is still on its way tells the Navigator it is cancelled.
struct Hovering {
    shown: HoverShown,
    anchor: Option<Bounds<Pixels>>,
    _asking: Task<()>,
}

struct Listing {
    shown: ReferencesShown,
    _asking: Option<Task<()>>,
}

pub(crate) struct Left {
    pub(crate) view: View,
    pub(crate) name: Option<(Cursor, usize)>,
}

#[derive(Default)]
pub(crate) struct Navigate {
    /// Index among the names of the line; holds for that line only.
    pub(crate) name: Option<(Cursor, usize)>,
    synced: Option<(Cursor, usize)>,
    hover: Option<Hovering>,
    references: Option<Listing>,
    definition: Option<Task<()>>,
    pub(crate) opening: Option<Task<()>>,
    resting: Option<Resting>,
    list_focus: Option<FocusHandle>,
    told_old_side: Option<(WorkspaceId, Option<String>)>,
    observing: bool,
    pub(crate) file: Option<FileShown>,
    pub(crate) file_lines: Vec<Range<usize>>,
    pub(crate) file_scroll: UniformListScrollHandle,
    pub(crate) left: Option<Left>,
}

struct Current {
    cursor: Cursor,
    text: String,
    names: Vec<Range<usize>>,
    index: usize,
}

impl Current {
    fn range(&self) -> Option<Range<usize>> {
        self.names.get(self.index).cloned()
    }
}

impl ReviewPane {
    pub fn name_at(&self) -> Option<String> {
        let current = self.current()?;
        Some(current.text[current.range()?].to_owned())
    }

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

    pub(crate) fn drop_hover(&mut self) -> bool {
        self.navigate.hover.take().is_some()
    }

    pub(crate) fn name_range(&self) -> Option<Range<usize>> {
        self.current()?.range()
    }

    pub fn hover(&self) -> Option<&HoverShown> {
        self.navigate.hover.as_ref().map(|hovering| &hovering.shown)
    }

    pub fn references(&self) -> Option<&ReferencesShown> {
        self.navigate
            .references
            .as_ref()
            .map(|listing| &listing.shown)
    }

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

    fn line_text(&self, cursor: &Cursor) -> Option<String> {
        match cursor {
            Cursor::Diff(position) => self.text(position),
            Cursor::File { line, .. } => self.file_line(*line).map(str::to_owned),
        }
    }

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

    /// Every frame calls it: that is where a move made by the actions of the lines is seen.
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
                    // The Navigator counts lines from 1, `Position` from 0.
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
                    // Asked by its absolute path, in the files on disk of the workspace on screen.
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

    fn listed_workspace(&self) -> Option<WorkspaceId> {
        match &self.listed {
            Some(Source::Versions { .. }) => self.tree.of.clone(),
            _ => self
                .changes
                .as_ref()
                .map(|changes| changes.workspace.clone()),
        }
    }

    pub(crate) fn old_version(&self) -> Option<String> {
        match &self.listed {
            Some(Source::Versions { old, .. }) => old.clone(),
            _ => self
                .changes
                .as_ref()
                .and_then(|changes| changes.counted_from.clone()),
        }
    }

    /// Each line is followed by a line feed, but for a last line the file ends without one.
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

    fn names_here(&mut self, cx: &mut Context<Self>) -> bool {
        if self.navigate.file.is_none() && self.view == View::Flow {
            self.say("Names are asked on the lines of Changes", cx);
            return false;
        }
        true
    }

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

    pub(crate) fn next_name(
        &mut self,
        _: &crate::NextName,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_name(1, cx);
    }

    pub(crate) fn previous_name(
        &mut self,
        _: &crate::PreviousName,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_name(-1, cx);
    }

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

    pub(crate) fn hover_name(
        &mut self,
        _: &crate::Hover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        if let Some(place) = self.place_to_ask(cx) {
            let anchor = self.cursor_hover_anchor(window);
            self.ask_hover(place, anchor, cx);
        }
    }

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

    fn ask_hover(&mut self, place: Place, anchor: Option<Bounds<Pixels>>, cx: &mut Context<Self>) {
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
            anchor,
            _asking: task,
        });
        cx.notify();
    }

    fn cursor_hover_anchor(&self, window: &Window) -> Option<Bounds<Pixels>> {
        if let Some(row) = self.position_row
            && let Some(bounds) = self.list.bounds_for_item(row)
        {
            return Some(bounds);
        }
        let pos = window.mouse_position();
        Some(Bounds::new(point(pos.x, pos.y), size(px(8.), px(20.))))
    }

    fn spot_hover_anchor(&self, spot: &Spot, window: &Window) -> Option<Bounds<Pixels>> {
        match spot {
            Spot::Diff { index, .. } => {
                if let Some(bounds) = self.list.bounds_for_item(*index) {
                    let x = window
                        .mouse_position()
                        .x
                        .clamp(bounds.left(), bounds.right());
                    return Some(Bounds::new(
                        point(x, bounds.top()),
                        size(px(8.), bounds.size.height),
                    ));
                }
            }
            Spot::File(_) => {}
        }
        let pos = window.mouse_position();
        Some(Bounds::new(
            point(pos.x, pos.y - px(10.)),
            size(px(8.), px(20.)),
        ))
    }

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

    /// One place sure of itself opens in the file view; several, or one found by text match, are
    /// listed.
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

    fn take_focus_for_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .navigate
            .list_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        focus.focus(window, cx);
    }

    pub(crate) fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigate.references.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn reference_down(
        &mut self,
        _: &crate::ReferenceDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.choose(1, cx);
    }

    pub(crate) fn reference_up(
        &mut self,
        _: &crate::ReferenceUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.choose(-1, cx);
    }

    fn choose(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(listing) = &mut self.navigate.references else {
            return;
        };
        let shown = &mut listing.shown;
        let last = shown.locations.len().saturating_sub(1);
        shown.chosen = shown.chosen.saturating_add_signed(by).min(last);
        cx.notify();
    }

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

    pub(crate) fn close_references(
        &mut self,
        _: &crate::CloseReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.close_list(window, cx);
    }

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

    /// The wait is the executor's, so a test moves its clock.
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

    /// The text under the pointer moved: a rest that went on would ask the hover of a name that is
    /// gone.
    pub(crate) fn scrolled(&mut self) {
        self.navigate.resting = None;
    }

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
        let anchor = self.spot_hover_anchor(spot, window);
        match self.place_for(&cursor, &text, range, cx) {
            Ok(place) => self.ask_hover(place, anchor, cx),
            Err(why) => self.say(why, cx),
        }
    }

    pub(crate) fn render_hover(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let hovering = self.navigate.hover.as_ref()?;
        let shown = &hovering.shown;
        let palette = palette(cx);
        let anchor = hovering.anchor.unwrap_or_else(|| {
            let vp = window.viewport_size();
            Bounds::new(point(px(24.), vp.height * 0.25), size(px(8.), px(20.)))
        });
        let words = |selector: String, words: String| {
            div().debug_selector(move || selector.clone()).child(words)
        };
        let mut popover = Popover::new(anchor).pass_through().selector("review-hover");
        if shown.waiting {
            popover = popover.child(
                div()
                    .px(SPACE_5)
                    .py(SPACE_4)
                    .typed(TypeRole::Meta)
                    .text_color(palette.text_muted)
                    .child(words(
                        "review-hover-waiting".to_owned(),
                        format!("Asking about {}", shown.place.name),
                    )),
            );
            return Some(popover.into_any_element());
        }
        if shown.guessed {
            popover = popover.child(
                div()
                    .debug_selector(|| "review-hover-guessed".to_owned())
                    .px(SPACE_5)
                    .pt(SPACE_4)
                    .child(
                        Chip::new(GUESSED)
                            .tone(Tone::Warn)
                            .label_selector(format!("review-hover-guessed-words:{GUESSED}")),
                    ),
            );
        }
        let body = match &shown.text {
            Some(text) => div()
                .px(SPACE_5)
                .py(SPACE_4)
                .max_h(px(240.))
                .overflow_hidden()
                .typed(TypeRole::CodeSmall)
                .text_color(palette.text)
                .child(words(format!("review-hover-text:{text}"), text.clone())),
            None => div()
                .px(SPACE_5)
                .py(SPACE_4)
                .typed(TypeRole::Meta)
                .text_color(palette.text_muted)
                .child(words(
                    "review-hover-nothing".to_owned(),
                    format!("Nothing to show for {}", shown.place.name),
                )),
        };
        Some(popover.child(body).into_any_element())
    }

    pub(crate) fn render_references(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let listing = self.navigate.references.as_ref()?;
        let shown = &listing.shown;
        let palette = palette(cx);
        let title = if shown.definitions {
            format!("Definitions of {}", shown.place.name)
        } else {
            format!("References to {}", shown.place.name)
        };
        let max_height = (window.viewport_size().height * 0.34).max(px(140.));
        let frame = div()
            .debug_selector(|| "review-references".to_owned())
            .flex_none()
            .max_h(max_height)
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(palette.pane)
            .text_color(palette.text)
            .key_context(REFERENCES_CONTEXT)
            .on_action(cx.listener(Self::reference_down))
            .on_action(cx.listener(Self::reference_up))
            .on_action(cx.listener(Self::open_reference))
            .on_action(cx.listener(Self::close_references));
        let frame = match &self.navigate.list_focus {
            Some(focus) => frame.track_focus(focus),
            None => frame,
        };
        let head = div()
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_6)
            .py(SPACE_3)
            .border_b_1()
            .border_color(palette.separator)
            .child(
                div()
                    .debug_selector({
                        let title = title.clone();
                        move || format!("review-references-title:{title}")
                    })
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .typed(TypeRole::MetaStrong)
                    .text_color(palette.text_muted)
                    .child(title),
            )
            .child(
                Button::new("review-references-close", "×")
                    .kind(ButtonKind::Ghost)
                    .small()
                    .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                        pane.close_references(&crate::CloseReferences, window, cx);
                    })),
            );
        if shown.waiting {
            return Some(
                frame
                    .child(Divider::horizontal().strong())
                    .child(head)
                    .child(
                        div()
                            .px(SPACE_6)
                            .py(SPACE_4)
                            .typed(TypeRole::Meta)
                            .text_color(palette.text_muted)
                            .debug_selector(|| "review-references-waiting".to_owned())
                            .child("Looking"),
                    )
                    .into_any_element(),
            );
        }
        let mark = shown.guessed.then(|| {
            Banner::custom()
                .tone(Tone::Warn)
                .selector("review-references-guessed")
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
                ListRow::new(("review-reference", index))
                    .compact()
                    .selected(chosen)
                    .focused(chosen)
                    .selector(format!("review-reference-{index}"))
                    .on_click(cx.listener(move |pane, _: &ClickEvent, window, cx| {
                        pane.acted(cx);
                        pane.open_entry(index, window, cx);
                    }))
                    .child(if chosen {
                        div()
                            .debug_selector(|| "review-reference-chosen".to_owned())
                            .flex_none()
                            .w(px(3.))
                            .h(px(14.))
                            .bg(palette.focus)
                    } else {
                        div().flex_none().w(px(3.)).h(px(14.))
                    })
                    .child(
                        div()
                            .debug_selector({
                                let words = words.clone();
                                move || format!("review-reference-words-{index}:{words}")
                            })
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .typed(TypeRole::CodeSmall)
                            .text_color(if chosen {
                                palette.text
                            } else {
                                palette.text_muted
                            })
                            .child(words),
                    )
            })
            .collect::<Vec<_>>();
        Some(
            frame
                .child(Divider::horizontal().strong())
                .child(head)
                .children(mark)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(SPACE_2)
                        .px(SPACE_3)
                        .py(SPACE_2)
                        .children(entries),
                )
                .into_any_element(),
        )
    }

    fn worktree_folder(&self, workspace: &WorkspaceId, cx: &App) -> Option<PathBuf> {
        self.models
            .workspaces
            .read(cx)
            .workspace(workspace)
            .map(|workspace| workspace.path.clone())
    }

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

/// The layout gives the character the pointer is over, so a name holds the pointer only on its own
/// characters.
fn name_under(text: &str, layout: &TextLayout, position: Point<Pixels>) -> Option<usize> {
    let at = layout.index_for_position(position).ok()?;
    names::name_at(text, at)
}
