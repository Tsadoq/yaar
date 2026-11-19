//! The state of each thread on its row, and what only the reviewer does to threads:
//! resolving one, reopening one, and resolving every addressed one at once. Built by the
//! review pane's package "attaching and the states of threads" of slice 6.
//!
//! Each actions sends its request, the user's: `Request::ResolveThread`,
//! `Request::ReopenThread`, `Request::ResolveAddressed`, for the thread on the line the
//! reviewer is on, or the review shown. The states come from the Keeper with the reviews,
//! read again at `Fact::ReviewsChanged`.

use gpui_kit::*;
use yaar_link::ThreadId;

use crate::{ReopenThread, ResolveAddressed, ResolveThread, ReviewPane};

impl ReviewPane {
    /// The mark of a thread's state on its row, under the selector
    /// `review-thread-state-<thread id>:<state>` (see [`ReviewPane::thread_states`]). Draws
    /// nothing until the package that builds it fills it in.
    pub(crate) fn thread_state_mark(&self, thread: &ThreadId, cx: &App) -> Option<AnyElement> {
        let _ = (thread, cx);
        None
    }

    /// [`ResolveThread`].
    pub(crate) fn resolve_thread(
        &mut self,
        _: &ResolveThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`ReopenThread`].
    pub(crate) fn reopen_thread(
        &mut self,
        _: &ReopenThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }

    /// [`ResolveAddressed`].
    pub(crate) fn resolve_addressed(
        &mut self,
        _: &ResolveAddressed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = (window, cx);
        // Does nothing until the package that builds it fills it in, so that the walks
        // of the keys of slices 1 to 5 can press it.
    }
}
