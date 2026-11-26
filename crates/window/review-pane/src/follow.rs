//! Follow: Flow moves to each step as it comes and pauses when the reviewer acts (`touched`); the
//! bar and the view/follow controls call `steered` and never pause. The fact of a new turn is heard
//! before the agents model's observers run, so `follow_told` meets it first.

use std::collections::HashSet;

use gpui_kit::*;
use yaar_link::{Fact, FlowForm, Step, TurnId, WorkspaceAgents, WorkspaceId};
use yaar_ui::metrics::{SPACE_4, SPACE_6};
use yaar_ui::pieces::{Button, ButtonKind, Card, Pill};

use crate::{ChangeShown, FlowAt, Following, ReviewPane, ToggleFollow, View};

pub(crate) struct Follow {
    workspace: Option<WorkspaceId>,
    met: HashSet<TurnId>,
    offer: Option<ChangeShown>,
}

impl Follow {
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        // The offer is for the reviewer in Flow: whatever shows Changes takes it away.
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

    pub(crate) fn offer(&self) -> Option<&ChangeShown> {
        self.offer.as_ref()
    }
}

impl ReviewPane {
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

    pub(crate) fn resume_follow(&mut self, cx: &mut Context<Self>) {
        self.steered(cx);
        match self.following {
            Following::Paused { .. } => {
                self.following = Following::On;
                if let Some(newest) = self.flow.newest() {
                    self.follow_to(newest, cx);
                }
                cx.notify();
            }
            Following::On => self.say("Follow is not paused", cx),
            Following::Off => self.say("Follow is off", cx),
        }
    }

    pub(crate) fn touched(&mut self, cx: &mut Context<Self>) {
        if self.following != Following::On || !self.turn_runs(cx) {
            return;
        }
        self.following = Following::Paused { missed: 0 };
        cx.notify();
    }

    pub(crate) fn follow_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        match fact {
            Fact::Step {
                workspace,
                turn,
                number,
                ..
            } if self.on_screen(workspace, cx) => match self.following {
                Following::On => {
                    self.follow_to(
                        FlowAt {
                            turn: turn.clone(),
                            number: *number,
                        },
                        cx,
                    );
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

    pub(crate) fn follow_workspace(&mut self, cx: &mut Context<Self>) {
        let now = self.models.changes.read(cx).workspace().cloned();
        if now != self.follow.workspace {
            self.follow.workspace = now;
            self.follow.met.clear();
            self.follow.offer = None;
            self.attaching.forget();
            if matches!(self.following, Following::Paused { .. }) {
                self.following = Following::On;
            }
            cx.notify();
        }
        self.meet_turns(cx);
    }

    pub(crate) fn follow_agents(&mut self, cx: &mut Context<Self>) {
        self.meet_turns(cx);
    }

    pub(crate) fn follow_paused_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Following::Paused { missed } = self.following else {
            return None;
        };
        let plural = if missed == 1 { "" } else { "s" };
        let words = format!("Follow paused: {missed} step{plural} since");
        let pill = Pill::new("flow-follow-paused", words.clone())
            .selector("flow-follow-paused")
            .text_selector(format!("flow-follow-paused-text:{words}"))
            .action(
                Button::new("flow-follow-resume", "Resume")
                    .kind(ButtonKind::Primary)
                    .small()
                    .selector("flow-follow-resume")
                    .on_click(cx.listener(|pane, _: &ClickEvent, _, cx| {
                        pane.resume_follow(cx);
                    })),
            );
        Some(
            div()
                .absolute()
                .bottom(px(14.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(pill)
                .into_any_element(),
        )
    }

    pub(crate) fn follow_offer_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.follow.offer.as_ref()?;
        Some(
            div()
                .flex_none()
                .px(SPACE_6)
                .py(SPACE_4)
                .child(
                    Card::new()
                        .dashed()
                        .selector("flow-offer")
                        .child("A turn ended: what changed since your last review")
                        .child(
                            Button::new("flow-offer-take", "Show")
                                .kind(ButtonKind::Primary)
                                .small()
                                .selector("flow-offer-take")
                                .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                                    pane.acted(cx);
                                    pane.take_flow_offer(window, cx);
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn take_flow_offer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.offer.is_some() {
            self.take(window, cx);
        } else if self.follow.offer.is_none() {
            return self.say("No turn has ended while Flow was on screen", cx);
        }
        self.follow.offer = None;
        self.show_view(View::Changes, cx);
    }

    fn on_screen(&self, workspace: &WorkspaceId, cx: &App) -> bool {
        self.models.changes.read(cx).workspace() == Some(workspace)
    }

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

    /// A turn not met before that a message of the user started shows Flow with follow on, unless
    /// follow is off. A turn the CLI started itself is only met: the user did not ask for it.
    fn follow_starts(&mut self, agents: &WorkspaceAgents, cx: &mut Context<Self>) {
        let mut started = false;
        for session in &agents.sessions {
            let Some(turn) = session.turn.clone() else {
                continue;
            };
            started |= self.follow.met.insert(turn) && session.turn_from_user;
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

    /// `ReviewPane::at` is kept in Changes too, and Flow scrolls to it when it comes on screen.
    fn follow_to(&mut self, at: FlowAt, cx: &mut Context<Self>) {
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
            FlowForm::LiveDiff => match self.flow.step(&at).cloned() {
                Some(Step::Read { path, lines }) => {
                    self.live_open_read(path, lines, cx);
                    if let Some(row) = self.live_row_of(&at) {
                        self.scroll_live_to(row);
                    } else {
                        self.live.mark_scrolled(&at);
                    }
                }
                Some(Step::Edit { .. } | Step::Write { .. }) => {
                    self.live_clear_read(cx);
                    if let Some(row) = self.live_row_of(&at) {
                        self.scroll_live_to(row);
                    }
                }
                _ => {
                    if let Some(row) = self.live_row_of(&at) {
                        self.scroll_live_to(row);
                    }
                }
            },
        }
    }
}
