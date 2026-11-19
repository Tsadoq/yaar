//! A thread that moved with the code: where the Keeper says it sits now, the notice that
//! the code changed or was deleted, and the line it was written on, shown on request.
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
//! or hides it. The notice and the control are drawn only on the lines the thread was
//! carried to: where it sits on its anchor, the code is as it was written.

use std::collections::BTreeSet;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{Notice, Thread, ThreadId};

use crate::threads::{Place, place_of};
use crate::{PaneRow, ReviewPane, ShowOriginal, View};

/// What the pane keeps of the threads that moved: those whose original line is shown.
#[derive(Default)]
pub(crate) struct Moved {
    shown: BTreeSet<ThreadId>,
}

impl ReviewPane {
    /// Whether the line `thread` was written on is shown below its row.
    pub fn shows_original(&self, thread: &ThreadId) -> bool {
        self.original_text(thread).is_some()
    }

    /// The text of the line `thread` was written on, as its row shows it; none while it is
    /// not shown. It is the anchor's text, which the Keeper keeps with a thread that moved.
    pub fn original_text(&self, thread: &ThreadId) -> Option<String> {
        let held = self.thread(thread)?;
        (self.moved.shown.contains(thread) && self.notice_of(held).is_some())
            .then(|| held.anchor.text.clone())
    }

    /// The thread of the review shown with this id.
    fn thread(&self, thread: &ThreadId) -> Option<&Thread> {
        self.review()?
            .threads
            .iter()
            .find(|held| held.id == *thread)
    }

    /// What became of the code `thread` was written on, when it is drawn on the lines the
    /// Keeper carried it to.
    fn notice_of(&self, thread: &Thread) -> Option<Notice> {
        match place_of(thread, self.new_side_shown().as_deref())? {
            Place::Now(now) => now.notice,
            Place::Anchor(_) => None,
        }
    }

    /// The notice of a thread that moved, with its control, and the line it was written on
    /// while it is shown, for its row.
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
            // A row holds one line: the first of the text, with a mark that more follows.
            let mut lines = text.lines();
            let first = lines.next().unwrap_or_default();
            let more = if lines.next().is_some() { " …" } else { "" };
            marks.push(
                div()
                    .debug_selector(move || format!("review-thread-original-text-{id}"))
                    .min_w_0()
                    .ml_2()
                    .truncate()
                    .text_color(theme.foreground)
                    .child(format!("Written as: {first}{more}"))
                    .into_any_element(),
            );
        }
        marks
    }

    /// Shows or hides the line `thread` was written on.
    fn toggle_original(&mut self, thread: &ThreadId, cx: &mut Context<Self>) {
        if !self.moved.shown.remove(thread) {
            self.moved.shown.insert(thread.clone());
        }
        cx.notify();
    }

    /// The threads on the line the reviewer is on that sit on lines the Keeper carried
    /// them to and carry a notice: the thread rows that follow the row of the line.
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

    /// [`ShowOriginal`].
    pub(crate) fn show_original(
        &mut self,
        _: &ShowOriginal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        // The live diff of Flow draws no notice and no original line.
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
        // One line holding several threads: all of them are shown, or, when they all are,
        // all are hidden.
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
