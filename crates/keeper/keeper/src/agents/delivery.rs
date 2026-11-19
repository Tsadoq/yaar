//! What a report of a CLI's piece waits for, and the app's answer to it: the review that
//! rides with a message of the user, the allowing of the read of a review file and of a
//! call of the app's reply tool, and the agent's reply to a thread. Built by the Keeper's
//! package "delivery" of slice 6.
//!
//! Called on this side's thread, under the Keeper's turn, once the report's events were
//! given to the Switchboard, for a report whose Adapter says it waits for something
//! (`yaar_agent::Adapter::asked`). What it gives is the piece's answer, sent at once;
//! `Fact::ReviewsChanged` is told through `Told::reviews_changed`, and the state of a
//! delivered round is recorded after, off the agent's way, through `Told::round_to_record`.
//!
//! - `Asks::Message`: the report's CLI session is mapped to the app's session
//!   (`Store::session_of_cli`), that session to its review (`Reviews::review_of_session`);
//!   when the review has something attached, `Reviews::send_attached` hands it out through
//!   the Adapter's delivery (`yaar_agent::Adapter::delivery`, with the Adapter's folder
//!   and the report's `Asked`), and the bytes it gives are the answer,
//!   `Reply::Answer`. The review's workspace's reviews changed, and the round is to be
//!   recorded. A report that reached the Keeper more than `DELIVER_BY` (1.5 s) before this
//!   runs is sent nothing and answered `Reply::Done`, its attachment kept: the hook program
//!   gives up at 2 s, and a review marked sent that no agent received would be lost.
//! - `Asks::ReadOwnFile`, `Asks::OwnTool`: `Adapter::answer` with `Answer::Allow`.
//! - `Asks::Reply`: `Reviews::agent_replied` for the session, answered with
//!   `Answer::Replied` (the thread, the mark, the reply's length in characters) or, when
//!   refused, `Answer::Refused` with the engine's reason; the reviews changed when kept.
//!
//! The session a report names. The reply tool's server reads the CLI's session id from its
//! environment once, when Claude Code starts it, and Claude Code keeps the server across
//! `/clear`, which gives the session a new id: the server's reports then name the session
//! `/clear` ended. So when the named CLI session does not run, or has no review, and a
//! running session of the same CLI has the report's process (`AgentReport::pid`, the CLI
//! itself, whose process a `/clear` keeps), that running session is the one the report is
//! of. Every other report is of the session it names.

use std::time::Instant;

use yaar_agent::Asked;
use yaar_link::{AgentReport, Reply};

use super::{Core, Told};
use crate::wiring::Kept;

impl Core {
    /// Answers what `report` waits for, `asked` as its Adapter read it: see this module's
    /// doc. `at` is the moment the report reached the Keeper. Answers `Reply::Done` until
    /// the package that builds delivery fills it in, as every report of slices 4 and 5
    /// was answered.
    pub(super) fn answer_asked(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
        at: Instant,
        told: &mut Told,
    ) -> Reply {
        let _ = (kept, report, asked, at, told);
        Reply::Done
    }
}
