//! The shared model that says how the Window stands with the Keeper, and through which
//! every part of the Window asks the Keeper and hears from it.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gpui_kit::*;
use yaar_link::{
    Fact, Hello, Message, Refusal, Reply, Request, RequestId, TerminalId, read_message,
    write_message,
};

use crate::keeper::Attached;

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
}

/// What the thread that reads shares with the model.
struct Shared {
    waiting: Mutex<Waiting>,
    /// Where the facts go on their way to the thread that draws. It is kept here and
    /// not by the reading thread, so that the channel stays open when that thread ends:
    /// the end of a connection is told to those who asked something, and to nobody else.
    facts: async_channel::Sender<Fact>,
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

    /// Keeps output of a terminal for whoever takes it: see [`KeeperLink::take_output`].
    fn output(&self, terminal: TerminalId, bytes: Vec<u8>) {
        let _ = (terminal, bytes);
        todo!()
    }

    /// Ends the connection: nothing more is asked, and everyone who waits learns that
    /// no answer comes, by the channel of the answer closing.
    fn end(&self) {
        let mut waiting = self.waiting();
        waiting.open = false;
        waiting.askers.clear();
    }
}

impl Reading {
    /// Starts reading the connection: answers go to those who asked, facts to the
    /// subscribers of the link.
    fn start(stream: &UnixStream, cx: &mut Context<KeeperLink>) -> io::Result<Self> {
        let stream = stream.try_clone()?;
        let (facts, told) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            waiting: Mutex::new(Waiting {
                open: true,
                next: 0,
                askers: HashMap::new(),
            }),
            facts,
        });
        std::thread::Builder::new()
            .name("keeper-link".to_owned())
            .spawn({
                let shared = shared.clone();
                move || read(stream, &shared)
            })?;
        let telling = cx.spawn(async move |link, cx| {
            while let Ok(fact) = told.recv().await {
                if link.update(cx, |_, cx| cx.emit(fact)).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            _telling: telling,
            shared,
        })
    }
}

/// Reads the connection until it ends, on a thread of its own.
fn read(mut stream: UnixStream, shared: &Shared) {
    loop {
        match read_message(&mut stream) {
            Ok(Message::Reply { to, reply }) => shared.answer(to, reply),
            Ok(Message::Fact(fact)) => {
                // Refused only when nobody takes facts any more: the link is gone.
                if shared.facts.try_send(fact).is_err() {
                    break;
                }
            }
            Ok(Message::Output { terminal, bytes }) => shared.output(terminal, bytes),
            // The Keeper closed, the connection failed, or what came is nothing a Keeper
            // says after its hello. After any of them the next bytes cannot be trusted.
            Ok(
                Message::Hello(_)
                | Message::Shutdown
                | Message::Request { .. }
                | Message::Input { .. },
            )
            | Err(_) => break,
        }
    }
    shared.end();
    let _ = stream.shutdown(Shutdown::Both);
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
            stream,
            reading: Some(reading),
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

        // Written here, on the one thread every question comes from: the Keeper reads
        // whole frames, in the order they were asked.
        let mut writer: &UnixStream = stream;
        if let Err(error) = write_message(&mut writer, &Message::Request { id, request }) {
            reading.shared.waiting().askers.remove(&id);
            // A frame that is too long is refused before a byte of it is written.
            if error.kind() == io::ErrorKind::InvalidInput {
                let reason = format!("the request cannot be sent: {error}");
                return Task::ready(Err(AskError::Refused(Refusal::Failed { reason })));
            }
            // Part of a frame may be out: nothing after it would be understood.
            reading.shared.end();
            let _ = stream.shutdown(Shutdown::Both);
            return Task::ready(Err(AskError::NotLinked));
        }

        cx.spawn(async move |_, _| match answered.recv().await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(refusal)) => Err(AskError::Refused(refusal)),
            Err(_) => Err(AskError::NotLinked),
        })
    }

    /// Sends `bytes` to the program of `terminal`, as one [`Message::Input`] frame: what
    /// the user typed or pasted, or what the terminal answers the program. It leaves after
    /// every request and every input given before it, and nothing answers it. While the
    /// status is anything but [`LinkStatus::Connected`] the bytes are dropped.
    pub fn send_input(&mut self, terminal: &TerminalId, bytes: Vec<u8>) {
        let _ = (terminal, bytes);
        todo!()
    }

    /// What the program of `terminal` writes, from the first [`Message::Output`] of that
    /// terminal the link read on: output that arrives before this is called waits for
    /// it, so nothing between the reply that named the terminal and this call is lost.
    /// Given once for each terminal; a second call gives none.
    ///
    /// The bytes end, after the last of them, when the Keeper tells
    /// [`Fact::TerminalEnded`] for the terminal, and when the connection ends.
    pub fn take_output(&mut self, terminal: &TerminalId) -> Option<TerminalOutput> {
        let _ = terminal;
        todo!()
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
