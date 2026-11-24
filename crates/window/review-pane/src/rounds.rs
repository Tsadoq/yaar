//! The Keeper sets `edited` on the newest round only, so a round shows as sent while no round
//! numbered as it or higher has been edited. A round with a turn is not counted until the
//! history holds that turn with a state.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{HistoryEntry, TurnId};

use crate::draw::ROW_HEIGHT;
use crate::tree::version_of;
use crate::{ChangeShown, ReviewPane, SinceRound, View};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundShown {
    pub number: u32,
    pub sent: bool,
    pub state: Option<String>,
}

enum Counted {
    To(String),
    NoState,
    Running,
    Nothing,
}

fn versions(entries: &[HistoryEntry]) -> Vec<&str> {
    entries.iter().filter_map(version_of).collect()
}

fn ended(entries: &[HistoryEntry], turn: &TurnId) -> bool {
    entries.iter().any(|entry| {
        matches!(entry, HistoryEntry::Turn { turn: held, state: Some(_), .. } if held == turn)
    })
}

fn counted(entries: &[HistoryEntry], state: &str, turn: Option<&TurnId>) -> Counted {
    if turn.is_some_and(|turn| !ended(entries, turn)) {
        return Counted::Running;
    }
    let versions = versions(entries);
    let Some(&newest) = versions.last() else {
        return Counted::NoState;
    };
    match versions.iter().rposition(|version| *version == state) {
        Some(at) if at + 1 == versions.len() => Counted::Nothing,
        _ => Counted::To(newest.to_owned()),
    }
}

fn still_runs(number: u32) -> String {
    format!("Round {number} was sent in a turn that still runs: nothing is counted since it yet")
}

impl ReviewPane {
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

    pub(crate) fn render_rounds(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.review()?;
        // Shown for every review so that a first thread or round does not move the diff down a row.
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

    fn counted_since(&self, state: &str, turn: Option<&TurnId>) -> Counted {
        match &self.tree.history {
            Some(history) => counted(&history.entries, state, turn),
            None => Counted::NoState,
        }
    }

    fn rounds_with_state(&self) -> Vec<(u32, String, Option<TurnId>)> {
        let Some(review) = self.review() else {
            return Vec::new();
        };
        review
            .rounds
            .iter()
            .filter_map(|round| Some((round.number, round.state.clone()?, round.turn.clone())))
            .collect()
    }

    fn show_since(&mut self, number: u32, cx: &mut Context<Self>) {
        let round = self
            .rounds_with_state()
            .into_iter()
            .find(|(held, ..)| *held == number);
        let Some((_, state, turn)) = round else {
            return self.say(format!("Round {number} has no recorded state yet"), cx);
        };
        match self.counted_since(&state, turn.as_ref()) {
            Counted::To(newest) => self.select_since(Some(state), newest, cx),
            Counted::Running => self.say(still_runs(number), cx),
            Counted::NoState => {
                self.say("No state of the worktree is recorded to count to yet", cx)
            }
            Counted::Nothing => self.say(
                format!("Nothing was recorded since round {number} was sent"),
                cx,
            ),
        }
    }

    pub(crate) fn since_round(
        &mut self,
        _: &SinceRound,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Read before `acted` clears it: a press that said a round's turn still runs selected
        // nothing, so the notice is where it left the presses.
        let said = self.notice.clone();
        self.acted(cx);
        if self.view == View::Flow {
            return self.say("Changes shows what changed since a round", cx);
        }
        let with_state = self.rounds_with_state();
        if with_state.is_empty() {
            return self.say("No round has a recorded state yet", cx);
        }
        let with_state: Vec<(u32, String, bool)> = with_state
            .into_iter()
            .filter_map(
                |(number, state, turn)| match self.counted_since(&state, turn.as_ref()) {
                    Counted::To(_) => Some((number, state, false)),
                    Counted::Running => Some((number, state, true)),
                    Counted::NoState | Counted::Nothing => None,
                },
            )
            .collect();
        let Some(&(newest, ..)) = with_state.last() else {
            return match self.tree.history.as_ref().map(|h| versions(&h.entries)) {
                Some(held) if !held.is_empty() => {
                    self.say("Nothing was recorded since any round was sent", cx)
                }
                _ => self.say("No state of the worktree is recorded to count to yet", cx),
            };
        };
        // Where the presses are: the round the last press said still runs, if it still does (the
        // notice outlives the turn); else the first round whose state the change shown starts from.
        let at = with_state
            .iter()
            .position(|(number, _, running)| {
                *running && said.as_ref() == Some(&still_runs(*number))
            })
            .or_else(|| match &self.tree.shown {
                ChangeShown::Since {
                    from: Some(from), ..
                } => with_state.iter().position(|(_, state, _)| state == from),
                _ => None,
            });
        let number = match at {
            Some(index) if index > 0 => with_state[index - 1].0,
            _ => newest,
        };
        self.show_since(number, cx);
    }
}
