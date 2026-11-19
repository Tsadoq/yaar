//! How the live diff of Flow is drawn: one list of rows of one height in the element that
//! holds the pane's key context, as Changes is, with the line `flow-activity` above it
//! while the step Flow is at has no row in it, and `flow-empty` while no turn is known.
//!
//! Built by the review pane's second package of slice 5, with `live.rs`. Row `i` is drawn
//! under `flow-row-<i>`, and the text of a line under `flow-text-<i>-old:<text>` or
//! `flow-text-<i>-new:<text>`, as Changes draws them under `review-`; a click on a line puts
//! the reviewer there.

use gpui_kit::*;

use crate::ReviewPane;

impl ReviewPane {
    /// The live diff, as the list Flow shows in its second form.
    ///
    /// Filled by the second package of the review pane; the contract's body draws an empty
    /// element, so that Flow can show its second form before the live diff is built.
    pub(crate) fn render_live(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = (window, cx);
        div().into_any_element()
    }
}
