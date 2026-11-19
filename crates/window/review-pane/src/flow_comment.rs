//! Comments written in Flow, on a line of the live diff.
//!
//! Built by the review pane's third package of slice 5 (row RP-14), on the live diff of the
//! second.
//!
//! `c` in Flow writes a comment on the line of the live diff the reviewer is on
//! (`ReviewPane::live_position`). The box is the pane's one comment box
//! (`ReviewPane::open_box`, with a `crate::CommentBox` whose `at` is that line); the draft
//! is saved as one written in Changes is, through the pane's saves, which make the rows
//! again (`ReviewPane::rebuild`, and so `ReviewPane::live_rebuild`). Its anchor is built as
//! Changes builds the anchor of a line of the same version: the line's path in the version
//! of its side (for the old side of a renamed file the path it had there, `Live::file`), its
//! side, its line and its text, and the tree of that side: for the new side the state the
//! turn ended with (`crate::LiveChange::to`), none while it runs (the files on disk); for
//! the old side the version the live diff counts from (`crate::LiveChange::from`), and where
//! that is none the pane says so in its notice and saves nothing, as Changes does. The
//! thread then shows on that line in Changes, since threads are placed by path, side and
//! line. In the timeline, `c` says in the notice that a comment is written on a line of
//! the live diff.

use gpui_kit::*;

use crate::ReviewPane;

impl ReviewPane {
    /// What [`crate::Comment`] does in Flow: on the line of the live diff the reviewer is
    /// on, the comment box opens for the first comment of a new thread; in the timeline,
    /// the pane says that a comment is written on a line of the live diff.
    pub(crate) fn comment_in_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = (window, cx);
        todo!("the third package of the review pane: a comment written in Flow")
    }
}
