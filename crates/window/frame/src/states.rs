//! The shared model of the room the recorded states take, and of their clean-up (FR-126).
//!
//! [`RecordedStates`] asks the Keeper `Request::GetStateSizes` when a reader asks it to
//! ([`RecordedStates::refresh`]: the settings screen, when it opens), never by itself at a
//! connect, and again at each `Fact::StatesDeleted` once it has been asked once. The user's
//! clean-up is [`RecordedStates::clean_up`], which sends `Request::CleanUpStates` and takes
//! the sizes it answers with. Of two answers, only that to the last question counts, as
//! every model of the Frame keeps it. A refusal leaves the sizes as they were and keeps its
//! sentence ([`RecordedStates::problem`]) until the next answer.

use gpui_kit::*;
use yaar_link::{Fact, Reply, Request, StateSizes};

use crate::link_model::{AskError, KeeperLink};

/// The room the recorded states take, as the Keeper last answered.
pub struct RecordedStates {
    link: Entity<KeeperLink>,
    sizes: Option<StateSizes>,
    /// Whether the sizes were asked for, by a refresh or a clean-up: from then on a
    /// deletion has them asked again.
    asked: bool,
    /// Counts the questions for the sizes, so that an answer can tell whether a later
    /// question took its place.
    questions: u64,
    /// The last refresh. A new one takes its place, which drops the answer to the old one.
    refreshing: Option<Task<()>>,
    /// The clean-up that is on its way, which no refresh may drop: the user is owed its end.
    cleaning: Option<Task<()>>,
    problem: Option<String>,
}

impl RecordedStates {
    /// The model of one window, fed by `link`: it asks nothing until it is first asked to.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> RecordedStates {
        cx.subscribe(&link, |states, _, fact: &Fact, cx| {
            if matches!(fact, Fact::StatesDeleted { .. }) && states.asked {
                states.refresh(cx);
            }
        })
        .detach();
        RecordedStates {
            link,
            sizes: None,
            asked: false,
            questions: 0,
            refreshing: None,
            cleaning: None,
            problem: None,
        }
    }

    /// Asks the Keeper for the sizes, as the module doc says.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.asked = true;
        self.questions += 1;
        let number = self.questions;
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::GetStateSizes, cx));
        self.refreshing = Some(cx.spawn(async move |states, cx| {
            let answer = asked.await;
            states
                .update(cx, |states, cx| states.take(number, answer, cx))
                .ok();
        }));
    }

    /// Asks the Keeper to clean up at once, as the module doc says. Does nothing while a
    /// clean-up is on its way.
    pub fn clean_up(&mut self, cx: &mut Context<Self>) {
        if self.cleaning.is_some() {
            return;
        }
        self.asked = true;
        self.questions += 1;
        let number = self.questions;
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::CleanUpStates, cx));
        self.cleaning = Some(cx.spawn(async move |states, cx| {
            let answer = asked.await;
            states
                .update(cx, |states, cx| {
                    states.cleaning = None;
                    states.take(number, answer, cx);
                    cx.notify();
                })
                .ok();
        }));
        cx.notify();
    }

    /// Takes the answer to question `number` unless a later question took its place.
    fn take(&mut self, number: u64, answer: Result<Reply, AskError>, cx: &mut Context<Self>) {
        if number != self.questions {
            return;
        }
        match answer {
            Ok(Reply::StateSizes(sizes)) => {
                self.problem = None;
                self.sizes = Some(sizes);
            }
            Ok(_) => self.problem = Some(AskError::Unexpected.to_string()),
            Err(error) => self.problem = Some(error.to_string()),
        }
        cx.notify();
    }

    /// The sizes as last answered; none before the first answer.
    pub fn sizes(&self) -> Option<&StateSizes> {
        self.sizes.as_ref()
    }

    /// Whether a clean-up was asked for and is not answered yet.
    pub fn cleaning(&self) -> bool {
        self.cleaning.is_some()
    }

    /// Why the last question was refused; none after an answer.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}
