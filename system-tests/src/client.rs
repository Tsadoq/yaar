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

use std::collections::{HashMap, HashSet, VecDeque};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use yaar_link::{
    ChangedFile, Changes, CommentId, DraftOn, Fact, FileSides, FileStatus, Hello, Message, Program,
    Refusal, Reply, Repository, RepositoryId, Request, RequestId, ReviewId, TerminalId,
    TerminalSize, ThreadId, WorkspaceId, WorkspaceReviews, handshake, read_message, write_message,
};

/// How long a call waits for the Keeper before it gives up.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// A connection to a Keeper, speaking the Link as a Window speaks it.
///
/// There is one call for each request of the Link. Facts that arrive while a call waits
/// for its reply are kept, in order, for [`Client::wait_for_fact`]. What the program of a
/// terminal writes is kept, as the bytes that came, for [`Client::wait_for_output`],
/// whichever call was waiting when it arrived.
pub struct Client {
    stream: UnixStream,
    keeper: Hello,
    next_id: u32,
    facts: VecDeque<Fact>,
    /// What each terminal's program has written so far, as it came.
    output: HashMap<TerminalId, Vec<u8>>,
    /// The requests sent with [`Client::send`] whose reply has not been taken.
    sent: HashSet<RequestId>,
    /// Replies that came while another was waited for, until their request is waited for.
    aside: HashMap<RequestId, Result<Reply, Refusal>>,
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
                            output: HashMap::new(),
                            sent: HashSet::new(),
                            aside: HashMap::new(),
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

    /// Sends one request and waits for its reply. The replies to requests sent with
    /// [`Client::send`] that come meanwhile are set aside.
    pub fn request(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.write(request);
        self.wait_for(id, Instant::now() + DEADLINE)
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

    /// Opens a terminal with the user's shell in it for a workspace, at `size`, and
    /// returns its id.
    pub fn open_terminal(
        &mut self,
        workspace: &WorkspaceId,
        size: TerminalSize,
    ) -> Result<TerminalId, Refusal> {
        match self.request(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size,
        })? {
            Reply::TerminalOpened { terminal } => {
                self.output.entry(terminal.clone()).or_default();
                Ok(terminal)
            }
            other => panic!("asked to open a terminal, the Keeper answered {other:?}"),
        }
    }

    /// Tells the Keeper that a terminal is shown at a new size.
    pub fn resize_terminal(
        &mut self,
        terminal: &TerminalId,
        size: TerminalSize,
    ) -> Result<(), Refusal> {
        match self.request(Request::ResizeTerminal {
            terminal: terminal.clone(),
            size,
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to resize a terminal, the Keeper answered {other:?}"),
        }
    }

    /// Sends bytes to the program of a terminal, as a Window sends what the user types.
    /// Nothing answers them.
    pub fn type_into(&mut self, terminal: &TerminalId, bytes: &[u8]) {
        write_message(
            &mut self.stream,
            &Message::Input {
                terminal: terminal.clone(),
                bytes: bytes.to_vec(),
            },
        )
        .unwrap_or_else(|e| panic!("the bytes for terminal {terminal} could not be sent: {e}"));
    }

    /// Waits until everything the program of `terminal` has written so far, as one run
    /// of bytes, is accepted by `wanted`, and returns that run. Panics, with what came,
    /// when it is not accepted within `limit`.
    ///
    /// The run starts at the program's first byte and is never cut: a later call sees
    /// what an earlier one saw, and more. A test that types a command and waits for its
    /// answer should look for text the command prints and does not hold itself, as
    /// `echo yaar-$((40 + 2))` prints `yaar-42`: the terminal echoes what is typed.
    pub fn wait_for_output(
        &mut self,
        terminal: &TerminalId,
        limit: Duration,
        mut wanted: impl FnMut(&[u8]) -> bool,
    ) -> Vec<u8> {
        let deadline = Instant::now() + limit;
        loop {
            let so_far = self.output.entry(terminal.clone()).or_default();
            if wanted(so_far) {
                return so_far.clone();
            }
            let so_far = so_far.len();
            match self.read(deadline) {
                Ok(Message::Output { terminal, bytes }) => self.keep(terminal, bytes),
                Ok(Message::Fact(fact)) => self.facts.push_back(fact),
                Ok(other) => panic!("waiting for output, the Keeper sent {other:?}"),
                Err(e) => panic!(
                    "the output of terminal {terminal} was not what was wanted within {limit:?}: \
                     {e}; the {so_far} bytes that came: {:?}",
                    String::from_utf8_lossy(&self.output[terminal])
                ),
            }
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
                    Ok(Message::Output { terminal, bytes }) => {
                        self.keep(terminal, bytes);
                        continue;
                    }
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

    /// Sends one request and does not wait for its reply: as a Window that goes on while
    /// the Keeper answers, and as a test that kills the Keeper while a request is on its
    /// way. Gives the request's id. The reply, when it comes, is set aside by the calls
    /// that wait, and is read with [`Client::wait_for_reply`].
    pub fn send(&mut self, request: Request) -> RequestId {
        let id = self.write(request);
        self.sent.insert(id);
        id
    }

    /// Waits for the reply to a request [`Client::send`] sent, among those set aside and
    /// those still to come. Panics, with what came, when none comes within `limit`.
    pub fn wait_for_reply(&mut self, id: RequestId, limit: Duration) -> Result<Reply, Refusal> {
        assert!(
            self.sent.contains(&id) || self.aside.contains_key(&id),
            "request {id:?} was not sent with Client::send, or its reply was taken already"
        );
        self.wait_for(id, Instant::now() + limit)
    }

    /// Asks for the reviews of a workspace and its files marked viewed. Panics when the
    /// Keeper refuses.
    pub fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.request(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("asked for the reviews, the Keeper answered {other:?}"),
        }
    }

    /// Saves the text of a draft, as a Window does at each change of a comment box, and
    /// waits until the Keeper keeps it.
    pub fn save_draft(
        &mut self,
        review: &ReviewId,
        thread: &ThreadId,
        comment: &CommentId,
        on: DraftOn,
        text: &str,
    ) -> Result<(), Refusal> {
        match self.request(Request::SaveDraft {
            review: review.clone(),
            thread: thread.clone(),
            comment: comment.clone(),
            on,
            text: text.to_owned(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to save a draft, the Keeper answered {other:?}"),
        }
    }

    /// Deletes a draft.
    pub fn delete_draft(&mut self, review: &ReviewId, comment: &CommentId) -> Result<(), Refusal> {
        match self.request(Request::DeleteDraft {
            review: review.clone(),
            comment: comment.clone(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to delete a draft, the Keeper answered {other:?}"),
        }
    }

    /// Copies a review as text: gives the text and the number of the round it started.
    pub fn copy_review(&mut self, review: &ReviewId) -> Result<(String, u32), Refusal> {
        match self.request(Request::CopyReview {
            review: review.clone(),
        })? {
            Reply::Copied { text, round } => Ok((text, round)),
            other => panic!("asked to copy a review, the Keeper answered {other:?}"),
        }
    }

    /// Marks the change of a review reviewed.
    pub fn mark_reviewed(&mut self, review: &ReviewId) -> Result<(), Refusal> {
        match self.request(Request::MarkReviewed {
            review: review.clone(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to mark a review reviewed, the Keeper answered {other:?}"),
        }
    }

    /// Marks a file of a workspace viewed, or takes the mark away.
    pub fn set_viewed(
        &mut self,
        workspace: &WorkspaceId,
        path: &Path,
        viewed: bool,
    ) -> Result<(), Refusal> {
        match self.request(Request::SetViewed {
            workspace: workspace.clone(),
            path: path.to_owned(),
            viewed,
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to mark a file viewed, the Keeper answered {other:?}"),
        }
    }

    /// Commits every uncommitted change of a workspace with `message`: gives the id of the
    /// commit.
    pub fn commit(&mut self, workspace: &WorkspaceId, message: &str) -> Result<String, Refusal> {
        match self.request(Request::Commit {
            workspace: workspace.clone(),
            message: message.to_owned(),
        })? {
            Reply::Committed { commit } => Ok(commit),
            other => panic!("asked to commit, the Keeper answered {other:?}"),
        }
    }

    /// Writes one request and gives the id it went out under.
    fn write(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        let kind = request.kind();
        write_message(&mut self.stream, &Message::Request { id, request })
            .unwrap_or_else(|e| panic!("the request {kind:?} could not be sent: {e}"));
        id
    }

    /// Reads until the reply to `id` is there, setting aside the replies to requests
    /// that [`Client::send`] sent.
    fn wait_for(&mut self, id: RequestId, deadline: Instant) -> Result<Reply, Refusal> {
        if let Some(reply) = self.aside.remove(&id) {
            return reply;
        }
        loop {
            match self.read(deadline) {
                Ok(Message::Reply { to, reply }) if to == id => {
                    self.sent.remove(&id);
                    return reply;
                }
                Ok(Message::Reply { to, reply }) if self.sent.remove(&to) => {
                    self.aside.insert(to, reply);
                }
                Ok(Message::Fact(fact)) => self.facts.push_back(fact),
                Ok(Message::Output { terminal, bytes }) => self.keep(terminal, bytes),
                Ok(other) => panic!("waiting for the reply to {id:?}, the Keeper sent {other:?}"),
                Err(e) => panic!("no reply to {id:?}: {e}; facts kept: {:?}", self.facts),
            }
        }
    }

    /// Keeps what a program wrote, after what it wrote before.
    fn keep(&mut self, terminal: TerminalId, bytes: Vec<u8>) {
        self.output.entry(terminal).or_default().extend(bytes);
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
