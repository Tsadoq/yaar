//! The room the recorded states take, and their clean-up. The sizes are asked only on `refresh`
//! (the settings screen, on open), never at connect, and again at each `Fact::StatesDeleted` once
//! asked.

use gpui_kit::*;
use yaar_link::{Fact, Reply, Request, StateSizes};

use crate::link_model::{AskError, KeeperLink};

pub struct RecordedStates {
    link: Entity<KeeperLink>,
    sizes: Option<StateSizes>,
    asked: bool,
    /// Counts the questions for the sizes, so that an answer can tell whether a later question took
    /// its place.
    questions: u64,
    refreshing: Option<Task<()>>,
    /// The clean-up on its way, which no refresh may drop: the user is owed its end.
    cleaning: Option<Task<()>>,
    problem: Option<String>,
}

impl RecordedStates {
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

    pub fn sizes(&self) -> Option<&StateSizes> {
        self.sizes.as_ref()
    }

    pub fn cleaning(&self) -> bool {
        self.cleaning.is_some()
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}
