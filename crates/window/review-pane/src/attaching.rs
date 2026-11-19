//! Attaching the review shown, or the thread the reviewer is on alone, to the user's next
//! message to its agent; detaching it; and the offer to start an agent when the review's
//! agent does not run, or the review belongs to no agent. Built by the review pane's
//! package "attaching and the states of threads" of slice 6.
//!
//! [`crate::AttachReview`] and [`crate::AttachThread`] send `Request::AttachReview`, the
//! user's; the answer, `Reply::AttachedReview`, names the CLIs to offer, which the pane
//! shows under `review-offer-start` with one control `review-offer-start-<agent>` for each,
//! until one is taken, the review shown changes, the workspace changes, or what is attached
//! goes. Taking one asks the terminal pane, through the shared list of terminals
//! (`yaar_frame::TerminalList::want` with `yaar_frame::Wanted::AgentFor`), to open that
//! CLI's launcher for the review: no pane calls another. [`crate::DetachReview`] sends
//! `Request::DetachReview`. A refusal is said in the pane's notice.

use gpui_kit::*;
use yaar_link::AgentCli;

use crate::{AttachReview, AttachThread, DetachReview, ReviewPane};

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

    /// The line of the offer to start an agent, drawn above the tree of Changes; none while
    /// none is offered. Draws nothing until the package that builds it fills it in.
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
}
