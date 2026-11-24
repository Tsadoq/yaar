use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yaar_frame::{AskError, Wanted};
use yaar_link::{
    AgentCli, Attachment, Refusal, Reply, Request, ReviewId, SessionState, ThreadId, WorkspaceId,
    WorkspaceReviews,
};

use crate::actions::words;
use crate::draw::ROW_HEIGHT;
use crate::{AttachReview, AttachThread, DetachReview, NextOfferedAgent, ReviewPane, StartAgent};

struct Offer {
    workspace: WorkspaceId,
    review: ReviewId,
    start: Vec<AgentCli>,
}

pub(crate) struct Attaching {
    offer: Option<Offer>,
    window_choice: Option<AgentCli>,
}

impl Attaching {
    pub(crate) fn new(window_choice: Option<AgentCli>) -> Self {
        Self {
            offer: None,
            window_choice,
        }
    }

    pub(crate) fn forget(&mut self) {
        self.offer = None;
    }

    /// An offer for a review no longer attached goes, so it does not return when another Window
    /// attaches it.
    pub(crate) fn reviews_read(&mut self, reviews: &WorkspaceReviews) {
        let gone = self.offer.as_ref().is_some_and(|offer| {
            offer.workspace == reviews.workspace
                && !reviews
                    .reviews
                    .iter()
                    .any(|review| review.id == offer.review && review.attached.is_some())
        });
        if gone {
            self.offer = None;
        }
    }
}

impl ReviewPane {
    /// Empty while the review's session runs: an agent started some other way is not started twice.
    pub fn offer_to_start(&self) -> Vec<AgentCli> {
        let Some(offer) = &self.attaching.offer else {
            return Vec::new();
        };
        let runs = self.sessions.iter().any(|session| {
            session.review.as_ref() == Some(&offer.review)
                && session.state != SessionState::NotRunning
        });
        let kept = !runs
            && self.reviews_of.as_ref() == Some(&offer.workspace)
            && self
                .review()
                .is_some_and(|review| review.id == offer.review && review.attached.is_some());
        if kept {
            offer.start.clone()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn render_offer_to_start(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let attached = self.review().and_then(|review| review.attached.as_ref());
        let offered = self.offer_to_start();
        if attached.is_none() && offered.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let mut line = div()
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
            .border_color(theme.border);
        line = match attached {
            Some(Attachment::Review) => line.child(
                div()
                    .debug_selector(|| "review-attached:review".to_owned())
                    .text_color(theme.warning)
                    .child("Attached to your next message: the review"),
            ),
            Some(Attachment::Thread(_)) => line.child(
                div()
                    .debug_selector(|| "review-attached:thread".to_owned())
                    .text_color(theme.warning)
                    .child("Attached to your next message: one thread"),
            ),
            None => line,
        };
        if !offered.is_empty() {
            let mut offer = div()
                .debug_selector(|| "review-offer-start".to_owned())
                .flex()
                .gap_2()
                .child("The agent is not running:");
            let chosen = self.offer_chosen();
            for cli in offered {
                let agent = cli.agent.clone();
                let mark = chosen.as_ref() == Some(&cli);
                offer = offer.child(
                    div()
                        .debug_selector({
                            let agent = agent.clone();
                            move || format!("review-offer-start-{agent}")
                        })
                        .relative()
                        .px_2()
                        .bg(theme.secondary)
                        .text_color(theme.secondary_foreground)
                        .child(format!("Start {}", cli.title))
                        .when(mark, |control| {
                            let agent = agent.clone();
                            control.child(
                                div()
                                    .debug_selector(move || format!("review-offer-chosen-{agent}"))
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .w_full()
                                    .h(px(2.))
                                    .bg(theme.primary),
                            )
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                                pane.acted(cx);
                                pane.start(&agent, cx);
                            }),
                        ),
                );
            }
            line = line.child(offer);
        }
        Some(line.into_any_element())
    }

    fn attach(&mut self, thread: Option<ThreadId>, cx: &mut Context<Self>) {
        let (Some(workspace), Some(review)) = (
            self.reviews_of.clone(),
            self.review().map(|review| review.id.clone()),
        ) else {
            return self.say("The reviews are not here yet", cx);
        };
        let alone = thread.is_some();
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::AttachReview {
                    review: review.clone(),
                    thread,
                },
                cx,
            )
        });
        cx.spawn(async move |pane, cx| {
            let said = match asked.await {
                Ok(Reply::AttachedReview { start }) => {
                    pane.update(cx, |pane, cx| {
                        pane.attaching.offer = Some(Offer {
                            workspace,
                            review,
                            start,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Ok(_) => AskError::Unexpected.to_string(),
                Err(AskError::Refused(Refusal::NothingToSend { .. })) => match alone {
                    true => "The thread has no unsent comment to attach",
                    false => "There is no unsent comment on an unresolved thread to attach",
                }
                .to_owned(),
                Err(error) => words(&error),
            };
            pane.update(cx, |pane, cx| pane.say(said, cx)).ok();
        })
        .detach();
    }

    fn start(&mut self, agent: &str, cx: &mut Context<Self>) {
        let Some(offer) = self.attaching.offer.take() else {
            return;
        };
        let wanted = Wanted::AgentFor {
            workspace: offer.workspace,
            agent: agent.to_owned(),
            review: offer.review,
        };
        self.models
            .terminals
            .update(cx, |terminals, cx| terminals.want(wanted, cx));
        cx.notify();
    }

    pub(crate) fn attach_review(
        &mut self,
        _: &AttachReview,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.attach(None, cx);
    }

    pub(crate) fn attach_thread(
        &mut self,
        _: &AttachThread,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        // A thread whose comments are all sent is still asked for, and the Keeper decides: a thread
        // the agent has not answered may still hold a follow-up.
        let Some((_, thread)) = self.thread_to_act_on(
            |thread| !thread.resolved,
            "The thread on this line is resolved: reopen it to attach it",
            cx,
        ) else {
            return;
        };
        self.attach(Some(thread), cx);
    }

    pub(crate) fn detach_review(
        &mut self,
        _: &DetachReview,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        self.attaching.offer = None;
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::DetachReview { review }, cx)
        });
        self.report(asked, cx);
    }

    /// The Window's choice, else the first CLI offered.
    pub fn offer_chosen(&self) -> Option<AgentCli> {
        let offered = self.offer_to_start();
        match &self.attaching.window_choice {
            Some(chosen) if offered.contains(chosen) => Some(chosen.clone()),
            _ => offered.into_iter().next(),
        }
    }

    pub(crate) fn cli_chosen(&mut self, cx: &mut Context<Self>) {
        let now = self.models.agents.read(cx).chosen_cli().cloned();
        if now != self.attaching.window_choice {
            self.attaching.window_choice = now;
            cx.notify();
        }
    }

    pub(crate) fn next_offered_agent(
        &mut self,
        _: &NextOfferedAgent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let offered = self.offer_to_start();
        let Some(current) = self.offer_chosen() else {
            return self.say("No agent is offered to start: attach a review first", cx);
        };
        if offered.len() < 2 {
            return;
        }
        // One step per CLI is a whole turn: the loop ends there when the offer holds only the
        // chosen one.
        let steps = self.models.agents.read(cx).clis().len();
        for _ in 0..steps {
            let landed = self.models.agents.update(cx, |agents, cx| {
                agents.choose_next_cli(cx);
                agents.chosen_cli().cloned()
            });
            if landed.is_some_and(|landed| landed != current && offered.contains(&landed)) {
                break;
            }
        }
    }

    pub(crate) fn start_agent(&mut self, _: &StartAgent, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        match self.offer_chosen() {
            Some(chosen) => self.start(&chosen.agent, cx),
            None => self.say("No agent is offered to start: attach a review first", cx),
        }
    }
}
