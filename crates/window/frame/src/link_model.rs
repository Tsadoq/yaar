//! The shared model that says how the Window stands with the Keeper, and through which
//! every part of the Window asks the Keeper and hears from it.

use std::fmt;
use std::os::unix::net::UnixStream;

use gpui_kit::*;
use yaar_link::{Fact, Hello, Message, Refusal, Reply, Request};

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

/// Why a question to the Keeper brought no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The Keeper refused, and says why.
    Refused(Refusal),
    /// There is no Keeper to ask: none is reached yet, the one reached speaks another
    /// protocol version, or the connection ended before the answer came.
    NotLinked,
    /// The Keeper answered with a reply of another kind than the request has. Whoever
    /// asks for one kind of reply turns any other into this.
    Unexpected,
}

impl fmt::Display for AskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(f),
            Self::NotLinked => write!(f, "the Keeper is not reached"),
            Self::Unexpected => write!(f, "the Keeper answered something else than was asked"),
        }
    }
}

impl std::error::Error for AskError {}

/// The Window's one connection to the Keeper: a model every part of the Window may
/// observe, ask through and subscribe to.
///
/// - Observing it tells when [`KeeperLink::status`] changes.
/// - [`KeeperLink::ask`] sends a request and gives its answer.
/// - Subscribing to it gives every [`Fact`] the Keeper tells, in the order they arrive.
///
/// Nothing else in the Window writes to the connection or reads from it.
pub struct KeeperLink {
    status: LinkStatus,
    // Held open so that the Keeper counts this Window as attached.
    _stream: Option<UnixStream>,
    // What the thread that reads the connection hands to the thread that draws.
    _incoming: Option<async_channel::Receiver<Message>>,
}

impl EventEmitter<Fact> for KeeperLink {}

impl KeeperLink {
    /// A link that has not reached a Keeper yet.
    pub fn new() -> Self {
        Self {
            status: LinkStatus::Starting,
            _stream: None,
            _incoming: None,
        }
    }

    /// Asks the Keeper, and resolves with its reply or its refusal. The reply is of
    /// whatever kind the Keeper sent: the caller checks that it is the kind it asked for.
    ///
    /// The request is written before this returns, so requests reach the Keeper in the
    /// order they were asked. The answer comes when the Keeper sends it; dropping the
    /// task drops the answer and takes nothing back. While the status is anything but
    /// [`LinkStatus::Connected`] nothing is sent and the task resolves to
    /// [`AskError::NotLinked`]; so does every waiting task when the connection ends.
    pub fn ask(
        &mut self,
        request: Request,
        cx: &mut Context<Self>,
    ) -> Task<Result<Reply, AskError>> {
        todo!()
    }

    /// How the Window stands with the Keeper.
    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    /// Records the outcome of [`crate::keeper::attach`] and tells the observers. With a
    /// Keeper of the same protocol version, the connection is read from here on: answers
    /// go to those who asked, and each fact is emitted to the subscribers.
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
