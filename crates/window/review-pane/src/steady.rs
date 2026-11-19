//! Changes holds still: no fact of the Keeper moves what the reviewer reads in Changes.
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

use crate::{PaneRow, ReviewPane};

impl ReviewPane {
    /// Makes the rows of Changes again for reviews read anew, or for another review shown,
    /// keeping what is in view where it was.
    ///
    /// The row at the top of the view is found again among the new rows, and the view moves
    /// by as many rows as it moved in the list. A row is the same row when it is equal to
    /// the one before: a thread and its comments are told by their ids, a line of a diff
    /// by its path and its line numbers. When the top row is itself gone (a thread that was
    /// deleted), the next row in view that is still there takes its place.
    pub(crate) fn rebuild_steady(&mut self, cx: &mut Context<Self>) {
        let in_view = self.rows_in_view();
        let top: Vec<(usize, PaneRow)> = self
            .rows
            .iter()
            .enumerate()
            .take(in_view.end)
            .skip(in_view.start)
            .map(|(index, row)| (index, row.clone()))
            .collect();
        self.rebuild(cx);
        let moved = top.iter().find_map(|(was, row)| {
            let now = self.rows.iter().position(|held| held == row)?;
            Some(now as f32 - *was as f32)
        });
        if let Some(rows) = moved.filter(|rows| *rows != 0.) {
            self.shift_view(rows);
        }
    }
}
