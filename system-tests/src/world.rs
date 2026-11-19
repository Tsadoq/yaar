//! The world of one test: its own folders, its own repositories, its own Keeper.
//!
//! Nothing of one world touches another or the machine's own: a home folder, the app's
//! folder and a runtime folder of the world's own, and git told to read no configuration
//! but what the world sets. Tests therefore run side by side. A program of the world
//! writes its temporary files in a folder of the world too, so that a program that is
//! killed leaves nothing in the machine's temporary folder.

use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_link::{APP_FOLDER, Hello, MAX_FRAME, Message, Request, handshake, read_message};

use crate::client::{Client, DEADLINE};

/// How long the Keeper a world starts stays with no Window attached: longer than any
/// test. It ends with its world, not by itself in the middle of a test that has no
/// client attached for a while.
const LINGER_MS: &str = "3600000";

/// One test's world. Everything in it lies under one folder with a short path. When the
/// world is dropped its Keeper is killed and the folder removed.
pub struct World {
    root: PathBuf,
    keeper: Option<Child>,
    shell: PathBuf,
}

impl World {
    /// A new world: empty folders, no repository, no Keeper.
    #[allow(clippy::new_without_default)]
    pub fn new() -> World {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS. The Keeper names a worktree by its resolved path,
        // and a test compares that with the paths of its world.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-w-{}-{n}", std::process::id()));
        for folder in ["home", "run", "repos", "tmp"] {
            fs::create_dir_all(root.join(folder)).expect("make the folders of the world");
        }
        World {
            root,
            keeper: None,
            shell: PathBuf::from("/bin/sh"),
        }
    }

    /// The home folder of the world's user.
    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// The app's folder in this world, where the Keeper keeps its database.
    pub fn data_dir(&self) -> PathBuf {
        // Where the programs find it by themselves, in the environment `command` gives.
        if cfg!(target_os = "macos") {
            self.home()
                .join("Library/Application Support")
                .join(APP_FOLDER)
        } else {
            self.data_home().join(APP_FOLDER)
        }
    }

    fn data_home(&self) -> PathBuf {
        self.root.join("data")
    }

    /// The path the Keeper of this world is asked to listen at.
    pub fn socket(&self) -> PathBuf {
        self.root.join("keeper.sock")
    }

    /// A command that runs `program` as the world's user: with the environment of this
    /// process cleared but for the search path, and the world's own in its place. The
    /// search path loses every folder that holds a program named `claude` or `pi`, so that
    /// no test starts a real agent CLI; after [`World::agents_on_path`] the world's folder
    /// of stand-ins comes first, and `STANDIN_SCRIPT` and `STANDIN_LOG` name the world's
    /// `default.script` and `default.log`. That
    /// names the home, data, runtime and temporary folders, the socket and the Keeper
    /// program, the shell of a terminal (`/bin/sh` unless
    /// [`World::keep_descriptors_in_terminals`] chose another), and tells git to read
    /// neither the machine's configuration nor the user's.
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        if let Some(search) = std::env::var_os("PATH") {
            command.env("PATH", search);
        }
        // The shell of a terminal is the same on every machine, and not the user's own.
        command
            .env("SHELL", &self.shell)
            .env("HOME", self.home())
            .env("XDG_DATA_HOME", self.data_home())
            .env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("YAAR_SOCKET", self.socket())
            .env("YAAR_KEEPER", crate::program("yaar-keeper"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }

    /// Makes the terminals of the Keeper this world starts next run a shell that keeps
    /// every descriptor its parent hands it, which a login `/bin/sh` does not on every
    /// system: macOS's closes what it inherits, so a test that looks for a descriptor
    /// that leaked from the Keeper into a terminal's programs would never see it there.
    ///
    /// The terminal starts its shell as a login shell, with `-l`. What runs is a script
    /// of the world that ignores that and becomes an interactive `/bin/sh`, which keeps
    /// what it inherits and hands it on to what it starts.
    pub fn keep_descriptors_in_terminals(&mut self) {
        let script = self.root.join("keeping-shell");
        fs::write(&script, "#!/bin/sh\nexec /bin/sh -i\n").expect("write the script of the shell");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("make the shell runnable");
        self.shell = script;
    }

    /// Starts the built Keeper program on the socket of this world and returns its
    /// process id once it listens. Panics if one already runs.
    ///
    /// This Keeper stays for as long as the world does, with or without a Window
    /// attached. What it writes as errors goes to a file of the world, and is shown when
    /// it does not come to listen.
    pub fn start_keeper(&mut self) -> u32 {
        assert!(
            self.keeper_pid().is_none(),
            "the Keeper of this world already runs"
        );
        let errors = self.root.join("keeper.err");
        let mut keeper = self
            .command(&crate::program("yaar-keeper"))
            .env("YAAR_KEEPER_LINGER_MS", LINGER_MS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&errors).expect("make the file for the Keeper's errors"))
            .spawn()
            .expect("start the Keeper");
        let pid = keeper.id();

        let mine = Hello::for_this_process("the-world");
        let deadline = Instant::now() + DEADLINE;
        let listens = loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket())
                && let Ok(answered) = handshake(&mut stream, &mine)
            {
                break answered.pid == pid;
            }
            let gone = keeper.try_wait().expect("ask after the Keeper").is_some();
            if gone || Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Kept before anything can panic, so that the world ends it in every case.
        self.keeper = Some(keeper);
        assert!(
            listens,
            "the Keeper of the world does not listen at {}: {}",
            self.socket().display(),
            fs::read_to_string(&errors).unwrap_or_default()
        );
        pid
    }

    /// Starts the built Keeper program as [`World::start_keeper`] does, but as the last
    /// arguments of `wrapper`: `["strace", "-f", "-o", <file>, ...]` runs it under a
    /// tracer that follows every process it starts. Gives the process id of the Keeper
    /// itself, the wrapper's child named `yaar-keeper` in `/proc`, once it listens, and
    /// remembers it: [`World::kill_keeper`] and the end of the world kill that process
    /// first with the kill signal and wait until it is gone, and only then kill and
    /// collect the wrapper, since a tracer that is killed lets its processes run on.
    /// Linux only, as the one row that needs it.
    pub fn start_keeper_under(&mut self, wrapper: &[std::ffi::OsString]) -> u32 {
        let _ = wrapper;
        todo!("the Keeper started under a wrapper")
    }

    /// Kills the Keeper this world started with the kill signal, as a crash or the system
    /// does, and returns once it is gone and collected; the world may then start another
    /// on the same folders. Nothing of the Keeper's own runs: no stop, no close of its
    /// database. Panics when no Keeper of this world runs.
    pub fn kill_keeper(&mut self) {
        let mut keeper = self
            .keeper
            .take()
            .expect("no Keeper of this world runs to kill");
        // It may have ended on its own already; the kill then finds nothing to do.
        let _ = keeper.kill();
        keeper.wait().expect("collect the killed Keeper");
    }

    /// The process id of the Keeper this world started, while it runs.
    pub fn keeper_pid(&mut self) -> Option<u32> {
        let keeper = self.keeper.as_mut()?;
        match keeper.try_wait() {
            Ok(None) => Some(keeper.id()),
            // It has ended and is collected: the world may start another.
            _ => {
                self.keeper = None;
                None
            }
        }
    }

    /// A program of this world that writes down what it receives on a terminal, and
    /// returns its path: a program the app knows nothing of.
    ///
    /// Started with the path of a file as its one argument, as when that line is typed
    /// in a shell of the app, it puts its terminal into raw mode with no echo and then
    /// appends every byte it reads to the file, as it reads it, unchanged. It reads until
    /// its terminal ends. The file therefore holds exactly the bytes the program received.
    ///
    /// The file is made once the terminal is in raw mode: a test that waits for it to
    /// exist knows that what it types from then on is recorded as typed.
    pub fn key_recorder(&self) -> PathBuf {
        let script = self.root.join("key-recorder");
        // `cat` with no options reads and writes at once, on every system.
        fs::write(&script, "#!/bin/sh\nstty raw -echo\nexec cat >> \"$1\"\n")
            .expect("write the script of the key recorder");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("make the key recorder runnable");
        script
    }

    /// Puts the stand-in agent on the search path of the programs this world starts from
    /// now on, the Keeper and its shells among them: a folder of the world, first on
    /// `PATH`, holding `claude`, `pi` and `otheragent`, each the built `standin-agent`.
    /// Call it before [`World::start_keeper`].
    ///
    /// Every shell of the app is a login shell, and a login shell reads the system's
    /// profile, which may put other folders first (macOS's `path_helper` does). So the
    /// world's home gets a `.profile` that puts the folder first again, which both a login
    /// `dash` and a login `bash` started as `sh` read.
    pub fn agents_on_path(&mut self) {
        todo!("the stand-in on the search path of the world")
    }

    /// Writes `script` as the file `<name>.script` of the world and gives its path. A
    /// stand-in started with [`crate::standin::SCRIPT_VARIABLE`] naming it follows it; the
    /// script named `default` is the one every stand-in of the world follows when nothing
    /// else names one, as one started from a launcher.
    pub fn agent_script(&self, name: &str, script: &crate::standin::Script) -> PathBuf {
        let _ = (name, script);
        todo!("a script for the stand-in")
    }

    /// What reached the stand-in whose log is `<name>.log` of the world, as it wrote it
    /// down so far; every stand-in of the world writes to `default.log` when nothing else
    /// names its log.
    pub fn agent_log(&self, name: &str) -> crate::standin::StandinLog {
        let _ = name;
        todo!("the log of a stand-in")
    }

    /// A scripted client attached to the Keeper of this world, as a Window would attach.
    pub fn client(&self) -> Client {
        Client::attach(&self.socket())
    }

    /// A new git repository named `name` in this world, with one commit on the branch
    /// `main`.
    pub fn repository(&self, name: &str) -> Repo {
        let repo = Repo {
            root: self.root.join("repos").join(name),
            world: self.root.clone(),
        };
        fs::create_dir(&repo.root).expect("make the folder of the repository");
        repo.git(&["init", "--quiet", "-b", "main"]);
        repo.write("README.md", &format!("# {name}\n"));
        repo.commit("Start the repository");
        repo
    }

    /// A tap between a Window and the Keeper of this world: a socket of its own that
    /// passes every frame on in both directions and keeps what it saw.
    ///
    /// Panics when the world has started no Keeper: a Window that finds none behind the
    /// tap would start one in the tap's place.
    pub fn tap(&self) -> Tap {
        assert!(
            self.keeper.is_some(),
            "start the Keeper of the world before a tap on it"
        );
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket = self.root.join(format!("tap-{n}.sock"));
        let listener = UnixListener::bind(&socket).expect("bind the socket of the tap");
        let seen = Arc::new(Seen::default());
        let stop = Arc::new(AtomicBool::new(false));
        let accepting = {
            let (keeper, seen, stop) = (self.socket(), Arc::clone(&seen), Arc::clone(&stop));
            std::thread::spawn(move || {
                for window in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    // A Window with no Keeper behind the tap is hung up on, as it would
                    // be refused where nothing listens.
                    if let (Ok(window), Ok(keeper)) = (window, yaar_link::connect(&keeper)) {
                        let _ = join(window, keeper, &seen);
                    }
                }
            })
        };
        Tap {
            socket,
            read: 0,
            seen,
            stop,
            accepting: Some(accepting),
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut keeper) = self.keeper.take() {
            let _ = keeper.kill();
            let _ = keeper.wait();
        }
        // A process the Keeper started, as the user's git of a commit whose Keeper was
        // killed, may still be finishing and writing into the folder: it is removed
        // again until it is gone, which is within moments.
        let deadline = Instant::now() + DEADLINE;
        while fs::remove_dir_all(&self.root).is_err()
            && self.root.exists()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.root.exists() {
            eprintln!("the world's folder {} was left behind", self.root.display());
        }
    }
}

/// A git repository of a world, driven as a user drives git: by commands. The author,
/// the committer and the dates are fixed, so the same steps give the same commits.
pub struct Repo {
    root: PathBuf,
    world: PathBuf,
}

impl Repo {
    /// The folder of the worktree.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Runs git in the worktree with these arguments and returns what it printed,
    /// without the line end. Panics, with what git wrote, when git fails.
    pub fn git(&self, args: &[&str]) -> String {
        let mut git = Command::new("git");
        git.env_clear();
        if let Some(search) = std::env::var_os("PATH") {
            git.env("PATH", search);
        }
        let output = git
            .current_dir(&self.root)
            .args(args)
            .env("HOME", self.world.join("home"))
            .env("TMPDIR", self.world.join("tmp"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "A Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z")
            .env("GIT_COMMITTER_NAME", "A Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .expect("start git");
        assert!(
            output.status.success(),
            "git {args:?} in {} failed: {}{}",
            self.root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let printed = String::from_utf8_lossy(&output.stdout);
        printed.strip_suffix('\n').unwrap_or(&printed).to_owned()
    }

    /// Writes a file of the worktree, making the folders above it.
    pub fn write(&self, path: &str, content: &str) {
        let file = self.root.join(path);
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).expect("make the folders above the file");
        }
        fs::write(&file, content).expect("write the file");
    }

    /// Stages everything and commits it with this message. Returns the id of the commit.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }
}

/// The requests a tap has seen pass, for the thread that passes them and the test that
/// reads them.
#[derive(Default)]
struct Seen {
    requests: Mutex<Vec<Request>>,
    more: Condvar,
}

impl Seen {
    fn requests(&self) -> MutexGuard<'_, Vec<Request>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A tap on the Link: what a test puts between a Window and a Keeper to read the
/// requests that reach the Keeper, without a door into either.
///
/// It listens at [`Tap::socket`]. Each connection made there is joined to a connection of
/// its own to the Keeper, and every frame is passed on unchanged, both ways.
///
/// The tap keeps every request, and a place in that list up to which the waits have
/// read. [`Tap::requests`] gives the whole list; [`Tap::wait_for_request`] looks only
/// past the place and moves it.
pub struct Tap {
    socket: PathBuf,
    // How many requests the waits have passed.
    read: usize,
    seen: Arc<Seen>,
    stop: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
}

impl Tap {
    /// Where a Window is told to find the Keeper, to go through the tap.
    pub fn socket(&self) -> PathBuf {
        self.socket.clone()
    }

    /// Every request the tap has taken from a Window so far, in order, whatever the
    /// waits have read. A request is in the list before it is handed to the Keeper, so
    /// one that the Keeper has answered is never missing from it.
    pub fn requests(&self) -> Vec<Request> {
        self.seen.requests().clone()
    }

    /// Waits, at most `limit`, until a Window has sent the stop-everything frame through
    /// the tap, and gives whether it did. The tap keeps that frame as it keeps requests,
    /// before it hands it on.
    pub fn wait_for_stop(&self, limit: Duration) -> bool {
        let _ = limit;
        todo!("the stop-everything frame, seen by the tap")
    }

    /// Waits for a request that `wanted` accepts, at most `limit`, and returns it. Only
    /// requests that no earlier wait has passed are looked at, and this wait passes the
    /// one it returns and every request before it: a request seen once does not answer
    /// a second wait, as with the facts of [`Client::wait_for_fact`]. Panics, with the
    /// requests looked at, when none comes.
    pub fn wait_for_request(
        &mut self,
        limit: Duration,
        wanted: impl Fn(&Request) -> bool,
    ) -> Request {
        let deadline = Instant::now() + limit;
        let mut requests = self.seen.requests();
        let mut looked_at = self.read;
        loop {
            if let Some(found) = requests[looked_at..].iter().position(&wanted) {
                self.read = looked_at + found + 1;
                return requests[looked_at + found].clone();
            }
            looked_at = requests.len();
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "the request did not pass within {limit:?}; those looked at: {:?}",
                &requests[self.read..]
            );
            requests = self
                .seen
                .more
                .wait_timeout(requests, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // The thread that accepts sits in `accept`: a connection wakes it, and it then
        // sees that it is to stop. Connections already joined end with their two ends.
        self.stop.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.socket);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

/// Joins a Window's connection to one to the Keeper: two threads, one for each
/// direction. When one side closes, the other is told so by closing what is written
/// to it.
fn join(window: UnixStream, keeper: UnixStream, seen: &Arc<Seen>) -> io::Result<()> {
    let (mut from_window, mut to_keeper) = (window.try_clone()?, keeper.try_clone()?);
    let seen = Arc::clone(seen);
    std::thread::spawn(move || {
        let _ = pass_requests(&mut from_window, &mut to_keeper, &seen);
        let _ = to_keeper.shutdown(Shutdown::Write);
    });
    let (mut from_keeper, mut to_window) = (keeper, window);
    std::thread::spawn(move || {
        // What the Keeper says is not kept: the Window shows it.
        let _ = io::copy(&mut from_keeper, &mut to_window);
        let _ = to_window.shutdown(Shutdown::Write);
    });
    Ok(())
}

/// Passes on what a Window sends, frame by frame, as the bytes it sent, and keeps a
/// copy of each frame that is a request before the frame goes on. A frame that cannot
/// be read as a message is passed on all the same: what the Keeper makes of it is the
/// Keeper's to say.
fn pass_requests(from: &mut UnixStream, to: &mut UnixStream, seen: &Seen) -> io::Result<()> {
    loop {
        let mut frame = vec![0u8; 4];
        match from.read(&mut frame[..1])? {
            0 => return Ok(()),
            _ => from.read_exact(&mut frame[1..])?,
        }
        let length = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
        if length > MAX_FRAME {
            // Longer than any message: not held here, only handed through.
            to.write_all(&frame)?;
            io::copy(&mut Read::take(&mut *from, u64::from(length)), to)?;
            continue;
        }
        frame.resize(4 + length as usize, 0);
        from.read_exact(&mut frame[4..])?;
        // Kept before it is handed on: whatever the Keeper has answered is then in the
        // list already, for a test that reads the list once the reply is there.
        if let Ok(Message::Request { request, .. }) = read_message(&mut frame.as_slice()) {
            seen.requests().push(request);
            seen.more.notify_all();
        }
        to.write_all(&frame)?;
    }
}
