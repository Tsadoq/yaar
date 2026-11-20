//! The read-only file view: a file reached through go to definition or a reference, shown
//! whole in the place of the rows, with a control that brings the reviewer back to the
//! place in the diff they left (FR-052, FR-053), and comments on any of its lines (FR-075).
//!
//! # Opening and going back
//!
//! A location is opened with `Navigation::open`, which reads its file. The pane then shows
//! the file view ([`FileShown`]) in the place of the rows and of Flow, under the selector
//! `review-file-view`, the reviewer on the location's line: each line under
//! `review-file-line-<n>`, `n` from 1, its text under `review-file-text-<n>:<text>`, the
//! reviewer's line marked `review-file-at`; its path above the lines under
//! `review-file-path:<path>`, relative to the worktree where the file lies in it; and the
//! control [`BACK`] under `review-file-back`, with the text selector
//! `review-file-back:<the label>`. The file view offers no way to edit (FR-053).
//!
//! The place left is kept when the first file view opens from the diff: the view (Changes
//! or Flow), the reviewer's position, the name and the scroll. [`crate::GoBack`]
//! (`escape`) and a click on the control close the file view, put that place back as it
//! was, and drop it. A definition or a reference followed from inside a file view opens
//! the next file view and keeps the place left in the diff: going back always returns to
//! the diff.
//!
//! In the file view `j`, `k` and the arrows move the reviewer's line within the file, the
//! name keys and the three asks work on its lines as on the diff's, the place asked being
//! the file's (`Tree::Worktree` and the relative path for [`yaar_navigator::FileKind::Worktree`],
//! `Tree::Version` for [`yaar_navigator::FileKind::Version`]; a file outside is asked
//! about with `Tree::Worktree` and its absolute path, which the Navigator answers by the
//! server of its language, or by text match).
//!
//! # Comments on its lines
//!
//! [`crate::Comment`] (`c`) on a line of the file view opens the comment box for a new
//! thread on that line, whether or not the line is in the diff (FR-075), with the anchor:
//!
//! | The file | `Anchor::tree` | `Anchor::path` | `Anchor::side` |
//! | --- | --- | --- | --- |
//! | in the worktree (`FileKind::Worktree`) | none: the files on disk | the relative path | new |
//! | in an export (`FileKind::Version`) | the version | the relative path | old |
//! | outside, or ignored by git (`FileKind::Outside`) | none | the absolute path | new |
//!
//! one line, its text the line's. The draft is saved as every draft is. A thread of the
//! review whose anchor names this file (by its relative path in the same version, or for a
//! file outside by its absolute path) is drawn on its line while that line's text is still
//! the anchor's text, as a thread is in the diff, under `review-thread-<id>`; else it stays
//! among the threads elsewhere, with its notice.

use gpui_kit::*;
use yaar_navigator::OpenedFile;

use crate::ReviewPane;

/// The label of the control that goes back to the diff.
pub const BACK: &str = "Back to the diff";

/// The file view the pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileShown {
    /// The file.
    pub file: OpenedFile,
    /// The line the reviewer is on, from 1.
    pub line: u32,
}

impl ReviewPane {
    /// The file view shown; none while the diff is.
    pub fn file_view(&self) -> Option<&FileShown> {
        todo!()
    }

    /// What [`crate::GoBack`] does.
    pub(crate) fn go_back(&mut self, _: &crate::GoBack, _: &mut Window, cx: &mut Context<Self>) {
        let _ = cx;
    }

    /// The file view, drawn in the place of the rows; none while the diff is shown.
    pub(crate) fn render_file_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let _ = (window, cx);
        None
    }

    /// [`crate::LineDown`] or [`crate::LineUp`] in the file view, `by` lines; true when
    /// the file view took it.
    pub(crate) fn file_view_step(&mut self, by: i64, cx: &mut Context<Self>) -> bool {
        let _ = (by, cx);
        false
    }

    /// [`crate::Comment`] in the file view; true when the file view took it.
    pub(crate) fn comment_in_file_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let _ = (window, cx);
        false
    }
}
