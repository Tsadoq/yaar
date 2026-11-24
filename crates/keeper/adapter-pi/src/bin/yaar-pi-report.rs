//! `yaar-pi-report [--tool] < <payload>`: the program pi's extension runs for each report.
//! Passes the payload to the Keeper and prints its answer.

use std::io::{Read, Write};
use std::process::ExitCode;

use yaar_adapter_pi::pass_on;
use yaar_link::ReportPlace;

fn main() -> ExitCode {
    let tool = std::env::args()
        .skip(1)
        .any(|argument| argument == "--tool");
    let mut payload = Vec::new();
    // Pass on what was read: pi goes on either way.
    let _ = std::io::stdin().read_to_end(&mut payload);
    let printed = pass_on(&payload, &ReportPlace::of_this_process(), tool);
    let _ = std::io::stdout().write_all(&printed);
    ExitCode::SUCCESS
}
