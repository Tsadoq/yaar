//! Claude Code companion: the hook program and the reply tool server of the app's plugin.

use std::time::{Duration, Instant};

use yaar_link::{Reply, ReportPlace, Reporter, report_to_keeper};

mod tools;

pub use tools::{SERVER_NAME, TOOL, TOOL_DEADLINE, ToolPlace, serve, tool};

const AGENT: &str = "claude";

/// The longest the hook program waits for the Keeper, from connecting to the reply.
/// Claude Code waits for the program, and so does the user at a permission prompt.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// A late or partial answer is not printed.
pub fn run_hook(input: &[u8], place: &ReportPlace) -> Vec<u8> {
    let sent = report_to_keeper(
        &Reporter {
            socket: &place.socket,
            who: concat!("yaar-claude-hook ", env!("CARGO_PKG_VERSION")),
            agent: AGENT,
            terminal: place.terminal.as_deref(),
            parent: place.parent,
        },
        input,
        Instant::now() + DEADLINE,
    );
    // A hook that said nothing serves Claude Code better than one that held the user up or failed.
    match sent {
        Ok(Ok(Reply::Answer { bytes })) => bytes,
        _ => Vec::new(),
    }
}
