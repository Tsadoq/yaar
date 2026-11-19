//! Claude Code companion (`X-CC-PLUGIN`).
//!
//! Owns: the hook program, the reply tool server and the two allow hooks that Claude Code
//! loads from the app's plugin folder.
//!
//! Surface: [`run_hook`], what the hook program `yaar-claude-hook` does, with
//! [`HookPlace`], where it finds the Keeper and what it says of its CLI, [`HookAnswer`],
//! what it gives Claude Code back, and [`DEADLINE`].
//!
//! Claude Code runs the hook program for every hook the app's plugin asks for, with the
//! hook's input on its standard input. The program passes that input on, unread, to the
//! Keeper as one `yaar_link::Request::Report`, with the terminal id its environment names
//! and its parent process, which is Claude Code. Its answer to Claude Code is empty: it
//! decides nothing, a permission request least of all, so Claude Code shows the user its
//! own dialog and the user answers it there. A Keeper that is not there, or that does not
//! answer within [`DEADLINE`], changes nothing of that: the program prints nothing and
//! succeeds, and Claude Code goes on as without the app.
//!
//! In slice 6 the same program answers the submit hook with the review that rides along,
//! and the reply tool server and the allow hooks join it.

use std::path::PathBuf;
use std::time::Duration;

/// The longest the hook program waits for the Keeper, from connecting to the reply. Claude
/// Code waits for the program, and so does the user at a permission prompt.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// Where the hook program finds the Keeper, and what it says of the CLI that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPlace {
    /// The socket the Keeper is asked to listen at: `yaar_link::socket_path` of the
    /// program's environment, which the app's shells set.
    pub socket: PathBuf,
    /// The terminal of the app Claude Code runs in, from `yaar_link::TERMINAL_VARIABLE`;
    /// none outside the app's terminals.
    pub terminal: Option<String>,
    /// The program's parent process: Claude Code.
    pub parent: u32,
}

impl HookPlace {
    /// What the environment of this process says, and its parent.
    pub fn of_this_process() -> HookPlace {
        todo!("the socket, the terminal and the parent of this process")
    }
}

/// What the hook program gives Claude Code back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAnswer {
    /// What it prints on its standard output. Empty: no decision, nothing added.
    pub printed: Vec<u8>,
    /// Its exit status: 0, whatever became of the report.
    pub status: i32,
}

/// Passes `input`, the hook's input as Claude Code gave it, to the Keeper at `place` as
/// one report of the Adapter `claude`, and gives the answer for Claude Code. Waits at
/// most [`DEADLINE`] for the Keeper.
pub fn run_hook(input: &[u8], place: &HookPlace) -> HookAnswer {
    let _ = (input, place);
    todo!("pass the hook's input on and answer with nothing")
}
