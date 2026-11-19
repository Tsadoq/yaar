//! Flow, the view of what the agent does as it works: the switch between Flow and
//! Changes, the switch between Flow's two forms, and the timeline of the agent's steps.
//!
//! Built by the review pane's first package of slice 5 (rows RP-02, RP-03, RP-06).
//!
//! What the timeline holds. The turns of the workspace on screen, each as its heading
//! (with its session's title, from `yaar_frame::AgentList`), its steps in the order of
//! their numbers, and its end once it ended, the turns in the order they started:
//! - when a workspace comes on screen, the timeline is emptied; it then holds every turn
//!   of the workspace that runs, as the agents model lists them (`Session::turn`), and
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
//! The live diff reads the timeline: after each `GetTurn` answer (or refusal) is taken, a
//! turn is added, or anything that may put the live diff on screen ([`ReviewPane::show_view`],
//! a change of the form by the key, the bar or the settings model, a workspace that comes
//! on screen), this file calls `ReviewPane::live_look` last, so that the live diff asks for
//! what it shows only while it is on screen.
//!
//! The four controls of this file and of follow, [`crate::ToggleView`] and
//! [`crate::ToggleFlowForm`] among them, by key or by a click on the bar, call `steered`
//! first, never `acted`: they do not pause follow.
//!
//! The form. When a workspace comes on screen, and whenever the settings model changes,
//! `ReviewPane::form` is the form chosen last in that workspace
//! (`yaar_frame::SettingList::flow_form`). [`crate::ToggleFlowForm`] switches it, in the
//! middle of a turn too, and sets it with `SettingList::set`, which sends
//! `Request::SetSetting` with `Setting::FlowForm` of the workspace on screen: the one
//! request of Flow that a user starts.

use std::ops::Range;

use gpui_kit::*;
use yaar_link::{Fact, Step, TurnId};

use crate::{FlowAt, ReviewPane, TimelineEntry, ToggleFlowForm, ToggleView, View};

/// What Flow keeps of the turns of the workspace on screen, and the timeline made of
/// them. The fields are the package's to choose.
pub(crate) struct Timeline {}

impl Timeline {
    /// An empty timeline, for a pane that has no workspace on screen yet.
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        let _ = cx;
        Self {}
    }

    /// The entries, in the order drawn.
    pub(crate) fn entries(&self) -> &[TimelineEntry] {
        todo!("the first package of the review pane: the timeline")
    }

    /// The entry the reviewer is on.
    pub(crate) fn cursor(&self) -> Option<usize> {
        todo!("the first package of the review pane: the reviewer's entry")
    }

    /// The rows of the timeline in view, as last laid out.
    pub(crate) fn in_view(&self) -> Range<usize> {
        todo!("the first package of the review pane: the rows in view")
    }

    /// The index of the entry of the step `at`; none for a step the timeline does not
    /// hold.
    pub(crate) fn row_of(&self, at: &FlowAt) -> Option<usize> {
        let _ = at;
        todo!("the first package of the review pane: the entry of a step")
    }

    /// The step `at`, as the timeline holds it: what the live diff and follow read of a
    /// step's kind and path.
    pub(crate) fn step(&self, at: &FlowAt) -> Option<&Step> {
        let _ = at;
        todo!("the first package of the review pane: a step by its place")
    }

    /// The state turn `turn` counts from, as its last `GetTurn` answer gave it
    /// (`yaar_link::Turn::start`): none until an answer or a refusal for it was taken;
    /// then `Some(None)` when it gave none or was refused, `Some(Some(state))` else. What
    /// the live diff counts from.
    pub(crate) fn start_of(&self, turn: &TurnId) -> Option<Option<&str>> {
        let _ = turn;
        todo!("the first package of the review pane: the start of a turn")
    }

    /// The step that came last: of the steps told as facts, the last told; before any,
    /// the last step of the newest turn read with `GetTurn`. What resuming follow jumps
    /// to.
    pub(crate) fn newest(&self) -> Option<FlowAt> {
        todo!("the first package of the review pane: the newest step")
    }
}

impl ReviewPane {
    /// What [`ToggleView`] does: calls `steered` first, never `acted`, then shows Flow
    /// when Changes is on screen, and Changes when Flow is, as [`ReviewPane::show_view`]
    /// does.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn toggle_view(
        &mut self,
        _: &ToggleView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
    }

    /// What [`ToggleFlowForm`] does: calls `steered` first, never `acted`, then shows the
    /// other form, kept for the workspace on screen with `yaar_frame::SettingList::set`,
    /// and ends with `ReviewPane::live_look`. With Changes on screen it switches the form
    /// all the same, for the next time Flow shows.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn toggle_flow_form(
        &mut self,
        _: &ToggleFlowForm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
    }

    /// Puts `view` on screen: Flow in the form of the workspace on screen, its list in view
    /// at the step Flow is at; Changes as the reviewer left it, scrolled where it was. Used
    /// by [`ToggleView`], the bar's two names, and follow (a message of the user to the
    /// agent, a turn end's offer taken). Ends with `ReviewPane::live_look`.
    pub(crate) fn show_view(&mut self, view: View, cx: &mut Context<Self>) {
        let _ = (view, cx);
        todo!("the first package of the review pane: the switch of the view")
    }

    /// The workspace on screen may be another: when it is, the timeline is emptied and
    /// filled for the new one, and the form is the one chosen there. Called at the end of
    /// `ReviewPane::new` and whenever the changes model changes; ends with
    /// `ReviewPane::live_look`.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn flow_workspace(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The agents model changed: a turn of the workspace on screen that the timeline does
    /// not hold yet is added, and read with `GetTurn`. Called at the end of
    /// `ReviewPane::new` and whenever the agents model changes; ends with
    /// `ReviewPane::live_look`.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn flow_agents(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The history of the workspace on screen was answered: with no turn running and none
    /// in the timeline, its newest turn is read with `GetTurn`.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn flow_history(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The settings model changed: the form is the one chosen in the workspace on screen.
    /// Called at the end of `ReviewPane::new` and whenever the settings model changes;
    /// ends with `ReviewPane::live_look`.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn flow_settings(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// A fact of the Keeper, for the timeline: `Step` and `TurnEnded` of the workspace on
    /// screen. Called for every fact, before the live diff and follow hear it, so that a
    /// step is in the timeline when they look for it.
    ///
    /// Filled by the first package of the review pane; the contract's body does nothing.
    pub(crate) fn flow_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        let _ = (fact, cx);
    }

    /// What [`crate::LineDown`] and [`crate::LineUp`] do in the timeline: the reviewer's
    /// entry moves by `by`, and stays at the first and the last.
    pub(crate) fn timeline_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let _ = (by, cx);
        todo!("the first package of the review pane: moving through the timeline")
    }

    /// Scrolls the timeline so that entry `row` is in view, as follow wants it.
    pub(crate) fn scroll_timeline_to(&mut self, row: usize) {
        let _ = row;
        todo!("the first package of the review pane: scrolling the timeline")
    }
}
