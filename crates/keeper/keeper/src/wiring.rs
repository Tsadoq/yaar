//! Between the Link and the parts of the Keeper.
//!
//! The parts speak in their own words and know nothing of the Link. Here each request of a
//! Window is handed to the part that answers it, and what the part returns is put into the
//! words of the Link: a reply for the Window that asked, and the facts that every attached
//! Window is told.
//!
//! | Request | Answered by | Facts told to every Window |
//! | --- | --- | --- |
//! | `ListWorkspaces` | `Workspaces::repositories` | none |
//! | `OpenFolder` | `Workspaces::open_folder` | `Workspaces`, when the list changed |
//! | `SetBase` | `Workspaces::set_base` | `ChangesChanged` of that workspace |
//! | `GetChanges` | `Workspaces::changes` | none |
//! | `ReadFile` | `Workspaces::file` | none |
//!
//! The reply goes out before the facts. A Window that opened a folder therefore learns
//! the id of the new workspace before the list that holds it.
//!
//! A version of a file is sent as text unless it is larger than `yaar_link::MAX_CONTENT`,
//! or holds a zero byte among its first 8,000, which is how git tells that a file is not
//! text; of such a version only the size is sent.
//!
//! A reply too large for one frame, as the changes of a folder with a hundred thousand
//! new files can be, is not sent: the Window that asked gets `Refusal::Failed` with the
//! reason in its place, so that no request stays without an answer.
//!
//! Nothing is watched yet. The list of workspaces is read from git when a Window asks
//! for it, and no fact tells of a worktree or a branch that changed outside the app.

use std::io;

use yaar_link::{Fact, Refusal, Reply, Request};
use yaar_recorder::Git;
use yaar_store::Store;
use yaar_workspaces::{WorkspaceError, Workspaces};

use crate::Options;

/// The parts of the Keeper, started and connected.
pub(crate) struct Parts {
    workspaces: Workspaces,
}

/// What one request leads to.
pub(crate) struct Outcome {
    /// For the Window that asked.
    pub(crate) reply: Result<Reply, Refusal>,
    /// For every attached Window, the one that asked included, after the reply.
    pub(crate) facts: Vec<Fact>,
}

impl Parts {
    /// Starts the parts: finds git, opens the database `yaar_store::FILE_NAME` in the
    /// folder of `options.data_dir`, and puts Workspaces on both.
    ///
    /// A Keeper with no git still starts and answers the hello, so that a Window can say
    /// what is wrong; each request is then refused with the reason.
    pub(crate) fn start(options: &Options) -> io::Result<Parts> {
        todo!()
    }

    /// Answers one request of a Window.
    pub(crate) fn answer(&self, request: Request) -> Outcome {
        todo!()
    }
}

/// Puts a failure of Workspaces into the words of the Link: what the user can mend keeps
/// its kind, everything else becomes `Refusal::Failed` with the sentence of the error.
pub(crate) fn refusal(error: WorkspaceError) -> Refusal {
    todo!()
}
