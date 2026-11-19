//! The rounds of the review shown: each shown as sent until its agent's first edit, and
//! what changed since its comments were sent. Built by the review pane's package "rounds
//! and threads that moved" of slice 6.
//!
//! A line above the tree of Changes, `review-rounds`, names each round under
//! `review-round-<n>`, with `:sent` while it shows as sent and `:answered` once its agent
//! edited, and a control `review-round-<n>-since`, drawn once the round has its state, that
//! shows in Changes what changed since that round's comments were sent:
//! `ChangeShown::Since` from the round's state to the newest state of the history after
//! it, through the selection of the tree of Changes (`select_since` of `tree.rs`).
//! [`crate::SinceRound`] does the same from the keyboard: the first press for the newest
//! round with a state, each press after it for the round before, and after the first
//! round the newest again; with no round that has a state, the notice says so.

use gpui_kit::*;

use crate::{ReviewPane, SinceRound};

/// One round as the pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundShown {
    /// Its number.
    pub number: u32,
    /// Whether it shows as sent: its agent has not edited since the send.
    pub sent: bool,
    /// The state recorded at its send, by its full id; none until it is recorded.
    pub state: Option<String>,
}

impl ReviewPane {
    /// The rounds of the review shown, the first first.
    pub fn rounds_shown(&self) -> Vec<RoundShown> {
        todo!()
    }

    /// The line of the rounds, drawn above the tree of Changes; none for a review with no
    /// round. Draws nothing until the package that builds it fills it in.
    pub(crate) fn render_rounds(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let _ = cx;
        None
    }

    /// [`SinceRound`].
    pub(crate) fn since_round(
        &mut self,
        _: &SinceRound,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }
}
