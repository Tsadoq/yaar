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
use std::time::{Duration, Instant};

use yaar_link::{
    ChangedFile, Changes, Fact, FileSides, FileStatus, Hello, Message, Refusal, Reply, Repository,
    RepositoryId, Request, RequestId, WorkspaceId, handshake, read_message, write_message,
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
    ///
    /// Panics at once when what is there is refused as not the user's own, and at the
    /// deadline when no Keeper has answered.
    pub fn attach(socket: &Path) -> Client {
        let mine = Hello::for_this_process("scripted-window");
        let deadline = Instant::now() + DEADLINE;
        loop {
            let last = match yaar_link::connect(socket) {
                Ok(mut stream) => match handshake(&mut stream, &mine) {
                    Ok(keeper) => {
                        return Client {
                            stream,
                            keeper,
                            next_id: 0,
                            facts: VecDeque::new(),
                        };
                    }
                    // A Keeper that is just leaving may accept and then close.
                    Err(e) => e.to_string(),
                },
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    panic!("{} is not this user's own: {e}", socket.display())
                }
                Err(e) => e.to_string(),
            };
            assert!(
                Instant::now() < deadline,
                "no Keeper at {}: {last}",
                socket.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Who the Keeper said it is.
    pub fn keeper(&self) -> &Hello {
        &self.keeper
    }

    /// Sends one request and waits for its reply.
    pub fn request(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        let kind = request.kind();
        write_message(&mut self.stream, &Message::Request { id, request })
            .unwrap_or_else(|e| panic!("the request {kind:?} could not be sent: {e}"));
        let deadline = Instant::now() + DEADLINE;
        loop {
            match self.read(deadline) {
                Ok(Message::Reply { to, reply }) if to == id => return reply,
                Ok(Message::Fact(fact)) => self.facts.push_back(fact),
                Ok(other) => panic!("waiting for the reply to {kind:?}, the Keeper sent {other:?}"),
                Err(e) => panic!("no reply to {kind:?}: {e}; facts kept: {:?}", self.facts),
            }
        }
    }

    /// Asks for every repository with its worktrees.
    pub fn workspaces(&mut self) -> Vec<Repository> {
        match self.request(Request::ListWorkspaces) {
            Ok(Reply::Workspaces(repositories)) => repositories,
            other => panic!("asked for the workspaces, the Keeper answered {other:?}"),
        }
    }

    /// Opens a folder as a workspace.
    pub fn open_folder(&mut self, folder: &Path) -> Result<(RepositoryId, WorkspaceId), Refusal> {
        match self.request(Request::OpenFolder {
            path: folder.to_owned(),
        })? {
            Reply::Opened {
                repository,
                workspace,
            } => Ok((repository, workspace)),
            other => panic!("asked to open a folder, the Keeper answered {other:?}"),
        }
    }

    /// Sets the base of a workspace to a branch or a commit.
    pub fn set_base(&mut self, workspace: &WorkspaceId, base: &str) -> Result<(), Refusal> {
        match self.request(Request::SetBase {
            workspace: workspace.clone(),
            base: base.to_owned(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to set a base, the Keeper answered {other:?}"),
        }
    }

    /// Asks for the changes of a workspace. Panics when the Keeper refuses.
    pub fn changes(&mut self, workspace: &WorkspaceId) -> Changes {
        match self.request(Request::GetChanges {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Changes(changes)) => changes,
            other => panic!("asked for the changes, the Keeper answered {other:?}"),
        }
    }

    /// Asks for the two versions of one file of `changes`, as a Window does: the old
    /// one in the commit the changes are counted from, at the path the file had there,
    /// and the new one in the worktree. For a new file no old version is asked for.
    pub fn file(&mut self, changes: &Changes, file: &ChangedFile) -> Result<FileSides, Refusal> {
        let (since, old_path) = match &file.status {
            FileStatus::Added => (None, file.path.clone()),
            FileStatus::Renamed { from } => (changes.counted_from.clone(), from.clone()),
            FileStatus::Modified | FileStatus::Deleted => {
                (changes.counted_from.clone(), file.path.clone())
            }
        };
        match self.request(Request::ReadFile {
            workspace: changes.workspace.clone(),
            since,
            old_path,
            path: file.path.clone(),
        })? {
            Reply::File(sides) => Ok(sides),
            other => panic!("asked for a file, the Keeper answered {other:?}"),
        }
    }

    /// Waits for a fact that `wanted` accepts, among those kept and those still to
    /// come, and returns it. Facts before it are dropped. Panics, with the facts seen,
    /// when none comes within `limit`.
    pub fn wait_for_fact(
        &mut self,
        limit: Duration,
        mut wanted: impl FnMut(&Fact) -> bool,
    ) -> Fact {
        let deadline = Instant::now() + limit;
        let mut seen = Vec::new();
        loop {
            let fact = match self.facts.pop_front() {
                Some(kept) => kept,
                None => match self.read(deadline) {
                    Ok(Message::Fact(fact)) => fact,
                    Ok(other) => panic!("waiting for a fact, the Keeper sent {other:?}"),
                    Err(e) => panic!("the fact did not come: {e}; facts seen: {seen:?}"),
                },
            };
            if wanted(&fact) {
                return fact;
            }
            seen.push(fact);
        }
    }

    /// The next frame from the Keeper, or why none came before `deadline`.
    fn read(&mut self, deadline: Instant) -> Result<Message, String> {
        // A deadline that has passed still leaves the time to take what is there.
        let left = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        self.stream
            .set_read_timeout(Some(left))
            .map_err(|e| e.to_string())?;
        read_message(&mut self.stream).map_err(|e| e.to_string())
    }
}
