//! The live diff of Flow: the change of one turn on screen, from the version the turn started at to
//! the files on disk, or to the state the turn ended with.

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

pub(crate) struct Live {
    change: Option<LiveChange>,
    files: Vec<ShownFile>,
    rows: Vec<PaneRow>,
    group: Vec<PathBuf>,
    position: Option<Position>,
    position_row: Option<usize>,
    scrolled_to: Option<FlowAt>,
    on_screen: bool,
    epoch: u64,
    stale: bool,
    everything: bool,
    named: Vec<PathBuf>,
    listing: bool,
    queue: VecDeque<PathBuf>,
    in_flight: usize,
    /// The number of the last read asked for each file: of two answers, the later question's.
    asked: HashMap<PathBuf, u64>,
    sequence: u64,
    ended: Option<(TurnId, Option<String>)>,
    pub(crate) scroll: UniformListScrollHandle,
    pub(crate) in_view: Rc<RefCell<Range<usize>>>,
}

impl Live {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn change(&self) -> Option<&LiveChange> {
        self.change.as_ref()
    }

    pub(crate) fn rows(&self) -> &[PaneRow] {
        &self.rows
    }

    pub(crate) fn position(&self) -> Option<&Position> {
        self.position.as_ref()
    }

    pub(crate) fn position_row(&self) -> Option<usize> {
        self.position_row
    }

    pub(crate) fn text(&self, position: &Position) -> Option<String> {
        let shown = self.diff_of(&position.path)?;
        (position.line < shown.diff.line_count(position.side))
            .then(|| shown.text(position.side, position.line).into_owned())
    }

    pub(crate) fn file(&self, path: &Path) -> Option<&ChangedFile> {
        self.files
            .iter()
            .find(|held| held.file.path == path)
            .map(|held| &held.file)
    }

    pub(crate) fn in_view(&self) -> Range<usize> {
        self.in_view.borrow().clone()
    }

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

    fn heading_of(&self, path: &Path) -> Option<usize> {
        self.files
            .iter()
            .find(|held| {
                held.file.path == path
                    || matches!(&held.file.status, FileStatus::Renamed { from } if from == path)
            })
            .map(|held| held.rows.start)
    }

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

    /// None while a file before it is still on its way.
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

    fn holds(&self, path: &Path) -> bool {
        self.heading_of(path).is_some()
    }

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
                    // The turn ended: the same files, read as they are in the state it ended with.
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
            self.live_pump(cx);
            self.live_follow_scroll();
        }
    }

    pub(crate) fn live_workspace(&mut self, cx: &mut Context<Self>) {
        let now = self.models.changes.read(cx).workspace().cloned();
        if self.live.change.as_ref().map(|held| &held.workspace) != now.as_ref() {
            self.live.start_over(None);
            self.live.ended = None;
            self.live_make_rows(cx);
        }
        self.live_look(cx);
    }

    pub(crate) fn live_agents(&mut self, cx: &mut Context<Self>) {
        self.live_look(cx);
    }

    pub(crate) fn live_history(&mut self, cx: &mut Context<Self>) {
        self.live_look(cx);
    }

    /// Called after the timeline heard the fact and before follow does: a read step's file, when
    /// the list does not hold it, is a row among the files read before this returns.
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
            // A change counted to a recorded state does not move with the files: only one that is
            // counted to the files on disk needs the fact.
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

    pub(crate) fn live_rebuild(&mut self, cx: &mut Context<Self>) {
        self.live_make_rows(cx);
    }

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

    pub(crate) fn live_go_to(&mut self, index: usize, side: Side, cx: &mut Context<Self>) {
        // A click is told by the index its row had when it was drawn; the rows may have changed
        // since.
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

    /// None for a command, and for an edit or a write whose file the list does not hold yet.
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

    pub(crate) fn scroll_live_to(&mut self, row: usize) {
        self.live
            .scroll
            .scroll_to_item(row, ScrollStrategy::Nearest);
        self.live.scrolled_to = self.at.clone();
    }

    /// What the live diff shows now: the newest turn not ended, else the newest. None while no
    /// turn, no `GetTurn` answer for it, or (where the answer gave no start) no history is known.
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
            // The newest turn not ended, not simply the newest: an earlier turn's end, told after a
            // later one started, must move nothing.
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

    fn live_remade(&mut self, cx: &mut Context<Self>) {
        self.live_make_rows(cx);
        if self.live.on_screen {
            self.live_follow_scroll();
        }
    }

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

    fn live_ask(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.live.change.clone() else {
            return;
        };
        let Some(from) = shown.from.clone() else {
            // The Keeper compares two versions: with none before the change there is nothing to
            // list its files from.
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
            // What is shown stays, and the next fact asks again. One that came while this question
            // was out asks now; with none, a Keeper that keeps refusing would be asked without end.
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

    fn live_read(&mut self, path: &Path, cx: &mut Context<Self>) {
        let (Some(shown), Some(file)) = (self.live.change.clone(), self.live.file(path).cloned())
        else {
            return;
        };
        self.live.sequence += 1;
        let sequence = self.live.sequence;
        self.live.asked.insert(file.path.clone(), sequence);
        self.live.in_flight += 1;
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
                        // The diff of a large file takes long enough to be seen: it is made off the
                        // thread that draws.
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

    /// Takes a file's two versions, unless a later question for the same file was asked since or
    /// the live diff started over.
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
