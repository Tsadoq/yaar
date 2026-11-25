//! What the pane does when a turn ends in the workspace on screen: it selects what changed
//! since the reviewer's last review and shows the review of the turn's session, or, set to
//! keep the selection, only offers that change.

use gpui_kit::*;
use yaar_link::{ReviewId, SessionId, Setting, TurnEndSelects};
use yaar_ui::metrics::SPACE_3;
use yaar_ui::pieces::{Banner, Button, ButtonKind, Tone};

use crate::tree::ListedFor;
use crate::{ChangeShown, ReviewPane, TakeOffer, ToggleKeepSelection};

pub(crate) struct Offer {
    pub(crate) change: ChangeShown,
    /// The review of the turn's session, which taking the offer shows; none when the pane holds no
    /// review of it.
    pub(crate) review: Option<ReviewId>,
}

impl ReviewPane {
    pub(crate) fn toggle_keep_selection(
        &mut self,
        _: &ToggleKeepSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        let next = match self.models.settings.read(cx).turn_end() {
            TurnEndSelects::WhatChanged => TurnEndSelects::KeepSelection,
            TurnEndSelects::KeepSelection => TurnEndSelects::WhatChanged,
        };
        self.models
            .settings
            .update(cx, |settings, cx| settings.set(Setting::TurnEnd(next), cx));
    }

    pub(crate) fn turn_end_setting(&mut self, cx: &mut Context<Self>) {
        let keep = self.models.settings.read(cx).turn_end() == TurnEndSelects::KeepSelection;
        if keep != self.keep {
            self.keep = keep;
            cx.notify();
        }
    }

    /// The change is counted from `since`, the state the reviewer last marked reviewed, else from
    /// the history's start, else from where the comparison counts. The caller asks again for the
    /// uncommitted changes of the review shown.
    pub(crate) fn turn_ended(
        &mut self,
        session: &SessionId,
        state: String,
        since: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let from = since
            .or_else(|| {
                self.tree
                    .history
                    .as_ref()
                    .and_then(|history| history.start.clone())
            })
            .or_else(|| {
                self.changes
                    .as_ref()
                    .and_then(|changes| changes.counted_from.clone())
            });
        let offer = Offer {
            change: ChangeShown::Since { from, to: state },
            review: self.review_of(session, cx),
        };
        if self.keep {
            self.tree.offer = Some(offer);
            cx.notify();
        } else {
            self.tree.offer = None;
            self.show_offer(offer, None, cx);
        }
    }

    fn review_of(&self, session: &SessionId, cx: &App) -> Option<ReviewId> {
        self.models
            .agents
            .read(cx)
            .session(session)
            .and_then(|session| session.review.clone())
            .or_else(|| {
                self.reviews
                    .as_ref()?
                    .reviews
                    .iter()
                    .find(|review| review.agent.as_ref() == Some(session))
                    .map(|review| review.id.clone())
            })
    }

    /// A reviewer's action gives the window, which lets it leave the comment box of the review it
    /// moves away from.
    fn show_offer(&mut self, offer: Offer, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if let Some(review) = offer.review.clone() {
            self.choose_review(review, window, cx);
        }
        let ChangeShown::Since { from, to } = offer.change else {
            return;
        };
        let listed_for = offer.review.map_or(ListedFor::NoReview, ListedFor::Review);
        self.select_since_for(from, to, listed_for, cx);
    }

    pub(crate) fn take_offer(
        &mut self,
        _: &TakeOffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        // In Flow the offer is that of a turn that ended while Flow was on screen.
        if self.view == crate::View::Flow {
            return self.take_flow_offer(window, cx);
        }
        self.take(window, cx);
    }

    pub(crate) fn take(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.tree.offer.take() else {
            return self.say("No turn has ended since the selection was kept", cx);
        };
        self.show_offer(offer, Some(window), cx);
        self.review_shown(cx);
        cx.notify();
    }

    pub(crate) fn offer_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.keep && self.tree.offer.is_none() {
            return None;
        }
        let mut banner = Banner::custom().tone(if self.tree.offer.is_some() {
            Tone::Info
        } else {
            Tone::Plain
        });
        if self.keep {
            banner = banner.child(
                div()
                    .debug_selector(|| "review-keeps-selection".to_owned())
                    .child("Keeping the selection at a turn end"),
            );
        }
        if self.tree.offer.is_some() {
            banner = banner.child(
                div()
                    .debug_selector(|| "review-offer".to_owned())
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .child("A turn ended: what changed since your last review")
                    .child(
                        Button::new("review-offer-take", "Show")
                            .kind(ButtonKind::Primary)
                            .small()
                            .selector("review-offer-take")
                            .on_click(cx.listener(|pane, _: &ClickEvent, window, cx| {
                                pane.acted(cx);
                                pane.take(window, cx);
                            })),
                    ),
            );
        }
        Some(banner.into_any_element())
    }
}
