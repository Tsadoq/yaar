//! A thread that moved with the code: where the Keeper says it sits now, the notice that
//! the code changed or was deleted, and the line it was written on, shown on request.
//! Built by the review pane's package "rounds and threads that moved" of slice 6.
//!
//! The rows of a thread follow `yaar_link::Thread::now` where it is some, else its anchor
//! (`threads.rs`). A thread with a notice draws it on its row under
//! `review-thread-notice-<thread id>:changed` or `:deleted`, with a control
//! `review-thread-original-<thread id>` that, as [`crate::ShowOriginal`] does on its line,
//! shows the anchor's text below the row under `review-thread-original-text-<thread id>`,
//! or hides it.

use std::collections::BTreeSet;

use gpui_kit::*;
use yaar_link::ThreadId;

use crate::{ReviewPane, ShowOriginal};

/// What the pane keeps of the threads that moved. Its fields are the package's to choose.
#[derive(Default)]
pub(crate) struct Moved {
    _shown: BTreeSet<ThreadId>,
}

impl ReviewPane {
    /// Whether the line `thread` was written on is shown below its row.
    pub fn shows_original(&self, thread: &ThreadId) -> bool {
        let _ = (thread, &self.moved);
        todo!()
    }

    /// The notice of a thread that moved, and the line it was written on while it is
    /// shown, for its row. Draws nothing until the package that builds it fills it in.
    pub(crate) fn thread_moved_marks(&self, thread: &ThreadId, cx: &App) -> Vec<AnyElement> {
        let _ = (thread, cx);
        Vec::new()
    }

    /// [`ShowOriginal`].
    pub(crate) fn show_original(
        &mut self,
        _: &ShowOriginal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }
}
