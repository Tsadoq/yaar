//! Claude Code starts this MCP server (stdio) for the app's reply tool.

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
