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
//!   runs is sent nothing and answered `Reply::Done`, its attachment kept, and a line on
//!   standard error says how long it waited: the hook program gives up at 2 s, and a review
//!   marked sent that no agent received would be lost.
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
//! of. Every other report is of the session it names. Each condition has its own case: a
//! session that ended without a `/clear` keeps its review while another session of its
//! process runs, and a running session whose review is of no agent (the app gives every
//! session that starts a review, so that state is one the Store allows and the app does not
//! yet make).

use std::time::{Duration, Instant};

use yaar_agent::{Answer, Asked, Asks};
use yaar_link::{AgentReport, Reply};
use yaar_review_engine::ThreadReply;
use yaar_store::{SentBy, SessionId, SessionRecord, ThreadId};
use yaar_switchboard::State;

use super::{Core, Told};
use crate::lock;
use crate::wiring::Kept;

/// The longest a report may have waited, from the moment it reached the Keeper, for a review
/// still to be sent with it. The hook program gives up at 2 s, and what it prints after that
/// goes nowhere; the comments are marked sent only here, so a send that late could mark
/// comments the agent never received. Half a second is left for the answer to reach the hook
/// program once the send is kept, which is a few writes to the drive.
const DELIVER_BY: Duration = Duration::from_millis(1500);

/// Whether a report that reached the Keeper at `at` is too old at `now` to be sent a review.
fn late(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) > DELIVER_BY
}

/// How long a report that reached the Keeper at `at` has waited, when that is too long for
/// it to be sent a review; none otherwise. The one place that reads the clock for `late`, so
/// that its two instants are never given in the wrong order at the call.
fn waited_too_long(at: Instant) -> Option<Duration> {
    let now = Instant::now();
    late(at, now).then(|| now.saturating_duration_since(at))
}

impl Core {
    /// Answers what `report` waits for, `asked` as its Adapter read it: see this module's
    /// doc. `at` is the moment the report reached the Keeper.
    pub(super) fn answer_asked(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
        at: Instant,
        told: &mut Told,
    ) -> Reply {
        match &asked.asks {
            Asks::Message => self.deliver(kept, report, asked, at, told),
            Asks::ReadOwnFile | Asks::OwnTool => self.answer(report, asked, &Answer::Allow),
            Asks::Reply(reply) => {
                let Some(session) = self.session_of(kept, report, asked) else {
                    return self.answer(
                        report,
                        asked,
                        &Answer::Refused {
                            why: "the app knows no session of this agent",
                        },
                    );
                };
                let given = ThreadReply {
                    thread: ThreadId::new(reply.thread.as_str()),
                    text: reply.text.clone(),
                    addressed: reply.addressed,
                };
                match kept.reviews.agent_replied(&session.id, &given) {
                    Ok(_) => {
                        told.reviews_changed(&session.workspace);
                        self.answer(
                            report,
                            asked,
                            &Answer::Replied {
                                thread: &reply.thread,
                                addressed: reply.addressed,
                                characters: reply.text.chars().count(),
                            },
                        )
                    }
                    Err(e) => self.answer(
                        report,
                        asked,
                        &Answer::Refused {
                            why: &e.to_string(),
                        },
                    ),
                }
            }
        }
    }

    /// What the Adapter of `report` hands its piece for `answer`, as `Reply::Answer`; `Done`
    /// for an Adapter the Keeper does not have.
    fn answer(&self, report: &AgentReport, asked: &Asked, answer: &Answer<'_>) -> Reply {
        let adapters = lock(&self.adapters);
        match adapters
            .iter()
            .find(|adapter| adapter.name() == report.agent)
        {
            Some(adapter) => Reply::Answer {
                bytes: adapter.answer(asked, answer),
            },
            None => Reply::Done,
        }
    }

    /// The user's message: sends what is attached to the review of its session, and answers
    /// with what the Adapter's delivery made of it; `Done` when there is nothing to carry, or
    /// when the report is too old to be answered in time.
    fn deliver(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
        at: Instant,
        told: &mut Told,
    ) -> Reply {
        let Some(session) = self.session_of(kept, report, asked) else {
            return Reply::Done;
        };
        let review = match kept.reviews.review_of_session(&session.id) {
            Ok(Some(review)) => review,
            Ok(None) => return Reply::Done,
            Err(e) => {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                return Reply::Done;
            }
        };
        // One lock of the Adapters for the delivery and the send: the engine calls the
        // delivery while it holds its own turn.
        let adapters = lock(&self.adapters);
        let Some(adapter) = adapters
            .iter()
            .find(|adapter| adapter.name() == report.agent)
        else {
            return Reply::Done;
        };
        let Some(folder) = self.folder_of(&report.agent) else {
            return Reply::Done;
        };
        // Checked last, as near the send as it can be.
        if let Some(waited) = waited_too_long(at) {
            eprintln!(
                "yaar-keeper: {}: a message waited {waited:?} for its answer, more than {DELIVER_BY:?}: \
                 nothing was sent and the review stays attached",
                report.agent
            );
            return Reply::Done;
        }
        let mut delivery = adapter.delivery(folder, asked);
        match kept
            .reviews
            .send_attached(&review, SentBy::Delivered, &mut *delivery)
        {
            Ok(Some((bytes, sent))) => {
                told.reviews_changed(&session.workspace);
                // The state of the worktree is recorded after, off the agent's way.
                told.round_to_record(&session.workspace, &review, sent.round.number);
                Reply::Answer { bytes }
            }
            Ok(None) => Reply::Done,
            // Nothing was sent: the comments stay attached and the message goes as it is.
            Err(e) => {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                Reply::Done
            }
        }
    }

    /// The session `report` is of: the one the report's CLI names, except where that one
    /// does not run or has no review and a session of the same CLI that runs has the
    /// report's process, which is a CLI whose session id changed under a tool server that
    /// kept the old one.
    fn session_of(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
    ) -> Option<SessionRecord> {
        let named = kept
            .store
            .session_of_cli(&report.agent, &asked.session.0)
            .unwrap_or_else(|e| {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                None
            });
        let running = lock(&self.switchboard)
            .sessions()
            .into_iter()
            .filter(|known| known.agent == report.agent && known.state != State::NotRunning)
            .collect::<Vec<_>>();
        if let Some(named) = &named
            && running.iter().any(|known| known.id.0 == named.id.as_str())
            && matches!(kept.reviews.review_of_session(&named.id), Ok(Some(_)))
        {
            return Some(named.clone());
        }
        let process = running
            .iter()
            .find(|known| known.pid == Some(report.pid) && known.cli != asked.session);
        match process {
            Some(known) => kept
                .store
                .session(&SessionId::new(known.id.0.clone()))
                .ok()
                .flatten()
                .or(named),
            None => named,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_late_once_it_has_waited_longer_than_the_limit_and_not_before() {
        let at = Instant::now();
        assert!(!late(at, at));
        assert!(!late(at, at + DELIVER_BY));
        assert!(late(at, at + DELIVER_BY + Duration::from_millis(1)));
        assert!(late(at, at + Duration::from_secs(10)));
    }

    #[test]
    fn the_wait_of_a_report_is_given_when_it_is_too_long_and_not_otherwise() {
        assert_eq!(waited_too_long(Instant::now()), None);
        let old = Instant::now()
            .checked_sub(DELIVER_BY * 2)
            .expect("the machine has been up for longer than three seconds");
        let waited = waited_too_long(old).expect("a report that waited twice the limit is late");
        assert!(waited >= DELIVER_BY * 2, "{waited:?}");
    }

    #[test]
    fn a_clock_that_reads_earlier_than_the_report_is_not_late() {
        let now = Instant::now();
        assert!(!late(now + Duration::from_secs(1), now));
    }

    #[test]
    fn the_limit_leaves_the_hook_program_time_to_print_what_it_is_answered() {
        // The hook program gives up at 2 s; a send kept at the limit is answered within the
        // half second that is left, which is a few writes to the drive and a local socket.
        assert!(DELIVER_BY + Duration::from_millis(500) <= Duration::from_secs(2));
    }
}
