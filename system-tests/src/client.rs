//! The scripted client that stands for the Window in a test, using only `yaar-link`'s public types.
//! No call sleeps for a fixed time: each waits up to a deadline and panics with what it saw.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use yaar_link::{
    AgentCli, ChangedFile, Changes, CommentId, Consent, ConsentAnswer, CreditedFile, DraftOn, Fact,
    FileSides, FileStatus, Hello, History, Message, Program, Refusal, Reply, Repository,
    RepositoryId, Request, RequestId, ReviewId, TerminalId, TerminalInfo, TerminalSize, ThreadId,
    Turn, TurnId, WorkspaceAgents, WorkspaceId, WorkspaceReviews, handshake, read_message,
    write_message,
};

pub const DEADLINE: Duration = Duration::from_secs(10);

struct Screen {
    rows: usize,
    columns: usize,
}

impl Dimensions for Screen {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// Facts that arrive while a call waits for its reply are kept, in order, for
/// [`Client::wait_for_fact`].
pub struct Client {
    stream: UnixStream,
    keeper: Hello,
    next_id: u32,
    facts: VecDeque<Fact>,
    output: HashMap<TerminalId, Vec<u8>>,
    sent: HashSet<RequestId>,
    aside: HashMap<RequestId, Result<Reply, Refusal>>,
}

impl Client {
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

    pub fn keeper(&self) -> &Hello {
        &self.keeper
    }

    pub fn request(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.write(request);
        self.wait_for(id, Instant::now() + DEADLINE)
    }

    pub fn workspaces(&mut self) -> Vec<Repository> {
        match self.request(Request::ListWorkspaces) {
            Ok(Reply::Workspaces(repositories)) => repositories,
            other => panic!("asked for the workspaces, the Keeper answered {other:?}"),
        }
    }

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

    pub fn set_base(&mut self, workspace: &WorkspaceId, base: &str) -> Result<(), Refusal> {
        match self.request(Request::SetBase {
            workspace: workspace.clone(),
            base: base.to_owned(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to set a base, the Keeper answered {other:?}"),
        }
    }

    pub fn changes(&mut self, workspace: &WorkspaceId) -> Changes {
        match self.request(Request::GetChanges {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Changes(changes)) => changes,
            other => panic!("asked for the changes, the Keeper answered {other:?}"),
        }
    }

    /// The old version is read from the commit the changes count from, at the file's old path; a
    /// new file has none.
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

    /// `wanted` sees everything the terminal wrote from its first byte, so a later call sees what
    /// an earlier one saw. The terminal echoes what is typed: wait for text the command prints
    /// (`echo yaar-$((40 + 2))` prints `yaar-42`), not for the command itself.
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

    /// Facts before the one accepted are dropped.
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

    pub fn send(&mut self, request: Request) -> RequestId {
        let id = self.write(request);
        self.sent.insert(id);
        id
    }

    pub fn wait_for_reply(&mut self, id: RequestId, limit: Duration) -> Result<Reply, Refusal> {
        assert!(
            self.sent.contains(&id) || self.aside.contains_key(&id),
            "request {id:?} was not sent with Client::send, or its reply was taken already"
        );
        self.wait_for(id, Instant::now() + limit)
    }

    pub fn reviews(&mut self, workspace: &WorkspaceId) -> WorkspaceReviews {
        match self.request(Request::GetReviews {
            workspace: workspace.clone(),
        }) {
            Ok(Reply::Reviews(reviews)) => reviews,
            other => panic!("asked for the reviews, the Keeper answered {other:?}"),
        }
    }

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

    pub fn delete_draft(&mut self, review: &ReviewId, comment: &CommentId) -> Result<(), Refusal> {
        match self.request(Request::DeleteDraft {
            review: review.clone(),
            comment: comment.clone(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to delete a draft, the Keeper answered {other:?}"),
        }
    }

    pub fn copy_review(&mut self, review: &ReviewId) -> Result<(String, u32), Refusal> {
        match self.request(Request::CopyReview {
            review: review.clone(),
        })? {
            Reply::Copied { text, round } => Ok((text, round)),
            other => panic!("asked to copy a review, the Keeper answered {other:?}"),
        }
    }

    pub fn mark_reviewed(&mut self, review: &ReviewId) -> Result<(), Refusal> {
        match self.request(Request::MarkReviewed {
            review: review.clone(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to mark a review reviewed, the Keeper answered {other:?}"),
        }
    }

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

    pub fn commit(&mut self, workspace: &WorkspaceId, message: &str) -> Result<String, Refusal> {
        match self.request(Request::Commit {
            workspace: workspace.clone(),
            message: message.to_owned(),
        })? {
            Reply::Committed { commit } => Ok(commit),
            other => panic!("asked to commit, the Keeper answered {other:?}"),
        }
    }

    pub fn open_launcher(
        &mut self,
        workspace: &WorkspaceId,
        agent: &str,
        size: TerminalSize,
    ) -> Result<TerminalId, Refusal> {
        match self.request(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Agent {
                agent: agent.to_owned(),
            },
            size,
        })? {
            Reply::TerminalOpened { terminal } => {
                self.output.entry(terminal.clone()).or_default();
                Ok(terminal)
            }
            other => panic!("asked to open a launcher, the Keeper answered {other:?}"),
        }
    }

    pub fn attach_review(
        &mut self,
        review: &ReviewId,
        thread: Option<&ThreadId>,
    ) -> Result<Vec<AgentCli>, Refusal> {
        match self.request(Request::AttachReview {
            review: review.clone(),
            thread: thread.cloned(),
        })? {
            Reply::AttachedReview { start } => Ok(start),
            other => panic!("asked to attach a review, the Keeper answered {other:?}"),
        }
    }

    pub fn detach_review(&mut self, review: &ReviewId) -> Result<(), Refusal> {
        match self.request(Request::DetachReview {
            review: review.clone(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("asked to detach a review, the Keeper answered {other:?}"),
        }
    }

    pub fn resolve_thread(
        &mut self,
        review: &ReviewId,
        thread: &ThreadId,
        resolved: bool,
    ) -> Result<(), Refusal> {
        let (review, thread) = (review.clone(), thread.clone());
        let request = match resolved {
            true => Request::ResolveThread { review, thread },
            false => Request::ReopenThread { review, thread },
        };
        match self.request(request)? {
            Reply::Done => Ok(()),
            other => panic!("asked to resolve or reopen a thread, the Keeper answered {other:?}"),
        }
    }

    pub fn resolve_addressed(&mut self, review: &ReviewId) -> Result<(), Refusal> {
        match self.request(Request::ResolveAddressed {
            review: review.clone(),
        })? {
            Reply::Done => Ok(()),
            other => {
                panic!("asked to resolve the addressed threads, the Keeper answered {other:?}")
            }
        }
    }

    pub fn attachment(&mut self, review: &ReviewId) -> Result<String, Refusal> {
        match self.request(Request::GetAttachment {
            review: review.clone(),
        })? {
            Reply::Attachment { text } => Ok(text),
            other => panic!("asked for an attachment, the Keeper answered {other:?}"),
        }
    }

    /// `Program::AgentFor` is the offer made on attaching, `Program::Resume` the resume control.
    pub fn open_program(
        &mut self,
        workspace: &WorkspaceId,
        program: Program,
        size: TerminalSize,
    ) -> Result<TerminalId, Refusal> {
        match self.request(Request::OpenTerminal {
            workspace: workspace.clone(),
            program,
            size,
        })? {
            Reply::TerminalOpened { terminal } => {
                self.output.entry(terminal.clone()).or_default();
                Ok(terminal)
            }
            other => panic!("asked to open a terminal, the Keeper answered {other:?}"),
        }
    }

    pub fn agents(&mut self) -> Vec<WorkspaceAgents> {
        match self.request(Request::ListAgents) {
            Ok(Reply::Agents(agents)) => agents,
            other => panic!("asked for the agents, the Keeper answered {other:?}"),
        }
    }

    pub fn wait_for_agents(
        &mut self,
        workspace: &WorkspaceId,
        limit: Duration,
        mut wanted: impl FnMut(&WorkspaceAgents) -> bool,
    ) -> WorkspaceAgents {
        let fact = self.wait_for_fact(limit, |fact| {
            matches!(fact, Fact::AgentsChanged(agents)
                if agents.workspace == *workspace && wanted(agents))
        });
        match fact {
            Fact::AgentsChanged(agents) => agents,
            other => unreachable!("the wait accepted {other:?}"),
        }
    }

    pub fn history(&mut self, workspace: &WorkspaceId) -> Result<History, Refusal> {
        match self.request(Request::GetHistory {
            workspace: workspace.clone(),
        })? {
            Reply::History(history) => Ok(history),
            other => panic!("asked for the history, the Keeper answered {other:?}"),
        }
    }

    pub fn uncommitted(
        &mut self,
        workspace: &WorkspaceId,
        review: Option<&ReviewId>,
    ) -> Result<Vec<CreditedFile>, Refusal> {
        match self.request(Request::GetUncommitted {
            workspace: workspace.clone(),
            review: review.cloned(),
        })? {
            Reply::Uncommitted(files) => Ok(files),
            other => panic!("asked for the uncommitted changes, the Keeper answered {other:?}"),
        }
    }

    pub fn compare(
        &mut self,
        workspace: &WorkspaceId,
        from: &str,
        to: Option<&str>,
    ) -> Result<Vec<ChangedFile>, Refusal> {
        match self.request(Request::CompareVersions {
            workspace: workspace.clone(),
            from: from.to_owned(),
            to: to.map(str::to_owned),
        })? {
            Reply::Compared(files) => Ok(files),
            other => panic!("asked to compare two versions, the Keeper answered {other:?}"),
        }
    }

    pub fn compare_for_review(
        &mut self,
        workspace: &WorkspaceId,
        review: &ReviewId,
        from: &str,
        to: &str,
    ) -> Result<Vec<CreditedFile>, Refusal> {
        match self.request(Request::CompareForReview {
            workspace: workspace.clone(),
            review: review.clone(),
            from: from.to_owned(),
            to: to.to_owned(),
        })? {
            Reply::ComparedForReview(files) => Ok(files),
            other => panic!("asked what a review shows of a change, the Keeper answered {other:?}"),
        }
    }

    pub fn clis(&mut self) -> Result<Vec<AgentCli>, Refusal> {
        match self.request(Request::ListClis)? {
            Reply::Clis(clis) => Ok(clis),
            other => panic!("asked for the agent CLIs, the Keeper answered {other:?}"),
        }
    }

    pub fn consents(&mut self) -> Result<Vec<Consent>, Refusal> {
        match self.request(Request::ListConsents)? {
            Reply::Consents(consents) => Ok(consents),
            other => panic!("asked for the questions for leave, the Keeper answered {other:?}"),
        }
    }

    pub fn answer_consent(
        &mut self,
        agent: &str,
        answer: ConsentAnswer,
    ) -> Result<Vec<Consent>, Refusal> {
        match self.request(Request::AnswerConsent {
            agent: agent.to_owned(),
            answer,
        })? {
            Reply::Consents(consents) => Ok(consents),
            other => panic!("answered a question for leave, the Keeper answered {other:?}"),
        }
    }

    pub fn turn(&mut self, turn: &TurnId) -> Result<Turn, Refusal> {
        match self.request(Request::GetTurn { turn: turn.clone() })? {
            Reply::Turn(turn) => Ok(turn),
            other => panic!("asked for a turn, the Keeper answered {other:?}"),
        }
    }

    pub fn show_workspace(&mut self, workspace: Option<&WorkspaceId>) -> Result<(), Refusal> {
        match self.request(Request::ShowWorkspace {
            workspace: workspace.cloned(),
        })? {
            Reply::Done => Ok(()),
            other => panic!("told which workspace is shown, the Keeper answered {other:?}"),
        }
    }

    pub fn terminals(&mut self) -> Vec<TerminalInfo> {
        match self.request(Request::ListTerminals) {
            Ok(Reply::Terminals(terminals)) => terminals,
            other => panic!("asked for the terminals, the Keeper answered {other:?}"),
        }
    }

    /// Gives whether the kept output was cut. On success it drops what this client holds of the
    /// terminal's output, since the Keeper resends the kept output from its start; a refusal keeps
    /// it.
    pub fn attach_terminal(
        &mut self,
        terminal: &TerminalId,
        size: TerminalSize,
    ) -> Result<bool, Refusal> {
        let id = self.write(Request::AttachTerminal {
            terminal: terminal.clone(),
            size,
        });
        // What the terminal wrote before the reply is held apart until the reply says whether the
        // Keeper sends the kept output.
        let mut early = Vec::new();
        let reply = loop {
            match self.read(Instant::now() + DEADLINE) {
                Ok(Message::Reply { to, reply }) if to == id => break reply,
                Ok(Message::Reply { to, reply }) if self.sent.remove(&to) => {
                    self.aside.insert(to, reply);
                }
                Ok(Message::Fact(fact)) => self.facts.push_back(fact),
                Ok(Message::Output {
                    terminal: other,
                    bytes,
                }) if other == *terminal => early.extend(bytes),
                Ok(Message::Output {
                    terminal: other,
                    bytes,
                }) => self.keep(other, bytes),
                Ok(other) => panic!("waiting for the reply to {id:?}, the Keeper sent {other:?}"),
                Err(e) => panic!("no reply to {id:?}: {e}; facts kept: {:?}", self.facts),
            }
        };
        match reply {
            Ok(Reply::Attached { cut }) => {
                self.output.insert(terminal.clone(), Vec::new());
                Ok(cut)
            }
            Err(refusal) => {
                self.keep(terminal.clone(), early);
                Err(refusal)
            }
            Ok(other) => panic!("asked to attach to a terminal, the Keeper answered {other:?}"),
        }
    }

    pub fn screen(&mut self, terminal: &TerminalId, size: TerminalSize) -> Vec<String> {
        let rows = usize::from(size.rows);
        let columns = usize::from(size.columns);
        let mut term = Term::new(Config::default(), &Screen { rows, columns }, VoidListener);
        let mut parser = Processor::<StdSyncHandler>::default();
        parser.advance(
            &mut term,
            self.output.get(terminal).map_or(&[][..], Vec::as_slice),
        );
        (0..rows)
            .map(|row| {
                let line = &term.grid()[Line(row as i32)];
                let mut text = String::new();
                for column in 0..columns {
                    let cell = &line[Column(column)];
                    // The second half of a wide character holds nothing of its own.
                    if !cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    {
                        text.push(cell.c);
                    }
                }
                text.trim_end().to_owned()
            })
            .collect()
    }

    pub fn stop_everything(&mut self) {
        write_message(&mut self.stream, &Message::Shutdown)
            .unwrap_or_else(|e| panic!("the stop-everything frame could not be sent: {e}"));
    }

    /// Goes away as a killed Window does: the connection is shut both ways with what the Keeper
    /// sent unread.
    pub fn vanish(self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    pub fn type_line(&mut self, terminal: &TerminalId, text: &str) {
        let mut line = text.as_bytes().to_vec();
        line.push(b'\r');
        self.type_into(terminal, &line);
    }

    /// Waits for the stand-in's [`crate::standin::PROMPT`] after byte `after` (the shell's own
    /// prompt does not count) and gives the byte just behind it, the `after` of the next wait.
    pub fn wait_for_prompt(&mut self, terminal: &TerminalId, after: usize) -> usize {
        let prompt = crate::standin::PROMPT.as_bytes();
        let found = |bytes: &[u8]| {
            bytes.get(after..).and_then(|new| {
                new.windows(prompt.len())
                    .position(|window| window == prompt)
                    .map(|at| after + at + prompt.len())
            })
        };
        let run = self.wait_for_output(terminal, DEADLINE, |bytes| found(bytes).is_some());
        found(&run).expect("the wait accepted a run that holds the prompt")
    }

    fn write(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        let kind = request.kind();
        write_message(&mut self.stream, &Message::Request { id, request })
            .unwrap_or_else(|e| panic!("the request {kind:?} could not be sent: {e}"));
        id
    }

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

    fn keep(&mut self, terminal: TerminalId, bytes: Vec<u8>) {
        self.output.entry(terminal).or_default().extend(bytes);
    }

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
