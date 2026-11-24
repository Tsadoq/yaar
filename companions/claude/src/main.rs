//! The program Claude Code runs for each hook: passes the input to the Keeper and prints its
//! `Reply::Answer` bytes, nothing for other replies.

use std::io::{Read, Write};
use std::process::ExitCode;

use yaar_companion_claude::run_hook;
use yaar_link::ReportPlace;

fn main() -> ExitCode {
    let mut input = Vec::new();
    // An unreadable input is passed on as far as read: the CLI goes on either way.
    let _ = std::io::stdin().read_to_end(&mut input);
    let printed = run_hook(&input, &ReportPlace::of_this_process());
    let _ = std::io::stdout().write_all(&printed);
    ExitCode::SUCCESS
}
