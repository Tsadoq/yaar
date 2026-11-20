//! When a server's answers count as sound: the three rules of a row's `ready` (ADR-015
//! point 2, FR-067).
//!
//! A [`Readiness`] is fed what the server reports, in the order it reports it, and says
//! where the server stands ([`Readying`]). It knows no server: every rule is data of a
//! row (NAV-13).

use serde_json::Value;

use crate::registry::Ready;

/// Where a server stands by its row's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readying {
    /// Started, and the rule is not met: answers are found by text match meanwhile.
    Starting,
    /// The rule was met: the server's answers are sound. A server once ready stays ready
    /// until it ends.
    Ready,
    /// The server ended, or failed to answer `initialize`: missing.
    Gone,
}

/// A server's readiness, fed what it reports.
#[derive(Debug, Clone)]
pub struct Readiness {
    rule: Ready,
    now: Readying,
    /// The tokens of the `$/progress` reports begun and not yet ended.
    open: Vec<Value>,
    /// Whether any `$/progress` began.
    began: bool,
}

impl Readiness {
    /// A server just started, by `rule`: [`Readying::Starting`].
    pub fn new(rule: &Ready) -> Readiness {
        Readiness {
            rule: rule.clone(),
            now: Readying::Starting,
            open: Vec::new(),
            began: false,
        }
    }

    /// The server answered `initialize` with a result: ready by [`Ready::Answer`]. Gives
    /// where it stands now.
    pub fn answered_initialize(&mut self) -> Readying {
        todo!()
    }

    /// The server sent a notification: `$/progress` with a `begin` or an `end` value counts
    /// for [`Ready::Progress`], any notification of the rule's method whose parameters hold
    /// the rule's `match` for [`Ready::Notification`]. Gives where it stands now.
    pub fn notification(&mut self, method: &str, params: &Value) -> Readying {
        let _ = (method, params);
        todo!()
    }

    /// The server ended, or its `initialize` failed. Gives [`Readying::Gone`].
    pub fn ended(&mut self) -> Readying {
        todo!()
    }

    /// Where it stands.
    pub fn now(&self) -> Readying {
        self.now
    }
}
