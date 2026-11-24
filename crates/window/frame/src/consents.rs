//! The questions for leave: an Adapter writes into its CLI's own settings only with the user's yes.
//! Nothing is asked at connect; the settings screen asks the whole list when it opens (`refresh`).

use gpui_kit::*;
use yaar_link::{Consent, ConsentAnswer, Fact, Reply, Request};

use crate::link_model::{AskError, KeeperLink};

pub struct ConsentList {
    link: Entity<KeeperLink>,
    consents: Vec<Consent>,
}

impl ConsentList {
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

    fn take(&mut self, consents: Vec<Consent>, cx: &mut Context<Self>) {
        if consents != self.consents {
            self.consents = consents;
            cx.notify();
        }
    }

    pub fn asked(&self) -> Vec<Consent> {
        self.consents
            .iter()
            .filter(|consent| {
                consent.answer == ConsentAnswer::NotAsked || consent.problem.is_some()
            })
            .cloned()
            .collect()
    }

    /// The Keeper tells a Window an Adapter only while a question waits or something went wrong, so
    /// this asks the whole list. A dropped task stops waiting, though the request stays sent.
    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Task<Result<(), AskError>> {
        let listed = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListConsents, cx));
        cx.spawn(async move |list, cx| {
            let Reply::Consents(consents) = listed.await? else {
                return Err(AskError::Unexpected);
            };
            list.update(cx, |list, cx| list.take(consents, cx)).ok();
            Ok(())
        })
    }

    pub fn all(&self) -> &[Consent] {
        &self.consents
    }

    pub fn answer(
        &mut self,
        agent: String,
        answer: ConsentAnswer,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), AskError>> {
        self.send_answer(agent, answer, cx)
    }

    /// Answers the Adapters `asked` shows, or when none show, every one the Keeper lists, so an
    /// answer can be changed later; true when an answer was sent. Shown ones are all sent and the
    /// first refusal returned; for listed ones the first failure stops the rest.
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
            // Every answer is awaited so that the list each brings is taken; the first refusal is
            // returned.
            let mut refused = None;
            for asked in asked {
                if let Err(error) = asked.await {
                    refused.get_or_insert(error);
                }
            }
            refused.map_or(Ok(true), Err)
        })
    }

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
