//! The shared model of the questions for leave: an Adapter that writes into the user's own
//! settings of its CLI only with the user's yes, and how that stands.
//!
//! The Keeper tells a Window `Fact::ConsentsChanged` at its connect when a question waits
//! for an answer, and every Window after each answer. The model asks nothing when the link
//! connects: a Window told nothing has nothing to ask. The terminal pane draws the questions
//! and sends the user's answers through it; it names no CLI, and shows each question in the
//! Adapter's own words.

use gpui_kit::*;
use yaar_link::{Consent, ConsentAnswer, Fact};

use crate::link_model::KeeperLink;

/// The Adapters that need the user's leave, as the Keeper last told or answered them.
pub struct ConsentList {
    link: Entity<KeeperLink>,
    /// What the Keeper last told or answered, in the order of its Adapters.
    consents: Vec<Consent>,
    /// The answer on its way, while it is.
    answering: Option<Task<()>>,
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
            answering: None,
        }
    }

    /// Takes what the Keeper told or answered: it replaces the whole list.
    ///
    /// A hook of the contract of slice 7 that keeps nothing, for the package of the
    /// Window's question to fill.
    fn take(&mut self, consents: Vec<Consent>, cx: &mut Context<Self>) {
        let _ = (consents, cx, &self.consents, &self.link, &self.answering);
    }

    /// The questions to show: each consent not answered, or whose CLI's settings could not
    /// be changed, in the order of the Adapters. Empty when nothing waits.
    pub fn asked(&self) -> Vec<Consent> {
        todo!("the questions for leave to show")
    }

    /// Sends `answer` for every Adapter that needs leave: for those [`ConsentList::asked`]
    /// shows while it shows any, else for every Adapter the Keeper lists, asked with
    /// `Request::ListConsents` first, so that the answer can be changed once the question
    /// is gone. Each `Request::AnswerConsent` is answered with the list, which replaces this
    /// one. Gives false, sending nothing, when the Keeper lists no Adapter that needs leave:
    /// the caller says so.
    pub fn answer_all(&mut self, answer: ConsentAnswer, cx: &mut Context<Self>) -> bool {
        let _ = (answer, cx);
        todo!("the user's answer to the questions for leave")
    }
}
