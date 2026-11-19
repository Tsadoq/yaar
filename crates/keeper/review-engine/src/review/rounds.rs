//! Whether a round shows as sent: until its agent's first edit after the send. Built by
//! the engine's package "rounds and what changed since" of slice 6.

use super::Review;

impl Review {
    /// The agent the review belongs to reported an edit: the newest round, when it is not
    /// edited yet, is edited from now on, whatever way it was sent. Gives its number then;
    /// none when nothing changed, as for a round edited already, or no round.
    pub fn agent_edited(&mut self) -> Option<u32> {
        todo!()
    }

    /// Whether the round shows as sent: it was sent and its agent reported no edit since.
    /// False for a round the review does not hold.
    pub fn shows_sent(&self, round: u32) -> bool {
        let _ = round;
        todo!()
    }
}
