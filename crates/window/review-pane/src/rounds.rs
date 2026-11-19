//! The rounds of the review shown: each shown as sent until its agent's first edit, and
//! what changed since its comments were sent.
//!
//! A line above the tree of Changes, `review-rounds`, names each round under
//! `review-round-<n>`, with `:sent` while it shows as sent and `:answered` once its agent
//! edited, and a control `review-round-<n>-since`, drawn once the round has its state, that
//! shows in Changes what changed since that round's comments were sent:
//! `ChangeShown::Since` from the round's state to the newest state of the history after
//! it, through the selection of the tree of Changes (`select_since` of `tree.rs`).
//! [`crate::SinceRound`] does the same from the keyboard: the first press for the newest
//! round with a state, each press after it for the round before, and after the first
//! round the newest again; with no round that has a state, or no state of the worktree
//! recorded to count to, the notice says so and nothing is selected.
//!
//! Only the newest round can be waiting for an edit, so an older round shows as answered
//! whatever its `edited` says: the Keeper sets that on the newest round only.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::HistoryEntry;

use crate::draw::ROW_HEIGHT;
use crate::{ChangeShown, ReviewPane, SinceRound, View};

/// One round as the pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundShown {
    /// Its number.
    pub number: u32,
    /// Whether it shows as sent: its agent has not edited since the send.
    pub sent: bool,
    /// The state recorded at its send, by its full id; none until it is recorded.
    pub state: Option<String>,
}

/// The newest version of a history: the last commit or turn end that has one.
fn newest_version(entries: &[HistoryEntry]) -> Option<&str> {
    entries.iter().rev().find_map(|entry| match entry {
        HistoryEntry::Commit(commit) => Some(commit.id.as_str()),
        HistoryEntry::Turn { state, .. } => state.as_deref(),
    })
}

impl ReviewPane {
    /// The rounds of the review shown, the first first.
    pub fn rounds_shown(&self) -> Vec<RoundShown> {
        let Some(review) = self.review() else {
            return Vec::new();
        };
        let newest = review.rounds.last().map(|round| round.number);
        review
            .rounds
            .iter()
            .map(|round| RoundShown {
                number: round.number,
                sent: !round.edited && Some(round.number) == newest,
                state: round.state.clone(),
            })
            .collect()
    }

    /// The line of the rounds, drawn above the tree of Changes; none for a review with no
    /// round.
    pub(crate) fn render_rounds(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let rounds = self.rounds_shown();
        if rounds.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let mut line = div()
            .debug_selector(|| "review-rounds".to_owned())
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
        for round in rounds {
            let number = round.number;
            let (kind, color) = match round.sent {
                true => ("sent", theme.warning),
                false => ("answered", theme.success),
            };
            let mut one = div().flex_none().flex().gap_2().child(
                div()
                    .debug_selector(|| format!("review-round-{number}:{kind}"))
                    .text_color(color)
                    .child(format!("Round {number}: {kind}")),
            );
            if round.state.is_some() {
                one = one.child(
                    div()
                        .debug_selector(|| format!("review-round-{number}-since"))
                        .px_2()
                        .bg(theme.secondary)
                        .child("Since")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                                pane.acted(cx);
                                pane.show_since(number, cx);
                            }),
                        ),
                );
            }
            line = line.child(one);
        }
        Some(line.into_any_element())
    }

    /// Shows in Changes what changed since the comments of round `number` were sent: from
    /// the state recorded at the send to the newest state of the history.
    fn show_since(&mut self, number: u32, cx: &mut Context<Self>) {
        let state = self
            .rounds_shown()
            .into_iter()
            .find(|round| round.number == number)
            .and_then(|round| round.state);
        let Some(state) = state else {
            return self.say(format!("Round {number} has no recorded state yet"), cx);
        };
        let newest = self
            .tree
            .history
            .as_ref()
            .and_then(|history| newest_version(&history.entries))
            .map(str::to_owned);
        let Some(newest) = newest else {
            return self.say("No state of the worktree is recorded to count to yet", cx);
        };
        self.select_since(Some(state), newest, cx);
    }

    /// [`SinceRound`].
    pub(crate) fn since_round(
        &mut self,
        _: &SinceRound,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        // Changes shows the selection; the live diff of Flow has none.
        if self.view == View::Flow {
            return self.say("Changes shows what changed since a round", cx);
        }
        let with_state: Vec<(u32, String)> = self
            .rounds_shown()
            .into_iter()
            .filter_map(|round| Some((round.number, round.state?)))
            .collect();
        let Some(&(newest, _)) = with_state.last() else {
            return self.say("No round has a recorded state yet", cx);
        };
        // Where the presses are: the round whose change is shown, the first of those with
        // one state, since they show the same change.
        let at = match &self.tree.shown {
            ChangeShown::Since {
                from: Some(from), ..
            } => with_state.iter().position(|(_, state)| state == from),
            _ => None,
        };
        let number = match at {
            Some(index) if index > 0 => with_state[index - 1].0,
            _ => newest,
        };
        self.show_since(number, cx);
    }
}
