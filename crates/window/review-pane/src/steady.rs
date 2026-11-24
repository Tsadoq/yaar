//! Keeps the diff and the line the reviewer is on where they were on screen when the rows are made
//! again.

use gpui_kit::*;

use crate::{PaneRow, ReviewPane};

impl ReviewPane {
    /// Makes the rows again, then moves the view by as many rows as the first row in view that is
    /// not a thread or comment moved. Rows match by ids, or by path and line numbers; when that
    /// row is gone, the next one in view that remains is used.
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
        // A thread or comment keeps the view only when no diff row is in view, so a moving thread
        // does not scroll the diff.
        let diff_first = top
            .iter()
            .filter(|(_, row)| !matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }));
        let thread_first = top
            .iter()
            .filter(|(_, row)| matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }));
        let moved = diff_first.chain(thread_first).find_map(|(was, row)| {
            let now = self.rows.iter().position(|held| held == row)?;
            Some(now as f32 - *was as f32)
        });
        if let Some(rows) = moved.filter(|rows| *rows != 0.) {
            self.shift_view(rows);
        }
    }
}
