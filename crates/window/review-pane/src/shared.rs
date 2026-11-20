//! What a review shows of a change two sessions made, and what a turn's end selects. Built
//! by the review pane's package "credits in Changes" of slice 7.
//!
//! The end of a turn of a session selects what changed since the last review in the review
//! of that session, as slice 4 made it (`turns.rs`), but lists only the files that review
//! shows: the pane asks `yaar_link::Request::CompareForReview` with that review, in place
//! of `yaar_link::Request::CompareVersions`, and shows the files of its answer, those
//! credited to the session and those no agent made. The offer a turn's end makes while the
//! pane keeps the selection is taken the same way. A change the reviewer selects in the
//! tree, what changed since a round, and every selection of the review of no agent are
//! listed as before, by `CompareVersions`.
//!
//! A file of the change shown that a session other than the review's own is credited with
//! too, in the answer of `CompareForReview` or in the uncommitted changes the review lists
//! (`yaar_link::Request::GetUncommitted`), carries a mark on its heading, under the selector
//! `review-shared-<index of its row>`, with the words "also changed by another agent". The
//! file is shown whole, as every file is: the pane never divides the changes of one file.

use std::path::Path;

use gpui_kit::*;

use crate::ReviewPane;

impl ReviewPane {
    /// Whether the change of `path` in the change shown was made by a session other than the
    /// review's own too, as the credits of the change shown say: those of the answer of
    /// `CompareForReview` for a turn's end, those of the uncommitted changes the review
    /// lists for them. False for a file of a change listed with no credits.
    pub fn changed_by_another_too(&self, path: &Path) -> bool {
        let _ = path;
        todo!("whether another agent changed a file shown too")
    }

    /// The mark on the heading of the file of row `index`, `path`, while
    /// [`ReviewPane::changed_by_another_too`] holds for it. A hook of the contract that
    /// draws nothing.
    pub(crate) fn shared_mark(
        &self,
        index: usize,
        path: &Path,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let _ = (index, path, cx);
        None
    }
}
