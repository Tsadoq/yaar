//! The rounds of the review shown: each shown as sent until its agent's first edit, and
//! what changed since its comments were sent.
//!
//! A line above the tree of Changes, `review-rounds`, names each round under
//! `review-round-<n>`, with `:sent` while it shows as sent and `:answered` once its agent
//! edited, and a control `review-round-<n>-since`, drawn once the round has its state, that
//! shows in Changes what changed since that round's comments were sent:
//! `ChangeShown::Since` from the round's state to the newest state of the history after
//! it, through the selection of the tree of Changes (`select_since` of `tree.rs`). The line
//! is drawn for every review shown, and says that no round was sent until the first one
//! comes, so that no fact of the Keeper and no comment of the reviewer's own brings it in
//! and moves the diff down by a row.
//! [`crate::SinceRound`] does the same from the keyboard: the first press for the newest
//! round with a state, each press after it for the round before, and after the first
//! round the newest again; with no round that has a state, or no state of the worktree
//! recorded to count to, or nothing recorded since the round's state, the notice says so and
//! nothing is selected; a press passes over a round with nothing recorded since, so that
//! the presses reach the rounds that have a change to show.
//!
//! The Keeper sets `edited` on the newest round only, at the agent's first edit after its
//! send. An edit after a later send is after an earlier send too, so a round shows as sent
//! while no round numbered as it is or higher has been edited.
//!
//! The newest state a round is counted to is the newest entry of the history. A round's
//! state is recorded at its send and is no entry of the history, so the pane cannot tell
//! whether an entry is newer than it: between a send and the end of the turn it starts, the
//! newest entry is the turn before, and the change shown is that of the worktree since
//! then, backward. A state that is an entry of the history is ordered by it.

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

/// What a round's state is counted to.
enum Counted {
    /// The newest version of the history, newer than the round's state or of unknown order.
    To(String),
    /// No version of the history is recorded.
    NoState,
    /// The round's state is the newest version of the history: nothing was recorded since.
    Nothing,
}

/// The versions of a history, oldest first: each commit and each turn end that has one.
fn versions(entries: &[HistoryEntry]) -> Vec<&str> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            HistoryEntry::Commit(commit) => Some(commit.id.as_str()),
            HistoryEntry::Turn { state, .. } => state.as_deref(),
        })
        .collect()
}

/// What the state `state` of a round is counted to in a history.
fn counted(entries: &[HistoryEntry], state: &str) -> Counted {
    let versions = versions(entries);
    let Some(&newest) = versions.last() else {
        return Counted::NoState;
    };
    match versions.iter().rposition(|version| *version == state) {
        Some(at) if at + 1 == versions.len() => Counted::Nothing,
        _ => Counted::To(newest.to_owned()),
    }
}

impl ReviewPane {
    /// The rounds of the review shown, the first first.
    pub fn rounds_shown(&self) -> Vec<RoundShown> {
        let Some(review) = self.review() else {
            return Vec::new();
        };
        review
            .rounds
            .iter()
            .map(|round| RoundShown {
                number: round.number,
                sent: !review
                    .rounds
                    .iter()
                    .any(|later| later.number >= round.number && later.edited),
                state: round.state.clone(),
            })
            .collect()
    }

    /// The line of the rounds, drawn above the tree of Changes for every review shown; none
    /// while no review is shown.
    pub(crate) fn render_rounds(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.review()?;
        // The line is there for every review shown, whatever it holds: a first thread, from
        // the Keeper or from the reviewer's own first comment, a first round, or another
        // review shown in place of this one would otherwise bring it in and move the diff
        // down by a row.
        let rounds = self.rounds_shown();
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
        if rounds.is_empty() {
            line = line.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("No round sent yet"),
            );
        }
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

    /// What the state of the round `state` is counted to in the history the pane holds.
    fn counted_since(&self, state: &str) -> Counted {
        match &self.tree.history {
            Some(history) => counted(&history.entries, state),
            None => Counted::NoState,
        }
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
        match self.counted_since(&state) {
            Counted::To(newest) => self.select_since(Some(state), newest, cx),
            Counted::NoState => {
                self.say("No state of the worktree is recorded to count to yet", cx)
            }
            Counted::Nothing => self.say(
                format!("Nothing was recorded since round {number} was sent"),
                cx,
            ),
        }
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
        if with_state.is_empty() {
            return self.say("No round has a recorded state yet", cx);
        }
        // The rounds with a change to show: one counted to nothing is passed over.
        let counted_to: Vec<Counted> = with_state
            .iter()
            .map(|(_, state)| self.counted_since(state))
            .collect();
        let with_state: Vec<(u32, String)> = with_state
            .into_iter()
            .zip(counted_to)
            .filter(|(_, counted)| matches!(counted, Counted::To(_)))
            .map(|(round, _)| round)
            .collect();
        let Some(&(newest, _)) = with_state.last() else {
            return match self.tree.history.as_ref().map(|h| versions(&h.entries)) {
                Some(held) if !held.is_empty() => {
                    self.say("Nothing was recorded since any round was sent", cx)
                }
                _ => self.say("No state of the worktree is recorded to count to yet", cx),
            };
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
