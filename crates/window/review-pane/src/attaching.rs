//! Attaching the review shown, or the thread the reviewer is on alone, to the user's next
//! message to its agent; detaching it; and the offer to start an agent when the review's
//! agent does not run, or the review belongs to no agent. Built by the review pane's
//! package "attaching and the states of threads" of slice 6.
//!
//! [`crate::AttachReview`] and [`crate::AttachThread`] send `Request::AttachReview`, the
//! user's; the answer, `Reply::AttachedReview`, names the CLIs to offer, which the pane
//! shows under `review-offer-start` with one control `review-offer-start-<agent>` for each,
//! until one is taken, the review shown changes, the workspace changes, or what is attached
//! goes. Taking one, by its control or by [`crate::StartAgent`] (the first CLI offered;
//! with none offered the notice says so), asks the terminal pane, through the shared list
//! of terminals (`yaar_frame::TerminalList::want` with `yaar_frame::Wanted::AgentFor`), to
//! open that CLI's launcher for the review: no pane calls another.
//! [`crate::DetachReview`] sends `Request::DetachReview`. A refusal is said in the pane's
//! notice.
//!
//! What is attached of the review shown is drawn on the same line, from the review as
//! `Request::GetReviews` answered it (`yaar_link::Review::attached`), whatever attached it
//! and whenever: `review-attached:review` for the review, `review-attached:thread` for one
//! thread alone, nothing while nothing is. So a review attached before a restart shows as
//! attached when a Window opens; attaching it again gives the offer again.

use gpui_kit::*;
use yaar_link::AgentCli;

use crate::{AttachReview, AttachThread, DetachReview, ReviewPane, StartAgent};

/// What attaching keeps. Its fields are the package's to choose.
#[derive(Default)]
pub(crate) struct Attaching {}

impl ReviewPane {
    /// The CLIs the pane offers to start with the review attached, as the last attach
    /// answered; empty while none is offered.
    pub fn offer_to_start(&self) -> Vec<AgentCli> {
        let _ = &self.attaching;
        todo!()
    }

    /// The line of what is attached of the review shown and of the offer to start an
    /// agent, drawn above the tree of Changes; none while nothing is attached and nothing
    /// offered. Draws nothing until the package that builds it fills it in.
    pub(crate) fn render_offer_to_start(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let _ = cx;
        None
    }

    /// [`AttachReview`].
    pub(crate) fn attach_review(
        &mut self,
        _: &AttachReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`AttachThread`]: the thread of the line the reviewer is on; with none, the notice
    /// says so and nothing is sent.
    pub(crate) fn attach_thread(
        &mut self,
        _: &AttachThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`DetachReview`].
    pub(crate) fn detach_review(
        &mut self,
        _: &DetachReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`StartAgent`].
    pub(crate) fn start_agent(
        &mut self,
        _: &StartAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }
}
