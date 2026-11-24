//! Comments written in Flow on a live-diff line. The new side's tree is the turn's end state (none
//! while it runs); the old side's is the version the live diff counts from, and with none nothing
//! is saved.

use gpui_kit::*;
use yaar_diff::Side;
use yaar_link::{Anchor, CommentId, Lines, ThreadId};

use crate::saves::new_id;
use crate::threads::{link_side, path_in};
use crate::{CommentBox, FlowForm, Position, ReviewPane};

impl ReviewPane {
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
