//! Finding the Keeper, or starting it.
//!
//! The Window starts the Keeper when none answers, as a process in a session of its own,
//! so that it gets no signal when the Window or the terminal that launched it goes away.

use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use yaar_link::{Hello, LinkError, handshake};

/// How long a Keeper that was just started may take to answer.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// A connection to a Keeper that has said who it is.
#[derive(Debug)]
pub struct Attached {
    /// The open connection. The Keeper counts the Window as attached while it lives.
    pub stream: UnixStream,
    /// Who the Keeper is.
    pub keeper: Hello,
    /// Whether this call started the Keeper.
    pub started: bool,
}

/// Why no Keeper could be reached.
#[derive(Debug)]
pub enum KeeperError {
    /// No Keeper answers and the Keeper program was not found.
    ProgramNotFound,
    /// The Keeper program could not be started.
    Start(PathBuf, io::Error),
    /// A Keeper was started and did not answer in time.
    NoAnswer(PathBuf),
    /// A Keeper answered with something that is not the Link.
    Link(LinkError),
}

impl fmt::Display for KeeperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramNotFound => write!(f, "the Keeper program was not found"),
            Self::Start(path, e) => write!(f, "could not start {}: {e}", path.display()),
            Self::NoAnswer(path) => write!(f, "{} started but did not answer", path.display()),
            Self::Link(e) => write!(f, "the Keeper answered wrongly: {e}"),
        }
    }
}

impl std::error::Error for KeeperError {}

/// Connects to the Keeper on `socket`, starting it first when none answers.
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

/// `None` when nothing listens, or when what listens goes away before it has answered.
fn try_attach(socket: &Path, mine: &Hello, started: bool) -> Option<Result<Attached, KeeperError>> {
    let mut stream = UnixStream::connect(socket).ok()?;
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

/// Where the Keeper program is: `YAAR_KEEPER` if set, else where each package puts it
/// relative to the Window program, else beside it.
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
    .map(|relative| folder.join(relative))
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
