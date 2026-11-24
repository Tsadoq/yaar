//! Consents of the Adapters to write into the user's own CLI settings. The Keeper knows no CLI's
//! settings; each Adapter reads and writes its own.

use yaar_agent::ConsentAnswer as Kept;
use yaar_link::{Consent, ConsentAnswer, Refusal};

use super::Agents;
use crate::lock;

impl Agents {
    pub(crate) fn consents(&self) -> Vec<Consent> {
        let adapters = lock(&self.core.adapters);
        self.all_of(&adapters, None)
    }

    /// Gives every Adapter's consent as `consents` does, the one answered as the Adapter returned
    /// it.
    /// Refused with "<agent> asks for no leave" for an unknown name or an Adapter that gives none.
    pub(crate) fn answer_consent(
        &self,
        agent: &str,
        answer: ConsentAnswer,
    ) -> Result<Vec<Consent>, Refusal> {
        let refused = || Refusal::Failed {
            reason: format!("{agent} asks for no leave"),
        };
        // One hold for the answer and the list, so that the list tells what the answer left.
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

fn kept(answer: ConsentAnswer) -> Kept {
    match answer {
        ConsentAnswer::NotAsked => Kept::NotAsked,
        ConsentAnswer::Yes => Kept::Yes,
        ConsentAnswer::No => Kept::No,
    }
}

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
