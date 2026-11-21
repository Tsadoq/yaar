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
use yaar_link::StateSizes;

use crate::link_model::KeeperLink;

/// The room the recorded states take, as the Keeper last answered.
pub struct RecordedStates {
    link: Entity<KeeperLink>,
    sizes: Option<StateSizes>,
}

impl RecordedStates {
    /// The model of one window, fed by `link`: it asks nothing until it is first asked to.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> RecordedStates {
        let _ = cx;
        RecordedStates { link, sizes: None }
    }

    /// Asks the Keeper for the sizes, as the module doc says.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let _ = (&self.link, cx);
        todo!("ask GetStateSizes, as the module doc says")
    }

    /// Asks the Keeper to clean up at once, as the module doc says.
    pub fn clean_up(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
        todo!("send CleanUpStates and take the sizes, as the module doc says")
    }

    /// The sizes as last answered; none before the first answer.
    pub fn sizes(&self) -> Option<&StateSizes> {
        self.sizes.as_ref()
    }

    /// Whether a clean-up was asked for and is not answered yet.
    pub fn cleaning(&self) -> bool {
        todo!("whether a clean-up is on its way")
    }

    /// Why the last question was refused; none after an answer.
    pub fn problem(&self) -> Option<&str> {
        todo!("the refusal's sentence")
    }
}
