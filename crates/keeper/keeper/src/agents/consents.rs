//! The Adapters that write into the user's own settings of their CLI only with the user's
//! leave: what they ask, and the user's answer handed to them. The Keeper knows no CLI's
//! settings; each Adapter reads and writes its own, in its own folder of the app and under
//! the home the Keeper was started with.

use yaar_agent::ConsentAnswer as Kept;
use yaar_link::{Consent, ConsentAnswer, Refusal};

use super::Agents;
use crate::lock;

impl Agents {
    /// `Request::ListConsents`: [`yaar_agent::Adapter::consent`] of every Adapter, with
    /// the Adapter's own folder, in the order of the Adapters, those that give none left
    /// out, in the Link's words. Read with the Adapters' lock held, as a report is, for the
    /// few files an Adapter reads.
    pub(crate) fn consents(&self) -> Vec<Consent> {
        let adapters = lock(&self.core.adapters);
        self.all_of(&adapters, None)
    }

    /// `Request::AnswerConsent`: hands `answer` to
    /// [`yaar_agent::Adapter::answer_consent`] of the Adapter named `agent`, with its own
    /// folder, and gives every Adapter's consent as [`Agents::consents`] does, the one
    /// answered as the Adapter gave it back. Refused with `Refusal::Failed`, "<agent> asks
    /// for no leave", for a name no Adapter has or an Adapter that gives none, and nothing is
    /// kept.
    pub(crate) fn answer_consent(
        &self,
        agent: &str,
        answer: ConsentAnswer,
    ) -> Result<Vec<Consent>, Refusal> {
        let refused = || Refusal::Failed {
            reason: format!("{agent} asks for no leave"),
        };
        // One hold for the answer and for the list that follows it, so that no report of
        // an Adapter comes between and the list tells what the answer left.
        let adapters = lock(&self.core.adapters);
        let (index, adapter) = adapters
            .iter()
            .enumerate()
            .find(|(_, adapter)| adapter.name() == agent)
            .ok_or_else(refused)?;
        let folder = &self.core.described[index].folder;
        let answered = adapter
            .answer_consent(folder, kept(answer))
            .ok_or_else(refused)?;
        Ok(self.all_of(&adapters, Some((index, answered))))
    }

    /// Every Adapter's consent in the Link's words, in the order of the Adapters, those that
    /// give none left out; the one at `answered`'s index as it was just given, not asked
    /// for again.
    fn all_of(
        &self,
        adapters: &[Box<dyn yaar_agent::Adapter>],
        answered: Option<(usize, yaar_agent::Consent)>,
    ) -> Vec<Consent> {
        let mut answered = answered;
        adapters
            .iter()
            .zip(&self.core.described)
            .enumerate()
            .filter_map(|(index, (adapter, described))| {
                let consent = match answered.take_if(|(at, _)| *at == index) {
                    Some((_, consent)) => consent,
                    None => adapter.consent(&described.folder)?,
                };
                Some(in_link_words(adapter.name(), consent))
            })
            .collect()
    }
}

/// An answer in the Adapters' words.
fn kept(answer: ConsentAnswer) -> Kept {
    match answer {
        ConsentAnswer::NotAsked => Kept::NotAsked,
        ConsentAnswer::Yes => Kept::Yes,
        ConsentAnswer::No => Kept::No,
    }
}

/// An Adapter's consent in the Link's words.
fn in_link_words(agent: &str, consent: yaar_agent::Consent) -> Consent {
    Consent {
        agent: agent.to_owned(),
        question: consent.question,
        answer: match consent.answer {
            Kept::NotAsked => ConsentAnswer::NotAsked,
            Kept::Yes => ConsentAnswer::Yes,
            Kept::No => ConsentAnswer::No,
        },
        problem: consent.problem,
    }
}
