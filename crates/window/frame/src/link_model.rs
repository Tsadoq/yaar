use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gpui_kit::*;
use yaar_link::{
    Fact, Hello, Message, Refusal, Reply, Request, RequestId, TerminalId, read_message,
    write_message,
};

use crate::keeper::Attached;

const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// A write that took this long is a stall: a little under the timeout, which a clock cannot be held
/// to.
const STALLED: Duration = Duration::from_millis(4750);

/// The most bytes one `Message::Input` frame carries, far under the Link's `MAX_FRAME`, so a paste
/// of any length goes out in frames the Link accepts.
const INPUT_PIECE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    Starting,
    Connected(Hello),
    OtherVersion(Hello),
    Failed(String),
    /// Nothing is asked or sent on it any more, and the Window does not look for a Keeper again by
    /// itself.
    Lost(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    Refused(Refusal),
    NotLinked,
    Unexpected,
}

impl fmt::Display for AskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(f),
            Self::NotLinked => write!(f, "the Keeper is not reached"),
            Self::Unexpected => write!(f, "the Keeper answered something else than was asked"),
        }
    }
}

impl std::error::Error for AskError {}

/// One thread writes every frame in order, so no call on the drawing thread waits; a Keeper that
/// takes nothing of a frame for 5 seconds is let go ([`LinkStatus::Lost`]). An answer the Keeper
/// wrote before a fact may be heard after it: whoever keeps both settles which is newer.
pub struct KeeperLink {
    status: LinkStatus,
    connection: Option<Connection>,
}

struct Connection {
    stream: UnixStream,
    reading: Option<Reading>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // The reading thread holds a handle of its own to the socket, so closing this one would
        // leave it reading; shutting the socket down ends both and tells the Keeper at once.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

struct Reading {
    // Dropped before `shared`, so that nothing listens for facts any more when the
    // channel they come through goes away.
    _telling: Task<()>,
    shared: Arc<Shared>,
    writing: mpsc::Sender<Outgoing>,
}

struct Outgoing {
    bytes: Vec<u8>,
    /// Told once the frame is out; dropped without a word when it could not be written.
    left: Option<mpsc::Sender<()>>,
}

impl From<Vec<u8>> for Outgoing {
    fn from(bytes: Vec<u8>) -> Self {
        Self { bytes, left: None }
    }
}

enum Told {
    Fact(Fact),
    Lost(String),
}

struct Shared {
    waiting: Mutex<Waiting>,
    told: async_channel::Sender<Told>,
    outputs: Mutex<Outputs>,
    why_lost: Mutex<Option<String>>,
}

struct Outputs {
    ended: bool,
    terminals: HashMap<TerminalId, Terminal>,
}

struct Terminal {
    sender: Option<async_channel::Sender<Vec<u8>>>,
    receiver: Option<async_channel::Receiver<Vec<u8>>>,
}

impl Terminal {
    fn new(ended: bool) -> Self {
        let (sender, receiver) = async_channel::unbounded();
        Self {
            sender: (!ended).then_some(sender),
            receiver: Some(receiver),
        }
    }
}

struct Waiting {
    open: bool,
    next: u32,
    askers: HashMap<RequestId, async_channel::Sender<Answer>>,
}

type Answer = std::result::Result<Reply, Refusal>;

impl Shared {
    fn waiting(&self) -> MutexGuard<'_, Waiting> {
        // Poison is ignored: nothing done under the lock can leave the table half changed.
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn answer(&self, to: RequestId, answer: Answer) {
        if let Some(asker) = self.waiting().askers.remove(&to) {
            let _ = asker.try_send(answer);
        }
    }

    fn outputs(&self) -> MutexGuard<'_, Outputs> {
        // Poison is ignored here too, as in `waiting`.
        self.outputs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn output(&self, terminal: TerminalId, bytes: Vec<u8>) {
        let mut outputs = self.outputs();
        let ended = outputs.ended;
        let kept = outputs
            .terminals
            .entry(terminal)
            .or_insert_with(|| Terminal::new(ended));
        if let Some(sender) = &kept.sender {
            let _ = sender.try_send(bytes);
        }
    }

    fn output_ended(&self, terminal: &TerminalId) {
        let mut outputs = self.outputs();
        let ended = outputs.ended;
        let kept = outputs
            .terminals
            .entry(terminal.clone())
            .or_insert_with(|| Terminal::new(ended));
        kept.sender = None;
    }

    /// Keeps the first reason: it is the cause of the others.
    fn lose(&self, why: String) {
        self.why_lost
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(why);
    }

    fn end(&self) {
        {
            let mut waiting = self.waiting();
            waiting.open = false;
            waiting.askers.clear();
        }
        let mut outputs = self.outputs();
        outputs.ended = true;
        for terminal in outputs.terminals.values_mut() {
            terminal.sender = None;
        }
    }
}

fn write(mut stream: UnixStream, frames: &mpsc::Receiver<Outgoing>, shared: &Shared) {
    while let Ok(frame) = frames.recv() {
        if let Err(why) = write_frame(&mut stream, &frame.bytes) {
            shared.lose(why);
            // Part of a frame may be out: nothing after it would be understood.
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        if let Some(left) = frame.left {
            let _ = left.send(());
        }
    }
}

fn write_frame(stream: &mut UnixStream, frame: &[u8]) -> Result<(), String> {
    let stalled = || {
        format!(
            "the Keeper took nothing for {} seconds",
            WRITE_TIMEOUT.as_secs()
        )
    };
    let mut rest = frame;
    while !rest.is_empty() {
        let started = Instant::now();
        match stream.write(rest) {
            Ok(0) => return Err("the connection failed: nothing could be written".to_owned()),
            Ok(written) => rest = &rest[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(stalled());
            }
            Err(error) => return Err(format!("the connection failed: {error}")),
        }
        // On macOS a write that waited the whole timeout still returns the few bytes that
        // fit, and only the next one fails. A write that waited that long, with the frame
        // not yet out, is a Keeper that takes nothing.
        if !rest.is_empty() && started.elapsed() >= STALLED {
            return Err(stalled());
        }
    }
    Ok(())
}

impl Reading {
    fn start(stream: &UnixStream, cx: &mut Context<KeeperLink>) -> io::Result<Self> {
        let reading = stream.try_clone()?;
        let writing = stream.try_clone()?;
        // Set before the first frame is queued. A socket whose Keeper has gone may refuse it; its
        // writes fail all the same, so the error is ignored.
        let _ = writing.set_write_timeout(Some(WRITE_TIMEOUT));
        let (told, heard) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            waiting: Mutex::new(Waiting {
                open: true,
                next: 0,
                askers: HashMap::new(),
            }),
            told,
            outputs: Mutex::new(Outputs {
                ended: false,
                terminals: HashMap::new(),
            }),
            why_lost: Mutex::new(None),
        });
        let (frames, to_write) = mpsc::channel();
        std::thread::Builder::new()
            .name("keeper-link-write".to_owned())
            .spawn({
                let shared = shared.clone();
                move || write(writing, &to_write, &shared)
            })?;
        std::thread::Builder::new()
            .name("keeper-link".to_owned())
            .spawn({
                let shared = shared.clone();
                move || read(reading, &shared)
            })?;
        let telling = cx.spawn(async move |link, cx| {
            while let Ok(told) = heard.recv().await {
                let kept = match told {
                    Told::Fact(fact) => link.update(cx, |_, cx| cx.emit(fact)),
                    Told::Lost(why) => {
                        let _ = link.update(cx, |link, cx| link.lost(why, cx));
                        break;
                    }
                };
                if kept.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            _telling: telling,
            shared,
            writing: frames,
        })
    }
}

fn read(mut stream: UnixStream, shared: &Shared) {
    let why = loop {
        match read_message(&mut stream) {
            Ok(Message::Reply { to, reply }) => shared.answer(to, reply),
            Ok(Message::Fact(fact)) => {
                // The output of a terminal ends before the fact that says so is heard,
                // so that whoever hears it finds all of the output there.
                if let Fact::TerminalEnded { terminal, .. } = &fact {
                    shared.output_ended(terminal);
                }
                if shared.told.try_send(Told::Fact(fact)).is_err() {
                    break None;
                }
            }
            Ok(Message::Output { terminal, bytes }) => shared.output(terminal, bytes),
            Ok(
                Message::Hello(_)
                | Message::Shutdown
                | Message::Request { .. }
                | Message::Input { .. },
            ) => break Some("the Keeper sent what a Keeper does not say".to_owned()),
            Err(error) => break Some(error.to_string()),
        }
    };
    shared.end();
    let _ = stream.shutdown(Shutdown::Both);
    let Some(why) = why else { return };
    let why = shared
        .why_lost
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or(why);
    let _ = shared.told.try_send(Told::Lost(why));
}

pub struct TerminalOutput(async_channel::Receiver<Vec<u8>>);

impl TerminalOutput {
    pub async fn next(&self) -> Option<Vec<u8>> {
        self.0.recv().await.ok()
    }

    pub fn ready(&self) -> Option<Vec<u8>> {
        self.0.try_recv().ok()
    }
}

impl EventEmitter<Fact> for KeeperLink {}

impl KeeperLink {
    pub fn new() -> Self {
        Self {
            status: LinkStatus::Starting,
            connection: None,
        }
    }

    /// The request is handed to the writing thread before this returns, so requests reach the
    /// Keeper in order and the caller never waits. A request too long for a frame is refused with
    /// [`Refusal::Failed`]. The reply is of whatever kind the Keeper sent: the caller checks it.
    pub fn ask(
        &mut self,
        request: Request,
        cx: &mut Context<Self>,
    ) -> Task<Result<Reply, AskError>> {
        let Some(Connection {
            reading: Some(reading),
            ..
        }) = &self.connection
        else {
            return Task::ready(Err(AskError::NotLinked));
        };

        let (answer, answered) = async_channel::bounded(1);
        let id = {
            let mut waiting = reading.shared.waiting();
            if !waiting.open {
                return Task::ready(Err(AskError::NotLinked));
            }
            let id = RequestId(waiting.next);
            waiting.next = waiting.next.wrapping_add(1);
            // Registered before the request is queued: the reading thread may hear its answer at
            // once.
            waiting.askers.insert(id, answer);
            id
        };

        let mut frame = Vec::new();
        if let Err(error) = write_message(&mut frame, &Message::Request { id, request }) {
            reading.shared.waiting().askers.remove(&id);
            let reason = format!("the request cannot be sent: {error}");
            return Task::ready(Err(AskError::Refused(Refusal::Failed { reason })));
        }
        if reading.writing.send(frame.into()).is_err() {
            reading.shared.waiting().askers.remove(&id);
            return Task::ready(Err(AskError::NotLinked));
        }

        cx.spawn(async move |_, _| match answered.recv().await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(refusal)) => Err(AskError::Refused(refusal)),
            Err(_) => Err(AskError::NotLinked),
        })
    }

    pub fn send_input(&mut self, terminal: &TerminalId, bytes: Vec<u8>) {
        if !matches!(self.status, LinkStatus::Connected(_)) {
            return;
        }
        let Some(Connection {
            reading: Some(reading),
            ..
        }) = &self.connection
        else {
            return;
        };
        for piece in bytes.chunks(INPUT_PIECE) {
            let mut frame = Vec::new();
            let message = Message::Input {
                terminal: terminal.clone(),
                bytes: piece.to_vec(),
            };
            if write_message(&mut frame, &message).is_err()
                || reading.writing.send(frame.into()).is_err()
            {
                return;
            }
        }
    }

    /// Output that arrives before this is called waits for it, so nothing between the reply that
    /// named the terminal and this call is lost. Given once for each terminal.
    pub fn take_output(&mut self, terminal: &TerminalId) -> Option<TerminalOutput> {
        let Some(Connection {
            reading: Some(reading),
            ..
        }) = &self.connection
        else {
            return None;
        };
        let mut outputs = reading.shared.outputs();
        let ended = outputs.ended;
        outputs
            .terminals
            .entry(terminal.clone())
            .or_insert_with(|| Terminal::new(ended))
            .receiver
            .take()
            .map(TerminalOutput)
    }

    /// The connection is kept, so that the output it brought before its end can still be taken.
    fn lost(&mut self, why: String, cx: &mut Context<Self>) {
        if matches!(self.status, LinkStatus::Connected(_)) {
            self.status = LinkStatus::Lost(why);
            cx.notify();
        }
    }

    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    pub fn stop_everything(&mut self) {
        drop(self.send_stop());
    }

    /// As `stop_everything`, then waits at most `within` for the stop frame to leave, so a Window
    /// that quits at once cannot take it down with it; gives whether it left. A Keeper of another
    /// protocol that takes none of the bytes holds the caller up to `WRITE_TIMEOUT`, not `within`.
    pub fn stop_everything_and_wait(&mut self, within: Duration) -> bool {
        self.send_stop()
            .is_some_and(|left| left.recv_timeout(within).is_ok())
    }

    fn send_stop(&mut self) -> Option<mpsc::Receiver<()>> {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &Message::Shutdown).ok()?;
        let (left, hears) = mpsc::channel();
        match &self.connection {
            Some(Connection {
                reading: Some(reading),
                ..
            }) => {
                reading
                    .writing
                    .send(Outgoing {
                        bytes,
                        left: Some(left),
                    })
                    .ok()?;
            }
            // A Keeper of another protocol is neither asked nor listened to, and nothing else was
            // written to it, so these few bytes fit its buffer. The timeout is for a Keeper that
            // does not read at all.
            Some(Connection {
                stream,
                reading: None,
            }) => {
                let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
                (&*stream).write_all(&bytes).ok()?;
                let _ = left.send(());
            }
            None => return None,
        }
        Some(hears)
    }

    pub fn attached(&mut self, outcome: Result<Attached, String>, cx: &mut Context<Self>) {
        self.connection = None;
        self.status = match outcome {
            Ok(attached) => {
                let mine = Hello::for_this_process("");
                if !mine.speaks_with(&attached.keeper) {
                    self.connection = Some(Connection {
                        stream: attached.stream,
                        reading: None,
                    });
                    LinkStatus::OtherVersion(attached.keeper)
                } else {
                    match Reading::start(&attached.stream, cx) {
                        Ok(reading) => {
                            self.connection = Some(Connection {
                                stream: attached.stream,
                                reading: Some(reading),
                            });
                            LinkStatus::Connected(attached.keeper)
                        }
                        Err(error) => LinkStatus::Failed(format!(
                            "the connection to the Keeper cannot be read: {error}"
                        )),
                    }
                }
            }
            Err(why) => LinkStatus::Failed(why),
        };
        cx.notify();
    }
}

impl Default for KeeperLink {
    fn default() -> Self {
        Self::new()
    }
}
