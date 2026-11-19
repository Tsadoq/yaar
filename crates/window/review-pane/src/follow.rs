//! Follow: Flow moves to each step of the agent as it comes, pauses when the reviewer
//! scrolls, clicks or types in the pane, counts the steps missed, and offers to resume; a
//! message of the user to the agent shows Flow; a turn that ends while Flow is on screen
//! offers to open in Changes what changed since the last review.
//!
//! Built by the review pane's fourth package of slice 5 (rows RP-08, RP-10, RP-11, RP-12).
//!
//! Follow is on when the pane opens, and stays on until the reviewer turns it off with
//! [`crate::ToggleFollow`] or a click on `flow-follow`; nothing else turns it off, and it is
//! not kept beyond the window. `ReviewPane::following` is what the reviewer sees.
//!
//! Moving. At each `Fact::Step` of the workspace on screen, after the timeline and the
//! live diff heard it, while follow is [`crate::Following::On`]: `ReviewPane::at` is that
//! step, in Changes too, and the list of the form on screen is scrolled so that its row is
//! in view (`ReviewPane::scroll_timeline_to`, `ReviewPane::scroll_live_to`). A read moves
//! Flow as a write does: in the timeline to its entry; in the live diff to the heading of
//! the file it names when the turn changed that file, else to the file's row among the
//! files read (`crate::PaneRow::Read`), which the live diff made when it heard the step,
//! before follow hears it. `ReviewPane::live_row_of` gives either.
//!
//! Pausing. [`ReviewPane::touched`] is the reviewer at work in the pane. It is called:
//! - by `acted`, which every action of the reviewer calls first, so for every key of the
//!   pane that an action takes (`j`, `k`, `c`, `shift-o` and the rest) and every click on a
//!   control that acts, before the action acts;
//! - by the element below the pane's bar, for every mouse button pressed there and every
//!   turn of the wheel there, before anything inside acts on it, and for every key that no
//!   action takes while the focus is there (text typed in the comment box).
//!
//! It is not called by the bar (`review-bar`: the names of the views, `flow-follow`, the
//! forms) nor by the four controls of the views and of follow, [`crate::ToggleView`],
//! [`crate::ToggleFlowForm`], [`crate::ToggleFollow`] and [`crate::ResumeFollow`], which call
//! `steered` instead: they choose what Flow shows, so a reviewer who presses `f` to watch
//! the agent sees Flow follow it.
//!
//! It acts only from [`crate::Following::On`], and only while a turn of the workspace on
//! screen runs (a session of it in the agents model holds a turn): follow becomes
//! `Paused { missed: 0 }`. From `Paused` it does nothing, so the count of missed steps runs
//! from the first touch; from `Off` it does nothing; with no turn running it does nothing.
//! Each `Fact::Step` of the workspace on screen while paused counts one more missed,
//! whatever its kind, and leaves `ReviewPane::at` where it was. Resuming, by
//! [`crate::ResumeFollow`] (`shift-g`, which acts only while Flow is on screen: in Changes
//! the pane's listener in `draw.rs` does not call [`ReviewPane::resume_follow`], so the key
//! does nothing there) or a click on `flow-follow-resume`, puts Flow at the newest step
//! (`Timeline::newest`), its row in view, and follow on; with follow on it says "Follow is
//! not paused" in the notice, with follow off "Follow is off". [`crate::ToggleFollow`] and a
//! click on `flow-follow` turn follow off from `On` or `Paused`, and on from `Off`. A click
//! on `flow-follow-resume` is below the bar, so it touches first, which leaves a pause as
//! it is. A turn that starts in the workspace ends a pause, with nothing missed.
//!
//! A message of the user to the agent. A `Fact::AgentsChanged` of the workspace on screen
//! in which a session holds a turn (`Session::turn`) that follow has not met is the start
//! of a turn, which the user's message to the agent starts. Follow meets the turns of the
//! workspace on screen three ways, none of which starts anything: [`ReviewPane::follow_workspace`]
//! (at the end of `ReviewPane::new` and whenever the changes model changes) forgets the
//! turns met and meets those the agents model shows for the workspace on screen now;
//! [`ReviewPane::follow_agents`] (at the end of `ReviewPane::new` and whenever the agents
//! model changes) meets every turn the agents model shows for it, which is how the agents a
//! Window reads when it connects (`ListAgents`) start no turn; and the start itself, once
//! taken, is met. The pane hears a fact (`follow_told`) before the agents model's observers
//! run for it (the model notifies after every subscriber of the link heard the fact), so a
//! turn an `AgentsChanged` brings is met by `follow_told` first. A turn that only changes its
//! session's state (works, waits for permission) is met already and starts nothing. At a
//! start, with follow on or paused, the pane shows Flow (`ReviewPane::show_view`), follow is
//! on with no step missed, and Flow is at the turn's first step once it comes. With follow
//! off, nothing changes.
//!
//! A turn's end. A `Fact::TurnEnded` of the workspace on screen with a state, while Flow
//! is on screen, leaves Flow on screen and offers what changed since the last review:
//! `Since { from: since or the history's start, to: state }`, the change slice 4's
//! `turn_ended` builds, drawn under `flow-offer` with the control `flow-offer-take`. What
//! the turn end does to Changes is slice 4's and is heard first (`told` runs `turn_ended`
//! before `follow_told`): under the default setting it has already selected that change
//! (asking `CompareVersions` and then `ReadVersions` for its files); while the pane keeps
//! the selection it holds slice 4's offer (`review-offer`) and asked nothing. Taking Flow's
//! offer, by the control or [`crate::TakeOffer`] (`take_flow_offer`), shows Changes: while
//! slice 4's offer is held it takes it as `shift-o` in Changes does (`ReviewPane::take`,
//! which selects the change and asks `CompareVersions`); otherwise the change is selected
//! already and taking it asks nothing more. The offer goes when it is taken, when the
//! reviewer shows Changes otherwise, when another turn starts, and when the workspace on
//! screen changes. With Changes on screen at a turn's end nothing is offered in Flow.

use std::collections::HashSet;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{Fact, FlowForm, TurnId, WorkspaceAgents, WorkspaceId};

use crate::draw::ROW_HEIGHT;
use crate::{ChangeShown, FlowAt, Following, ResumeFollow, ReviewPane, ToggleFollow, View};

/// What follow keeps beside `ReviewPane::following`.
pub(crate) struct Follow {
    /// The workspace follow's other fields are about: the one on screen when follow last
    /// looked.
    workspace: Option<WorkspaceId>,
    /// The turns of that workspace follow has met: those the agents model showed when the
    /// workspace came on screen or changed, and those that started since. A turn not in
    /// it is a turn that starts.
    met: HashSet<TurnId>,
    /// What a turn end offered while Flow was on screen.
    offer: Option<ChangeShown>,
}

impl Follow {
    /// Follow of a pane that just opened.
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        // The offer is for the reviewer who is in Flow: whatever shows Changes takes it
        // away, however it did, and a notification of the pane is the one place that
        // sees them all.
        cx.observe(&cx.entity(), |pane, _, _| {
            if pane.view != View::Flow {
                pane.follow.offer = None;
            }
        })
        .detach();
        Self {
            workspace: None,
            met: HashSet::new(),
            offer: None,
        }
    }

    /// What a turn end offered in Flow.
    pub(crate) fn offer(&self) -> Option<&ChangeShown> {
        self.offer.as_ref()
    }
}

impl ReviewPane {
    /// What [`ToggleFollow`] does: calls `steered` first, never `acted`. Turns follow off
    /// from `On` or `Paused`, and on from `Off`.
    pub(crate) fn toggle_follow(
        &mut self,
        _: &ToggleFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        self.steered(cx);
        self.following = match self.following {
            Following::On | Following::Paused { .. } => Following::Off,
            Following::Off => Following::On,
        };
        cx.notify();
    }

    /// What [`ResumeFollow`] does while Flow is on screen: calls `steered` first, never
    /// `acted`. Called only while Flow is on screen, by the pane's listener of the action
    /// and by a click on `flow-follow-resume`, which Flow alone draws: in Changes the key
    /// does nothing, and says nothing.
    ///
    /// From a pause, Flow goes to the newest step and follow is on; otherwise the notice
    /// says why nothing was resumed.
    pub(crate) fn resume_follow(
        &mut self,
        _: &ResumeFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        self.steered(cx);
        match self.following {
            Following::Paused { .. } => {
                self.following = Following::On;
                if let Some(newest) = self.flow.newest() {
                    self.follow_to(newest);
                }
                cx.notify();
            }
            Following::On => self.say("Follow is not paused", cx),
            Following::Off => self.say("Follow is off", cx),
        }
    }

    /// The reviewer is at work in the pane: an action of the reviewer, a mouse button or a
    /// turn of the wheel below the bar, or a key no action takes. Pauses follow from
    /// [`crate::Following::On`] while a turn of the workspace on screen runs; does nothing
    /// otherwise, from `Paused` included.
    pub(crate) fn touched(&mut self, cx: &mut Context<Self>) {
        if self.following != Following::On || !self.turn_runs(cx) {
            return;
        }
        self.following = Following::Paused { missed: 0 };
        cx.notify();
    }

    /// A fact of the Keeper, for follow: `Step`, `AgentsChanged` and `TurnEnded` of the
    /// workspace on screen. Called after the timeline and the live diff heard it.
    pub(crate) fn follow_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        match fact {
            Fact::Step {
                workspace,
                turn,
                number,
                ..
            } if self.on_screen(workspace, cx) => match self.following {
                Following::On => {
                    self.follow_to(FlowAt {
                        turn: turn.clone(),
                        number: *number,
                    });
                    cx.notify();
                }
                Following::Paused { missed } => {
                    self.following = Following::Paused { missed: missed + 1 };
                    cx.notify();
                }
                Following::Off => {}
            },
            Fact::AgentsChanged(agents) if self.on_screen(&agents.workspace, cx) => {
                self.follow_starts(agents, cx);
            }
            Fact::TurnEnded {
                workspace,
                state: Some(state),
                since,
                ..
            } if self.on_screen(workspace, cx) && self.view == View::Flow => {
                // The change slice 4's turn end builds: from the state last reviewed, or
                // from where the history starts before the first review.
                let from = since
                    .clone()
                    .or_else(|| {
                        self.tree
                            .history
                            .as_ref()
                            .and_then(|history| history.start.clone())
                    })
                    .or_else(|| {
                        self.changes
                            .as_ref()
                            .and_then(|changes| changes.counted_from.clone())
                    });
                self.follow.offer = Some(ChangeShown::Since {
                    from,
                    to: state.clone(),
                });
                cx.notify();
            }
            _ => {}
        }
    }

    /// The workspace on screen may be another: what follow kept of the one before goes (the
    /// turns met, a pause, the offer), and the turns the agents model shows for the
    /// workspace on screen now are met. Called at the end of `ReviewPane::new` and
    /// whenever the changes model changes.
    pub(crate) fn follow_workspace(&mut self, cx: &mut Context<Self>) {
        let now = self.models.changes.read(cx).workspace().cloned();
        if now != self.follow.workspace {
            self.follow.workspace = now;
            self.follow.met.clear();
            self.follow.offer = None;
            if matches!(self.following, Following::Paused { .. }) {
                self.following = Following::On;
            }
            cx.notify();
        }
        self.meet_turns(cx);
    }

    /// The agents model changed: every turn it shows for the workspace on screen is met,
    /// and starts nothing. Called at the end of `ReviewPane::new` and whenever the agents
    /// model changes, after `follow_told` heard the fact that changed it.
    pub(crate) fn follow_agents(&mut self, cx: &mut Context<Self>) {
        self.meet_turns(cx);
    }

    /// The lines of follow above Flow's list: `flow-follow-paused` with
    /// `flow-follow-resume`, and `flow-offer` with `flow-offer-take`.
    pub(crate) fn follow_lines(&mut self, cx: &mut Context<Self>) -> Vec<Div> {
        let theme = cx.theme();
        let line = || {
            div()
                .flex_none()
                .h(ROW_HEIGHT)
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .overflow_hidden()
                .whitespace_nowrap()
                .border_b_1()
                .border_color(theme.border)
        };
        let control = |label: &str| {
            div()
                .px_2()
                .rounded_sm()
                .cursor_pointer()
                .bg(theme.secondary)
                .text_color(theme.secondary_foreground)
                .child(label.to_owned())
        };
        let mut lines = Vec::new();
        if let Following::Paused { missed } = self.following {
            let plural = if missed == 1 { "" } else { "s" };
            // The words are in the selector: a test reads what was handed to the text
            // system, as it does for the entries of the timeline.
            let words = format!("Follow paused: {missed} step{plural} since");
            lines.push(
                line()
                    .debug_selector(|| "flow-follow-paused".to_owned())
                    .child(
                        div()
                            .debug_selector({
                                let words = words.clone();
                                move || format!("flow-follow-paused-text:{words}")
                            })
                            .text_color(theme.muted_foreground)
                            .child(words),
                    )
                    .child(
                        control("Resume")
                            .debug_selector(|| "flow-follow-resume".to_owned())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                                    pane.resume_follow(&ResumeFollow, window, cx);
                                }),
                            ),
                    ),
            );
        }
        if self.follow.offer.is_some() {
            lines.push(
                line()
                    .debug_selector(|| "flow-offer".to_owned())
                    .child("A turn ended: what changed since your last review")
                    .child(
                        control("Show")
                            .debug_selector(|| "flow-offer-take".to_owned())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                                    pane.acted(cx);
                                    pane.take_flow_offer(window, cx);
                                }),
                            ),
                    ),
            );
        }
        lines
    }

    /// What [`crate::TakeOffer`] does in Flow, and a click on `flow-offer-take`: shows
    /// Changes with what the turn end offered, taking slice 4's offer (`ReviewPane::take`)
    /// while the pane keeps the selection, asking nothing more otherwise. With nothing
    /// offered it says so in the notice.
    pub(crate) fn take_flow_offer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.offer.is_some() {
            // The pane kept the selection at the turn end: the change is selected now.
            self.take(window, cx);
        } else if self.follow.offer.is_none() {
            return self.say("No turn has ended while Flow was on screen", cx);
        }
        self.follow.offer = None;
        self.show_view(View::Changes, cx);
    }

    /// Whether `workspace` is the one on screen.
    fn on_screen(&self, workspace: &WorkspaceId, cx: &App) -> bool {
        self.models.changes.read(cx).workspace() == Some(workspace)
    }

    /// Whether a session of the workspace on screen holds a turn that runs.
    fn turn_runs(&self, cx: &App) -> bool {
        let Some(workspace) = self.models.changes.read(cx).workspace() else {
            return false;
        };
        self.models
            .agents
            .read(cx)
            .of(workspace)
            .is_some_and(|agents| agents.sessions.iter().any(|session| session.turn.is_some()))
    }

    /// Follow meets every turn the agents model shows for the workspace on screen.
    fn meet_turns(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.models.changes.read(cx).workspace().cloned() else {
            return;
        };
        let turns: Vec<TurnId> = self
            .models
            .agents
            .read(cx)
            .of(&workspace)
            .map(|agents| {
                agents
                    .sessions
                    .iter()
                    .filter_map(|session| session.turn.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.follow.met.extend(turns);
    }

    /// The agents of the workspace on screen changed: a turn follow has not met is a turn
    /// that starts, which a message of the user to the agent does. The offer of the turn
    /// before goes; with follow on or paused the pane shows Flow and follow is on, with
    /// nothing missed; with follow off nothing else changes.
    fn follow_starts(&mut self, agents: &WorkspaceAgents, cx: &mut Context<Self>) {
        let mut started = false;
        for turn in agents
            .sessions
            .iter()
            .filter_map(|session| session.turn.clone())
        {
            started |= self.follow.met.insert(turn);
        }
        if !started {
            return;
        }
        self.follow.offer = None;
        if self.following != Following::Off {
            self.following = Following::On;
            self.show_view(View::Flow, cx);
        }
        cx.notify();
    }

    /// Flow is at `at`, and the list of the form on screen is scrolled to its row, as far
    /// as Flow is on screen: `ReviewPane::at` is kept in Changes too, and Flow scrolls to
    /// it when it comes on screen. In the live diff a step whose file the list does not
    /// hold yet is scrolled to by the live diff when the list arrives.
    fn follow_to(&mut self, at: FlowAt) {
        self.at = Some(at.clone());
        if self.view != View::Flow {
            return;
        }
        match self.form {
            FlowForm::Timeline => {
                if let Some(row) = self.flow.row_of(&at) {
                    self.scroll_timeline_to(row);
                }
            }
            FlowForm::LiveDiff => {
                if let Some(row) = self.live_row_of(&at) {
                    self.scroll_live_to(row);
                }
            }
        }
    }
}
