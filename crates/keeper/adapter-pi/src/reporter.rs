//! What the reporting program `yaar-pi-report` does: pi's extension runs it for each report
//! with the payload on its standard input, and reads what it prints.
//!
//! ```text
//! yaar-pi-report [--tool] < <the payload>
//! ```
//!
//! It passes the payload on, unread, to the Keeper at [`ReportPlace::socket`] as one
//! `yaar_link::Request::Report` of the Adapter [`crate::NAME`], with the terminal its
//! environment names and its parent, pi, as `pid`, reached with `yaar_link::connect`, which
//! makes sure the Keeper is the user's own, and says hello first as the Link asks. It reads
//! past the facts the Keeper may send until the reply to its one request.
//!
//! Without `--tool` it waits at most [`DEADLINE`] for the reply and prints the bytes of a
//! `yaar_link::Reply::Answer`, and nothing for any other reply, a refusal, no Keeper, a
//! Keeper of another protocol, or no reply in time: pi goes on as without the app.
//!
//! With `--tool`, for a call of the reply tool, it waits at most [`TOOL_DEADLINE`] and
//! prints one JSON object in every case: the bytes of the Keeper's `yaar_link::Reply::Answer`
//! as they are, else `{"error": <why>}` with `why` one of "the app did not keep the reply"
//! for `yaar_link::Reply::Done`, the Keeper's reason for a refusal, "the app did not answer
//! in time" past the deadline, and "the app is not running" with no Keeper or one of another
//! protocol. It ends with 0 in every case.

use std::path::PathBuf;
use std::time::Duration;

/// The longest the program waits for the Keeper, from connecting to the reply, for any
/// report but a call of the reply tool: pi waits for the extension's handler, and the user
/// for pi.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// The longest it waits for the Keeper's answer to a call of the reply tool: a reply is
/// kept with one write to the drive, and the agent waits for the tool's result.
pub const TOOL_DEADLINE: Duration = Duration::from_secs(10);

/// Where the program finds the Keeper, and what it says of the pi that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPlace {
    /// The socket the Keeper is asked to listen at: `yaar_link::socket_path` of the
    /// program's environment, which the app's shells set.
    pub socket: PathBuf,
    /// The terminal of the app pi runs in, from `yaar_link::TERMINAL_VARIABLE`; none
    /// outside the app's terminals and for an empty value.
    pub terminal: Option<String>,
    /// The program's parent process: pi, which runs it with no shell between.
    pub parent: u32,
}

impl ReportPlace {
    /// What the environment of this process says, and its parent.
    pub fn of_this_process() -> ReportPlace {
        todo!("where the reporting program finds the Keeper")
    }
}

/// What the program gives pi back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportAnswer {
    /// What it prints on its standard output, as the module's doc says.
    pub printed: Vec<u8>,
    /// Its exit status: 0, whatever became of the report.
    pub status: i32,
}

/// Passes `payload` to the Keeper at `place` as one report of pi, and gives what the
/// program prints: as the module's doc says, `tool` being whether it was run with `--tool`.
pub fn pass_on(payload: &[u8], place: &ReportPlace, tool: bool) -> ReportAnswer {
    let _ = (payload, place, tool);
    todo!("a report of pi passed to the Keeper")
}
