//! Attaching the review shown, or the thread the reviewer is on alone, to the user's next
//! message to its agent; detaching it; and the offer to start an agent when the review's
//! agent does not run, or the review belongs to no agent. Built by the review pane's
//! package "attaching and the states of threads" of slice 6.
//!
//! [`crate::AttachReview`] and [`crate::AttachThread`] send `Request::AttachReview`, the
//! user's; the answer, `Reply::AttachedReview`, names the CLIs to offer, which the pane
//! shows under `review-offer-start` with one control `review-offer-start-<agent>` for each,
//! until one is taken, the review shown changes, the workspace changes, what is attached
//! goes, or the review's session runs. Taking one, by its control or by
//! [`crate::StartAgent`] (the chosen CLI, the first offered until
//! [`crate::NextOfferedAgent`] chooses the next; with none offered the notice says so),
//! asks the terminal pane, through the shared list of terminals
//! (`yaar_frame::TerminalList::want` with `yaar_frame::Wanted::AgentFor`), to open that
//! CLI's launcher for the review: no pane calls another.
//! [`crate::DetachReview`] sends `Request::DetachReview`. A refusal is said in the pane's
//! notice.
//!
//! What is attached of the review shown is drawn on the same line, from the review as
//! `Request::GetReviews` answered it (`yaar_link::Review::attached`), whatever attached it
//! and whenever: `review-attached:review` for the review, `review-attached:thread` for one
//! thread alone, nothing while nothing is. So a review attached before a restart shows as
//! attached when a Window opens; attaching it again gives the offer again.

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

/// The CLIs one attach offered to start, and what they were offered for.
struct Offer {
    /// The workspace on screen when the review was attached.
    workspace: WorkspaceId,
    /// The review that was attached.
    review: ReviewId,
    /// The CLIs the Keeper named, in the order it gave them.
    start: Vec<AgentCli>,
    /// The place in `start` of the CLI Start Agent takes: the first until the reviewer
    /// chooses another.
    chosen: usize,
}

/// What attaching keeps: the offer the last attach was answered with.
#[derive(Default)]
pub(crate) struct Attaching {
    offer: Option<Offer>,
}

impl Attaching {
    /// The offer goes: the review it was made for is not the one shown any more, or the
    /// workspace is not on screen.
    pub(crate) fn forget(&mut self) {
        self.offer = None;
    }

    /// The reviews of the workspace were read anew: an offer for a review that is not
    /// attached in them goes, so that it does not come back if the review is attached again
    /// by another Window.
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
    /// The CLIs the pane offers to start with the review attached, as the last attach
    /// answered; empty while none is offered. The offer is of one review of one workspace:
    /// it is not offered with another review shown, in another workspace, or once nothing
    /// is attached of the review any more, or while the review's session runs: an agent
    /// started some other way is not started a second time.
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

    /// The line of what is attached of the review shown and of the offer to start an
    /// agent, drawn above the tree of Changes; none while nothing is attached and nothing
    /// offered.
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
                            // An underline over the control of the CLI Start Agent takes.
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

    /// Asks the Keeper to attach the review shown, or the one thread of it, and keeps what
    /// it offers to start when it answers.
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
                            chosen: 0,
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

    /// Asks the terminal pane, through the shared list of terminals, to open the launcher
    /// of the CLI `agent` in the workspace on screen for the review the offer was made
    /// for. The offer is taken: it goes.
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

    /// [`AttachReview`].
    pub(crate) fn attach_review(
        &mut self,
        _: &AttachReview,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.attach(None, cx);
    }

    /// [`AttachThread`]: the thread of the line the reviewer is on; with none, the notice
    /// says so and nothing is sent.
    pub(crate) fn attach_thread(
        &mut self,
        _: &AttachThread,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        // A thread whose comments are all sent has nothing to carry, but the Keeper says
        // so: a thread the agent has not answered may still hold a follow-up.
        let Some((_, thread)) = self.thread_to_act_on(
            |thread| !thread.resolved,
            "The thread on this line is resolved: reopen it to attach it",
            cx,
        ) else {
            return;
        };
        self.attach(Some(thread), cx);
    }

    /// [`DetachReview`].
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
        // The review is not attached any more once the Keeper says so, and the offer made
        // for it goes with it.
        self.attaching.offer = None;
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::DetachReview { review }, cx)
        });
        self.report(asked, cx);
    }

    /// The CLI of the offer that [`StartAgent`] starts: the first offered until
    /// [`NextOfferedAgent`] chooses another, drawn with `review-offer-chosen-<agent>` over
    /// its control; none while nothing is offered. A new offer chooses its first again.
    pub fn offer_chosen(&self) -> Option<AgentCli> {
        let offered = self.offer_to_start();
        let chosen = self.attaching.offer.as_ref()?.chosen;
        offered.into_iter().nth(chosen)
    }

    /// [`NextOfferedAgent`]: the next CLI of the offer is the chosen one, after the last the
    /// first; with none offered the notice says so.
    pub(crate) fn next_offered_agent(
        &mut self,
        _: &NextOfferedAgent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let offered = self.offer_to_start().len();
        match &mut self.attaching.offer {
            Some(offer) if offered > 0 => {
                offer.chosen = (offer.chosen + 1) % offered;
                cx.notify();
            }
            _ => self.say("No agent is offered to start: attach a review first", cx),
        }
    }

    /// [`StartAgent`]: the CLI chosen of the offer; with none offered the notice says so.
    pub(crate) fn start_agent(&mut self, _: &StartAgent, _: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        match self.offer_chosen() {
            Some(chosen) => self.start(&chosen.agent, cx),
            None => self.say("No agent is offered to start: attach a review first", cx),
        }
    }
}
