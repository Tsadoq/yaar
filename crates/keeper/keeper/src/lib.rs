//! The Keeper program: the background process that owns what must survive a closed window.
//!
//! It starts the Keeper's components and connects them: it listens on its socket, tells
//! each Window who it is, hands each request of a Window to the part that answers it,
//! tells every attached Window what changed, and leaves when it is asked to or when
//! nothing needs it any more.
//!
//! Surface: [`run`] with its [`Options`], and the [`Exit`] it returns. The program
//! `yaar-keeper` is this function behind a command line. Everything else a Window can
//! reach, it reaches over the Link.
//!
//! No interface code belongs here or in any crate beside this one.

mod wiring;

use std::fs;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};

use yaar_link::{
    Fact, Hello, LinkError, MAX_FRAME, Message, Refusal, Request, RequestId, read_message,
    write_message,
};

use wiring::{Outcome, Parts};

/// How long a Window may take to say who it is.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a Window may take nothing of what is written to it before it is let go. A
/// Window reads on a thread of its own, so one that takes nothing for this long is stuck
/// or stopped, and what waits behind it must not wait with it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How the Keeper is started.
#[derive(Debug, Clone)]
pub struct Options {
    /// Where to listen.
    pub socket: PathBuf,
    /// How long to stay with no Window attached and nothing to keep alive.
    pub linger: Duration,
    /// The version to tell each Window.
    pub app_version: String,
    /// The app's folder, where the database is kept. Made when it is not there.
    pub data_dir: PathBuf,
}

impl Options {
    /// The options of the program as installed: the socket of [`yaar_link::socket_path`],
    /// a linger of five seconds and the folder of [`yaar_link::data_dir`].
    pub fn new(app_version: &str) -> Self {
        Self {
            socket: yaar_link::socket_path(),
            linger: Duration::from_secs(5),
            app_version: app_version.to_owned(),
            data_dir: yaar_link::data_dir(),
        }
    }
}

/// Why [`run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// A Window asked for the stop-everything quit.
    Shutdown,
    /// No Window was attached and nothing was left to keep alive for the linger time.
    Idle,
    /// Another Keeper already serves this socket.
    AlreadyRunning,
}

#[derive(Default)]
struct State {
    windows: usize,
    idle_since: Option<Instant>,
    exit: Option<Exit>,
}

/// What the threads of the Keeper share: one for each Window, and the one that waits for
/// the end.
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    // The parts, until the Keeper leaves.
    parts: RwLock<Option<Parts>>,
    // The Windows that are told the facts; none any more once the Keeper leaves.
    told: Mutex<Option<Vec<Arc<Window>>>>,
    // Held while a request that waits its turn is answered and its facts are told.
    turn: Mutex<()>,
}

impl Shared {
    fn new(parts: Parts) -> Shared {
        Shared {
            state: Mutex::new(State {
                idle_since: Some(Instant::now()),
                ..State::default()
            }),
            changed: Condvar::new(),
            parts: RwLock::new(Some(parts)),
            told: Mutex::new(Some(Vec::new())),
            turn: Mutex::new(()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Puts a Window among those that are told the facts. False when the Keeper is
    /// leaving and tells nobody any more.
    fn admit(&self, window: &Arc<Window>) -> bool {
        match lock(&self.told).as_mut() {
            Some(told) => {
                told.push(Arc::clone(window));
                true
            }
            None => false,
        }
    }

    fn forget(&self, window: &Arc<Window>) {
        if let Some(told) = lock(&self.told).as_mut() {
            told.retain(|other| !Arc::ptr_eq(other, window));
        }
    }

    /// Ends the telling for good and gives the Windows that were told until now. None is
    /// admitted after.
    fn tell_nobody_any_more(&self) -> Vec<Arc<Window>> {
        lock(&self.told).take().unwrap_or_default()
    }

    /// Tells every attached Window these facts, in this order.
    fn tell(&self, facts: &[Fact]) {
        if facts.is_empty() {
            return;
        }
        // The list is not held while writing: a Window that attaches or leaves does not
        // wait for one that is slow to read.
        let windows = lock(&self.told).clone().unwrap_or_default();
        for fact in facts {
            // A fact too long for a frame can be told to nobody; the Window that asks
            // for what it carries learns why.
            let Ok(frame) = frame_of(&Message::Fact(fact.clone())) else {
                continue;
            };
            for window in &windows {
                // A Window that cannot be told is let go by `send`, and its own thread
                // sees to the rest.
                let _ = window.send(&frame);
            }
        }
    }
}

/// A thread that panicked while holding a lock leaves behind nothing half done that
/// another could trip over: counters, a list, a connection that is closed with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The side of one attached Window that is written to.
///
/// A reply comes from the thread that serves the Window, a fact from the thread of
/// whichever Window caused it. The lock is held for a whole frame, so two frames never
/// mix.
struct Window {
    out: Mutex<UnixStream>,
}

impl Window {
    /// Writes one frame, whole. A Window that does not take it within [`WRITE_TIMEOUT`],
    /// or that is gone, is let go: after half a frame nothing more can be said to it.
    fn send(&self, frame: &[u8]) -> io::Result<()> {
        let mut out = lock(&self.out);
        out.write_all(frame).inspect_err(|_| {
            let _ = out.shutdown(Shutdown::Both);
        })
    }

    /// Closes the connection. The thread that reads it finds it closed and ends.
    fn let_go(&self) {
        let _ = lock(&self.out).shutdown(Shutdown::Both);
    }
}

/// The bytes of one frame; an error when the message is too long for one.
fn frame_of(message: &Message) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    write_message(&mut frame, message)?;
    Ok(frame)
}

/// Serves Windows on the socket until asked to stop or until nothing needs the Keeper.
///
/// The socket is bound at [`yaar_link::socket_address`] of `options.socket`, so that a
/// path too long for the system still leads Windows to this Keeper. The folder of such a
/// short path is made for the user alone and has to pass
/// [`yaar_link::check_private_folder`], or the Keeper does not start: anyone can make
/// that folder first. The lock file lies beside the socket really bound.
///
/// Returns at once with [`Exit::AlreadyRunning`] when another Keeper holds the socket.
/// Before it returns for any other reason, every Window still attached is let go and the
/// parts are ended: the git processes they keep, and the database.
pub fn run(options: Options) -> io::Result<Exit> {
    let socket = yaar_link::socket_address(&options.socket);
    let socket = socket.as_path();
    make_its_folder(socket, socket != options.socket)?;

    // The lock, not the socket file, says whether a Keeper runs: a killed Keeper leaves
    // its socket file behind, and the system drops its lock.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(socket))?;
    // SAFETY: `flock` on a descriptor this function owns for as long as it runs.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        return match error.kind() {
            io::ErrorKind::WouldBlock => Ok(Exit::AlreadyRunning),
            _ => Err(error),
        };
    }

    // Only the Keeper that holds the lock opens the database, and it is open before the
    // first Window can ask for anything.
    let shared = Arc::new(Shared::new(Parts::start(&options)));

    match fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;

    let acceptor = {
        let shared = Arc::clone(&shared);
        let options = options.clone();
        std::thread::spawn(move || accept_windows(listener, shared, options))
    };

    let exit = wait_for_the_end(&shared, options.linger);

    // Wake the acceptor, which sits in `accept`, so that it sees the end and lets go of
    // the listener before the socket file is removed.
    let _ = UnixStream::connect(socket);
    let _ = acceptor.join();

    // Each thread that serves a Window ends when its connection does.
    for window in shared.tell_nobody_any_more() {
        window.let_go();
    }
    // The parts end here and not with the last thread that holds `shared`: a request
    // that is being answered is waited for, and none is begun after. The next Keeper may
    // start as soon as the lock is let go, and must find the database closed.
    let parts = shared
        .parts
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    drop(parts);

    let _ = fs::remove_file(socket);
    drop(lock);
    Ok(exit)
}

/// Makes the folder the socket lies in, for the user alone.
///
/// The folder of a short path stands in a place where anyone can make folders, under a
/// name anyone can compute. One that is there already is used only if it is the user's
/// own, and so is the one just made: between the making and the binding nobody else can
/// then put anything into it.
fn make_its_folder(socket: &Path, short: bool) -> io::Result<()> {
    let Some(folder) = socket.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    let mut make = fs::DirBuilder::new();
    make.mode(0o700);
    if !short {
        return make.recursive(true).create(folder);
    }
    match make.create(folder) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    yaar_link::check_private_folder(folder)
}

fn lock_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

fn wait_for_the_end(shared: &Shared, linger: Duration) -> Exit {
    let mut state = shared.lock();
    loop {
        if let Some(exit) = state.exit {
            return exit;
        }
        let wait = match (state.windows, state.idle_since) {
            (0, Some(since)) => {
                let idle = since.elapsed();
                if idle >= linger {
                    state.exit = Some(Exit::Idle);
                    return Exit::Idle;
                }
                linger - idle
            }
            _ => Duration::from_secs(3600),
        };
        state = shared
            .changed
            .wait_timeout(state, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

fn accept_windows(listener: UnixListener, shared: Arc<Shared>, options: Options) {
    for stream in listener.incoming() {
        if shared.lock().exit.is_some() {
            return;
        }
        let Ok(stream) = stream else { continue };
        let shared = Arc::clone(&shared);
        let version = options.app_version.clone();
        std::thread::spawn(move || {
            // A Window that sends nonsense or goes away is that Window's affair.
            let _ = serve_window(stream, &shared, &version);
        });
    }
}

fn serve_window(mut stream: UnixStream, shared: &Shared, version: &str) -> Result<(), LinkError> {
    stream.set_read_timeout(Some(HELLO_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let Message::Hello(theirs) = read_message(&mut stream)? else {
        return Err(LinkError::Malformed("the first frame was not a hello"));
    };
    // The answer goes out whatever the Window's protocol version is: the Window is the
    // one that shows a mismatch, and it may still ask for the stop.
    let mine = Hello::for_this_process(version);
    write_message(&mut stream, &Message::Hello(mine.clone()))?;
    stream.set_read_timeout(None)?;

    // Every frame but the hello and the stop is spoken between equal versions only: a
    // Window of another version is told no fact and can ask for nothing else.
    let window = if mine.speaks_with(&theirs) {
        Some(Arc::new(Window {
            out: Mutex::new(stream.try_clone()?),
        }))
    } else {
        None
    };

    {
        let mut state = shared.lock();
        state.windows += 1;
        state.idle_since = None;
    }
    let result = match &window {
        Some(window) if !shared.admit(window) => Ok(()),
        _ => converse(&mut stream, window.as_ref(), shared),
    };
    if let Some(window) = &window {
        shared.forget(window);
    }
    {
        let mut state = shared.lock();
        state.windows -= 1;
        if state.windows == 0 {
            state.idle_since = Some(Instant::now());
        }
    }
    shared.changed.notify_all();
    result
}

/// Reads what one Window sends, until it goes away or asks for the stop. `window` is
/// where it is answered; none for a Window of another protocol version.
fn converse(
    stream: &mut UnixStream,
    window: Option<&Arc<Window>>,
    shared: &Shared,
) -> Result<(), LinkError> {
    loop {
        match read_message(stream) {
            Ok(Message::Shutdown) => {
                shared.lock().exit = Some(Exit::Shutdown);
                return Ok(());
            }
            Ok(Message::Hello(_)) => {}
            Ok(Message::Request { id, request }) => {
                let Some(window) = window else {
                    return Err(LinkError::Malformed(
                        "a Window of another version asked for more than the stop",
                    ));
                };
                if !answer(shared, window, id, request) {
                    return Ok(());
                }
            }
            // Only the Keeper answers and tells.
            Ok(Message::Reply { .. } | Message::Fact(_)) => {
                return Err(LinkError::Malformed(
                    "a Window sent what only the Keeper sends",
                ));
            }
            Err(LinkError::Closed) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

/// Answers one request: the reply to the Window that asked, then the facts the request
/// led to, to every Window. False when the Window that asked could not be answered and
/// was let go.
fn answer(shared: &Shared, window: &Window, id: RequestId, request: Request) -> bool {
    // Held until the facts are out, so that they reach the Windows in the order in which
    // the requests were answered.
    let _turn = wiring::waits_its_turn(request.kind()).then(|| lock(&shared.turn));
    let Outcome { reply, facts } = match shared
        .parts
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        Some(parts) => parts.answer(request),
        None => Outcome::failed("the Keeper is leaving"),
    };
    // No request stays without an answer: what does not fit in a frame is answered with
    // the reason.
    let frame = frame_of(&Message::Reply { to: id, reply }).or_else(|_| {
        frame_of(&Message::Reply {
            to: id,
            reply: Err(Refusal::Failed {
                reason: format!("the answer does not fit in one frame of {MAX_FRAME} bytes"),
            }),
        })
    });
    let answered = match frame {
        Ok(frame) => window.send(&frame).is_ok(),
        Err(_) => {
            window.let_go();
            false
        }
    };
    // What was done is told, whatever became of the Window that asked for it.
    shared.tell(&facts);
    answered
}
