//! Keeps the diff and the line the reviewer is on where they were on screen when the rows are made
//! again.

use gpui_kit::*;

use crate::{PaneRow, ReviewPane};

impl ReviewPane {
    /// Makes the rows again, then places the first surviving row in view (preferring a diff row
    /// over a thread or comment) back at its exact vertical offset from the top of the viewport.
    pub(crate) fn rebuild_steady(&mut self, cx: &mut Context<Self>) {
        let in_view = self.rows_in_view();
        let top: Vec<(PaneRow, Option<Pixels>)> = self
            .rows
            .iter()
            .enumerate()
            .take(in_view.end)
            .skip(in_view.start)
            .map(|(index, row)| (row.clone(), self.row_offset_from_top(index)))
            .collect();
        self.rebuild(cx);
        // A thread or comment keeps the view only when no diff row is in view, so a moving thread
        // does not scroll the diff.
        let diff_first = top
            .iter()
            .filter(|(row, _)| !matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }));
        let thread_first = top
            .iter()
            .filter(|(row, _)| matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }));
        let anchor = diff_first.chain(thread_first).find_map(|(row, delta)| {
            let delta = (*delta)?;
            let now = self.rows.iter().position(|held| held == row)?;
            Some((now, delta))
        });
        if let Some((now, delta)) = anchor {
            self.scroll_row_to_offset(now, delta);
        }
    }
}
