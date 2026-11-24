//! The Keeper program: the background process that owns what must survive a closed window.
//! A piece of the app inside an agent CLI connects to the same socket with one `Origin::Agent`
//! request and is let go once answered.

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

/// `home` is the user's home, where a CLI keeps its own settings: Adapters read none from the
/// environment.
pub fn adapters(home: &Path) -> Vec<Box<dyn yaar_agent::Adapter>> {
    vec![
        Box::new(yaar_adapter_claude::ClaudeCode::new()),
        Box::new(yaar_adapter_pi::Pi::in_home(home)),
    ]
}

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// A Window that takes nothing for this long is stuck or stopped: what waits behind it must not
/// wait with it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_QUEUED: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Options {
    pub socket: PathBuf,
    pub linger: Duration,
    pub app_version: String,
    pub data_dir: PathBuf,
    /// The folder of the programs that pass a CLI's reports on. None: the folder of the Keeper's
    /// own program.
    pub companions: Option<PathBuf>,
    /// A test names its own folder so that nothing it runs reaches the real settings.
    pub home: PathBuf,
    /// Run once at start on a thread of its own for the environment of the Keeper's git and hooks.
    /// A read never waits for it; the user's own git does, for at most the shell's deadline. None:
    /// the Keeper's own environment.
    pub login_shell: Option<yaar_link::login::LoginShell>,
}

impl Options {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Shutdown,
    Idle,
    AlreadyRunning,
}

#[derive(Default)]
struct State {
    windows: usize,
    idle_since: Option<Instant>,
    exit: Option<Exit>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    parts: RwLock<Option<Parts>>,
    told: Mutex<Option<Vec<Arc<Window>>>>,
    turn: Mutex<()>,
    // Set once a reply is queued: the sweep of the states waits for it so that its repacks do not
    // slow the first Window's start.
    answered: AtomicBool,
    works: Works,
}

/// How long [`run`] waits for works answered apart still in flight: hooks have no bound. Past it
/// the Keeper leaves and git finishes without it.
const WORKS_DEADLINE: Duration = Duration::from_secs(30);

/// The works answered apart that are in flight. A work counts from the moment the parts give
/// it until its outcome is queued and told, so the stop, which takes the parts after every
/// request let them go, finds every work to wait for.
#[derive(Default)]
struct Works {
    running: Mutex<Vec<RequestKind>>,
    ended: Condvar,
}

struct InFlight<'a> {
    works: &'a Works,
    kind: RequestKind,
}

impl Works {
    fn begin(&self, kind: RequestKind) -> InFlight<'_> {
        lock(&self.running).push(kind);
        InFlight { works: self, kind }
    }

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

    fn has_answered(&self) -> bool {
        self.answered.load(Ordering::Acquire)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

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

    fn program_ended(&self) {
        // The lock is taken and let go so that the thread is either not yet looking, and
        // will see it, or already waiting, and is woken: never in between.
        drop(self.lock());
        self.changed.notify_all();
    }

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

    fn tell_nobody_any_more(&self) -> Vec<Arc<Window>> {
        lock(&self.told).take().unwrap_or_default()
    }

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

    fn tell(&self, facts: &[Fact]) {
        if facts.is_empty() {
            return;
        }
        // The list is not held while writing: a Window that attaches or leaves does not wait for
        // one slow to read.
        let windows = lock(&self.told).clone().unwrap_or_default();
        for fact in facts {
            let Ok(frame) = frame_of(&Message::Fact(fact.clone())) else {
                continue;
            };
            for window in &windows {
                let _ = window.queue(frame.clone());
            }
        }
    }
}

/// Poisoning is ignored: no lock here guards anything a panicking holder could leave half done.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Window {
    stream: UnixStream,
    queue: Mutex<Queue>,
    queued: Condvar,
}

#[derive(Default)]
struct Queue {
    frames: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
    finished: bool,
}

impl Window {
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

    fn finish(&self) {
        lock(&self.queue).finished = true;
        self.queued.notify_one();
    }

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
            if self.write(&frame).is_err() {
                self.let_go();
                return;
            }
        }
    }

    fn write(&self, frame: &[u8]) -> io::Result<()> {
        let mut out = &self.stream;
        let mut rest = frame;
        loop {
            let began = Instant::now();
            match out.write(rest) {
                Ok(written) if written == rest.len() => return Ok(()),
                // The system waits for room until everything is taken: back with less after the
                // whole time allowed, it has given up waiting.
                Ok(_) if began.elapsed() >= WRITE_TIMEOUT => {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                Ok(written) => rest = &rest[written..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

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

fn frame_of(message: &Message) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    write_message(&mut frame, message)?;
    Ok(frame)
}

/// Returns at once with [`Exit::AlreadyRunning`] when another Keeper holds the socket. Works in
/// flight are waited for at most `WORKS_DEADLINE`; a worktree one of them made is taken away
/// again, so that nothing half made is left in the user's repository.
pub fn run(options: Options) -> io::Result<Exit> {
    run_with_clock(options, Arc::new(yaar_recorder::SystemClock))
}

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

    let environment = environment::waiting_for(&options);
    let shared = Shared::start(&options, Arc::clone(&clock), environment.clone());
    let sweeps = cleaning::Sweeps::start(Arc::downgrade(&shared), clock);

    match fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;

    // The login shell is taken once the Keeper listens: a Window's first read is not held up by it.
    let take = environment::start(&options, environment);

    let acceptor = {
        let shared = Arc::clone(&shared);
        let options = options.clone();
        std::thread::spawn(move || accept_windows(listener, shared, options))
    };

    let exit = wait_for_the_end(&shared, options.linger);
    // Kills a login shell still running, so that a work waiting for it goes on in the Keeper's
    // own environment.
    take.end();

    // Wakes the acceptor sitting in `accept`, so that it lets go of the listener before the socket
    // file is removed. With the file gone already nothing reaches it: it is not waited for.
    if UnixStream::connect(socket).is_ok() {
        let _ = acceptor.join();
    }

    // The sweep ends before the parts it reads.
    drop(sweeps);
    for window in shared.tell_nobody_any_more() {
        window.let_go();
    }
    // The parts end here, not with the last thread that holds `shared`: the next Keeper may start
    // once the lock is let go and must find the database closed.
    let parts = shared
        .parts
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    drop(parts);
    // The lock is held until what is in flight ends: the next Keeper may start as soon as it is let
    // go.
    for kind in shared.works.wait(WORKS_DEADLINE) {
        eprintln!("yaar-keeper: leaving with the work of a {kind:?} request still running");
    }

    let _ = fs::remove_file(socket);
    drop(lock);
    Ok(exit)
}

/// A short path stands where anyone can make folders: an existing folder is used only if it is the
/// user's own, and so is the one just made, so nobody can put anything in it before the binding.
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
    // The answer goes out whatever the Window's protocol version: the Window shows a mismatch, and
    // it may still ask for the stop.
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
    // A fact told between the hello and the admission is missed, as one told before the Window
    // came: it asks for what it shows.
    let result = if speaks && !shared.admit(&window) {
        Ok(())
    } else {
        converse(&window, speaks, shared)
    };
    shared.forget(&window);
    if let Some(parts) = shared
        .parts
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        parts.window_gone(&window);
    }
    window.finish();
    {
        let mut state = shared.lock();
        state.windows -= 1;
        shared.settle(&mut state);
    }
    shared.changed.notify_all();
    result
}

fn converse(window: &Arc<Window>, speaks: bool, shared: &Arc<Shared>) -> Result<(), LinkError> {
    let mut in_order = InOrder::default();
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
                if !greeted && request.kind().origin() != Origin::Agent {
                    greeted = true;
                    shared.greet(window);
                }
                match wiring::answerer(request.kind()) {
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

/// Answers one Window's review requests one after another on a thread of its own: saves of a
/// draft keep the order typed while the Window's other requests go on being read. Dropping
/// waits for every request handed to it.
#[derive(Default)]
struct InOrder {
    lane: Option<(mpsc::Sender<Handed>, JoinHandle<()>)>,
}

type Handed = (RequestId, Request);

impl InOrder {
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
    // After the parts' read lock is dropped: the stop's write lock would otherwise wait on the
    // user's git.
    let Outcome { reply, facts, then } = match answered {
        Answered::Now(outcome) => outcome,
        Answered::Later(job) => job(),
    };
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
    if let Some(then) = then {
        then();
    }
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
        window.finish();
        for frame in &frames {
            let mut got = vec![0u8; frame.len()];
            theirs.read_exact(&mut got).expect("a whole frame");
            assert_eq!(&got, frame);
        }
    }

    #[test]
    fn a_window_that_falls_more_than_the_limit_behind_is_let_go_and_takes_nothing_more() {
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

        let mut rest = Vec::new();
        theirs.read_to_end(&mut rest).expect("read to the end");
        assert!(rest.len() < MAX_QUEUED, "{} bytes", rest.len());
    }
}
