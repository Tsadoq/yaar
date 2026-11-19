//! Changes holds still: no fact of the Keeper moves what the reviewer reads in Changes.
//!
//! Built by the package of slice 5 that holds row RP-13.
//!
//! A step, a turn's start and its end move Flow only; a turn end changes the selection of
//! Changes only as the reviewer's setting says (it selects what changed since the last
//! review, shown from its top, or keeps the selection and offers that change). What can
//! still move Changes is its rows made again under the reviewer: a reply in a thread, a
//! thread that moved to another line, a thread made or deleted in another Window, each
//! read with `GetReviews` after `Fact::ReviewsChanged`; and another review shown when a
//! session starts or stops running and so moves the review the pane picks
//! (`ReviewPane::review_moved`, after `Fact::AgentsChanged`).
//! [`ReviewPane::rebuild_steady`] makes the rows again so that the row at the top of the
//! view, and the line the reviewer is on,
//! stay where they were on screen: when rows come or go above the first row in view, the
//! view moves by as many, as a file that arrives above it does (`ReviewPane::loaded`).

use gpui_kit::*;

use crate::ReviewPane;

impl ReviewPane {
    /// Makes the rows of Changes again for reviews read anew, or for another review shown,
    /// keeping what is in view where it was.
    ///
    /// Filled by the package that holds RP-13; the contract's body makes the rows again as
    /// slice 4 did.
    pub(crate) fn rebuild_steady(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
    }
}
