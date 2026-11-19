//! Attaching a review, with the offer to start its agent, and a send whose state is
//! recorded after it. Built by the engine's package "attaching and sending" of slice 6.

use yaar_store::{SentBy, SessionId};

use super::{Attachment, Review, ReviewError, Scope, Sent};

/// One agent session of the review's workspace as it is now, as the Keeper knows it: what
/// attaching needs to know to offer to start an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionNow {
    /// The session, by the app's id for it.
    pub id: SessionId,
    /// The Adapter of its CLI, by its name: `claude`.
    pub cli: String,
    /// Whether it runs.
    pub running: bool,
    /// Whether it waits at a permission prompt.
    pub waits_for_permission: bool,
}

/// What attaching a review offers to start, so that the review rides with the first
/// message of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// Nothing: the agent the review belongs to runs, and its next message carries it.
    Nothing,
    /// The CLI of the agent the review belongs to, which does not run, whether or not
    /// another agent runs in the workspace.
    Start {
        /// The Adapter of that CLI, by its name.
        cli: String,
    },
    /// Any CLI the app supports: the review belongs to no agent.
    AnyCli,
}

impl Review {
    /// Attaches the review, or one thread alone, as [`Review::attach`] does, whatever the
    /// state of its agent, a permission prompt included, and says what to offer: see
    /// [`Offer`]. `sessions` are the sessions of the review's workspace, those that ended
    /// among them; the review's own is found among them by its id. A review whose session
    /// is not among them, whose CLI is then unknown, offers [`Offer::AnyCli`], as the
    /// review of no agent does. Refused with
    /// [`ReviewError::NothingToSend`], and nothing attached, when what would be attached
    /// holds no unsent comment on an unresolved thread; with
    /// [`ReviewError::UnknownThread`] for a thread the review does not hold.
    pub fn attach_with(
        &mut self,
        attachment: Attachment,
        sessions: &[SessionNow],
    ) -> Result<Offer, ReviewError> {
        let _ = (attachment, sessions);
        todo!()
    }

    /// Sends what is attached, as a message of the user carries it: what
    /// [`Review::unsent`] gives for [`Scope::Attached`] starts the next round, sent by
    /// `sent_by`, with no state yet, which [`Review::set_round_state`] gives it later; the
    /// attachment goes, and every thread one of whose comments it sends loses its addressed
    /// mark: the agent has a follow-up to answer. Refused with [`ReviewError::NothingToSend`], and nothing changed,
    /// when nothing is attached or what is attached carries nothing: the attachment then
    /// stays.
    pub fn send_attached(&mut self, sent_by: SentBy) -> Result<Sent, ReviewError> {
        let _ = (sent_by, Scope::Attached);
        todo!()
    }

    /// Gives a round that was started with no state the state recorded for it. Refused
    /// with [`ReviewError::UnknownRound`] for a round the review does not hold, and with
    /// [`ReviewError::RoundHasState`] for one that has a state already.
    pub fn set_round_state(&mut self, round: u32, state: &str) -> Result<(), ReviewError> {
        let _ = (round, state);
        todo!()
    }
}
