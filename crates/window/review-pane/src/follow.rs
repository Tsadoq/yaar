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
//! Flow as a write does: in the timeline to its entry, in the live diff to the heading of
//! the file it names when the live diff holds that file.
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
//! [`crate::ResumeFollow`] or a click on `flow-follow-resume`, puts Flow at the newest step
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

use gpui_kit::*;
use yaar_link::Fact;

use crate::{ChangeShown, ResumeFollow, ReviewPane, ToggleFollow};

/// What follow keeps beside `ReviewPane::following`. The fields are the package's to
/// choose.
pub(crate) struct Follow {}

impl Follow {
    /// Follow of a pane that just opened.
    pub(crate) fn new(cx: &mut Context<ReviewPane>) -> Self {
        let _ = cx;
        Self {}
    }

    /// What a turn end offered in Flow.
    pub(crate) fn offer(&self) -> Option<&ChangeShown> {
        todo!("the fourth package of the review pane: the offer at a turn end")
    }
}

impl ReviewPane {
    /// What [`ToggleFollow`] does: calls `steered` first, never `acted`.
    ///
    /// Filled by the fourth package of the review pane; the contract's body does nothing.
    pub(crate) fn toggle_follow(
        &mut self,
        _: &ToggleFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
    }

    /// What [`ResumeFollow`] does: calls `steered` first, never `acted`.
    ///
    /// Filled by the fourth package of the review pane; the contract's body does nothing.
    pub(crate) fn resume_follow(
        &mut self,
        _: &ResumeFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
    }

    /// The reviewer is at work in the pane: an action of the reviewer, a mouse button or a
    /// turn of the wheel below the bar, or a key no action takes. Pauses follow from
    /// [`crate::Following::On`] while a turn of the workspace on screen runs; does nothing
    /// otherwise, from `Paused` included.
    ///
    /// Filled by the fourth package of the review pane; the contract's body does nothing.
    pub(crate) fn touched(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// A fact of the Keeper, for follow: `Step`, `AgentsChanged` and `TurnEnded` of the
    /// workspace on screen. Called after the timeline and the live diff heard it.
    ///
    /// Filled by the fourth package of the review pane; the contract's body does nothing.
    pub(crate) fn follow_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        let _ = (fact, cx);
    }

    /// The workspace on screen may be another: what follow kept of the one before goes
    /// (the turns met, a pause, the offer), and the turns the agents model shows for the
    /// workspace on screen now are met. Called at the end of `ReviewPane::new` and
    /// whenever the changes model changes.
    ///
    /// Filled by the fourth package of the review pane; the contract's body does nothing.
    pub(crate) fn follow_workspace(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The agents model changed: every turn it shows for the workspace on screen is met,
    /// and starts nothing. Called at the end of `ReviewPane::new` and whenever the agents
    /// model changes, after `follow_told` heard the fact that changed it.
    ///
    /// Filled by the fourth package of the review pane; the contract's body does nothing.
    pub(crate) fn follow_agents(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The lines of follow above Flow's list: `flow-follow-paused` with
    /// `flow-follow-resume`, and `flow-offer` with `flow-offer-take`.
    ///
    /// Filled by the fourth package of the review pane; the contract's body draws none, so
    /// that Flow can be drawn before follow is built.
    pub(crate) fn follow_lines(&mut self, cx: &mut Context<Self>) -> Vec<Div> {
        let _ = cx;
        Vec::new()
    }

    /// What [`crate::TakeOffer`] does in Flow, and a click on `flow-offer-take`: shows
    /// Changes with what the turn end offered, taking slice 4's offer (`ReviewPane::take`)
    /// while the pane keeps the selection, asking nothing more otherwise. With nothing
    /// offered it says so in the notice.
    pub(crate) fn take_flow_offer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = (window, cx);
        todo!("the fourth package of the review pane: taking the offer of a turn end")
    }
}
