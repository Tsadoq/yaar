//! Follow: Flow moves to each step of the agent as it comes, pauses when the reviewer
//! scrolls, clicks or types in the pane, counts the steps missed, and offers to resume; a
//! message of the user to the agent shows Flow; a turn that ends while Flow is on screen
//! offers to open in Changes what changed since the last review.
//!
//! Built by the review pane's third package of slice 5 (rows RP-08, RP-10, RP-11, RP-12).
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
//! Pausing. [`ReviewPane::touched`] is called by the pane's root element for every mouse
//! button pressed over the pane, every turn of the wheel over it, and every key typed while
//! the focus is in it, the comment box included, before any action runs. While follow is
//! on and a turn of the workspace on screen runs, it pauses follow with no step missed;
//! otherwise it does nothing. Each `Fact::Step` of the workspace on screen while paused
//! counts one more missed. The controls of follow act after the pause: turning follow off
//! leaves it off, and resuming, by [`crate::ResumeFollow`] or a click on
//! `flow-follow-resume`, puts Flow at the newest step (`Timeline::newest`), its row in
//! view, and follow on again.
//!
//! A message of the user to the agent. A `Fact::AgentsChanged` of the workspace on screen
//! in which a session holds a turn it did not hold in the last such fact (`Session::turn`)
//! is the start of a turn, which the user's message to the agent starts. With follow on or
//! paused, the pane shows Flow (`ReviewPane::show_view`), follow is on with no step missed,
//! and Flow is at the turn's first step once it comes. With follow off, nothing changes.
//! The agents a Window first reads when it connects start no turn.
//!
//! A turn's end. A `Fact::TurnEnded` of the workspace on screen with a state, while Flow
//! is on screen, leaves Flow on screen and offers what changed since the last review:
//! `Since { from: since or the history's start, to: state }`, drawn under `flow-offer`
//! with the control `flow-offer-take`. Taking it, by the control or [`crate::TakeOffer`],
//! shows Changes with that change selected, as a turn end selects it by default. The offer
//! goes when it is taken, when the reviewer shows Changes otherwise, and when another turn
//! starts.

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
        todo!("the third package of the review pane: the offer at a turn end")
    }
}

impl ReviewPane {
    /// What [`ToggleFollow`] does.
    ///
    /// Filled by the third package of the review pane; the contract's body does nothing.
    pub(crate) fn toggle_follow(
        &mut self,
        _: &ToggleFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
    }

    /// What [`ResumeFollow`] does.
    ///
    /// Filled by the third package of the review pane; the contract's body does nothing.
    pub(crate) fn resume_follow(
        &mut self,
        _: &ResumeFollow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
    }

    /// The reviewer pressed a mouse button over the pane, turned the wheel over it, or typed
    /// a key in it.
    ///
    /// Filled by the third package of the review pane; the contract's body does nothing.
    pub(crate) fn touched(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// A fact of the Keeper, for follow: `Step`, `AgentsChanged` and `TurnEnded` of the
    /// workspace on screen. Called after the timeline and the live diff heard it.
    ///
    /// Filled by the third package of the review pane; the contract's body does nothing.
    pub(crate) fn follow_told(&mut self, fact: &Fact, cx: &mut Context<Self>) {
        let _ = (fact, cx);
    }

    /// The workspace on screen may be another: what follow kept of the one before goes.
    ///
    /// Filled by the third package of the review pane; the contract's body does nothing.
    pub(crate) fn follow_workspace(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The lines of follow above Flow's list: `flow-follow-paused` with
    /// `flow-follow-resume`, and `flow-offer` with `flow-offer-take`.
    ///
    /// Filled by the third package of the review pane; the contract's body draws none, so
    /// that Flow can be drawn before follow is built.
    pub(crate) fn follow_lines(&mut self, cx: &mut Context<Self>) -> Vec<Div> {
        let _ = cx;
        Vec::new()
    }

    /// What [`crate::TakeOffer`] does in Flow: opens in Changes what the turn end offered.
    pub(crate) fn take_flow_offer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = (window, cx);
        todo!("the third package of the review pane: taking the offer of a turn end")
    }
}
