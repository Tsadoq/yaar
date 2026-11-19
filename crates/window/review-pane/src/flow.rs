//! Flow, the view of what the agent does as it works: the switch between Flow and
//! Changes, the switch between Flow's two forms, and the timeline of the agent's steps.
//!
//! Built by the review pane's first package of slice 5 (rows RP-02, RP-03, RP-06).
//!
//! What the timeline holds. The turns of the workspace on screen, each as its heading
//! (with its session's title, from `yaar_frame::AgentList`), its steps in the order of
//! their numbers, and its end once it ended, the turns in the order they started:
//! - when a workspace comes on screen, the timeline is emptied; it then holds every turn
//!   of the workspace that runs, as the agents model lists them (`Session::turn`; two that
//!   run at once come in that list's order, which the model does not tie to their start), and
//!   where none runs, the newest turn of the history (`yaar_link::HistoryEntry::Turn`, the
//!   last of the entries), each read with `Request::GetTurn`, which gives the steps so far,
//!   the end, and the state its change counts from;
//! - each turn that starts later in the workspace, from the agents model, as its heading
//!   and then a `Request::GetTurn` of its own;
//! - each `Fact::Step` of the workspace on screen, added under its turn by its number. A
//!   step already held is not added twice; a step whose number is more than one past the
//!   last held step of its turn has the turn read again with `Request::GetTurn`, whose
//!   answer fills what is missing, so that a step told while the answer was on its way
//!   and one the Window never heard are both in order;
//! - each `Fact::TurnEnded` of the workspace on screen, as the end of its turn; and then
//!   every other turn of the timeline that has not ended and whose last `GetTurn` answer
//!   gave no start is read again with `GetTurn`: a turn that started while the Keeper was
//!   still recording the state of the turn before gets its start when that recording is
//!   kept, which is before the `TurnEnded` of the turn before is told
//!   (`Timeline::start_of`).
//!
//! The live diff reads the timeline: after each `GetTurn` answer (or refusal) is taken, the
//! newest turn of the history is added, or anything that may put the live diff on screen
//! ([`ReviewPane::show_view`], a change of the form by the key, the bar or the settings
//! model, a workspace that comes on screen, the agents model), this file calls
//! `ReviewPane::live_look` last, so that the live diff asks for what it shows only while
//! it is on screen. A `Fact::Step` or `Fact::TurnEnded` makes no call of its own: the
//! live diff hears the same fact right after the timeline took it.
//!
//! The four controls of this file and of follow, [`crate::ToggleView`] and
//! [`crate::ToggleFlowForm`] among them, by key or by a click on the bar, call `steered`
//! first, never `acted`: they do not pause follow.
//!
//! The form. When a workspace comes on screen, and whenever the settings model changes,
//! `ReviewPane::form` is the form chosen last in that workspace
//! (`yaar_frame::SettingList::flow_form`). [`crate::ToggleFlowForm`] switches it while
//! Flow is on screen, in the middle of a turn too, and sets it with `SettingList::set`,
//! which sends `Request::SetSetting` with `Setting::FlowForm` of the workspace on screen:
//! the one request of Flow that a user starts. In Changes its key and its action do
//! nothing: the pane's listener (`draw.rs`) calls [`ReviewPane::toggle_flow_form`] only
//! while Flow is on screen.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{
    Fact, FlowForm, Reply, Request, SessionId, Setting, Step, TurnEnd, TurnId, WorkspaceId,
};

use crate::{FlowAt, Following, ReviewPane, TimelineEntry, ToggleFlowForm, ToggleView, View};

/// What the heading of a turn says when the agents model knows no session of that name:
/// a turn read from the history, whose session ended and left the model.
const UNKNOWN_SESSION: &str = "Agent session";

/// One turn of the timeline, with what the pane knows of it.
struct Held {
    id: TurnId,
    session: SessionId,
    title: String,
    /// The steps by their numbers, so that one told twice is one and one that came out of
    /// order finds its place.
    steps: BTreeMap<u32, Step>,
    /// Whether the turn ended, and, inside, how, when a `GetTurn` answer said so: a
    /// `TurnEnded` fact does not.
    ended: Option<Option<TurnEnd>>,
    /// What the last `GetTurn` answer gave as the turn's start; none until one was taken.
    start: Option<Option<String>>,
    /// Whether a `GetTurn` for this turn is on its way: one at a time.
    asking: bool,
    /// Whether a reason to ask came while one was on its way.
    ask_again: bool,
    /// The question on its way. Dropped with the turn, which drops its answer.
    task: Option<Task<()>>,
}

/// An entry of the timeline by what makes it the entry it is, which no later fact
/// changes: the reviewer's entry stays on it when entries come in above.
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

/// What Flow keeps of the turns of the workspace on screen, and the timeline made of
/// them.
pub(crate) struct Timeline {
    /// The workspace the turns are of.
    workspace: Option<WorkspaceId>,
    /// The turns, in the order they started.
    turns: Vec<Held>,
    /// The entries, made again from the turns at each change.
    entries: Vec<TimelineEntry>,
    /// The entry the reviewer is on.
    cursor: Option<Mark>,
    /// The step told last as a fact.
    last_told: Option<FlowAt>,
    /// Where the list is scrolled.
    pub(crate) scroll: UniformListScrollHandle,
    /// The rows the list last laid out, as the list itself told it.
    pub(crate) in_view: Rc<RefCell<Range<usize>>>,
}

impl Timeline {
    /// An empty timeline, for a pane that has no workspace on screen yet.
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        let _ = cx;
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

    /// The entries, in the order drawn.
    pub(crate) fn entries(&self) -> &[TimelineEntry] {
        &self.entries
    }

    /// The entry the reviewer is on.
    pub(crate) fn cursor(&self) -> Option<usize> {
        let mark = self.cursor.as_ref()?;
        self.entries
            .iter()
            .position(|entry| mark_of(entry) == *mark)
    }

    /// The rows of the timeline in view, as last laid out.
    pub(crate) fn in_view(&self) -> Range<usize> {
        self.in_view.borrow().clone()
    }

    /// The index of the entry of the step `at`; none for a step the timeline does not
    /// hold.
    pub(crate) fn row_of(&self, at: &FlowAt) -> Option<usize> {
        self.entries.iter().position(|entry| {
            matches!(entry, TimelineEntry::Step { turn, number, .. }
                if *turn == at.turn && *number == at.number)
        })
    }

    /// The step `at`, as the timeline holds it: what the live diff and follow read of a
    /// step's kind and path.
    pub(crate) fn step(&self, at: &FlowAt) -> Option<&Step> {
        self.held(&at.turn)?.steps.get(&at.number)
    }

    /// The state turn `turn` counts from, as its last `GetTurn` answer gave it
    /// (`yaar_link::Turn::start`): none until an answer or a refusal for it was taken;
    /// then `Some(None)` when it gave none or was refused, `Some(Some(state))` else. What
    /// the live diff counts from.
    pub(crate) fn start_of(&self, turn: &TurnId) -> Option<Option<&str>> {
        let start = self.held(turn)?.start.as_ref()?;
        Some(start.as_deref())
    }

    /// The step that came last: of the steps told as facts, the last told; before any,
    /// the last step of the newest turn read with `GetTurn`. What resuming follow jumps
    /// to.
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

    /// Empties the timeline for another workspace. The questions on their way go with the
    /// turns, which drops their answers.
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

    /// Makes the entries again from the turns.
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

    /// Puts the reviewer on entry `index`, when there is one.
    pub(crate) fn put_cursor(&mut self, index: usize) {
        self.cursor = self.entries.get(index).map(mark_of);
    }
}

impl ReviewPane {
    /// What [`ToggleView`] does: calls `steered` first, never `acted`, then shows Flow
    /// when Changes is on screen, and Changes when Flow is, as [`ReviewPane::show_view`]
    /// does.
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

    /// What [`ToggleFlowForm`] does while Flow is on screen: calls `steered` first, never
    /// `acted`, then shows the other form, kept for the workspace on screen with
    /// `yaar_frame::SettingList::set`, and ends with `ReviewPane::live_look`. Called only
    /// while Flow is on screen, by the pane's listener of the action and by a click on a
    /// form in the bar, which Flow alone draws: in Changes the key does nothing.
    pub(crate) fn toggle_flow_form(
        &mut self,
        _: &ToggleFlowForm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        self.steered(cx);
        let other = match self.form {
            FlowForm::Timeline => FlowForm::LiveDiff,
            FlowForm::LiveDiff => FlowForm::Timeline,
        };
        self.choose_form(other, cx);
    }

    /// Shows `form` in the workspace on screen and has the Keeper keep it. A form that is
    /// shown already is left as it is.
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

    /// Puts `view` on screen: Flow in the form of the workspace on screen, its list in view
    /// at the step Flow is at; Changes as the reviewer left it, scrolled where it was. Used
    /// by [`ToggleView`], the bar's two names, and follow (a message of the user to the
    /// agent, a turn end's offer taken). Ends with `ReviewPane::live_look`.
    ///
    /// Changes keeps its rows, its scroll and the reviewer's line: nothing here touches
    /// them, and its list, which is not drawn meanwhile, keeps its scroll handle.
    pub(crate) fn show_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        if view == View::Flow {
            self.bring_timeline_to_at();
        }
        cx.notify();
        self.live_look(cx);
    }

    /// While follow is on, the timeline is scrolled to the step Flow is at when it comes
    /// on screen, or when it is the form shown again. The live diff does the same itself,
    /// when its rows are made.
    fn bring_timeline_to_at(&mut self) {
        if self.following != Following::On || self.form != FlowForm::Timeline {
            return;
        }
        if let Some(row) = self.at.as_ref().and_then(|at| self.flow.row_of(at)) {
            self.scroll_timeline_to(row);
        }
    }

    /// The workspace on screen may be another: when it is, the timeline is emptied and
    /// filled for the new one, and the form is the one chosen there. Called at the end of
    /// `ReviewPane::new` and whenever the changes model changes; ends with
    /// `ReviewPane::live_look`.
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

    /// The agents model changed: a turn of the workspace on screen that the timeline does
    /// not hold yet is added, and read with `GetTurn`. Called at the end of
    /// `ReviewPane::new` and whenever the agents model changes; ends with
    /// `ReviewPane::live_look`.
    pub(crate) fn flow_agents(&mut self, cx: &mut Context<Self>) {
        self.add_running_turns(cx);
        // A session may have taken its title since the heading was made.
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

    /// The history of the workspace on screen was answered: with no turn running and none
    /// in the timeline, its newest turn is read with `GetTurn`.
    pub(crate) fn flow_history(&mut self, cx: &mut Context<Self>) {
        if !self.flow.turns.is_empty() || self.flow.workspace != self.tree.of {
            return;
        }
        let newest = self.tree.history.as_ref().and_then(|history| {
            history.entries.iter().rev().find_map(|entry| match entry {
                yaar_link::HistoryEntry::Turn { turn, session, .. } => {
                    Some((turn.clone(), session.clone()))
                }
                yaar_link::HistoryEntry::Commit(_) => None,
            })
        });
        if let Some((turn, session)) = newest {
            self.add_turn(turn, session, cx);
            self.live_look(cx);
        }
    }

    /// The settings model changed: the form is the one chosen in the workspace on screen.
    /// Called at the end of `ReviewPane::new` and whenever the settings model changes;
    /// ends with `ReviewPane::live_look`.
    pub(crate) fn flow_settings(&mut self, cx: &mut Context<Self>) {
        self.take_form(cx);
        self.live_look(cx);
    }

    /// A fact of the Keeper, for the timeline: `Step` and `TurnEnded` of the workspace on
    /// screen. Called for every fact, before the live diff and follow hear it, so that a
    /// step is in the timeline when they look for it.
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
                // A step more than one past the last held leaves a gap, which the
                // Keeper's account of the turn fills.
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
                // A turn that started while the state of the turn before was being
                // recorded got no start; it has one by now.
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

    /// What [`crate::LineDown`] and [`crate::LineUp`] do in the timeline: the reviewer's
    /// entry moves by `by`, and stays at the first and the last.
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

    /// Scrolls the timeline so that entry `row` is in view, as follow wants it. The
    /// reviewer's entry goes there too: that is where follow moved Flow.
    pub(crate) fn scroll_timeline_to(&mut self, row: usize) {
        self.flow.put_cursor(row);
        self.flow
            .scroll
            .scroll_to_item(row, ScrollStrategy::Nearest);
    }

    /// What a click on entry `index` does: the reviewer is on it.
    pub(crate) fn click_timeline_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.flow.put_cursor(index);
        cx.notify();
    }

    /// The form chosen in the workspace on screen is the form shown.
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

    /// The turns the agents model shows running in `workspace`, with their sessions and
    /// the sessions' titles.
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

    /// Adds each turn the agents model shows running in the workspace on screen that the
    /// timeline does not hold.
    fn add_running_turns(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.flow.workspace.clone() else {
            return;
        };
        for (turn, session) in self.running_turns(&workspace, cx) {
            self.add_turn(turn, session, cx);
        }
    }

    /// Adds `turn` as a heading and reads it, unless the timeline holds it.
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

    /// Asks the Keeper for `turn` with its steps. One question for a turn is on its way at
    /// a time; a reason to ask that comes meanwhile asks once more when the answer is here.
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

    /// Takes the Keeper's answer for a turn: its steps are merged by their numbers, its end
    /// and its start taken; a refusal leaves the heading with the steps told so far.
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
            // A turn the Keeper does not hold, or an answer of another kind: nothing is
            // said, and the live diff counts the turn from the commit checked out.
            Ok(_) | Err(AskError::Refused(_) | AskError::Unexpected) => {
                held.start.get_or_insert(None);
            }
            // No Keeper to ask: nothing was learned.
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
