//! `yaar-claude-tools`: the server of the app's reply tool, which Claude Code starts from
//! the app's plugin and speaks MCP with over the program's standard input and output.
//!
//! ```text
//! yaar-claude-tools
//! ```
//!
//! See `yaar_companion_claude::serve`.

use std::process::ExitCode;

use yaar_companion_claude::{ToolPlace, serve};

fn main() -> ExitCode {
    let stdin = std::io::stdin();
    match serve(
        stdin.lock(),
        std::io::stdout(),
        &ToolPlace::of_this_process(),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        // Claude Code has gone, or no longer reads: there is nobody to tell.
        Err(_) => ExitCode::FAILURE,
    }
}
