//! The reporting program `yaar-pi-report [--tool] < <payload>`: sends the payload unread to the
//! Keeper as one `Request::Report` and prints the bytes of a `Reply::Answer`, else nothing. With
//! `--tool` it prints one JSON object in every case: the answer, else `{"error": <why>}`.

use std::time::{Duration, Instant};

use yaar_link::{Reply, ReportPlace, Reporter, Unanswered, report_to_keeper};

use crate::NAME;

const NOT_KEPT: &str = "the app did not keep the reply";

const TOO_LATE: &str = "the app did not answer in time";

const NOT_RUNNING: &str = "the app is not running";

/// Longest wait for the Keeper, connect to reply: pi waits for the handler, the user for pi.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// Longest wait for the answer to a reply-tool call: the agent waits for the tool's result.
pub const TOOL_DEADLINE: Duration = Duration::from_secs(10);

pub fn pass_on(payload: &[u8], place: &ReportPlace, tool: bool) -> Vec<u8> {
    let waits = if tool { TOOL_DEADLINE } else { DEADLINE };
    let sent = report_to_keeper(
        &Reporter {
            socket: &place.socket,
            who: concat!("yaar-pi-report ", env!("CARGO_PKG_VERSION")),
            agent: NAME,
            terminal: place.terminal.as_deref(),
            parent: place.parent,
        },
        payload,
        Instant::now() + waits,
    );
    match (sent, tool) {
        (Ok(Ok(Reply::Answer { bytes })), _) => bytes,
        (_, false) => Vec::new(),
        (Ok(Ok(_)), true) | (Err(Unanswered::Failed), true) => error_object(NOT_KEPT),
        (Ok(Err(refusal)), true) => error_object(&refusal.to_string()),
        (Err(Unanswered::TooLate), true) => error_object(TOO_LATE),
        (Err(Unanswered::NotRunning), true) => error_object(NOT_RUNNING),
    }
}

fn error_object(why: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "error": why })).unwrap_or_default()
}
