//! What the pane does when a turn ends in the workspace on screen: it selects what changed
//! since the reviewer's last review and shows the review of the turn's session, or, set to
//! keep the selection, offers that change and leaves everything as it is.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{ReviewId, SessionId};

use crate::draw::ROW_HEIGHT;
use crate::{ChangeShown, ReviewPane, TakeOffer, ToggleKeepSelection};

/// What a turn end offered while the pane kept the selection.
pub(crate) struct Offer {
    /// What changed since the reviewer's last review: always [`ChangeShown::Since`].
    pub(crate) change: ChangeShown,
    /// The review of the turn's session, which taking the offer shows; none when the pane
    /// holds no review of it.
    pub(crate) review: Option<ReviewId>,
}

impl ReviewPane {
    /// What [`ToggleKeepSelection`] does.
    pub(crate) fn toggle_keep_selection(
        &mut self,
        _: &ToggleKeepSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.keep = !self.keep;
        cx.notify();
    }

    /// A turn of `session` ended in the workspace on screen and its state `state` was
    /// recorded. The change it leaves is counted from the state the reviewer last marked
    /// reviewed, `since`, or from the history's start before the first mark. It is
    /// selected, with the review of the session; or, while the pane keeps the selection,
    /// only offered.
    ///
    /// The uncommitted changes of the review shown are for the caller to ask for again.
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

    /// The review of `session`: the one the agents model gives it, else the one the pane
    /// holds whose agent it is.
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

    /// Shows the change of an offer and the review it came with, the change listed for that
    /// review (`shared.rs`). A reviewer's action gives the window, which lets it leave the
    /// comment box of the review it moves away from.
    fn show_offer(&mut self, offer: Offer, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if let Some(review) = offer.review.clone() {
            self.choose_review(review, window, cx);
        }
        let ChangeShown::Since { from, to } = offer.change else {
            return;
        };
        self.select_since_for(from, to, offer.review, cx);
    }

    /// What [`TakeOffer`] does: shows what a turn end offered, and the review of its
    /// session.
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

    /// Shows what a turn end offered, if it offered anything. Also how Flow's offer is
    /// taken while the pane keeps the selection (`take_flow_offer`).
    pub(crate) fn take(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offer) = self.tree.offer.take() else {
            return self.say("No turn has ended since the selection was kept", cx);
        };
        self.show_offer(offer, Some(window), cx);
        self.review_shown(cx);
        cx.notify();
    }

    /// The lines above the tree that tell the reviewer the pane keeps the selection and
    /// what a turn end offered, under `review-keeps-selection`, `review-offer` and, the
    /// control that takes the offer, `review-offer-take`; none while neither is true.
    pub(crate) fn offer_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.keep && self.tree.offer.is_none() {
            return None;
        }
        let theme = cx.theme();
        let mut bar = div()
            .flex_none()
            .h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_b_1()
            .border_color(theme.border);
        if self.keep {
            bar = bar.child(
                div()
                    .debug_selector(|| "review-keeps-selection".to_owned())
                    .text_color(theme.muted_foreground)
                    .child("Keeping the selection at a turn end"),
            );
        }
        if self.tree.offer.is_some() {
            bar = bar.child(
                div()
                    .debug_selector(|| "review-offer".to_owned())
                    .flex()
                    .gap_2()
                    .child("A turn ended: what changed since your last review")
                    .child(
                        div()
                            .debug_selector(|| "review-offer-take".to_owned())
                            .px_2()
                            .bg(theme.secondary)
                            .child("Show")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|pane, _: &MouseDownEvent, window, cx| {
                                    pane.acted(cx);
                                    pane.take(window, cx);
                                }),
                            ),
                    ),
            );
        }
        Some(bar)
    }
}
