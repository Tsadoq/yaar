//! The shared model that says how the Window stands with the Keeper.

use std::os::unix::net::UnixStream;

use gpui_kit::*;
use yaar_link::Hello;

use crate::keeper::Attached;

/// How the Window stands with the Keeper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    /// The Window is looking for the Keeper, or starting it.
    Starting,
    /// The Keeper answered and speaks the Window's protocol.
    Connected(Hello),
    /// The Keeper answered and speaks another protocol version: it belongs to another
    /// version of the app.
    OtherVersion(Hello),
    /// No Keeper could be reached.
    Failed(String),
}

/// A model every part of the Window may observe.
pub struct KeeperLink {
    status: LinkStatus,
    // Held open so that the Keeper counts this Window as attached.
    _stream: Option<UnixStream>,
}

impl KeeperLink {
    /// A link that has not reached a Keeper yet.
    pub fn new() -> Self {
        Self {
            status: LinkStatus::Starting,
            _stream: None,
        }
    }

    /// How the Window stands with the Keeper.
    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    /// Records the outcome of [`crate::keeper::attach`] and tells the observers.
    pub fn attached(&mut self, outcome: Result<Attached, String>, cx: &mut Context<Self>) {
        match outcome {
            Ok(attached) => {
                let mine = Hello::for_this_process("");
                self.status = if mine.speaks_with(&attached.keeper) {
                    LinkStatus::Connected(attached.keeper)
                } else {
                    LinkStatus::OtherVersion(attached.keeper)
                };
                self._stream = Some(attached.stream);
            }
            Err(why) => {
                self.status = LinkStatus::Failed(why);
                self._stream = None;
            }
        }
        cx.notify();
    }
}

impl Default for KeeperLink {
    fn default() -> Self {
        Self::new()
    }
}
