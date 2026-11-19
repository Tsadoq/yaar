//! How Flow is drawn, and the bar at the top of the pane that names both views.
//!
//! Built by the review pane's first package of slice 5, with `flow.rs`.
//!
//! The bar, `review-bar`, is drawn in both views, one row high, outside the element that
//! holds the pane's key context: `review-view-changes` and `review-view-flow` name the two
//! views, the one on screen marked, and a click on either shows it; `flow-follow` is the
//! control of follow, which says whether follow is on, paused or off and turns it on or
//! off on a click; in Flow, `flow-form-timeline` and `flow-form-live` name the two forms,
//! the one shown marked, and a click on the other switches to it, as
//! [`crate::ToggleFlowForm`] does. A click on `flow-follow` runs what
//! [`crate::ToggleFollow`] runs (`ReviewPane::toggle_follow`). No click on the bar calls
//! `acted`: the bar's controls call `steered`, and the bar lies outside the element whose
//! listeners pause follow. A mouse down anywhere on the bar puts the focus on the pane's
//! rows, so that the pane's keys work after a click on it.
//!
//! Flow itself is one list in the element that holds the pane's key context, as the rows
//! of Changes are: the timeline, one entry a row, drawn under `flow-entry-<i>`, the entry
//! of the reviewer marked; or, in the second form, an element `flow-live` that fills the
//! same place and holds the live diff, which `live_draw.rs` draws in it
//! ([`ReviewPane::render_live`]); no `flow-entry-<i>` and no `flow-empty` of the timeline is
//! drawn then. Above the list
//! go the lines of follow, `follow.rs`'s to draw. With no turn in the timeline, the list is
//! one line, `flow-empty`, that says so.

use gpui_kit::*;

use crate::ReviewPane;

impl ReviewPane {
    /// The bar at the top of the pane, in both views.
    ///
    /// Filled by the first package of the review pane; the contract's body draws no bar.
    pub(crate) fn render_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let _ = (window, cx);
        None
    }

    /// Flow, in the form on screen, with the lines of follow above it: the timeline, or
    /// `flow-live` around [`ReviewPane::render_live`].
    pub(crate) fn render_flow(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = (window, cx);
        todo!("the first package of the review pane: drawing Flow")
    }
}
