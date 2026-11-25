//! A thread the Keeper carried with the code: the notice that the code changed or was
//! deleted, and the line it was written on, shown on request. A row holds one line, so the
//! lines of an anchor of several are joined on it.

use std::collections::BTreeSet;

use gpui_kit::*;
use yaar_link::{Notice, Thread, ThreadId};
use yaar_ui::metrics::{SPACE_2, SPACE_3, SPACE_5};
use yaar_ui::pieces::{Button, ButtonKind, Chip, Tone};
use yaar_ui::themes::palette;
use yaar_ui::typography::{TypeRole, Typed};

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
        let id = held.id.clone();
        let (kind, words) = match notice {
            Notice::Changed => ("changed", "The code changed"),
            Notice::Deleted => ("deleted", "The code was deleted"),
        };
        let shown = self.original_text(thread).is_some();
        vec![
            Chip::new(words)
                .tone(Tone::Warn)
                .selector(format!("review-thread-notice-{id}:{kind}"))
                .into_any_element(),
            Button::new(
                SharedString::from(format!("review-thread-original-{id}")),
                if shown {
                    "Hide original"
                } else {
                    "Show original"
                },
            )
            .kind(ButtonKind::Ghost)
            .small()
            .selector(format!("review-thread-original-{id}"))
            .on_click(cx.listener(move |pane, _: &ClickEvent, _, cx| {
                pane.acted(cx);
                pane.toggle_original(&id, cx);
            }))
            .into_any_element(),
        ]
    }

    pub(crate) fn thread_original_block(
        &self,
        thread: &ThreadId,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let text = self.original_text(thread)?;
        let palette = palette(cx);
        let id = thread.clone();
        Some(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .px(SPACE_5)
                .py(SPACE_2)
                .bg(palette.del_ground)
                .border_b_1()
                .border_color(palette.separator)
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .typed(TypeRole::Meta)
                        .text_color(palette.text_muted)
                        .child("Written as:"),
                )
                .child(
                    div()
                        .debug_selector(move || format!("review-thread-original-text-{id}"))
                        .flex_none()
                        .whitespace_nowrap()
                        .typed(TypeRole::CodeSmall)
                        .text_color(palette.text)
                        .child(text),
                )
                .into_any_element(),
        )
    }

    fn toggle_original(&mut self, thread: &ThreadId, cx: &mut Context<Self>) {
        if !self.moved.shown.remove(thread) {
            self.moved.shown.insert(thread.clone());
        }
        self.list.remeasure();
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
