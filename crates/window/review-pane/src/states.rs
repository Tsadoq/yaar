//! The state of each thread on its row, and what only the reviewer does to threads:
//! resolving one, reopening one, and resolving every addressed one at once. Built by the
//! review pane's package "attaching and the states of threads" of slice 6.
//!
//! Each action sends its request, the user's: `Request::ResolveThread`,
//! `Request::ReopenThread`, `Request::ResolveAddressed`, for the thread on the line the
//! reviewer is on, or the review shown. The states come from the Keeper with the reviews,
//! read again at `Fact::ReviewsChanged`.
//!
//! The thread on the reviewer's line is the thread whose row follows the row of that line
//! (`ReviewPane::threads_on_the_line`), wherever the Keeper carried it to: a thread that
//! moved is on the line it was carried to. With several, the first that the action applies
//! to is the one acted on; with none the notice says why, and nothing is sent.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{Request, ReviewId, Thread, ThreadId, ThreadState};

use crate::{PaneRow, ReopenThread, ResolveAddressed, ResolveThread, ReviewPane, View};

/// How a state is written: in the selector of its mark, and in words for the reviewer.
fn written(state: ThreadState) -> (&'static str, &'static str) {
    match state {
        ThreadState::Draft => ("draft", "Draft"),
        ThreadState::Sent => ("sent", "Sent"),
        ThreadState::Addressed => ("addressed", "Addressed"),
        ThreadState::WaitsForReviewer => ("waiting", "Waiting for you"),
        ThreadState::Resolved => ("resolved", "Resolved"),
    }
}

impl ReviewPane {
    /// The mark of a thread's state on its row, under the selector
    /// `review-thread-state-<thread id>:<state>` (see [`ReviewPane::thread_states`]).
    pub(crate) fn thread_state_mark(&self, thread: &ThreadId, cx: &App) -> Option<AnyElement> {
        let held = self
            .review()?
            .threads
            .iter()
            .find(|held| held.id == *thread)?;
        let (name, words) = written(held.state);
        let theme = cx.theme();
        let color = match held.state {
            ThreadState::Draft | ThreadState::WaitsForReviewer => theme.warning,
            ThreadState::Addressed | ThreadState::Resolved => theme.success,
            ThreadState::Sent => theme.muted_foreground,
        };
        let id = held.id.clone();
        Some(
            div()
                .debug_selector(move || format!("review-thread-state-{id}:{name}"))
                .flex_none()
                .ml_2()
                .text_color(color)
                .child(words)
                .into_any_element(),
        )
    }

    /// The threads whose rows follow the row of the line the reviewer is on, in the order
    /// of the rows. None while the reviewer is on no line, and for a thread that sits
    /// under the heading of the threads on no line.
    pub(crate) fn threads_on_the_line(&self) -> Vec<&Thread> {
        let (Some(row), Some(review)) = (self.position_row(), self.review()) else {
            return Vec::new();
        };
        self.rows()
            .iter()
            .skip(row + 1)
            .take_while(|row| matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }))
            .filter_map(|row| match row {
                PaneRow::Thread { thread } => review.threads.iter().find(|held| held.id == *thread),
                _ => None,
            })
            .collect()
    }

    /// The review shown and the first thread on the reviewer's line that `applies`, for an
    /// action on a thread. When there is none, says in the notice why, `otherwise` being
    /// what to say when threads are on the line and none applies, and gives nothing.
    pub(crate) fn thread_to_act_on(
        &mut self,
        applies: impl Fn(&Thread) -> bool,
        otherwise: &str,
        cx: &mut Context<Self>,
    ) -> Option<(ReviewId, ThreadId)> {
        // The reviewer's line in Flow is one of the live diff, which draws no threads.
        if self.view == View::Flow {
            self.say("Changes shows the threads", cx);
            return None;
        }
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            self.say("The reviews are not here yet", cx);
            return None;
        };
        let on_the_line = self.threads_on_the_line();
        let found = on_the_line
            .iter()
            .find(|thread| applies(thread))
            .map(|thread| thread.id.clone());
        let none_there = on_the_line.is_empty();
        match found {
            Some(thread) => Some((review, thread)),
            None => {
                let words = if none_there {
                    "There is no thread on this line"
                } else {
                    otherwise
                };
                self.say(words, cx);
                None
            }
        }
    }

    /// [`ResolveThread`]: the thread on the line the reviewer is on that is not resolved.
    pub(crate) fn resolve_thread(
        &mut self,
        _: &ResolveThread,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some((review, thread)) = self.thread_to_act_on(
            |thread| !thread.resolved,
            "The thread on this line is resolved already",
            cx,
        ) else {
            return;
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::ResolveThread { review, thread }, cx)
        });
        self.report(asked, cx);
    }

    /// [`ReopenThread`]: the thread on the line the reviewer is on that is resolved.
    pub(crate) fn reopen_thread(
        &mut self,
        _: &ReopenThread,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some((review, thread)) = self.thread_to_act_on(
            |thread| thread.resolved,
            "The thread on this line is not resolved",
            cx,
        ) else {
            return;
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::ReopenThread { review, thread }, cx)
        });
        self.report(asked, cx);
    }

    /// [`ResolveAddressed`]: every addressed thread of the review shown, wherever the
    /// reviewer is.
    pub(crate) fn resolve_addressed(
        &mut self,
        _: &ResolveAddressed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::ResolveAddressed { review }, cx)
        });
        self.report(asked, cx);
    }
}
