//! The live diff of Flow: the change of one turn of the workspace on screen, as it grows.
//!
//! Built by the review pane's second package of slice 5 (row RP-04). Comments written on
//! its lines are `flow_comment.rs`'s.
//!
//! Which turn. The turn the live diff shows is the newest turn of the timeline that has not
//! ended, so that the end of an earlier turn, told after a later one started, moves
//! nothing; where every turn has ended, the newest of them, which is the one the timeline
//! was filled with from the history. The version it
//! counts from is that turn's start as the timeline's `GetTurn` answer gave it
//! (`Timeline::start_of`): until that answer is taken the live diff asks nothing; where it
//! gave none, the commit checked out (`crate::tree::checked_out` of the history, once the
//! history is here, `ReviewPane::live_history`). When a later answer gives the turn
//! another start (a turn that started while the state of the turn before was being
//! recorded), the live diff starts over from it. The state a turn that ended is read to
//! is the one the fact of its end gave, or the history's entry for it.
//!
//! The threads of the review on screen are placed under the lines they sit on, as Changes
//! places them; the threads whose line no row of the live diff shows are not listed
//! there: they are Changes' to list.
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
//! for the list once, then for the files; the files a list brought while it was hidden are
//! read then, not before. [`ReviewPane::live_look`] is where it looks: the
//! timeline calls it after each change the live diff reads (a turn added or read, the
//! view, the form, a workspace on screen), and the live diff's own hooks below end with it.
//!
//! What makes it grow. While the turn runs, the list is asked again at each `Fact::Step`
//! of that turn that is an edit or a file written whole, and at each `Fact::ChangesChanged`
//! of the workspace on screen, which is how a file a shell command wrote reaches it; a read
//! step asks nothing, and neither does a `ChangesChanged` once the turn's change runs to a
//! recorded state, which no write changes. At the `Fact::TurnEnded` of that turn with a
//! state, the list is asked again to that state. One question for the list is on its way at
//! a time; a reason to ask again that comes meanwhile asks once more when the answer is here.
//! When the list is here, a file new to it is read, and so is every file a step named since
//! the last list, and every file of the list when a `ChangesChanged` came since; a file keeps
//! what is shown of it until its new versions are here, so that nothing blinks.
//!
//! The files read. After the rows of the files the turn changed, and after every other
//! row, the live diff lists each file the turn read and did not change, one row each,
//! [`crate::PaneRow::Read`]: its path, marked as read, and nothing below it (no diff, no
//! line the reviewer can be on, no comment). They are the paths of the turn's read steps
//! as the timeline holds them (`Timeline::entries`, its `TimelineEntry::Step`s of the
//! turn shown whose step is `Step::Read`), each once, in the order of its first read,
//! less every path the live diff's list holds, as its path or as the path a renamed file
//! had in the version counted from. So a file leaves the group, and shows as a changed
//! file, once a list holds it that the turn changed it; before that list is here it stays
//! a file read. The path is the step's own (a file outside the worktree keeps its
//! absolute path). The Link's read step names no lines, so no lines read are drawn. The
//! group is made again from the timeline whenever the rows are made, so a read the
//! timeline filled in from a `GetTurn` answer finds its place; a read step heard as a
//! fact makes the rows again in `ReviewPane::live_told`, asking nothing, before follow
//! hears the step, so that follow finds the row.
//!
//! Following. When the rows are made again while follow is `On` and Flow is at a step
//! (`ReviewPane::at`) whose file the live diff now holds, the list is scrolled so that the
//! heading of that file (or its row among the files read) is in view: a step that came
//! before its file was in the list is followed once it is. While follow is paused or off
//! the scroll stays where it is.
//!
//! Its rows are [`crate::PaneRow`]s, as Changes' are, in the pane's layout, with the
//! threads of the review on screen on their lines (`crate::threads`), made again whenever
//! Changes' are (`ReviewPane::rebuild` calls [`ReviewPane::live_rebuild`]). The reviewer is
//! on one line of it, moved by `j`, `k` and a click.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::*;
use yaar_diff::{FileDiff, Row, Side};
use yaar_frame::AskError;
use yaar_link::{
    ChangedFile, Fact, FileStatus, FlowForm, HistoryEntry, Reply, Request, Step, TurnId,
};

use crate::tree::checked_out;
use crate::{
    FILES_ON_THEIR_WAY, FileContent, FlowAt, Following, LiveChange, PaneRow, Position, ReviewPane,
    ShownDiff, ShownFile, TimelineEntry, Unshown, View, file_rows, stops, stops_of, versions,
    whose,
};

/// What the live diff keeps.
pub(crate) struct Live {
    /// The change shown; none while no turn of the workspace on screen is known.
    change: Option<LiveChange>,
    /// The files of the last list, in its order, with what is shown of each.
    files: Vec<ShownFile>,
    /// The rows: the files, then the files read.
    rows: Vec<PaneRow>,
    /// The files read among `rows`, in order, as they were when the rows were made.
    group: Vec<PathBuf>,
    /// The line the reviewer is on, and the row that shows it.
    position: Option<Position>,
    position_row: Option<usize>,
    /// The step the list was last scrolled to for follow.
    scrolled_to: Option<FlowAt>,
    /// Whether the live diff was on screen when it last looked.
    on_screen: bool,
    /// Counts the changes it starts over for; an answer of an earlier one is let go.
    epoch: u64,
    /// Whether the list is out of date.
    stale: bool,
    /// Whether every file is read again with the next list, as after a change the watcher
    /// told of or a turn's end.
    everything: bool,
    /// The files steps named since the list was last asked for.
    named: Vec<PathBuf>,
    /// Whether the question for the list is on its way: one at a time.
    listing: bool,
    /// The files waiting to be read, and how many reads are on their way.
    queue: VecDeque<PathBuf>,
    in_flight: usize,
    /// The number of the last read asked for each file: of two answers, the later question's.
    asked: HashMap<PathBuf, u64>,
    sequence: u64,
    /// How the turn shown ended, as the fact of its end said it.
    ended: Option<(TurnId, Option<String>)>,
    /// Where the list is scrolled, and the rows it last laid out.
    pub(crate) scroll: UniformListScrollHandle,
    pub(crate) in_view: Rc<RefCell<Range<usize>>>,
}

impl Live {
    /// A live diff that shows nothing yet.
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        let _ = cx;
        Self {
            change: None,
            files: Vec::new(),
            rows: Vec::new(),
            group: Vec::new(),
            position: None,
            position_row: None,
            scrolled_to: None,
            on_screen: false,
            epoch: 0,
            stale: true,
            everything: false,
            named: Vec::new(),
            listing: false,
            queue: VecDeque::new(),
            in_flight: 0,
            asked: HashMap::new(),
            sequence: 0,
            ended: None,
            scroll: UniformListScrollHandle::new(),
            in_view: Rc::new(RefCell::new(0..0)),
        }
    }

    /// The change shown.
    pub(crate) fn change(&self) -> Option<&LiveChange> {
        self.change.as_ref()
    }

    /// Its rows.
    pub(crate) fn rows(&self) -> &[PaneRow] {
        &self.rows
    }

    /// The line the reviewer is on.
    pub(crate) fn position(&self) -> Option<&Position> {
        self.position.as_ref()
    }

    /// The index of the row that shows the line the reviewer is on.
    pub(crate) fn position_row(&self) -> Option<usize> {
        self.position_row
    }

    /// The text of a line of a file it holds.
    pub(crate) fn text(&self, position: &Position) -> Option<String> {
        let shown = self.diff_of(&position.path)?;
        (position.line < shown.diff.line_count(position.side))
            .then(|| shown.text(position.side, position.line).into_owned())
    }

    /// The file of the list at `path`, as the last `CompareVersions` answer named it: its
    /// status, and for a renamed file the path it had in the version counted from. What a
    /// comment's anchor is built from.
    pub(crate) fn file(&self, path: &Path) -> Option<&ChangedFile> {
        self.files
            .iter()
            .find(|held| held.file.path == path)
            .map(|held| &held.file)
    }

    /// The rows in view, as last laid out.
    pub(crate) fn in_view(&self) -> Range<usize> {
        self.in_view.borrow().clone()
    }

    /// The diff of the file at `path`, when it holds one.
    pub(crate) fn diff_of(&self, path: &Path) -> Option<&ShownDiff> {
        match &self
            .files
            .iter()
            .find(|held| held.file.path == path)?
            .content
        {
            FileContent::Diff(shown) => Some(shown),
            FileContent::Unshown(_) => None,
        }
    }

    /// The index of the heading of the file at `path`, or of the file a renamed file of
    /// the list was called `path`.
    fn heading_of(&self, path: &Path) -> Option<usize> {
        self.files
            .iter()
            .find(|held| {
                held.file.path == path
                    || matches!(&held.file.status, FileStatus::Renamed { from } if from == path)
            })
            .map(|held| held.rows.start)
    }

    /// Where the reviewer is, among the rows: the index of the row of that line.
    fn locate(&self, position: &Position) -> Option<usize> {
        let held = self
            .files
            .iter()
            .find(|held| held.file.path == position.path)?;
        let start = held.rows.start;
        self.rows[held.rows.clone()]
            .iter()
            .position(|row| {
                stops_of(row).any(|(side, line)| (side, line) == (position.side, position.line))
            })
            .map(|index| start + index)
    }

    /// The first changed line of the first file that has one. None while a file before it
    /// is still on its way: the first file may yet be that one.
    fn first_change(&self) -> Option<Position> {
        for held in &self.files {
            match &held.content {
                FileContent::Unshown(Unshown::Loading) => return None,
                FileContent::Unshown(_) => {}
                FileContent::Diff(_) => {
                    let changed = self.rows[held.rows.clone()]
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
                            path: held.file.path.clone(),
                            side,
                            line,
                        });
                    }
                }
            }
        }
        None
    }

    /// Whether the list holds a file called `path`, now or before a rename.
    fn holds(&self, path: &Path) -> bool {
        self.heading_of(path).is_some()
    }

    /// Forgets everything shown and asked, for another change or none.
    fn start_over(&mut self, change: Option<LiveChange>) {
        self.epoch += 1;
        self.change = change;
        self.files.clear();
        self.rows.clear();
        self.group.clear();
        self.position = None;
        self.position_row = None;
        self.scrolled_to = None;
        self.stale = true;
        self.everything = false;
        self.named.clear();
        self.listing = false;
        self.queue.clear();
        self.in_flight = 0;
        self.asked.clear();
        *self.in_view.borrow_mut() = 0..0;
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(0.)));
    }
}

impl ReviewPane {
    /// The live diff looks at what it shows and whether it is on screen, as the module
    /// says, and asks for the list when it is on screen and what it holds is stale, of
    /// another turn, or counted from another version; when the timeline's steps of the
    /// turn shown changed, it makes its rows again for the files read, asking nothing for
    /// them. Called by the timeline after each change the live diff reads, and last by
    /// the live diff's own hooks.
    pub(crate) fn live_look(&mut self, cx: &mut Context<Self>) {
        let mut remake = false;
        match self.live_target(cx) {
            None => {
                if self.live.change.is_some() {
                    self.live.start_over(None);
                    remake = true;
                }
            }
            Some(target) => match &mut self.live.change {
                Some(held)
                    if (&held.workspace, &held.turn, &held.from)
                        == (&target.workspace, &target.turn, &target.from) =>
                {
                    // The turn ended, or its end is known now: the same files, read as
                    // they are in the state it ended with.
                    if held.to != target.to {
                        held.to = target.to;
                        self.live.stale = true;
                        self.live.everything = true;
                    }
                }
                _ => {
                    self.live.start_over(Some(target));
                    remake = true;
                }
            },
        }
        if remake || self.live_group() != self.live.group {
            self.live_make_rows(cx);
        }
        let on = self.view == View::Flow && self.form == FlowForm::LiveDiff;
        if on != self.live.on_screen {
            self.live.on_screen = on;
            self.live.scrolled_to = None;
        }
        if on {
            self.live_ask(cx);
            // What a list brought while the live diff was hidden waits for it here.
            self.live_pump(cx);
            self.live_follow_scroll();
        }
    }

    /// The workspace on screen may be another: the live diff is emptied when it is.
    pub(crate) fn live_workspace(&mut self, cx: &mut Context<Self>) {
        let now = self.models.changes.read(cx).workspace().cloned();
        if self.live.change.as_ref().map(|held| &held.workspace) != now.as_ref() {
            self.live.start_over(None);
            self.live.ended = None;
            self.live_make_rows(cx);
        }
        self.live_look(cx);
    }

    /// The agents model changed: a turn that started in the workspace on screen becomes
    /// the turn shown.
    pub(crate) fn live_agents(&mut self, cx: &mut Context<Self>) {
        self.live_look(cx);
    }

    /// The history was answered: what the change of a turn with no start counts from may
    /// be known now.
    pub(crate) fn live_history(&mut self, cx: &mut Context<Self>) {
        self.live_look(cx);
    }

    /// A fact of the Keeper, for the live diff: `Step`, `ChangesChanged` and `TurnEnded`
    /// of the workspace on screen. Called after the timeline heard it, and before follow
    /// does: a read step of the turn shown asks nothing, and its file, when the list does
    /// not hold it, is a row among the files read before this returns.
    pub(crate) fn live_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        let Some(shown) = self.live.change.clone() else {
            return;
        };
        match fact {
            Fact::Step {
                workspace,
                turn,
                step,
                ..
            } if *workspace == shown.workspace && *turn == shown.turn => {
                if let Step::Edit { path, .. } | Step::Write { path } = step {
                    self.live.named.push(path.clone());
                    self.live.stale = true;
                }
                self.live_look(cx);
            }
            // A change counted to a recorded state does not move with the files: only one
            // that is counted to the files on disk needs the fact.
            Fact::ChangesChanged { workspace }
                if *workspace == shown.workspace && shown.to.is_none() =>
            {
                self.live.stale = true;
                self.live.everything = true;
                self.live_look(cx);
            }
            Fact::TurnEnded {
                workspace,
                turn,
                state,
                ..
            } if *workspace == shown.workspace && *turn == shown.turn => {
                self.live.ended = Some((turn.clone(), state.clone()));
                self.live.stale = true;
                self.live.everything = true;
                self.live_look(cx);
            }
            _ => {}
        }
    }

    /// Makes the rows of the live diff again from what it holds, in the pane's layout and
    /// with the threads of the review on screen, the files read last, keeping the
    /// reviewer's line.
    pub(crate) fn live_rebuild(&mut self, cx: &mut Context<Self>) {
        self.live_make_rows(cx);
    }

    /// What [`crate::LineDown`] and [`crate::LineUp`] do in the live diff, as they do in
    /// Changes.
    pub(crate) fn live_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let (Some(position), Some(row)) = (&self.live.position, self.live.position_row) else {
            return;
        };
        let stops = stops(&self.live.rows);
        let Some(now) = stops.iter().position(|stop| *stop == (row, position.side)) else {
            return;
        };
        let Some(&(row, side)) = now.checked_add_signed(by).and_then(|next| stops.get(next)) else {
            return;
        };
        self.live_go_to(row, side, cx);
        self.live
            .scroll
            .scroll_to_item(row, ScrollStrategy::Nearest);
    }

    /// Puts the reviewer on a line of the row at `index`: what a click on it does.
    pub(crate) fn live_go_to(&mut self, index: usize, side: Side, cx: &mut Context<Self>) {
        // A click is told by the index its row had when it was drawn; the rows may have
        // changed since.
        let Some(PaneRow::Diff { path, row }) = self.live.rows.get(index) else {
            return;
        };
        let Some(line) = row.line(side) else {
            return;
        };
        self.live.position = Some(Position {
            path: path.clone(),
            side,
            line,
        });
        self.live.position_row = Some(index);
        cx.notify();
    }

    /// The index in the live diff's rows of the heading of the file the step `at` names,
    /// or, for a read of a file the turn did not change, of that file's row among the
    /// files read; none for a command, and for an edit or a write whose file the list
    /// does not hold yet.
    pub(crate) fn live_row_of(&self, at: &FlowAt) -> Option<usize> {
        let shown = self.live.change.as_ref()?;
        if at.turn != shown.turn {
            return None;
        }
        match self.flow.step(at)? {
            Step::Read { path } => self.live.heading_of(path).or_else(|| {
                self.live
                    .rows
                    .iter()
                    .position(|row| matches!(row, PaneRow::Read { path: held } if held == path))
            }),
            Step::Edit { path, .. } | Step::Write { path } => self.live.heading_of(path),
            Step::Command { .. } => None,
        }
    }

    /// Scrolls the live diff so that row `row` is in view, as follow wants it.
    pub(crate) fn scroll_live_to(&mut self, row: usize) {
        self.live
            .scroll
            .scroll_to_item(row, ScrollStrategy::Nearest);
        self.live.scrolled_to = self.at.clone();
    }

    /// What the live diff is the diff of, as things are now: the newest turn of the
    /// timeline that has not ended, or, when all have, the newest; counted from what the
    /// timeline's last answer for it gave, else the commit checked out; to the state it
    /// ended with, as the fact of its end or the history says. None while that is not
    /// known: no turn, no answer for it yet, or no history to count from.
    fn live_target(&self, cx: &App) -> Option<LiveChange> {
        let workspace = self.models.changes.read(cx).workspace().cloned()?;
        let mut turns: Vec<(&TurnId, bool)> = Vec::new();
        for entry in self.flow.entries() {
            match entry {
                TimelineEntry::Turn { turn, .. } => turns.push((turn, false)),
                TimelineEntry::End { turn, .. } => {
                    if let Some(held) = turns.iter_mut().find(|(held, _)| *held == turn) {
                        held.1 = true;
                    }
                }
                TimelineEntry::Step { .. } => {}
            }
        }
        let (turn, _) = turns
            .iter()
            .rev()
            .find(|(_, ended)| !ended)
            .or_else(|| turns.last())?;
        let history = self
            .tree
            .history
            .as_ref()
            .filter(|history| history.workspace == workspace);
        let from = match self.flow.start_of(turn)? {
            Some(start) => Some(start.to_owned()),
            None => checked_out(history?),
        };
        let to = match &self.live.ended {
            Some((ended, state)) if ended == *turn => state.clone(),
            _ => history.and_then(|history| {
                history.entries.iter().find_map(|entry| match entry {
                    HistoryEntry::Turn {
                        turn: held, state, ..
                    } if held == *turn => Some(state.clone()),
                    _ => None,
                })?
            }),
        };
        Some(LiveChange {
            workspace,
            turn: (*turn).clone(),
            from,
            to,
        })
    }

    /// The files the turn shown read and did not change, in the order of their first read.
    fn live_group(&self) -> Vec<PathBuf> {
        let Some(shown) = &self.live.change else {
            return Vec::new();
        };
        let mut group: Vec<PathBuf> = Vec::new();
        for entry in self.flow.entries() {
            if let TimelineEntry::Step {
                turn,
                step: Step::Read { path },
                ..
            } = entry
                && *turn == shown.turn
                && !group.contains(path)
                && !self.live.holds(path)
            {
                group.push(path.clone());
            }
        }
        group
    }

    /// Makes the rows from the files and the group, and puts the reviewer back on their
    /// line, or on the first changed one when that is gone.
    fn live_make_rows(&mut self, cx: &mut Context<Self>) {
        let review = whose::shown(self.reviews.as_ref(), self.chosen.as_ref(), &self.sessions);
        let mut rows = Vec::new();
        for held in &mut self.live.files {
            let start = rows.len();
            let (shown, placed) = file_rows(held, self.layout, review);
            rows.extend(shown);
            held.placed = placed;
            held.rows = start..rows.len();
        }
        self.live.rows = rows;
        // The group is counted with the list as it is now: a file it holds is no file read.
        let group = self.live_group();
        self.live.rows.extend(
            group
                .iter()
                .map(|path| PaneRow::Read { path: path.clone() }),
        );
        self.live.group = group;
        self.live.position_row = self
            .live
            .position
            .as_ref()
            .and_then(|at| self.live.locate(at));
        if self.live.position_row.is_none() {
            self.live.position = self.live.first_change();
            self.live.position_row = self
                .live
                .position
                .as_ref()
                .and_then(|at| self.live.locate(at));
        }
        if self.live.rows.is_empty() {
            *self.live.in_view.borrow_mut() = 0..0;
        }
        cx.notify();
    }

    /// The rows are made again for what came in, and follow, while it is on, finds its step
    /// in them.
    fn live_remade(&mut self, cx: &mut Context<Self>) {
        self.live_make_rows(cx);
        if self.live.on_screen {
            self.live_follow_scroll();
        }
    }

    /// While follow is on, scrolls to the row of the step Flow is at, once for each step,
    /// when its file comes into the list after the step did.
    fn live_follow_scroll(&mut self) {
        if self.following != Following::On {
            return;
        }
        let Some(at) = self.at.clone() else {
            return;
        };
        if self.live.scrolled_to.as_ref() == Some(&at) {
            return;
        }
        if let Some(row) = self.live_row_of(&at) {
            self.scroll_live_to(row);
        }
    }

    /// Asks for the list while the live diff is on screen and what it holds is stale: one
    /// question at a time.
    fn live_ask(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.live.change.clone() else {
            return;
        };
        let Some(from) = shown.from.clone() else {
            // The Keeper compares two versions: with none before the change there is
            // nothing to list its files from.
            return;
        };
        if self.live.listing || !self.live.stale {
            return;
        }
        self.live.listing = true;
        self.live.stale = false;
        let named = std::mem::take(&mut self.live.named);
        let everything = std::mem::take(&mut self.live.everything);
        let epoch = self.live.epoch;
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::CompareVersions {
                    workspace: shown.workspace,
                    from,
                    to: shown.to,
                },
                cx,
            )
        });
        cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| {
                pane.live_listed(epoch, named, everything, answer, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Takes the list: a file new to it, or changed in what the list says of it, a file a
    /// step named and, after a change the watcher told of, every file is read; the others
    /// keep what is shown.
    fn live_listed(
        &mut self,
        epoch: u64,
        named: Vec<PathBuf>,
        everything: bool,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.live.epoch {
            return;
        }
        self.live.listing = false;
        let Ok(Reply::Compared(files)) = answer else {
            // What is shown stays, and a fact that comes asks again for what this question
            // was asked for. One that came while it was on its way asks now; with none,
            // nothing is asked, as a Keeper that keeps refusing would be asked without end.
            let came_meanwhile = self.live.stale;
            self.live.named.extend(named);
            self.live.everything |= everything;
            self.live.stale = true;
            if came_meanwhile {
                self.live_look(cx);
            }
            return;
        };
        let mut before: HashMap<PathBuf, ShownFile> = self
            .live
            .files
            .drain(..)
            .map(|held| (held.file.path.clone(), held))
            .collect();
        let mut to_read = Vec::new();
        for file in files {
            let kept = before.remove(&file.path).filter(|held| held.file == file);
            if kept.is_none() || everything || named.contains(&file.path) {
                to_read.push(file.path.clone());
            }
            self.live.files.push(match kept {
                Some(held) => held,
                None => ShownFile {
                    file,
                    content: FileContent::Unshown(Unshown::Loading),
                    rows: 0..0,
                    placed: Vec::new(),
                },
            });
        }
        for path in to_read {
            if !self.live.queue.contains(&path) {
                self.live.queue.push_back(path);
            }
        }
        self.live_remade(cx);
        self.live_pump(cx);
        self.live_look(cx);
    }

    /// Starts reads while fewer than [`FILES_ON_THEIR_WAY`] are on their way, and only
    /// while the live diff is on screen: a list that was answered after it was hidden
    /// leaves its files queued until it is shown again.
    fn live_pump(&mut self, cx: &mut Context<Self>) {
        if !self.live.on_screen {
            return;
        }
        while self.live.in_flight < FILES_ON_THEIR_WAY {
            let Some(path) = self.live.queue.pop_front() else {
                return;
            };
            self.live_read(&path, cx);
        }
    }

    /// Asks the Keeper for the two versions of one file of the list.
    fn live_read(&mut self, path: &Path, cx: &mut Context<Self>) {
        let (Some(shown), Some(file)) = (self.live.change.clone(), self.live.file(path).cloned())
        else {
            return;
        };
        self.live.sequence += 1;
        let sequence = self.live.sequence;
        self.live.asked.insert(file.path.clone(), sequence);
        self.live.in_flight += 1;
        // A renamed file is read by the path it had in the old version.
        let old_path = match &file.status {
            FileStatus::Renamed { from } => from.clone(),
            _ => file.path.clone(),
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::ReadVersions {
                    workspace: shown.workspace,
                    old: shown.from,
                    old_path,
                    new: shown.to,
                    path: file.path.clone(),
                },
                cx,
            )
        });
        let epoch = self.live.epoch;
        cx.spawn(async move |pane, cx| {
            let content = match asked.await {
                Ok(Reply::File(sides)) => match versions(sides) {
                    Ok((old, new)) => {
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
                    Err(why) => FileContent::Unshown(why),
                },
                Ok(_) | Err(_) => FileContent::Unshown(Unshown::Refused),
            };
            pane.update(cx, |pane, cx| {
                pane.live_loaded(epoch, &file.path, sequence, content, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Takes the two versions of a file, unless a later question for the same file was
    /// asked since, or the live diff started over.
    fn live_loaded(
        &mut self,
        epoch: u64,
        path: &Path,
        sequence: u64,
        content: FileContent,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.live.epoch {
            return;
        }
        self.live.in_flight = self.live.in_flight.saturating_sub(1);
        if self.live.asked.get(path) == Some(&sequence)
            && let Some(held) = self
                .live
                .files
                .iter_mut()
                .find(|held| held.file.path == path)
        {
            held.content = content;
        }
        self.live_remade(cx);
        self.live_pump(cx);
    }
}
