//! Review pane (`W-REVIEW`).
//!
//! Owns: Flow and Changes, unified and split diff, threads and comment boxes, file views, follow.
//!
//! Surface:
//! - [`entry`], the pane as the Window program registers it: the one line that program
//!   needs. It holds [`pane`], which builds the pane for the Frame, and [`controls`];
//! - [`ReviewPane`], the view, and the model of what it shows: its [`PaneRow`]s, its
//!   [`Layout`], and the [`Position`] of the line the reviewer is on;
//! - [`controls`], the actions of the pane with their keys: [`LineDown`], [`LineUp`],
//!   [`ToggleLayout`].
//!
//! The pane shows the changes of the workspace on screen: it observes
//! `yaar_frame::WorkspaceChanges`, asks through it for the two versions of each changed
//! file, and diffs them with `yaar-diff` off the thread that draws. It reads and shows;
//! nothing in it changes a file, and it offers no way to.
//!
//! It depends on the Frame and on no other pane.

mod draw;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::*;
use yaar_diff::{CONTEXT_LINES, Emphasis, FileDiff, Row, Side};
use yaar_frame::{Asks, BuildPane, Control, Models, Pane, PaneEntry};
use yaar_link::{ChangedFile, Changes, Comparison, Content, FileSides, FileStatus};

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
    ]
);

/// The key context of the pane: its keys work while it has the focus.
pub const KEY_CONTEXT: &str = "ReviewPane";

/// The layout a diff is shown in until the reviewer switches it.
pub const FIRST_LAYOUT: Layout = Layout::Unified;

/// The name of the pane in the Frame.
pub const NAME: &str = "review";

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

/// The actions of the pane with their keys, all in [`KEY_CONTEXT`]: `j` and `down` for
/// [`LineDown`], `k` and `up` for [`LineUp`], `s` for [`ToggleLayout`]. None of them
/// sends a message a user starts.
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
            context: Some(KEY_CONTEXT),
            sends: None,
            asks: Asks::Nothing,
        }
    }
    vec![
        control("Line Down", || Box::new(LineDown), "j"),
        control("Line Down", || Box::new(LineDown), "down"),
        control("Line Up", || Box::new(LineUp), "k"),
        control("Line Up", || Box::new(LineUp), "up"),
        control("Unified or Split Diff", || Box::new(ToggleLayout), "s"),
    ]
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
}

/// A changed file as the pane holds it.
struct ShownFile {
    file: ChangedFile,
    content: FileContent,
    /// Where its heading and the rows under it lie in the rows of the pane.
    rows: Range<usize>,
}

/// What the pane holds of a changed file.
enum FileContent {
    /// Nothing it could show lines of.
    Unshown(Unshown),
    /// The difference between its two versions.
    Diff(Box<ShownDiff>),
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
/// The reviewer is always on one line once a diff is shown: at first the first changed
/// line of the first file. [`LineDown`] and [`LineUp`] move to the next and the previous
/// line of the diff, across files, and a click on a line moves there. The pane scrolls so
/// that the line stays in view; the wheel scrolls without moving the reviewer.
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
/// `review-row-<index>-old` and `review-row-<index>-new` as well. The control that
/// offers to set a base carries `review-offer-base`.
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
    /// The questions for the files of `changes` that are still on their way. New
    /// changes take their place, which drops the answers to the old ones.
    reading: Vec<Task<()>>,
    scroll: UniformListScrollHandle,
    /// The rows the list last laid out, as the list itself told it.
    in_view: Rc<RefCell<Range<usize>>>,
}

impl ReviewPane {
    /// A pane that shows the changes of the workspace on screen of `models`, in
    /// [`FIRST_LAYOUT`].
    pub fn new(models: &Models, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&models.changes, |pane, _, cx| pane.follow(cx))
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
            scroll: UniformListScrollHandle::new(),
            in_view: Rc::new(RefCell::new(0..0)),
        };
        pane.follow(cx);
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

    /// The line the reviewer is on; none while no diff is shown.
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

    /// Takes up what the shared model holds, each time the model says it changed.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let model = self.models.changes.read(cx);
        let Some(changes) = model.changes() else {
            // No workspace is on screen, or another one came and has no answer yet.
            self.changes = None;
            self.files.clear();
            self.by_path.clear();
            self.reading.clear();
            self.position = None;
            self.to_top();
            self.rebuild(cx);
            return;
        };
        // The model keeps the last answer while it asks again, and after a refusal:
        // either way nothing new came, and what is shown stays.
        let nothing_new = model.is_loading() || model.refusal().is_some();
        if nothing_new && self.changes.as_ref() == Some(changes) {
            cx.notify();
            return;
        }
        let changes = changes.clone();
        self.show(changes, cx);
    }

    /// Shows an answer of the Keeper, and asks for the two versions of each file of it.
    ///
    /// An answer for the workspace that is shown already is the same changes read
    /// again, as when the window comes back to the front: each file keeps what is shown
    /// of it until its new versions are here, so the diff does not blink.
    fn show(&mut self, changes: Changes, cx: &mut Context<Self>) {
        let same_workspace = self
            .changes
            .as_ref()
            .is_some_and(|shown| shown.workspace == changes.workspace);
        let mut held: HashMap<PathBuf, FileContent> = HashMap::new();
        if same_workspace {
            held.extend(
                self.files
                    .drain(..)
                    .map(|shown| (shown.file.path, shown.content)),
            );
        } else {
            self.position = None;
            self.to_top();
        }
        self.files = changes
            .files
            .iter()
            .map(|file| ShownFile {
                content: held
                    .remove(&file.path)
                    .unwrap_or(FileContent::Unshown(Unshown::Loading)),
                file: file.clone(),
                rows: 0..0,
            })
            .collect();
        self.by_path = changes
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| (file.path.clone(), index))
            .collect();
        // In the order of the list, so the files arrive top to bottom.
        self.reading = changes
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| self.read(index, file, cx))
            .collect();
        self.changes = Some(changes);
        self.rebuild(cx);
    }

    /// Asks for the two versions of the file at `index` and diffs them.
    fn read(&self, index: usize, file: &ChangedFile, cx: &mut Context<Self>) -> Task<()> {
        let asked = self
            .models
            .changes
            .update(cx, |changes, cx| changes.file(file, cx));
        cx.spawn(async move |pane, cx| {
            let content = match asked.await.map(versions) {
                Ok(Ok((old, new))) => {
                    // The diff of a large file takes long enough to be seen: it is made
                    // off the thread that draws.
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
            pane.update(cx, |pane, cx| pane.loaded(index, content, cx))
                .ok();
        })
    }

    /// Puts what came for the file at `index` in the place of what was shown of it.
    fn loaded(&mut self, index: usize, content: FileContent, cx: &mut Context<Self>) {
        let file = &mut self.files[index];
        file.content = content;
        let rows = file_rows(file, self.layout);
        let was = file.rows.clone();
        file.rows = was.start..was.start + rows.len();
        let now = file.rows.end;
        drop(self.rows.splice(was.clone(), rows));
        for below in &mut self.files[index + 1..] {
            below.rows = below.rows.start - was.end + now..below.rows.end - was.end + now;
        }
        // What the reviewer reads stays where it is on screen when a file above it
        // arrives with more rows, or with fewer, than it had.
        if was.end <= self.in_view.borrow().start {
            self.shift_view(now as f32 - was.end as f32);
        }
        self.settle(cx);
    }

    /// Makes the rows again from what the pane holds.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let mut rows = Vec::new();
        if let Some(changes) = &self.changes {
            rows.push(PaneRow::Comparison {
                comparison: changes.comparison.clone(),
                offers_base: matches!(changes.comparison, Comparison::NothingToCompare { .. }),
            });
            rows.extend(changes.commits.iter().map(|commit| PaneRow::Commit {
                id: commit.id.clone(),
                summary: commit.summary.clone(),
            }));
            for file in &mut self.files {
                let start = rows.len();
                rows.extend(file_rows(file, self.layout));
                file.rows = start..rows.len();
            }
        }
        self.rows = rows;
        self.settle(cx);
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

    pub(crate) fn line_down(&mut self, _: &LineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    pub(crate) fn line_up(&mut self, _: &LineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    pub(crate) fn toggle_layout(
        &mut self,
        _: &ToggleLayout,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let PaneRow::Diff { path, row } = &self.rows[index] else {
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

/// The heading of a file and the rows under it.
fn file_rows(file: &ShownFile, layout: Layout) -> Vec<PaneRow> {
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
    rows
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
    let mut stops = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        // The rows of one change in the split view; one row in every other case.
        let run = rows[index..]
            .iter()
            .take_while(|row| changed(row))
            .count()
            .max(1);
        let run = index..index + run;
        for side in [Side::Old, Side::New] {
            for index in run.clone() {
                if stops_of(&rows[index]).any(|(stop, _)| stop == side) {
                    stops.push((index, side));
                }
            }
        }
        index = run.end;
    }
    stops
}
