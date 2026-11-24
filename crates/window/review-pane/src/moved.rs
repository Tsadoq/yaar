//! A thread the Keeper carried with the code: the notice that the code changed or was
//! deleted, and the line it was written on, shown on request. A row holds one line, so the
//! lines of an anchor of several are joined on it.

use std::collections::BTreeSet;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{Notice, Thread, ThreadId};

use crate::threads::{Place, place_of};
use crate::{PaneRow, ReviewPane, ShowOriginal, View};

const LINE_BREAK: &str = " | ";

#[derive(Default)]
pub(crate) struct Moved {
    shown: BTreeSet<ThreadId>,
}

impl ReviewPane {
    pub fn shows_original(&self, thread: &ThreadId) -> bool {
        self.original_text(thread).is_some()
    }

    pub fn original_text(&self, thread: &ThreadId) -> Option<String> {
        let held = self.thread(thread)?;
        (self.moved.shown.contains(thread) && self.notice_of(held).is_some()).then(|| {
            held.anchor
                .text
                .lines()
                .collect::<Vec<_>>()
                .join(LINE_BREAK)
        })
    }

    fn thread(&self, thread: &ThreadId) -> Option<&Thread> {
        self.review()?
            .threads
            .iter()
            .find(|held| held.id == *thread)
    }

    /// The notice of `thread`, only where it sits on the lines the Keeper carried it to: on its
    /// anchor the code is as it was written.
    fn notice_of(&self, thread: &Thread) -> Option<Notice> {
        match place_of(thread, self.new_side_shown().as_deref())? {
            Place::Now(now) => now.notice,
            Place::Anchor(_) => None,
        }
    }

    pub(crate) fn thread_moved_marks(
        &self,
        thread: &ThreadId,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(held) = self.thread(thread) else {
            return Vec::new();
        };
        let Some(notice) = self.notice_of(held) else {
            return Vec::new();
        };
        let theme = cx.theme();
        let id = held.id.clone();
        let (kind, words) = match notice {
            Notice::Changed => ("changed", "The code changed"),
            Notice::Deleted => ("deleted", "The code was deleted"),
        };
        let shown = self.original_text(thread);
        let mut marks = vec![
            div()
                .debug_selector({
                    let id = id.clone();
                    move || format!("review-thread-notice-{id}:{kind}")
                })
                .flex_none()
                .ml_2()
                .text_color(theme.warning)
                .child(words)
                .into_any_element(),
            div()
                .debug_selector({
                    let id = id.clone();
                    move || format!("review-thread-original-{id}")
                })
                .flex_none()
                .ml_2()
                .px_2()
                .bg(theme.secondary)
                .text_color(theme.secondary_foreground)
                .child(if shown.is_some() {
                    "Hide original"
                } else {
                    "Show original"
                })
                .on_mouse_down(MouseButton::Left, {
                    let id = id.clone();
                    cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                        pane.acted(cx);
                        pane.toggle_original(&id, cx);
                    })
                })
                .into_any_element(),
        ];
        if let Some(text) = shown {
            marks.push(
                div()
                    .flex_none()
                    .ml_2()
                    .text_color(theme.muted_foreground)
                    .child("Written as:")
                    .into_any_element(),
            );
            marks.push(
                div()
                    .debug_selector(move || format!("review-thread-original-text-{id}"))
                    .min_w_0()
                    .ml_1()
                    .truncate()
                    .text_color(theme.foreground)
                    .child(text)
                    .into_any_element(),
            );
        }
        marks
    }

    fn toggle_original(&mut self, thread: &ThreadId, cx: &mut Context<Self>) {
        if !self.moved.shown.remove(thread) {
            self.moved.shown.insert(thread.clone());
        }
        cx.notify();
    }

    fn moved_threads_on_the_line(&self) -> Vec<ThreadId> {
        let Some(row) = self.position_row() else {
            return Vec::new();
        };
        self.rows()
            .iter()
            .skip(row + 1)
            .take_while(|row| matches!(row, PaneRow::Thread { .. } | PaneRow::Comment { .. }))
            .filter_map(|row| match row {
                PaneRow::Thread { thread } => Some(thread),
                _ => None,
            })
            .filter(|thread| {
                self.thread(thread)
                    .is_some_and(|held| self.notice_of(held).is_some())
            })
            .cloned()
            .collect()
    }

    pub(crate) fn show_original(
        &mut self,
        _: &ShowOriginal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        if self.view == View::Flow {
            return self.say("Changes shows the line a thread was written on", cx);
        }
        let on_the_line = self.moved_threads_on_the_line();
        if on_the_line.is_empty() {
            return self.say(
                "No thread on this line moved to code that changed or was deleted",
                cx,
            );
        }
        let all_shown = on_the_line
            .iter()
            .all(|thread| self.moved.shown.contains(thread));
        for thread in on_the_line {
            match all_shown {
                true => self.moved.shown.remove(&thread),
                false => self.moved.shown.insert(thread),
            };
        }
        cx.notify();
    }
}
