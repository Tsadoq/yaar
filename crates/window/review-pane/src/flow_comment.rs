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
use yaar_diff::Side;
use yaar_link::{Anchor, CommentId, Lines, ThreadId};

use crate::saves::new_id;
use crate::threads::{link_side, path_in};
use crate::{CommentBox, FlowForm, Position, ReviewPane};

impl ReviewPane {
    /// What [`crate::Comment`] does in Flow: on the line of the live diff the reviewer is
    /// on, the comment box opens for the first comment of a new thread; in the timeline,
    /// the pane says that a comment is written on a line of the live diff.
    pub(crate) fn comment_in_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form != FlowForm::LiveDiff {
            return self.say("A comment is written on a line of the live diff", cx);
        }
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        let Some(position) = self.live.position().cloned() else {
            return self.say("The reviewer is on no line", cx);
        };
        let anchor = match self.live_anchor_at(&position) {
            Ok(anchor) => anchor,
            Err(why) => return self.say(why, cx),
        };
        let (Ok(thread), Ok(comment)) = (new_id(), new_id()) else {
            return self.say("An id for the comment could not be drawn", cx);
        };
        self.open_box(
            CommentBox {
                review,
                thread: ThreadId::new(thread),
                comment: CommentId::new(comment),
                anchor: Some(anchor),
                at: position,
                text: String::new(),
            },
            window,
            cx,
        );
    }

    /// Where a line of the live diff would be anchored, built as Changes builds the anchor
    /// of the same line (`ReviewPane::anchor_at`): the path the file has in the version of
    /// the line's side, the side, the line and its text, and the tree that side is read
    /// in. The new side is the files on disk, no tree, while the turn runs, and the state
    /// recorded at its end once it ended; the old side is the version the live diff
    /// counts from.
    fn live_anchor_at(&self, position: &Position) -> Result<Anchor, String> {
        let change = self.live.change().ok_or("The live diff shows no turn")?;
        let file = self
            .live
            .file(&position.path)
            .ok_or("The line is of a file the pane does not hold")?;
        let text = self
            .live
            .text(position)
            .ok_or("The text of the line is not here")?;
        let tree = match position.side {
            Side::Old => Some(
                change
                    .from
                    .clone()
                    .ok_or("There is no version the old lines could be counted from")?,
            ),
            Side::New => change.to.clone(),
        };
        let line = position.line + 1;
        Ok(Anchor {
            tree,
            path: path_in(file, position.side).to_owned(),
            side: link_side(position.side),
            lines: Lines {
                first: line,
                last: line,
            },
            text,
        })
    }
}
