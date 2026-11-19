//! The shared model that says how the Window stands with the Keeper, and through which
//! every part of the Window asks the Keeper and hears from it.

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

/// How long the writing thread waits for the Keeper to take part of a frame before it
/// lets the Keeper go.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// A write that took this long is taken for a stall, a little less than the timeout it
/// was given, which a clock cannot be held to.
const STALLED: Duration = Duration::from_millis(4750);

/// The most bytes one [`Message::Input`] frame carries, so that a paste of any length goes
/// out in frames the Link accepts.
const INPUT_PIECE: usize = 64 * 1024;

/// How the Window stands with the Keeper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    /// The Window is looking for the Keeper, or starting it.
    Starting,
    /// The Keeper answered and speaks the Window's protocol.
    Connected(Hello),
    /// The Keeper answered and speaks another protocol version: it belongs to another
    /// version of the app.
    OtherVersion(Hello),
    /// No Keeper could be reached.
    Failed(String),
    /// The connection to a Keeper of the Window's protocol ended, for the reason given:
    /// the Keeper closed it or went away, it failed, it took nothing for too long, or the
    /// Keeper sent what a Keeper does not say. Nothing is asked or sent on it any more,
    /// and the Window does not look for a Keeper again by itself.
    Lost(String),
}

/// Why a question to the Keeper brought no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The Keeper refused, and says why.
    Refused(Refusal),
    /// There is no Keeper to ask: none is reached yet, the one reached speaks another
    /// protocol version, or the connection ended before the answer came.
    NotLinked,
    /// The Keeper answered with a reply of another kind than the request has. Whoever
    /// asks for one kind of reply turns any other into this.
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

/// The Window's one connection to the Keeper: a model every part of the Window may
/// observe, ask through and subscribe to.
///
/// - Observing it tells when [`KeeperLink::status`] changes.
/// - [`KeeperLink::ask`] sends a request and gives its answer.
/// - Subscribing to it gives every [`Fact`] the Keeper tells, in the order they arrive.
/// - [`KeeperLink::send_input`] sends what is typed into a terminal, and
///   [`KeeperLink::take_output`] gives what the program of a terminal writes.
///
/// Nothing else in the Window writes to the connection or reads from it. One thread of
/// the link's own writes every frame, in the order they were given, so that no call made
/// on the thread that draws ever waits for the Keeper to read. A Keeper that takes
/// nothing of a frame for 5 seconds is let go: the status becomes [`LinkStatus::Lost`].
///
/// An answer and a fact reach those who wait for them in no order the Window could rely
/// on: an answer the Keeper wrote before a fact may be heard after it. Whoever keeps
/// what both carry has to settle which is newer; the shared models do.
pub struct KeeperLink {
    status: LinkStatus,
    connection: Option<Connection>,
}

/// The connection to one Keeper. Dropping it lets the Keeper go.
struct Connection {
    /// The end the Window writes on. While it is open the Keeper counts this Window as
    /// attached.
    stream: UnixStream,
    /// None with a Keeper of another protocol version, which is neither asked nor
    /// listened to.
    reading: Option<Reading>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // The thread that reads holds a handle of its own to the socket, so closing this
        // one would leave it reading. Shutting the socket down ends both, and tells the
        // Keeper at once that the Window is gone.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// The reading side of a connection: a thread of its own reads the socket, since a read
/// waits and the thread that draws must not.
struct Reading {
    // Dropped before `shared`, so that nothing listens for facts any more when the
    // channel they come through goes away.
    _telling: Task<()>,
    shared: Arc<Shared>,
    /// Whole frames, for the thread that writes them. When this is dropped that thread
    /// ends.
    writing: mpsc::Sender<Vec<u8>>,
}

/// What the reading thread tells the thread that draws.
enum Told {
    /// A fact of the Keeper.
    Fact(Fact),
    /// The connection ended, for this reason. Nothing follows.
    Lost(String),
}

/// What the thread that reads shares with the model.
struct Shared {
    waiting: Mutex<Waiting>,
    /// Where the facts, and at the end the loss of the connection, go on their way to
    /// the thread that draws.
    told: async_channel::Sender<Told>,
    /// What the output of each terminal travels through.
    outputs: Mutex<Outputs>,
    /// Why the writing thread let the Keeper go, when it did: it knows better than the
    /// reading thread, which only sees the socket close.
    why_lost: Mutex<Option<String>>,
}

/// The output of the terminals the link has heard of.
struct Outputs {
    /// True once the connection has ended: no more output comes.
    ended: bool,
    terminals: HashMap<TerminalId, Terminal>,
}

/// One terminal's output on its way to whoever takes it.
struct Terminal {
    /// None once the terminal ended or the connection did: nothing more is kept.
    sender: Option<async_channel::Sender<Vec<u8>>>,
    /// None once taken.
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

/// The requests that wait for their answer.
struct Waiting {
    /// False once the connection has ended: nothing more is asked on it.
    open: bool,
    /// The id of the next request.
    next: u32,
    /// For each request that waits, where its answer goes.
    askers: HashMap<RequestId, async_channel::Sender<Answer>>,
}

type Answer = std::result::Result<Reply, Refusal>;

impl Shared {
    fn waiting(&self) -> MutexGuard<'_, Waiting> {
        // Nothing done under the lock can leave the table half changed.
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hands an answer to whoever asked.
    fn answer(&self, to: RequestId, answer: Answer) {
        if let Some(asker) = self.waiting().askers.remove(&to) {
            // An asker that dropped its task no longer listens, and need not.
            let _ = asker.try_send(answer);
        }
    }

    fn outputs(&self) -> MutexGuard<'_, Outputs> {
        // Nothing done under the lock can leave the table half changed.
        self.outputs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps output of a terminal for whoever takes it: see [`KeeperLink::take_output`].
    fn output(&self, terminal: TerminalId, bytes: Vec<u8>) {
        let mut outputs = self.outputs();
        let ended = outputs.ended;
        let kept = outputs
            .terminals
            .entry(terminal)
            .or_insert_with(|| Terminal::new(ended));
        // Bytes of a terminal that ended are dropped: its end was told already.
        if let Some(sender) = &kept.sender {
            // The channel is unbounded, and its receiver lives as long as this entry.
            let _ = sender.try_send(bytes);
        }
    }

    /// Ends the output of a terminal: what it wrote stays for whoever takes it, and then
    /// it ends. Called before the fact that says so is passed on.
    fn output_ended(&self, terminal: &TerminalId) {
        let mut outputs = self.outputs();
        let ended = outputs.ended;
        let kept = outputs
            .terminals
            .entry(terminal.clone())
            .or_insert_with(|| Terminal::new(ended));
        kept.sender = None;
    }

    /// Records why the Keeper was let go, unless a reason is there already: the first
    /// one is the cause of the others.
    fn lose(&self, why: String) {
        self.why_lost
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(why);
    }

    /// Ends the connection: nothing more is asked, and everyone who waits learns that
    /// no answer comes, by the channel of the answer closing. The output of every
    /// terminal ends after what it holds.
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

/// Writes whole frames to the Keeper, in the order they come, until the link lets go of
/// the channel or the Keeper stops taking them.
///
/// A Keeper that takes none of a frame for [`WRITE_TIMEOUT`] is let go. The socket is
/// shut down both ways, which ends the reading thread and so tells the Window.
fn write(mut stream: UnixStream, frames: &mpsc::Receiver<Vec<u8>>, shared: &Shared) {
    while let Ok(frame) = frames.recv() {
        if let Err(why) = write_frame(&mut stream, &frame) {
            shared.lose(why);
            // Part of a frame may be out: nothing after it would be understood.
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    }
}

/// Writes one frame whole, or says why it could not.
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
    /// Starts reading the connection: answers go to those who asked, facts to the
    /// subscribers of the link.
    fn start(stream: &UnixStream, cx: &mut Context<KeeperLink>) -> io::Result<Self> {
        let reading = stream.try_clone()?;
        let writing = stream.try_clone()?;
        // Set here and not on the writing thread, so that it is in force before the first
        // frame is queued. A socket whose Keeper has gone refuses it on some systems; its
        // writes fail all the same, so nothing is lost by going on.
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
                        // Nothing follows.
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

/// Reads the connection until it ends, on a thread of its own.
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
                // Refused only when nobody takes facts any more: the link is gone.
                if shared.told.try_send(Told::Fact(fact)).is_err() {
                    break None;
                }
            }
            Ok(Message::Output { terminal, bytes }) => shared.output(terminal, bytes),
            // What came is nothing a Keeper says after its hello. The next bytes cannot
            // be trusted.
            Ok(
                Message::Hello(_)
                | Message::Shutdown
                | Message::Request { .. }
                | Message::Input { .. },
            ) => break Some("the Keeper sent what a Keeper does not say".to_owned()),
            // The Keeper closed, or the connection failed.
            Err(error) => break Some(error.to_string()),
        }
    };
    shared.end();
    let _ = stream.shutdown(Shutdown::Both);
    let Some(why) = why else { return };
    // The writing thread's reason, when there is one, is the cause of the socket closing.
    let why = shared
        .why_lost
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or(why);
    let _ = shared.told.try_send(Told::Lost(why));
}

/// What the program of one terminal writes, as the Window reads it: see
/// [`KeeperLink::take_output`].
pub struct TerminalOutput(async_channel::Receiver<Vec<u8>>);

impl TerminalOutput {
    /// The next bytes the program wrote, waiting until they arrive; none once its output
    /// has ended.
    pub async fn next(&self) -> Option<Vec<u8>> {
        self.0.recv().await.ok()
    }

    /// The next bytes the program wrote, if they have arrived; does not wait.
    pub fn ready(&self) -> Option<Vec<u8>> {
        self.0.try_recv().ok()
    }
}

impl EventEmitter<Fact> for KeeperLink {}

impl KeeperLink {
    /// A link that has not reached a Keeper yet.
    pub fn new() -> Self {
        Self {
            status: LinkStatus::Starting,
            connection: None,
        }
    }

    /// Asks the Keeper, and resolves with its reply or its refusal. The reply is of
    /// whatever kind the Keeper sent: the caller checks that it is the kind it asked for.
    ///
    /// The request is handed to the writing thread before this returns, so requests and
    /// input reach the Keeper in the order they were given; this never waits for the
    /// Keeper to read. The answer comes when the Keeper sends it; dropping the task drops
    /// the answer and takes nothing back. While the status is anything but
    /// [`LinkStatus::Connected`] nothing is sent and the task resolves to
    /// [`AskError::NotLinked`]; so does every waiting task when the connection ends, and
    /// every later one. When the connection ends the status becomes
    /// [`LinkStatus::Lost`].
    ///
    /// A request too long for one frame is not sent either; it is refused with
    /// [`Refusal::Failed`], and the connection goes on.
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

        // The asker is known before the request leaves: its answer may come at once.
        let (answer, answered) = async_channel::bounded(1);
        let id = {
            let mut waiting = reading.shared.waiting();
            if !waiting.open {
                return Task::ready(Err(AskError::NotLinked));
            }
            let id = RequestId(waiting.next);
            waiting.next = waiting.next.wrapping_add(1);
            waiting.askers.insert(id, answer);
            id
        };

        // Encoded here, so that a request too long for a frame is refused before anything
        // is queued, and queued here, on the one thread every question comes from: the
        // Keeper reads whole frames, in the order they were asked.
        let mut frame = Vec::new();
        if let Err(error) = write_message(&mut frame, &Message::Request { id, request }) {
            reading.shared.waiting().askers.remove(&id);
            let reason = format!("the request cannot be sent: {error}");
            return Task::ready(Err(AskError::Refused(Refusal::Failed { reason })));
        }
        if reading.writing.send(frame).is_err() {
            // The writing thread let the Keeper go.
            reading.shared.waiting().askers.remove(&id);
            return Task::ready(Err(AskError::NotLinked));
        }

        cx.spawn(async move |_, _| match answered.recv().await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(refusal)) => Err(AskError::Refused(refusal)),
            Err(_) => Err(AskError::NotLinked),
        })
    }

    /// Sends `bytes` to the program of `terminal`, as [`Message::Input`] frames: what
    /// the user typed or pasted, or what the terminal answers the program. Bytes are cut
    /// into frames of at most 64 KiB, in order, so that a paste of any length goes out and
    /// no frame is too long for the Link. They leave after every request and every input
    /// given before them, and nothing answers them. While the status is anything but
    /// [`LinkStatus::Connected`] the bytes are dropped.
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
            // A piece of 64 KiB is always a frame the Link accepts.
            if write_message(&mut frame, &message).is_err() || reading.writing.send(frame).is_err()
            {
                // The writing thread let the Keeper go: the status follows.
                return;
            }
        }
    }

    /// What the program of `terminal` writes, from the first [`Message::Output`] of that
    /// terminal the link read on: output that arrives before this is called waits for
    /// it, so nothing between the reply that named the terminal and this call is lost.
    /// Given once for each terminal; a second call gives none.
    ///
    /// The bytes end, after the last of them, when the Keeper tells
    /// [`Fact::TerminalEnded`] for the terminal, and when the connection ends.
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

    /// The connection to a Keeper of the Window's protocol ended. The connection is kept,
    /// so that the output it brought before its end can still be taken.
    fn lost(&mut self, why: String, cx: &mut Context<Self>) {
        if matches!(self.status, LinkStatus::Connected(_)) {
            self.status = LinkStatus::Lost(why);
            cx.notify();
        }
    }

    /// How the Window stands with the Keeper.
    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    /// Records the outcome of [`crate::keeper::attach`] and tells the observers. With a
    /// Keeper of the same protocol version, the connection is read from here on: answers
    /// go to those who asked, and each fact is emitted to the subscribers.
    ///
    /// A connection held from an earlier outcome is let go: it is closed, and whoever
    /// still waits for an answer on it gets [`AskError::NotLinked`].
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
