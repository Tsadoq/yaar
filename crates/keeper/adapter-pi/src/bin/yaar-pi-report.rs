//! `yaar-pi-report`: the program pi's extension runs for each report.
//!
//! ```text
//! yaar-pi-report [--tool] < <the payload>
//! ```
//!
//! It passes the payload to the Keeper and prints what the Keeper answers: see
//! `yaar_adapter_pi::pass_on`.

use std::io::{Read, Write};
use std::process::ExitCode;

use yaar_adapter_pi::{ReportPlace, pass_on};

fn main() -> ExitCode {
    let tool = std::env::args()
        .skip(1)
        .any(|argument| argument == "--tool");
    let mut payload = Vec::new();
    // A payload that cannot be read is passed on as far as it was read: pi goes on either
    // way.
    let _ = std::io::stdin().read_to_end(&mut payload);
    let answer = pass_on(&payload, &ReportPlace::of_this_process(), tool);
    let _ = std::io::stdout().write_all(&answer.printed);
    ExitCode::from(u8::try_from(answer.status).unwrap_or(1))
}
