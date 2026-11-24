//! `yaar-pi-report [--tool] < <payload>`: the program pi's extension runs for each report.
//! Passes the payload to the Keeper and prints its answer.

use std::io::{Read, Write};
use std::process::ExitCode;

use yaar_adapter_pi::{ReportPlace, pass_on};

fn main() -> ExitCode {
    let tool = std::env::args()
        .skip(1)
        .any(|argument| argument == "--tool");
    let mut payload = Vec::new();
    // Pass on what was read: pi goes on either way.
    let _ = std::io::stdin().read_to_end(&mut payload);
    let answer = pass_on(&payload, &ReportPlace::of_this_process(), tool);
    let _ = std::io::stdout().write_all(&answer.printed);
    ExitCode::SUCCESS
}
