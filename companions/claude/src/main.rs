//! `yaar-claude-hook`: the program Claude Code runs for each hook of the app's plugin.
//!
//! ```text
//! yaar-claude-hook < <the hook's input>
//! ```
//!
//! It passes the input to the Keeper and prints Claude Code nothing: see
//! `yaar_companion_claude::run_hook`.

use std::io::{Read, Write};
use std::process::ExitCode;

use yaar_companion_claude::{HookPlace, run_hook};

fn main() -> ExitCode {
    let mut input = Vec::new();
    // An input that cannot be read is passed on as far as it was read: the CLI goes on
    // either way.
    let _ = std::io::stdin().read_to_end(&mut input);
    let answer = run_hook(&input, &HookPlace::of_this_process());
    let _ = std::io::stdout().write_all(&answer.printed);
    ExitCode::from(u8::try_from(answer.status).unwrap_or(1))
}
