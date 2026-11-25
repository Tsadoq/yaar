//! The timeline of the turns on screen, fed by `Fact::Step` and `TurnEnded`. A gap in step numbers
//! re-reads the turn. Each `TurnEnded` re-reads unended turns with no start: a turn's start is
//! recorded before the previous `TurnEnded` is told.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{
    Fact, FlowForm, Reply, Request, SessionId, Setting, Step, TurnEnd, TurnId, WorkspaceId,
};

use crate::{FlowAt, Following, ReviewPane, TimelineEntry, ToggleView, View};

const UNKNOWN_SESSION: &str = "Agent session";

struct Held {
    id: TurnId,
    session: SessionId,
    title: String,
    steps: BTreeMap<u32, Step>,
    /// Some once the turn ended; inside, how it ended when a `GetTurn` answer said so (a
    /// `TurnEnded` fact does not say).
    ended: Option<Option<TurnEnd>>,
    start: Option<Option<String>>,
    asking: bool,
    ask_again: bool,
    task: Option<Task<()>>,
}

/// A timeline entry named by what no later fact changes, so the reviewer stays on their entry
/// when entries come in above.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mark {
    Turn(TurnId),
    Step(TurnId, u32),
    End(TurnId),
}

fn mark_of(entry: &TimelineEntry) -> Mark {
    match entry {
        TimelineEntry::Turn { turn, .. } => Mark::Turn(turn.clone()),
        TimelineEntry::Step { turn, number, .. } => Mark::Step(turn.clone(), *number),
        TimelineEntry::End { turn, .. } => Mark::End(turn.clone()),
    }
}

pub(crate) struct Timeline {
    workspace: Option<WorkspaceId>,
    turns: Vec<Held>,
    entries: Vec<TimelineEntry>,
    cursor: Option<Mark>,
    last_told: Option<FlowAt>,
    pub(crate) scroll: UniformListScrollHandle,
    pub(crate) in_view: Rc<RefCell<Range<usize>>>,
}

impl Timeline {
    pub(crate) fn new() -> Self {
        Self {
            workspace: None,
            turns: Vec::new(),
            entries: Vec::new(),
            cursor: None,
            last_told: None,
            scroll: UniformListScrollHandle::new(),
            in_view: Rc::new(RefCell::new(0..0)),
        }
    }

    pub(crate) fn entries(&self) -> &[TimelineEntry] {
        &self.entries
    }

    pub(crate) fn cursor(&self) -> Option<usize> {
        let mark = self.cursor.as_ref()?;
        self.entries
            .iter()
            .position(|entry| mark_of(entry) == *mark)
    }

    pub(crate) fn in_view(&self) -> Range<usize> {
        self.in_view.borrow().clone()
    }

    pub(crate) fn row_of(&self, at: &FlowAt) -> Option<usize> {
        self.entries.iter().position(|entry| {
            matches!(entry, TimelineEntry::Step { turn, number, .. }
                if *turn == at.turn && *number == at.number)
        })
    }

    pub(crate) fn step(&self, at: &FlowAt) -> Option<&Step> {
        self.held(&at.turn)?.steps.get(&at.number)
    }

    pub(crate) fn step_count(&self, turn: &TurnId) -> usize {
        self.held(turn).map_or(0, |held| held.steps.len())
    }

    pub(crate) fn is_running(&self, turn: &TurnId) -> bool {
        self.held(turn).is_some_and(|held| held.ended.is_none())
    }

    /// None until an answer or a refusal was taken; then `Some(None)` when it gave none or was
    /// refused.
    pub(crate) fn start_of(&self, turn: &TurnId) -> Option<Option<&str>> {
        let start = self.held(turn)?.start.as_ref()?;
        Some(start.as_deref())
    }

    /// The last step told as a fact; before any, the last step of the newest turn read.
    pub(crate) fn newest(&self) -> Option<FlowAt> {
        self.last_told.clone().or_else(|| {
            let newest = self
                .turns
                .iter()
                .rev()
                .find(|held| !held.steps.is_empty())?;
            let (number, _) = newest.steps.last_key_value()?;
            Some(FlowAt {
                turn: newest.id.clone(),
                number: *number,
            })
        })
    }

    fn held(&self, turn: &TurnId) -> Option<&Held> {
        self.turns.iter().find(|held| held.id == *turn)
    }

    fn held_mut(&mut self, turn: &TurnId) -> Option<&mut Held> {
        self.turns.iter_mut().find(|held| held.id == *turn)
    }

    fn reset(&mut self, workspace: Option<WorkspaceId>) {
        self.workspace = workspace;
        self.turns.clear();
        self.entries.clear();
        self.cursor = None;
        self.last_told = None;
        *self.in_view.borrow_mut() = 0..0;
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(0.)));
    }

    fn remake(&mut self) {
        let mut entries = Vec::new();
        for held in &self.turns {
            entries.push(TimelineEntry::Turn {
                turn: held.id.clone(),
                session: held.session.clone(),
                title: held.title.clone(),
            });
            for (number, step) in &held.steps {
                entries.push(TimelineEntry::Step {
                    turn: held.id.clone(),
                    number: *number,
                    step: step.clone(),
                });
            }
            if let Some(how) = held.ended {
                entries.push(TimelineEntry::End {
                    turn: held.id.clone(),
                    how,
                });
            }
        }
        self.entries = entries;
    }

    pub(crate) fn put_cursor(&mut self, index: usize) {
        self.cursor = self.entries.get(index).map(mark_of);
    }
}

impl ReviewPane {
    pub(crate) fn toggle_view(
        &mut self,
        _: &ToggleView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        self.steered(cx);
        let other = match self.view {
            View::Changes => View::Flow,
            View::Flow => View::Changes,
        };
        self.show_view(other, cx);
    }

    pub(crate) fn toggle_flow_form(&mut self, cx: &mut Context<Self>) {
        self.steered(cx);
        let other = match self.form {
            FlowForm::Timeline => FlowForm::LiveDiff,
            FlowForm::LiveDiff => FlowForm::Timeline,
        };
        self.choose_form(other, cx);
    }

    pub(crate) fn choose_form(&mut self, form: FlowForm, cx: &mut Context<Self>) {
        let Some(workspace) = self.flow.workspace.clone() else {
            return;
        };
        if self.form == form {
            return;
        }
        self.form = form;
        self.models.settings.update(cx, |settings, cx| {
            settings.set(Setting::FlowForm { workspace, form }, cx);
        });
        if form == FlowForm::Timeline {
            self.bring_timeline_to_at();
        }
        cx.notify();
        self.live_look(cx);
    }

    pub(crate) fn show_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        if view == View::Flow {
            self.bring_timeline_to_at();
        }
        cx.notify();
        self.live_look(cx);
    }

    fn bring_timeline_to_at(&mut self) {
        if self.following != Following::On || self.form != FlowForm::Timeline {
            return;
        }
        if let Some(row) = self.at.as_ref().and_then(|at| self.flow.row_of(at)) {
            self.scroll_timeline_to(row);
        }
    }

    pub(crate) fn flow_workspace(&mut self, cx: &mut Context<Self>) {
        let now = self.models.changes.read(cx).workspace().cloned();
        if now != self.flow.workspace {
            self.flow.reset(now);
            // A step of the workspace before is no step of this one.
            self.at = None;
            self.add_running_turns(cx);
            self.flow_history(cx);
            cx.notify();
        }
        self.take_form(cx);
        self.live_look(cx);
    }

    pub(crate) fn flow_agents(&mut self, cx: &mut Context<Self>) {
        self.add_running_turns(cx);
        let titles: Vec<(usize, String)> = self
            .flow
            .turns
            .iter()
            .enumerate()
            .filter_map(|(index, held)| {
                let title = self
                    .models
                    .agents
                    .read(cx)
                    .session(&held.session)?
                    .title
                    .clone();
                (title != held.title).then_some((index, title))
            })
            .collect();
        if !titles.is_empty() {
            for (index, title) in titles {
                self.flow.turns[index].title = title;
            }
            self.flow.remake();
            cx.notify();
        }
        self.live_look(cx);
    }

    pub(crate) fn flow_history(&mut self, cx: &mut Context<Self>) {
        if !self.flow.turns.is_empty() || self.flow.workspace != self.tree.of {
            return;
        }
        let newest = self.tree.history.as_ref().and_then(|history| {
            history.entries.iter().rev().find_map(|entry| match entry {
                yaar_link::HistoryEntry::Turn { turn, session, .. } => {
                    Some((turn.clone(), session.clone()))
                }
                // A turn whose state expired has no live diff to show.
                yaar_link::HistoryEntry::Commit(_)
                | yaar_link::HistoryEntry::ExpiredTurn { .. } => None,
            })
        });
        if let Some((turn, session)) = newest {
            self.add_turn(turn, session, cx);
            self.live_look(cx);
        }
    }

    pub(crate) fn flow_settings(&mut self, cx: &mut Context<Self>) {
        self.take_form(cx);
        self.live_look(cx);
    }

    pub(crate) fn flow_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        match fact {
            Fact::Step {
                workspace,
                session,
                turn,
                number,
                step,
            } if self.flow.workspace.as_ref() == Some(workspace) => {
                self.add_turn(turn.clone(), session.clone(), cx);
                let Some(held) = self.flow.held_mut(turn) else {
                    return;
                };
                let last = held.steps.last_key_value().map_or(0, |(last, _)| *last);
                held.steps.entry(*number).or_insert_with(|| step.clone());
                // A step more than one past the last held leaves a gap, which the Keeper's account
                // fills.
                if *number > last + 1 {
                    self.read_turn(turn, cx);
                }
                self.flow.last_told = Some(FlowAt {
                    turn: turn.clone(),
                    number: *number,
                });
                self.flow.remake();
                cx.notify();
            }
            Fact::TurnEnded {
                workspace,
                session,
                turn,
                ..
            } if self.flow.workspace.as_ref() == Some(workspace) => {
                self.add_turn(turn.clone(), session.clone(), cx);
                if let Some(held) = self.flow.held_mut(turn) {
                    // A fact does not say how the turn ended: what an answer said stays.
                    held.ended.get_or_insert(None);
                }
                let without_start: Vec<TurnId> = self
                    .flow
                    .turns
                    .iter()
                    .filter(|held| {
                        held.id != *turn
                            && held.ended.is_none()
                            && !matches!(held.start, Some(Some(_)))
                    })
                    .map(|held| held.id.clone())
                    .collect();
                for other in without_start {
                    self.read_turn(&other, cx);
                }
                self.flow.remake();
                cx.notify();
            }
            _ => {}
        }
    }

    pub(crate) fn timeline_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(last) = self.flow.entries.len().checked_sub(1) else {
            return;
        };
        let next = match self.flow.cursor() {
            Some(now) => now.saturating_add_signed(by).min(last),
            None => 0,
        };
        self.flow.put_cursor(next);
        self.flow
            .scroll
            .scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(crate) fn scroll_timeline_to(&mut self, row: usize) {
        self.flow.put_cursor(row);
        self.flow
            .scroll
            .scroll_to_item(row, ScrollStrategy::Nearest);
    }

    pub(crate) fn click_timeline_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.flow.put_cursor(index);
        cx.notify();
    }

    fn take_form(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = &self.flow.workspace else {
            return;
        };
        let form = self.models.settings.read(cx).flow_form(workspace);
        if form != self.form {
            self.form = form;
            cx.notify();
        }
    }

    fn running_turns(&self, workspace: &WorkspaceId, cx: &App) -> Vec<(TurnId, SessionId)> {
        self.models
            .agents
            .read(cx)
            .of(workspace)
            .map(|agents| {
                agents
                    .sessions
                    .iter()
                    .filter_map(|session| Some((session.turn.clone()?, session.id.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn add_running_turns(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.flow.workspace.clone() else {
            return;
        };
        for (turn, session) in self.running_turns(&workspace, cx) {
            self.add_turn(turn, session, cx);
        }
    }

    fn add_turn(&mut self, turn: TurnId, session: SessionId, cx: &mut Context<Self>) {
        if self.flow.held(&turn).is_some() {
            return;
        }
        let title = self
            .models
            .agents
            .read(cx)
            .session(&session)
            .map_or_else(|| UNKNOWN_SESSION.to_owned(), |found| found.title.clone());
        self.flow.turns.push(Held {
            id: turn.clone(),
            session,
            title,
            steps: BTreeMap::new(),
            ended: None,
            start: None,
            asking: false,
            ask_again: false,
            task: None,
        });
        self.flow.remake();
        self.read_turn(&turn, cx);
        cx.notify();
    }

    fn read_turn(&mut self, turn: &TurnId, cx: &mut Context<Self>) {
        let Some(held) = self.flow.held_mut(turn) else {
            return;
        };
        if held.asking {
            held.ask_again = true;
            return;
        }
        held.asking = true;
        let id = turn.clone();
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::GetTurn { turn: id.clone() }, cx)
        });
        let task = cx.spawn(async move |pane, cx| {
            let answer = asked.await;
            pane.update(cx, |pane, cx| pane.turn_answered(&id, answer, cx))
                .ok();
        });
        if let Some(held) = self.flow.held_mut(turn) {
            held.task = Some(task);
        }
    }

    fn turn_answered(
        &mut self,
        turn: &TurnId,
        answer: Result<Reply, AskError>,
        cx: &mut Context<Self>,
    ) {
        let Some(held) = self.flow.held_mut(turn) else {
            return;
        };
        held.asking = false;
        match answer {
            Ok(Reply::Turn(read)) if read.id == *turn => {
                for (index, step) in read.steps.into_iter().enumerate() {
                    held.steps.entry(index as u32 + 1).or_insert(step);
                }
                if let Some(how) = read.ended {
                    held.ended = Some(Some(how));
                }
                held.session = read.session;
                held.start = Some(read.start);
            }
            // A turn the Keeper does not hold, or an answer of another kind: the live diff counts
            // the turn from the commit checked out.
            Ok(_) | Err(AskError::Refused(_) | AskError::Unexpected) => {
                held.start.get_or_insert(None);
            }
            Err(AskError::NotLinked) => {}
        }
        let again = std::mem::take(&mut held.ask_again);
        self.flow.remake();
        if again {
            self.read_turn(turn, cx);
        }
        cx.notify();
        self.live_look(cx);
    }
}
