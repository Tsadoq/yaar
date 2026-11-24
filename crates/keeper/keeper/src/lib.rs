//! The Keeper program: the background process that owns what must survive a closed window.
//!
//! It starts the Keeper's components and connects them: it listens on its socket, tells
//! each Window who it is, hands each request of a Window to the part that answers it,
//! tells every attached Window what changed, runs the programs of the terminals and passes
//! their bytes both ways, and leaves when it is asked to or when nothing needs it any
//! more: no Window is attached and no program runs.
//!
//! Surface: [`run`] with its [`Options`], and the [`Exit`] it returns; and [`adapters`],
//! the Adapters of the agent CLIs the Keeper supports, the one place of the app that names
//! them. The program `yaar-keeper` is this function behind a command line. Everything else
//! a Window can reach, it reaches over the Link.
//!
//! A piece of the app inside an agent CLI connects to the same socket: it says hello,
//! sends one request of the origin `yaar_link::Origin::Agent`, and is let go once that is
//! answered. It is never asked for anything.
//!
//! No interface code belongs here or in any crate beside this one.

mod agents;
mod base_moves;
mod cleaning;
mod environment;
mod reviewing;
mod settings;
mod watching;
mod wiring;

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_link::{
    Fact, Hello, LinkError, MAX_FRAME, Message, Origin, Refusal, Request, RequestId, RequestKind,
    read_message, write_message,
};

use wiring::{Answered, Answerer, Outcome, Parts};

/// The Adapters of every agent CLI the Keeper supports, in the order the app offers them:
/// Claude Code, then pi. The one list of the app that names a CLI: adding one is a line
/// here, beside its own integration module. `home` is the user's home, where a CLI keeps
/// its own settings: an Adapter that writes into them after the user's yes finds them under
/// it, and reads no home from its environment.
pub fn adapters(home: &Path) -> Vec<Box<dyn yaar_agent::Adapter>> {
    vec![
        Box::new(yaar_adapter_claude::ClaudeCode::new()),
        Box::new(yaar_adapter_pi::Pi::in_home(home)),
    ]
}

/// How long a Window may take to say who it is.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a Window may take nothing of what is written to it before it is let go. A
/// Window reads on a thread of its own, so one that takes nothing for this long is stuck
/// or stopped, and what waits behind it must not wait with it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How many bytes may wait to be written to one Window. A Window that falls this far
/// behind, as one that is stopped while a program writes on, is let go.
const MAX_QUEUED: usize = 64 * 1024 * 1024;

/// How the Keeper is started.
#[derive(Debug, Clone)]
pub struct Options {
    /// Where to listen.
    pub socket: PathBuf,
    /// How long to stay with no Window attached and no program of a terminal running.
    pub linger: Duration,
    /// The version to tell each Window.
    pub app_version: String,
    /// The app's folder, where the database is kept. Made when it is not there.
    pub data_dir: PathBuf,
    /// The folder of the app's programs that pass a CLI's reports on, as the hook program
    /// of Claude Code: where the pieces the Adapters write find them. None for the folder
    /// of the Keeper's own program, where the build and the packages put them beside it;
    /// a test that runs the Keeper in its own process names the folder of the built
    /// programs.
    pub companions: Option<PathBuf>,
    /// The user's home, under which each agent CLI keeps its own settings: what
    /// [`adapters`] hands the Adapters. The program as installed takes `HOME`; a test names
    /// a folder of its own, so that nothing it runs can reach the user's real settings.
    pub home: PathBuf,
    /// The user's login shell, run once at the start on a thread of its own for the
    /// environment of a terminal of the app: the Keeper's runs of the user's git, and the
    /// hooks that git runs, get it (`yaar_link::login`, `yaar_recorder::GitEnvironment`). A
    /// read never waits for it; the user's own git (a worktree made, a commit) does, for at
    /// most the shell's deadline. None for the Keeper's own environment, as before slice 10:
    /// [`Options::new`] has none, the program as installed hands its user's shell, and a test
    /// hands a script of its own, never the user's.
    pub login_shell: Option<yaar_link::login::LoginShell>,
}

impl Options {
    /// The options of the program as installed: the socket of [`yaar_link::socket_path`],
    /// a linger of five seconds, the folder of [`yaar_link::data_dir`] and the home `HOME`
    /// names (`/` where it names none, under which no CLI keeps settings), and no login
    /// shell: the program as installed sets one.
    pub fn new(app_version: &str) -> Self {
        Self {
            socket: yaar_link::socket_path(),
            linger: Duration::from_secs(5),
            app_version: app_version.to_owned(),
            data_dir: yaar_link::data_dir(),
            companions: None,
            home: std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map_or_else(|| PathBuf::from("/"), PathBuf::from),
            login_shell: None,
        }
    }
}

/// Why [`run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// A Window asked for the stop-everything quit.
    Shutdown,
    /// No Window was attached and no program of a terminal ran for the linger time.
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
    // Whether a reply to a Window has been queued since the start: the sweep of the states
    // by the set time waits for it, so that its repacks do not slow the first Window's start.
    answered: AtomicBool,
    // The works answered apart that are in flight, which `run` waits for before it returns.
    works: Works,
}

/// How long [`run`] waits, once the parts are ended, for the works answered apart that are
/// still in flight: the user's own git and hooks make a worktree or a commit in them, and
/// the hooks have no bound. Past it the Keeper leaves, says which work it left, and git
/// finishes what it was doing without it.
const WORKS_DEADLINE: Duration = Duration::from_secs(30);

/// The works answered apart (`Answered::Later` of a request answered on a thread of its
/// own) that are in flight, by the kind of request each answers. A work is counted from the
/// moment the parts give it, before they are let go, until its outcome is queued and told,
/// so that the stop, which takes the parts after every request being answered has let them
/// go, finds every work it has to wait for.
#[derive(Default)]
struct Works {
    running: Mutex<Vec<RequestKind>>,
    ended: Condvar,
}

/// One work in flight; it ends when this is dropped, also when its thread panics.
struct InFlight<'a> {
    works: &'a Works,
    kind: RequestKind,
}

impl Works {
    /// Counts a work of `kind` as in flight, until the value given back is dropped.
    fn begin(&self, kind: RequestKind) -> InFlight<'_> {
        lock(&self.running).push(kind);
        InFlight { works: self, kind }
    }

    /// Waits until no work is in flight, for at most `deadline`; gives the kinds of the
    /// works still in flight then.
    fn wait(&self, deadline: Duration) -> Vec<RequestKind> {
        let (running, _) = self
            .ended
            .wait_timeout_while(lock(&self.running), deadline, |running| !running.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        running.clone()
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut running = lock(&self.works.running);
        if let Some(at) = running.iter().position(|kind| *kind == self.kind) {
            running.remove(at);
        }
        drop(running);
        self.works.ended.notify_all();
    }
}

impl Shared {
    /// The shared state, with the parts started on it: the parts tell the Windows what a
    /// program of a terminal does, and so need a way to this state.
    fn start(
        options: &Options,
        clock: Arc<dyn yaar_recorder::Clock>,
        environment: yaar_recorder::GitEnvironment,
    ) -> Arc<Shared> {
        Arc::new_cyclic(|this| Shared::new(Parts::start(options, clock, environment, this.clone())))
    }

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
            answered: AtomicBool::new(false),
            works: Works::default(),
        }
    }

    /// Whether the Keeper has queued a reply to a Window since it started.
    fn has_answered(&self) -> bool {
        self.answered.load(Ordering::Acquire)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Whether anything needs the Keeper to stay, and since when nothing does: no Window
    /// attached and no program running. Called with the state held, wherever either may
    /// have changed.
    fn settle(&self, state: &mut State) {
        let running = self
            .parts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, Parts::running);
        state.idle_since = if state.windows > 0 || running > 0 {
            None
        } else {
            state.idle_since.or_else(|| Some(Instant::now()))
        };
    }

    /// A program of a terminal has ended: the thread that waits for the end looks again.
    fn program_ended(&self) {
        // The lock is taken and let go so that the thread is either not yet looking, and
        // will see it, or already waiting, and is woken: never in between.
        drop(self.lock());
        self.changed.notify_all();
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

    /// Tells `window` alone how the Adapters that need the user's leave stand, when one of
    /// them waits for an answer or could not change its CLI's settings: the question the
    /// Window shows at its connect. Tells it nothing when nothing waits. Under the turn, so
    /// that an answer of another Window is told after it, never before.
    fn greet(&self, window: &Arc<Window>) {
        let _turn = lock(&self.turn);
        let consents = match self
            .parts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            Some(parts) => parts.consents(),
            None => return,
        };
        let waits = consents.iter().any(|consent| {
            consent.answer == yaar_link::ConsentAnswer::NotAsked || consent.problem.is_some()
        });
        if !waits {
            return;
        }
        if let Ok(frame) = frame_of(&Message::Fact(Fact::ConsentsChanged(consents))) {
            let _ = window.queue(frame);
        }
    }

    /// Tells every attached Window these facts, in this order, by queuing them: no Window
    /// is waited for.
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
                // A Window that cannot be told is let go by `queue`, and its own thread
                // sees to the rest.
                let _ = window.queue(frame.clone());
            }
        }
    }
}

/// A thread that panicked while holding a lock leaves behind nothing half done that
/// another could trip over: counters, a list, a connection that is closed with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One attached Window.
///
/// Its connection is read by the thread that serves it. Everything the Keeper says to it
/// is queued and written by a thread of its own, in the order queued, so that nobody who
/// has something to say waits for a Window that is slow to read: the replies, the facts
/// and the output of the programs all go through the one queue.
struct Window {
    stream: UnixStream,
    queue: Mutex<Queue>,
    queued: Condvar,
}

#[derive(Default)]
struct Queue {
    frames: VecDeque<Vec<u8>>,
    /// The bytes in `frames`.
    bytes: usize,
    /// The connection is let go: nothing is taken any more and nothing is written.
    closed: bool,
    /// Nothing more will be queued: the writing thread ends when the frames are written.
    finished: bool,
}

impl Window {
    /// A Window on this connection, and the thread that writes to it.
    fn new(stream: UnixStream) -> Arc<Window> {
        let window = Arc::new(Window {
            stream,
            queue: Mutex::new(Queue::default()),
            queued: Condvar::new(),
        });
        let writer = Arc::clone(&window);
        std::thread::spawn(move || writer.write_queued());
        window
    }

    /// Queues one frame, whole, and returns at once. A Window that is gone, or whose
    /// queue would hold more than [`MAX_QUEUED`] bytes, is let go and takes nothing:
    /// false.
    fn queue(&self, frame: Vec<u8>) -> bool {
        let mut queue = lock(&self.queue);
        if queue.closed {
            return false;
        }
        if queue.bytes + frame.len() > MAX_QUEUED {
            drop(queue);
            self.let_go();
            return false;
        }
        queue.bytes += frame.len();
        queue.frames.push_back(frame);
        drop(queue);
        self.queued.notify_one();
        true
    }

    /// Nothing more will be queued, as the Window has gone: what is queued is written
    /// and the writing thread ends.
    fn finish(&self) {
        lock(&self.queue).finished = true;
        self.queued.notify_one();
    }

    /// Writes the queued frames, one at a time, until the Window is let go or finished.
    fn write_queued(&self) {
        loop {
            let frame = {
                let mut queue = lock(&self.queue);
                loop {
                    if queue.closed {
                        return;
                    }
                    if let Some(frame) = queue.frames.pop_front() {
                        queue.bytes -= frame.len();
                        break frame;
                    }
                    if queue.finished {
                        return;
                    }
                    queue = self
                        .queued
                        .wait(queue)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            // A Window that takes nothing for [`WRITE_TIMEOUT`], or that is gone, is let
            // go: after half a frame nothing more can be said to it.
            if self.write(&frame).is_err() {
                self.let_go();
                return;
            }
        }
    }

    /// Writes one frame, whole.
    fn write(&self, frame: &[u8]) -> io::Result<()> {
        let mut out = &self.stream;
        let mut rest = frame;
        loop {
            let began = Instant::now();
            match out.write(rest) {
                Ok(written) if written == rest.len() => return Ok(()),
                // The system waits for room until everything is taken. When it comes
                // back with less after the whole time allowed, it has given up waiting:
                // to go on would be to wait that long once more.
                Ok(_) if began.elapsed() >= WRITE_TIMEOUT => {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                Ok(written) => rest = &rest[written..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Closes the connection and drops what was queued. The thread that reads it finds
    /// it closed and ends, and so does the one that writes to it.
    fn let_go(&self) {
        let mut queue = lock(&self.queue);
        queue.closed = true;
        queue.frames.clear();
        queue.bytes = 0;
        drop(queue);
        self.queued.notify_all();
        let _ = self.stream.shutdown(Shutdown::Both);
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
/// With [`Options::login_shell`] some, the user's login shell runs once on a thread of its
/// own; see [`Options::login_shell`]. The thread starts after the listener is bound and its
/// permissions are set, and not before: a Keeper that cannot bind (the socket is taken, the
/// folder is refused) returns the error with no shell run, and a Window that connects as
/// soon as the socket is there is answered while the shell still runs, as no read waits
/// for it. A shell that gives no environment is said in one line on the standard error and
/// the Keeper goes on in its own.
///
/// Returns at once with [`Exit::AlreadyRunning`] when another Keeper holds the socket.
/// Before it returns for any other reason, every Window still attached is let go and the
/// parts are ended: the programs of the terminals and what they started, the git
/// processes the Keeper keeps, and the database.
///
/// The works answered apart that are in flight then (the user's own git making a worktree
/// or a commit, with their hooks) are waited for, for at most [`WORKS_DEADLINE`], before
/// this returns and the lock is let go: they find the parts gone, and a worktree one of them
/// made is taken away again, so that nothing half made is left in the user's repository.
/// Past the deadline this returns and says on the error output which work it left.
pub fn run(options: Options) -> io::Result<Exit> {
    run_with_clock(options, Arc::new(yaar_recorder::SystemClock))
}

/// [`run`], with the time read from `clock`: the time each recorded state is stamped with,
/// and the time the clean-up of states counts their age to. The program as installed hands
/// the system's clock, through [`run`]; a test of the clean-up by the set time hands one of
/// its own that it moved forward, so that the sweep at the start finds its states that old.
pub fn run_with_clock(options: Options, clock: Arc<dyn yaar_recorder::Clock>) -> io::Result<Exit> {
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
    let environment = environment::waiting_for(&options);
    let shared = Shared::start(&options, Arc::clone(&clock), environment.clone());
    // The clean-up of states by the set time: once the first reply is queued, then every
    // hour by the clock, until the end.
    let sweeps = cleaning::Sweeps::start(Arc::downgrade(&shared), clock);

    match fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;

    // The login shell is taken once the Keeper listens: a Window's first read is not held up
    // by it, and the user's own git waits for it.
    let take = environment::start(&options, environment);

    let acceptor = {
        let shared = Arc::clone(&shared);
        let options = options.clone();
        std::thread::spawn(move || accept_windows(listener, shared, options))
    };

    let exit = wait_for_the_end(&shared, options.linger);
    // A shell still running is ended, so that a work that waits for it goes on with the
    // Keeper's own environment, and none is left behind.
    take.end();

    // Wake the acceptor, which sits in `accept`, so that it sees the end and lets go of
    // the listener before the socket file is removed. Where the file is gone already,
    // nothing wakes the acceptor and nothing reaches it either: it is not waited for.
    if UnixStream::connect(socket).is_ok() {
        let _ = acceptor.join();
    }

    // The sweep ends before the parts it reads do.
    drop(sweeps);
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
    // What is still in flight finds the parts gone. The lock is held until it ends: the next
    // Keeper may start as soon as the lock is let go.
    for kind in shared.works.wait(WORKS_DEADLINE) {
        eprintln!("yaar-keeper: leaving with the work of a {kind:?} request still running");
    }

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
        // A program that ends by itself leaves the Keeper with nothing to keep.
        shared.settle(&mut state);
        let wait = match state.idle_since {
            Some(since) => {
                let idle = since.elapsed();
                if idle >= linger {
                    state.exit = Some(Exit::Idle);
                    return Exit::Idle;
                }
                linger - idle
            }
            None => Duration::from_secs(3600),
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

fn serve_window(stream: UnixStream, shared: &Arc<Shared>, version: &str) -> Result<(), LinkError> {
    stream.set_read_timeout(Some(HELLO_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let Message::Hello(theirs) = read_message(&mut &stream)? else {
        return Err(LinkError::Malformed("the first frame was not a hello"));
    };
    // The answer goes out whatever the Window's protocol version is: the Window is the
    // one that shows a mismatch, and it may still ask for the stop.
    let mine = Hello::for_this_process(version);
    write_message(&mut &stream, &Message::Hello(mine.clone()))?;
    stream.set_read_timeout(None)?;

    // Every frame but the hello and the stop is spoken between equal versions only: a
    // Window of another version is told no fact and can ask for nothing else.
    let speaks = mine.speaks_with(&theirs);
    let window = Window::new(stream);

    {
        let mut state = shared.lock();
        state.windows += 1;
        shared.settle(&mut state);
    }
    // The Window is told the facts from its admission on, which the Keeper does a moment
    // after the hello. A fact told between the two is missed, as one told before the Window
    // came: what it shows it asks for, and the first reply it gets comes after its admission.
    let result = if speaks && !shared.admit(&window) {
        // The Keeper is leaving.
        Ok(())
    } else {
        converse(&window, speaks, shared)
    };
    shared.forget(&window);
    // It is shown no terminal and no workspace any more.
    if let Some(parts) = shared
        .parts
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        parts.window_gone(&window);
    }
    // What is owed to the Window is still written, if it can take it.
    window.finish();
    {
        let mut state = shared.lock();
        state.windows -= 1;
        shared.settle(&mut state);
    }
    shared.changed.notify_all();
    result
}

/// Reads what one Window sends, until it goes away or asks for the stop. `speaks` says
/// whether it speaks the protocol version of this Keeper.
///
/// A request is answered where [`wiring::answerer`] says: most here, before the next
/// message is read; those of reviews by the Window's own thread for them, in the order
/// they came; the rest on a thread of their own. Whatever was handed to the Window's
/// thread for reviews is answered before this returns.
fn converse(window: &Arc<Window>, speaks: bool, shared: &Arc<Shared>) -> Result<(), LinkError> {
    let mut in_order = InOrder::default();
    // Whether the Window has been told what waits for the user's answer.
    let mut greeted = false;
    loop {
        match read_message(&mut &window.stream) {
            Ok(Message::Shutdown) => {
                shared.lock().exit = Some(Exit::Shutdown);
                return Ok(());
            }
            Ok(Message::Hello(_)) => {}
            Ok(Message::Request { .. }) if !speaks => {
                return Err(LinkError::Malformed(
                    "a Window of another version asked for more than the stop",
                ));
            }
            Ok(Message::Request { id, request }) => {
                // A Window, not a piece of a CLI, by its first request: told once what waits
                // for the user's answer, before that request is answered.
                if !greeted && request.kind().origin() != Origin::Agent {
                    greeted = true;
                    shared.greet(window);
                }
                match wiring::answerer(request.kind()) {
                    // A piece of an agent CLI asks one thing and goes: what it was owed is
                    // written, and nothing more is read from it.
                    Answerer::Here if request.kind().origin() == Origin::Agent => {
                        answer(shared, window, id, request);
                        return Ok(());
                    }
                    Answerer::Here => {
                        if !answer(shared, window, id, request) {
                            return Ok(());
                        }
                    }
                    Answerer::InOrder => {
                        if !in_order.hand(shared, window, id, request) {
                            return Ok(());
                        }
                    }
                    Answerer::Apart => {
                        let (shared, window) = (Arc::clone(shared), Arc::clone(window));
                        // Its answer is queued like any other. A Window let go meanwhile
                        // takes nothing, and its reading thread finds the connection closed.
                        std::thread::spawn(move || answer(&shared, &window, id, request));
                    }
                }
            }
            Ok(Message::Input { .. }) if !speaks => {
                return Err(LinkError::Malformed(
                    "a Window of another version asked for more than the stop",
                ));
            }
            Ok(Message::Input { terminal, bytes }) => {
                if let Some(parts) = shared
                    .parts
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                {
                    parts.input(&terminal, &bytes);
                }
            }
            // Only the Keeper answers, tells, and passes on what a program wrote.
            Ok(Message::Reply { .. } | Message::Fact(_) | Message::Output { .. }) => {
                return Err(LinkError::Malformed(
                    "a Window sent what only the Keeper sends",
                ));
            }
            Err(LinkError::Closed) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

/// The thread that answers one Window's requests of reviews, one after the other in the
/// order they came, so that the saves of a draft are kept in the order typed while the
/// Window's other requests and its typing into terminals go on being read. Started at the
/// first such request; waited for when dropped, so that every request handed to it is
/// answered before the Window is finished.
#[derive(Default)]
struct InOrder {
    lane: Option<(mpsc::Sender<Handed>, JoinHandle<()>)>,
}

/// A request handed to a Window's thread for reviews, with the id it is answered by.
type Handed = (RequestId, Request);

impl InOrder {
    /// Hands one request to the thread. False when the thread has stopped, as it does
    /// once the Window was let go.
    fn hand(
        &mut self,
        shared: &Arc<Shared>,
        window: &Arc<Window>,
        id: RequestId,
        request: Request,
    ) -> bool {
        let (sender, _) = self.lane.get_or_insert_with(|| {
            let (sender, received) = mpsc::channel::<Handed>();
            let (shared, window) = (Arc::clone(shared), Arc::clone(window));
            let thread = std::thread::spawn(move || {
                for (id, request) in received {
                    if !answer(&shared, &window, id, request) {
                        return;
                    }
                }
            });
            (sender, thread)
        });
        sender.send((id, request)).is_ok()
    }
}

impl Drop for InOrder {
    fn drop(&mut self) {
        if let Some((sender, thread)) = self.lane.take() {
            drop(sender);
            let _ = thread.join();
        }
    }
}

/// Answers one request: the reply to the Window that asked, then the facts the request
/// led to, to every Window. False when the Window that asked could not be answered and
/// was let go.
fn answer(shared: &Shared, window: &Arc<Window>, id: RequestId, request: Request) -> bool {
    // Held until the facts are out, so that they reach the Windows in the order in which
    // the requests were answered.
    let _turn = wiring::waits_its_turn(request.kind()).then(|| lock(&shared.turn));
    let kind = request.kind();
    let mut in_flight = None;
    let answered = match shared
        .parts
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        Some(parts) => {
            let answered = parts.answer(window, request);
            // Counted while the parts are held: the stop takes them with a write lock, which
            // waits for this, and then waits for the work.
            if matches!(answered, Answered::Later(_)) && wiring::answerer(kind) == Answerer::Apart {
                in_flight = Some(shared.works.begin(kind));
            }
            answered
        }
        None => Answered::Now(Outcome::failed("the Keeper is leaving")),
    };
    // The parts are let go by now: work left for later, as the user's own git, holds up
    // neither the Keeper's stop nor anything else that waits for the parts.
    let Outcome { reply, facts, then } = match answered {
        Answered::Now(outcome) => outcome,
        Answered::Later(job) => job(),
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
        Ok(frame) => window.queue(frame),
        Err(_) => {
            window.let_go();
            false
        }
    };
    if answered {
        shared.answered.store(true, Ordering::Release);
    }
    // What waited for the reply to be on its way, as the output of a terminal it names,
    // goes after it.
    if let Some(then) = then {
        then();
    }
    // What was done is told, whatever became of the Window that asked for it.
    shared.tell(&facts);
    drop(in_flight);
    answered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn pair() -> (Arc<Window>, UnixStream) {
        let (ours, theirs) = UnixStream::pair().expect("a pair of sockets");
        theirs
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set the deadline of a read");
        (Window::new(ours), theirs)
    }

    #[test]
    fn what_is_queued_for_a_window_is_written_whole_in_the_order_queued_also_after_it_is_finished()
    {
        let (window, mut theirs) = pair();
        let frames: Vec<Vec<u8>> = (0..50u8).map(|n| vec![n; 1000 + n as usize]).collect();
        for frame in &frames {
            assert!(window.queue(frame.clone()));
        }
        // The Window's own thread has nothing more to say: what is owed is still sent.
        window.finish();
        for frame in &frames {
            let mut got = vec![0u8; frame.len()];
            theirs.read_exact(&mut got).expect("a whole frame");
            assert_eq!(&got, frame);
        }
    }

    #[test]
    fn a_window_that_falls_more_than_the_limit_behind_is_let_go_and_takes_nothing_more() {
        // Nobody reads from `theirs`: what the sockets hold is a few hundred kilobytes,
        // and the rest waits in the queue.
        let (window, mut theirs) = pair();
        let megabyte = vec![b'x'; 1 << 20];
        let mut queued = 0;
        while window.queue(megabyte.clone()) {
            queued += 1;
            assert!(queued < 200, "the queue has no limit");
        }
        assert!(
            (MAX_QUEUED >> 20..=(MAX_QUEUED >> 20) + 1).contains(&queued),
            "{queued} megabytes were taken"
        );
        assert!(
            !window.queue(vec![0]),
            "a Window that was let go takes nothing"
        );

        // The connection is closed: what was written comes, and then the end.
        let mut rest = Vec::new();
        theirs.read_to_end(&mut rest).expect("read to the end");
        assert!(rest.len() < MAX_QUEUED, "{} bytes", rest.len());
    }
}
