//! The shared model of the questions for leave: an Adapter that writes into the user's own
//! settings of its CLI only with the user's yes, and how that stands.
//!
//! The Keeper tells a Window `Fact::ConsentsChanged` when the Window first asks it something,
//! before it answers that request, if a question waits for an answer, and every Window after
//! each answer. The model asks nothing when the link connects: a Window told nothing has
//! nothing to ask. The terminal pane draws the questions and sends the user's answers
//! through it; it names no CLI, and shows each question in the Adapter's own words.

use gpui_kit::*;
use yaar_link::{Consent, ConsentAnswer, Fact, Reply, Request};

use crate::link_model::{AskError, KeeperLink};

/// The Adapters that need the user's leave, as the Keeper last told or answered them.
pub struct ConsentList {
    link: Entity<KeeperLink>,
    /// What the Keeper last told or answered, in the order of its Adapters.
    consents: Vec<Consent>,
}

impl ConsentList {
    /// The list of one window, fed by `link`: empty until the Keeper tells it one.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::ConsentsChanged(consents) = fact {
                list.take(consents.clone(), cx);
            }
        })
        .detach();
        Self {
            link,
            consents: Vec::new(),
        }
    }

    /// Takes what the Keeper told or answered: it replaces the whole list.
    fn take(&mut self, consents: Vec<Consent>, cx: &mut Context<Self>) {
        if consents != self.consents {
            self.consents = consents;
            cx.notify();
        }
    }

    /// The questions to show: each consent not answered, or whose CLI's settings could not
    /// be changed, in the order of the Adapters. Empty when nothing waits.
    pub fn asked(&self) -> Vec<Consent> {
        self.consents
            .iter()
            .filter(|consent| {
                consent.answer == ConsentAnswer::NotAsked || consent.problem.is_some()
            })
            .cloned()
            .collect()
    }

    /// Sends `answer` for every Adapter that needs leave: for those [`ConsentList::asked`]
    /// shows while it shows any, else for every Adapter the Keeper lists, asked with
    /// `Request::ListConsents` first, so that the answer can be changed once the question
    /// is gone. Each `Request::AnswerConsent` is answered with the list, which replaces this
    /// one.
    ///
    /// The requests for the questions shown are handed to the link before this returns, in
    /// the order of the Adapters; the question for the list too. The task resolves to
    /// whether an answer was sent: false, with nothing sent but the question for the list,
    /// when the Keeper lists no Adapter that needs leave, which the caller says. It resolves
    /// to the first refusal the Keeper gave or the loss of the connection. For the questions
    /// shown every answer is still awaited, so that the list each one brings is taken; after
    /// the question for the list, a failure ends it and no more is sent. The task is the
    /// caller's to keep: a dropped one stops asking, though what was handed to the link stays
    /// sent.
    pub fn answer_all(
        &mut self,
        answer: ConsentAnswer,
        cx: &mut Context<Self>,
    ) -> Task<Result<bool, AskError>> {
        let shown = self.asked();
        if shown.is_empty() {
            let listed = self
                .link
                .update(cx, |link, cx| link.ask(Request::ListConsents, cx));
            return cx.spawn(async move |list, cx| {
                let Reply::Consents(consents) = listed.await? else {
                    return Err(AskError::Unexpected);
                };
                let agents: Vec<String> = consents.iter().map(|c| c.agent.clone()).collect();
                list.update(cx, |list, cx| list.take(consents, cx)).ok();
                let mut sent = false;
                for agent in agents {
                    // A list that is gone has no one left to answer for.
                    let Ok(asked) = list.update(cx, |list, cx| list.send_answer(agent, answer, cx))
                    else {
                        return Err(AskError::NotLinked);
                    };
                    asked.await?;
                    sent = true;
                }
                Ok(sent)
            });
        }
        let asked: Vec<_> = shown
            .into_iter()
            .map(|consent| self.send_answer(consent.agent, answer, cx))
            .collect();
        cx.spawn(async move |_, _| {
            // Every answer is awaited, so that the list each one brings is taken, and the
            // first refusal is the one given back.
            let mut refused = None;
            for asked in asked {
                if let Err(error) = asked.await {
                    refused.get_or_insert(error);
                }
            }
            refused.map_or(Ok(true), Err)
        })
    }

    /// Hands the link the answer to one Adapter's question. The task takes the list the
    /// Keeper answers with, and resolves once it is taken.
    fn send_answer(
        &mut self,
        agent: String,
        answer: ConsentAnswer,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), AskError>> {
        let asked = self.link.update(cx, |link, cx| {
            link.ask(Request::AnswerConsent { agent, answer }, cx)
        });
        cx.spawn(async move |list, cx| {
            let Reply::Consents(consents) = asked.await? else {
                return Err(AskError::Unexpected);
            };
            list.update(cx, |list, cx| list.take(consents, cx)).ok();
            Ok(())
        })
    }
}
