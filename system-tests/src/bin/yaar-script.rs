//! `yaar-script`: a scripted Window, as one command, for the jobs of the workflow that drive
//! the installed app where no key can be pressed (row PKG-12). Test support: no package ships
//! it.
//!
//! ```text
//! yaar-script worktree-with-agent --repository <folder> --branch <new branch> --agent <claude|pi>
//!             [--socket <path>] [--deadline <seconds>]
//! yaar-script stop [--socket <path>] [--deadline <seconds>]
//! ```
//!
//! Connects to the Keeper that already listens at `--socket` (default: the socket of
//! `yaar_link::socket_path()`, where the Keeper of the installed app listens) as a Window
//! does, with the hello of a Window of this build's version. It never starts a Keeper: when
//! none listens it says so and exits 2. Then, as a user would with Cmd-Shift-N:
//!
//! 1. opens `--repository` as a workspace (the request a Window sends for Open Folder), unless
//!    the Keeper lists it already;
//! 2. sends `CreateWorktree` for that repository, from the branch its workspace is on, on the
//!    new branch `--branch`, with the agent `--agent`, and a terminal of 80 by 24;
//! 3. waits for the reply `WorktreeCreated`, then for the Keeper's fact that a session of
//!    that agent runs in the new workspace.
//!
//! Each wait lasts at most `--deadline` seconds (default 60). On success it prints one line,
//! `worktree=<absolute folder> workspace=<id> session=<id>`, and exits 0. A refusal prints the
//! Keeper's sentence and exits 1; a deadline passed prints which wait ran out and what had
//! arrived, and exits 1. A command line it does not take prints the usage above and exits 2.
//!
//! `stop` is the quit that also stops the agents: it connects as above (exit 2 when no Keeper
//! listens), says hello, sends the Link's stop (`Message::Shutdown`, whose shape no version
//! changes), and waits until the socket no longer answers (a connect refused or the socket
//! gone), for at most `--deadline` seconds (default 20). Done, it prints `stopped` and exits
//! 0; past the deadline it says so and exits 1.

fn main() -> std::process::ExitCode {
    todo!()
}
