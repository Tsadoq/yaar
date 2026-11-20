//! The Adapters that write into the user's own settings of their CLI only with the user's
//! leave: what they ask, and the user's answer handed to them. The Keeper knows no CLI's
//! settings; each Adapter reads and writes its own, in its own folder of the app and under
//! the home the Keeper was started with.

use yaar_agent::ConsentAnswer as Kept;
use yaar_link::{Consent, ConsentAnswer, Refusal};

use super::Agents;

impl Agents {
    /// `Request::ListConsents`: [`yaar_agent::Adapter::consent`] of every Adapter, with
    /// the Adapter's own folder, in the order of the Adapters, those that give none left
    /// out, in the Link's words. Read with the Adapters' lock held, as a report is, for the
    /// few files an Adapter reads.
    ///
    /// A hook of the contract of slice 7 that gives none, so that no Window is asked
    /// anything until the package of the Keeper's answer fills it.
    pub(crate) fn consents(&self) -> Vec<Consent> {
        Vec::new()
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
        let _ = (agent, answer);
        todo!("the user's answer to an Adapter's question for leave")
    }
}

/// An answer in the Adapters' words.
#[expect(dead_code, reason = "the package of the Keeper's answer uses it")]
fn kept(answer: ConsentAnswer) -> Kept {
    match answer {
        ConsentAnswer::NotAsked => Kept::NotAsked,
        ConsentAnswer::Yes => Kept::Yes,
        ConsentAnswer::No => Kept::No,
    }
}

/// An Adapter's consent in the Link's words.
#[expect(dead_code, reason = "the package of the Keeper's answer uses it")]
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
