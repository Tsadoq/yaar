//! The state of each thread on its row, and resolving and reopening threads. Of several
//! threads on the reviewer's line, the first the action applies to is acted on; with none,
//! the notice says why and nothing is sent.

use gpui_kit::*;
use yaar_link::{Request, ReviewId, Thread, ThreadId, ThreadState};
use yaar_ui::pieces::{Chip, Tone};

use crate::{PaneRow, ReopenThread, ResolveAddressed, ResolveThread, ReviewPane, View};

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
    pub(crate) fn thread_state_mark(&self, thread: &ThreadId, _cx: &App) -> Option<AnyElement> {
        let held = self
            .review()?
            .threads
            .iter()
            .find(|held| held.id == *thread)?;
        let (name, words) = written(held.state);
        let tone = match held.state {
            ThreadState::Draft | ThreadState::WaitsForReviewer => Tone::Warn,
            ThreadState::Addressed | ThreadState::Resolved => Tone::Ok,
            ThreadState::Sent => Tone::Plain,
        };
        let id = held.id.clone();
        Some(
            Chip::new(words)
                .tone(tone)
                .selector(format!("review-thread-state-{id}:{name}"))
                .into_any_element(),
        )
    }

    /// None while the reviewer is on no line, and for a thread under the heading of the threads on
    /// no line.
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

    pub(crate) fn thread_to_act_on(
        &mut self,
        applies: impl Fn(&Thread) -> bool,
        otherwise: &str,
        cx: &mut Context<Self>,
    ) -> Option<(ReviewId, ThreadId)> {
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
