//! Reviewer actions that ask the Keeper for more than a save. None changes the pane itself:
//! it rereads reviews and changes when the Keeper reports them.

use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::*;
use yaar_frame::AskError;
use yaar_link::{Refusal, Reply, Request};

use crate::{Commit, CopyReview, MarkReviewed, ReviewPane, ToggleViewed};

pub(crate) struct CommitField {
    pub(crate) input: Entity<InputState>,
    _heard: Subscription,
}

pub(crate) fn words(error: &AskError) -> String {
    match error {
        // The ids of a review and a workspace say nothing to the reviewer.
        AskError::Refused(Refusal::NothingToSend { .. }) => {
            "There is no unsent comment on an unresolved thread to copy".to_owned()
        }
        AskError::Refused(Refusal::NothingToCommit { .. }) => {
            "Nothing is uncommitted, so there is nothing to commit".to_owned()
        }
        error => error.to_string(),
    }
}

impl ReviewPane {
    /// Copying sends the unsent comments; they show as sent once the Keeper reports the reviews
    /// changed.
    pub(crate) fn copy_review(
        &mut self,
        _: &CopyReview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        // The open box's draft is among the comments the copy sends, and a sent comment is not
        // edited.
        if self.open_box.is_some() {
            self.close_box(window, cx);
        }
        let asked = self
            .models
            .link
            .update(cx, |link, cx| link.ask(Request::CopyReview { review }, cx));
        cx.spawn(async move |pane, cx| {
            let said = match asked.await {
                Ok(Reply::Copied { text, .. }) => {
                    pane.update(cx, |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    })
                    .ok();
                    return;
                }
                Ok(_) => AskError::Unexpected.to_string(),
                Err(error) => words(&error),
            };
            pane.update(cx, |pane, cx| pane.say(said, cx)).ok();
        })
        .detach();
    }

    pub(crate) fn toggle_viewed(
        &mut self,
        _: &ToggleViewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let (Some(workspace), Some(reviews)) = (self.reviews_of.clone(), &self.reviews) else {
            return self.say("The reviews are not here yet", cx);
        };
        let Some(position) = self.position.clone() else {
            return self.say("The reviewer is in no file", cx);
        };
        let viewed = !reviews.viewed.contains(&position.path);
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(
                Request::SetViewed {
                    workspace,
                    path: position.path,
                    viewed,
                },
                cx,
            )
        });
        self.report(asked, cx);
    }

    pub(crate) fn mark_reviewed(
        &mut self,
        _: &MarkReviewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let Some(review) = self.review().map(|review| review.id.clone()) else {
            return self.say("The reviews are not here yet", cx);
        };
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::MarkReviewed { review }, cx)
        });
        self.report(asked, cx);
    }

    pub(crate) fn commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        self.acted(cx);
        if let Some(open) = &self.commit_field {
            open.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.reviews_of.is_none() {
            return self.say("No workspace is on screen to commit in", cx);
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let heard = cx.subscribe_in(&input, window, |pane, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                pane.take_message(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.commit_field = Some(CommitField {
            input,
            _heard: heard,
        });
        cx.notify();
    }

    /// Blank-only text keeps the field open: git refuses an empty message.
    fn take_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = &self.commit_field else {
            return;
        };
        let message = field.input.read(cx).value().to_string();
        if message.trim().is_empty() {
            return self.say("Write the commit message first", cx);
        }
        let Some(workspace) = self.reviews_of.clone() else {
            return;
        };
        self.close_commit_field(window, cx);
        let asked = self.models.link.update(cx, |link, cx| {
            link.ask(Request::Commit { workspace, message }, cx)
        });
        cx.spawn(async move |pane, cx| {
            let said = match asked.await {
                // The commit shows through the changes the Keeper reports.
                Ok(Reply::Committed { .. }) => return,
                Ok(_) => AskError::Unexpected.to_string(),
                Err(error) => words(&error),
            };
            pane.update(cx, |pane, cx| pane.say(said, cx)).ok();
        })
        .detach();
    }

    pub(crate) fn close_commit_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.commit_field.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn escape_commit(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_commit_field(window, cx);
    }
}
