//! The Keeper program: the background process that owns what must survive a closed window.
//!
//! It starts the Keeper's components and connects them. In this first form it has none
//! yet: it listens on its socket, tells each Window who it is, and leaves when it is asked
//! to or when nothing needs it any more.
//!
//! Surface: [`run`] with its [`Options`], and the [`Exit`] it returns. The program
//! `yaar-keeper` is this function behind a command line.
//!
//! No interface code belongs here or in any crate beside this one.

use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use yaar_link::{Hello, LinkError, Message, read_message, write_message};

/// How long a Window may take to say who it is.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How the Keeper is started.
#[derive(Debug, Clone)]
pub struct Options {
    /// Where to listen.
    pub socket: PathBuf,
    /// How long to stay with no Window attached and nothing to keep alive.
    pub linger: Duration,
    /// The version to tell each Window.
    pub app_version: String,
}

impl Options {
    /// The options of the program as installed: the socket of [`yaar_link::socket_path`]
    /// and a linger of five seconds.
    pub fn new(app_version: &str) -> Self {
        Self {
            socket: yaar_link::socket_path(),
            linger: Duration::from_secs(5),
            app_version: app_version.to_owned(),
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

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A thread that panicked while holding the lock leaves plain counters behind.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Serves Windows on the socket until asked to stop or until nothing needs the Keeper.
///
/// Returns at once with [`Exit::AlreadyRunning`] when another Keeper holds the socket.
pub fn run(options: Options) -> io::Result<Exit> {
    let socket = &options.socket;
    if let Some(folder) = socket.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)?;
    }

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

    match fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;

    let shared = Arc::new(Shared::default());
    shared.lock().idle_since = Some(Instant::now());

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
    let _ = fs::remove_file(socket);
    drop(lock);
    Ok(exit)
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
            .unwrap_or_else(|e| e.into_inner())
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
    let Message::Hello(_window) = read_message(&mut stream)? else {
        return Err(LinkError::Malformed("the first frame was not a hello"));
    };
    // The answer goes out whatever the Window's protocol version is: the Window is the
    // one that shows a mismatch, and it may still ask for the stop.
    write_message(
        &mut stream,
        &Message::Hello(Hello::for_this_process(version)),
    )?;
    stream.set_read_timeout(None)?;

    {
        let mut state = shared.lock();
        state.windows += 1;
        state.idle_since = None;
    }
    let result = loop {
        match read_message(&mut stream) {
            Ok(Message::Shutdown) => {
                shared.lock().exit = Some(Exit::Shutdown);
                break Ok(());
            }
            Ok(Message::Hello(_)) => {}
            Err(LinkError::Closed) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
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
