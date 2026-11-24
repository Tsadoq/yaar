//! Finds the Keeper, or starts it in a session of its own so that it gets no signal when the Window
//! or its terminal goes away.

use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use yaar_link::{Hello, LinkError, handshake};

const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Attached {
    /// The Keeper counts the Window as attached while it lives.
    pub stream: UnixStream,
    pub keeper: Hello,
    pub started: bool,
}

#[derive(Debug)]
pub enum KeeperError {
    ProgramNotFound,
    Start(PathBuf, io::Error),
    NoAnswer(PathBuf),
    Link(LinkError),
    /// The socket is not the user's own: the folder of a shortened socket path belongs to somebody
    /// else or is open to others, or the listener runs as another user. Nothing was sent to it.
    NotOurs(PathBuf),
}

impl fmt::Display for KeeperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramNotFound => write!(f, "the Keeper program was not found"),
            Self::Start(path, e) => write!(f, "could not start {}: {e}", path.display()),
            Self::NoAnswer(path) => write!(f, "{} started but did not answer", path.display()),
            Self::Link(e) => write!(f, "the Keeper answered wrongly: {e}"),
            Self::NotOurs(path) => write!(
                f,
                "what listens at {} does not belong to this user",
                path.display()
            ),
        }
    }
}

impl std::error::Error for KeeperError {}

/// `socket` goes through `yaar_link::socket_address`, so a path too long for the system leads to
/// the short one the Keeper binds. A socket refused as not the user's own starts nothing.
pub fn attach(socket: &Path, app_version: &str) -> Result<Attached, KeeperError> {
    let mine = Hello::for_this_process(app_version);
    if let Some(attached) = try_attach(socket, &mine, false) {
        return attached;
    }

    let program = keeper_program().ok_or(KeeperError::ProgramNotFound)?;
    start(&program, socket).map_err(|e| KeeperError::Start(program.clone(), e))?;

    // A Keeper that is just leaving may still accept and then close. Keep asking until
    // one says hello.
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(attached) = try_attach(socket, &mine, true) {
            return attached;
        }
        if Instant::now() >= deadline {
            return Err(KeeperError::NoAnswer(program));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn try_attach(socket: &Path, mine: &Hello, started: bool) -> Option<Result<Attached, KeeperError>> {
    let mut stream = match yaar_link::connect(socket) {
        Ok(stream) => stream,
        // Somebody else's: starting a Keeper would not make it ours.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Some(Err(KeeperError::NotOurs(yaar_link::socket_address(socket))));
        }
        Err(_) => return None,
    };
    match handshake(&mut stream, mine) {
        Ok(keeper) => Some(Ok(Attached {
            stream,
            keeper,
            started,
        })),
        Err(LinkError::Closed | LinkError::Io(_)) => None,
        Err(e) => Some(Err(KeeperError::Link(e))),
    }
}

pub fn keeper_program() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("YAAR_KEEPER").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let folder = exe.parent()?;
    [
        "../Helpers/yaar-keeper",             // inside YAAR.app
        "../libexec/yaar-review/yaar-keeper", // the .deb and the archive
        "yaar-keeper",                        // a build folder
    ]
    .iter()
    .filter_map(|relative| folder.join(relative).canonicalize().ok())
    .find(|candidate| candidate.is_file())
}

fn start(program: &Path, socket: &Path) -> io::Result<()> {
    let mut command = Command::new(program);
    command
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe, and nothing else runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command.spawn()?;
    // The Keeper may leave while this Window still runs. Somebody has to collect it then.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
