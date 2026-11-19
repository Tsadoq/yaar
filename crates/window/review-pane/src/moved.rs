//! A thread that moved with the code: where the Keeper says it sits now, the notice that
//! the code changed or was deleted, and the line it was written on, shown on request.
//! Built by the review pane's package "rounds and threads that moved" of slice 6.
//!
//! Where a thread's rows go depends on the version the pane shows on the new side
//! (`threads.rs`). `yaar_link::Thread::now` names a version, `now.tree`, and lines in it:
//! the thread is placed on those lines when the new side shown is `now.tree`, or the files
//! on disk, which the Keeper carries every thread toward at each state it records. With no
//! `now`, it is placed on its anchor, as before slice 6. With a `now`, it is placed on its
//! anchor only when the new side shown is the anchor's own version (`anchor.tree`). In any
//! other version, an older turn or a commit chosen in the tree of Changes, it is listed
//! under `ThreadsElsewhere`, as a thread on a file not shown is: it is never drawn on lines
//! of a version it was not carried to.
//!
//! A thread with a notice draws it on its row under
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
