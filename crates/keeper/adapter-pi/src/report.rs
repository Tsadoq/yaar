//! What pi's extension reports, read as the app's events, what a report asks, and what the
//! Adapter answers: the tables of the crate's doc.

use std::collections::HashSet;
use std::path::Path;

use yaar_agent::{Answer, Asked, Report, ReportError, Reported};

/// What the Adapter remembers between reports: for each session, by pi's id of it, whether
/// a run of its agent is open, so that a message pi queues while it works starts no turn.
#[derive(Debug, Default)]
pub(crate) struct Remembered {
    /// pi's session ids whose run is open.
    open: HashSet<String>,
}

impl Remembered {
    /// The events of one report, as the crate's doc says. A payload that is no JSON object,
    /// or names no `event`, is [`ReportError::Unreadable`]; one whose `event` the crate doc's
    /// table does not name gives no event and no error, whatever else it holds (a recording's
    /// `factory` line names no session); one the table names with no `sessionId` is
    /// [`ReportError::Unreadable`].
    pub(crate) fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        let _ = (report, &mut self.open);
        todo!("pi's events, read as the app's")
    }

    /// What one report asks: see the crate's doc. It remembers nothing; `folder` is not
    /// read, since pi's delivery writes no file.
    pub(crate) fn asked(
        &self,
        folder: &Path,
        report: &Report<'_>,
    ) -> Result<Option<Asked>, ReportError> {
        let _ = (folder, report);
        todo!("what a report of pi asks")
    }

    /// The answer to a call of the reply tool, as bytes the extension reads:
    /// `{"text": "recorded reply to <thread> (addressed|open, <n> characters)"}` for
    /// [`Answer::Replied`], `{"error": <why>}` for [`Answer::Refused`], and nothing for
    /// [`Answer::Allow`], which pi never asks for.
    pub(crate) fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        let _ = (asked, answer);
        todo!("what pi's extension is answered")
    }
}
