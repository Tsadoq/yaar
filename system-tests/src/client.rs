//! The scripted client: what stands for the Window in a test.
//!
//! Four rules keep it honest.
//!
//! 1. It uses the public types of `yaar-link` and nothing else of the app. A test that
//!    needs something the Link cannot say has found a missing message, not a reason for
//!    a side door.
//! 2. A test asserts on what came over the Link and on the world outside the app: git,
//!    files, processes. Never on what the Keeper holds in memory.
//! 3. Each test has its own [`crate::World`], and so its own Keeper on its own socket.
//! 4. No call sleeps for a fixed time. Each waits for what it wants, up to a deadline,
//!    and panics with what it saw when the deadline passes.

use std::collections::VecDeque;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use yaar_link::{
    ChangedFile, Changes, Fact, FileSides, Hello, Refusal, Reply, Repository, RepositoryId,
    Request, WorkspaceId,
};

/// How long a call waits for the Keeper before it gives up.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// A connection to a Keeper, speaking the Link as a Window speaks it.
///
/// There is one call for each request of the Link. Facts that arrive while a call waits
/// for its reply are kept, in order, for [`Client::wait_for_fact`].
pub struct Client {
    stream: UnixStream,
    keeper: Hello,
    next_id: u32,
    facts: VecDeque<Fact>,
}

impl Client {
    /// Connects to the Keeper asked to listen at `socket` and exchanges hellos, waiting
    /// up to [`DEADLINE`] for a Keeper to listen. The socket is reached through
    /// `yaar_link::connect`, as a Window reaches it: where `yaar_link::socket_address`
    /// puts it, and only when what is there is the user's own.
    pub fn attach(socket: &Path) -> Client {
        todo!()
    }

    /// Who the Keeper said it is.
    pub fn keeper(&self) -> &Hello {
        todo!()
    }

    /// Sends one request and waits for its reply.
    pub fn request(&mut self, request: Request) -> Result<Reply, Refusal> {
        todo!()
    }

    /// Asks for every repository with its worktrees.
    pub fn workspaces(&mut self) -> Vec<Repository> {
        todo!()
    }

    /// Opens a folder as a workspace.
    pub fn open_folder(&mut self, folder: &Path) -> Result<(RepositoryId, WorkspaceId), Refusal> {
        todo!()
    }

    /// Sets the base of a workspace to a branch or a commit.
    pub fn set_base(&mut self, workspace: &WorkspaceId, base: &str) -> Result<(), Refusal> {
        todo!()
    }

    /// Asks for the changes of a workspace. Panics when the Keeper refuses.
    pub fn changes(&mut self, workspace: &WorkspaceId) -> Changes {
        todo!()
    }

    /// Asks for the two versions of one file of `changes`, as a Window does: the old
    /// one in the commit the changes are counted from, at the path the file had there,
    /// and the new one in the worktree. For a new file no old version is asked for.
    pub fn file(&mut self, changes: &Changes, file: &ChangedFile) -> Result<FileSides, Refusal> {
        todo!()
    }

    /// Waits for a fact that `wanted` accepts, among those kept and those still to
    /// come, and returns it. Facts before it are dropped. Panics, with the facts seen,
    /// when none comes within `limit`.
    pub fn wait_for_fact(&mut self, limit: Duration, wanted: impl FnMut(&Fact) -> bool) -> Fact {
        todo!()
    }
}
