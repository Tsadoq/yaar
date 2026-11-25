//! The mark on a file of the change shown that a session other than the review's own changed
//! too. The file is shown whole: the pane never divides the changes of one file.

use std::path::Path;

use gpui_kit::*;
use yaar_ui::pieces::{Chip, Tone};

use crate::{ChangeShown, ReviewPane};

impl ReviewPane {
    /// False for a change listed with no credits, and for the review of no agent, which has no
    /// session.
    pub fn changed_by_another_too(&self, path: &Path) -> bool {
        let Some(own) = self.review_session() else {
            return false;
        };
        let credited = match self.tree.shown {
            ChangeShown::Uncommitted => self.tree.uncommitted.as_deref(),
            // Credits made for another review say nothing about this one: `own` would be read
            // against another session's list.
            ChangeShown::Since { .. } => self
                .tree
                .credits
                .as_ref()
                .filter(|credits| Some(&credits.review) == self.review().map(|review| &review.id))
                .map(|credits| credits.files.as_slice()),
            _ => None,
        };
        credited.unwrap_or_default().iter().any(|credited| {
            credited.file.path == path
                && credited.by.contains(own)
                && credited.by.iter().any(|session| session != own)
        })
    }

    pub(crate) fn shared_mark(
        &self,
        index: usize,
        path: &Path,
        _cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.changed_by_another_too(path).then(|| {
            Chip::new("also changed by another agent")
                .tone(Tone::Warn)
                .selector(format!("review-shared-{index}"))
                .into_any_element()
        })
    }
}
