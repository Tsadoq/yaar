//! The world of one test: its own folders, its own repositories, its own Keeper.
//!
//! Nothing of one world touches another or the machine's own: a home folder, the app's
//! folder and a runtime folder of the world's own, and git told to read no configuration
//! but what the world sets. Tests therefore run side by side. A program of the world
//! writes its temporary files in a folder of the world too, so that a program that is
//! killed leaves nothing in the machine's temporary folder.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_link::{
    APP_FOLDER, ConsentAnswer, Hello, MAX_FRAME, Message, Request, handshake, read_message,
};

use crate::client::{Client, DEADLINE};
use crate::descendants_of;
use crate::dialect::Dialect;
use crate::standin::{LOG_VARIABLE, PROMPT, SCRIPT_VARIABLE};

/// How long the Keeper a world starts stays with no Window attached: longer than any
/// test. It ends with its world, not by itself in the middle of a test that has no
/// client attached for a while.
const LINGER_MS: &str = "3600000";

/// The names the stand-in is found under in a world that put it on the search path.
const AGENT_NAMES: [&str; 3] = ["claude", "pi", "otheragent"];

/// `search` without every folder that holds a program named `claude` or `pi`, so that a
/// program of a world never finds the user's own agent CLI: what [`World::command`] makes
/// of the search path of the process that runs the test.
pub fn without_agent_clis(search: &OsStr) -> OsString {
    let holds = |folder: &Path| {
        ["claude", "pi"].iter().any(|name| {
            fs::metadata(folder.join(name))
                .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
        })
    };
    let kept: Vec<_> = std::env::split_paths(search)
        .filter(|folder| !holds(folder))
        .collect();
    std::env::join_paths(kept).expect("join the search path")
}

/// Panics, naming the paths, unless `printed` is the line `command -v` printed for each
/// stand-in, in order, and each is the one in the world's folder.
fn assert_found_first(printed: &str, wanted: &[PathBuf]) {
    let found: Vec<_> = printed.lines().map(PathBuf::from).collect();
    assert!(
        found == wanted,
        "a login shell of this world finds {found:?} for {AGENT_NAMES:?}, not {wanted:?}"
    );
}

/// One test's world. Everything in it lies under one folder with a short path. When the
/// world is dropped its Keeper is killed and the folder removed.
pub struct World {
    root: PathBuf,
    /// What the world started as its Keeper: the Keeper itself, or the wrapper it runs
    /// under.
    keeper: Option<Child>,
    /// The Keeper's own process id when it runs under a wrapper.
    keeper_under_wrapper: Option<u32>,
    shell: PathBuf,
    /// The folder of stand-ins, once [`World::agents_on_path`] made it.
    agents: Option<PathBuf>,
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
        let world = World {
            root,
            keeper: None,
            keeper_under_wrapper: None,
            shell: PathBuf::from("/bin/sh"),
            agents: None,
        };
        world.write_profile(&world.profile());
        world
    }

    /// The search path of the programs of this world: the machine's own less the folders
    /// that hold an agent CLI, with the folder of stand-ins in front once there is one.
    fn search_path(&self) -> Option<OsString> {
        let machine = std::env::var_os("PATH").map(|search| without_agent_clis(&search));
        match (&self.agents, machine) {
            (Some(agents), Some(search)) => {
                let folders = std::iter::once(agents.clone()).chain(std::env::split_paths(&search));
                Some(std::env::join_paths(folders).expect("join the path"))
            }
            (Some(agents), None) => Some(agents.clone().into_os_string()),
            (None, search) => search,
        }
    }

    /// The profile of the world's user, which a login shell reads after the system's own:
    /// it keeps what the shell's search path holds and puts the folder of stand-ins, once
    /// there is one, in front, and the machine's own folders after, which a system's
    /// profile may have dropped (Ubuntu's sets the path outright). A folder that holds a
    /// program named `claude` or `pi` is left out whatever put it there, since the system's
    /// profile may add one too (macOS's adds those of `/etc/paths`). Both a login `dash`
    /// and a login `bash` started as `sh` read it.
    ///
    /// The profile holds no line for the app: the app puts no folder first on the path of
    /// a shell, and pi finds the app's extension in its own settings instead.
    fn profile(&self) -> String {
        let quoted = |text: &OsStr| format!("'{}'", text.to_string_lossy().replace('\'', r"'\''"));
        let machine = std::env::var_os("PATH").map(|search| without_agent_clis(&search));
        let agents = self
            .agents
            .as_deref()
            .map(|folder| quoted(folder.as_os_str()));
        // A subshell, so that its options and variables are not the login shell's. The
        // opening bracket of the pattern keeps macOS's bash 3 from reading its closing one
        // as the end of the subshell.
        format!(
            "PATH=$(\n\
             set -f\n\
             machine={machine}\n\
             new={first}\n\
             seen=':'{first}':'\n\
             IFS=:\n\
             for folder in $PATH $machine; do\n\
               [ -n \"$folder\" ] || continue\n\
               case \"$seen\" in (*\":$folder:\"*) continue ;; esac\n\
               seen=\"$seen$folder:\"\n\
               [ -f \"$folder/claude\" ] && [ -x \"$folder/claude\" ] && continue\n\
               [ -f \"$folder/pi\" ] && [ -x \"$folder/pi\" ] && continue\n\
               new=\"$new${{new:+:}}$folder\"\n\
             done\n\
             printf '%s' \"$new\"\n\
             ); export PATH\n",
            machine = machine.as_deref().map_or_else(|| "''".to_owned(), quoted),
            first = agents.as_deref().unwrap_or("''"),
        )
    }

    fn write_profile(&self, profile: &str) {
        fs::write(self.home().join(".profile"), profile)
            .expect("write the profile of the world's user");
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
    /// `default.script` and `default.log`. A login shell of the world, as every terminal
    /// runs, ends up with that same search path: the world's `.profile` sets it, since the
    /// system's own profile may add folders (macOS's does, with those of `/etc/paths`).
    /// The command also names the home, data, runtime and temporary folders, the socket
    /// and the Keeper program, the shell of a terminal (`/bin/sh` unless
    /// [`World::keep_descriptors_in_terminals`] chose another), and tells git to read
    /// neither the machine's configuration nor the user's.
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        if let Some(search) = self.search_path() {
            command.env("PATH", search);
        }
        if self.agents.is_some() {
            command
                .env(SCRIPT_VARIABLE, self.agent_file("default", "script"))
                .env(LOG_VARIABLE, self.agent_file("default", "log"));
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
        self.launch(&[])
    }

    /// Starts the built Keeper program as [`World::start_keeper`] does, but as the last
    /// argument of `wrapper`: `["strace", "-f", "-o", <file>, ...]` runs it under a
    /// tracer that follows every process it starts. Gives the process id of the Keeper
    /// itself, as it told it in its hello once it listens, and remembers it:
    /// [`World::kill_keeper`] and the end of the world kill that process first with the
    /// kill signal and wait until it is gone, and only then kill and collect the wrapper,
    /// since a tracer that is killed lets its processes run on. [`World::keeper_pid`]
    /// gives that process too. Meant for the one row that needs a tracer, which runs on
    /// Linux only; a wrapper that is no tracer works as well.
    pub fn start_keeper_under(&mut self, wrapper: &[OsString]) -> u32 {
        assert!(!wrapper.is_empty(), "a wrapper names at least its program");
        self.launch(wrapper)
    }

    fn launch(&mut self, wrapper: &[OsString]) -> u32 {
        assert!(
            self.keeper_pid().is_none(),
            "the Keeper of this world already runs"
        );
        let errors = self.root.join("keeper.err");
        let keeper_program = crate::program("yaar-keeper");
        let mut command = match wrapper.split_first() {
            None => self.command(&keeper_program),
            Some((first, rest)) => {
                let mut command = self.command(Path::new(first));
                command.args(rest).arg(&keeper_program);
                command
            }
        };
        let mut keeper = command
            .env("YAAR_KEEPER_LINGER_MS", LINGER_MS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&errors).expect("make the file for the Keeper's errors"))
            .spawn()
            .unwrap_or_else(|e| {
                panic!(
                    "could not start {}: {e}",
                    wrapper
                        .first()
                        .map_or(keeper_program.as_os_str(), |w| w)
                        .to_string_lossy()
                )
            });
        let started = keeper.id();

        let mine = Hello::for_this_process("the-world");
        let deadline = Instant::now() + DEADLINE;
        // The process that answers at the socket: the Keeper, whether or not something
        // runs it.
        let mut answering = None;
        let listens = loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket())
                && let Ok(answered) = handshake(&mut stream, &mine)
            {
                answering = Some(answered.pid);
                // Run by a wrapper, the Keeper is not the process that was started.
                break !wrapper.is_empty() || answered.pid == started;
            }
            let gone = keeper.try_wait().expect("ask after the Keeper").is_some();
            if gone || Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Kept before anything can panic, so that the world ends it in every case.
        self.keeper = Some(keeper);
        self.keeper_under_wrapper = answering.filter(|pid| !wrapper.is_empty() && *pid != started);
        assert!(
            listens,
            "the Keeper of the world does not listen at {}: {}",
            self.socket().display(),
            fs::read_to_string(&errors).unwrap_or_default()
        );
        answering.expect("the Keeper answered")
    }

    /// Kills everything a restart of the machine would end, and empties what a restart
    /// empties: every process the Keeper this world started has started, and their
    /// descendants, the shells and agents of its terminals and every hook among them, and
    /// then the Keeper, each with the kill signal; returns once all are gone, the Keeper
    /// collected, and the world's runtime folder (`XDG_RUNTIME_DIR`) and temporary folder
    /// (`TMPDIR`) emptied, the folders themselves kept. Nothing of theirs runs: no stop, no
    /// `SessionEnd`, no close of the Keeper's database. What a restart keeps stays: the
    /// home and the app's data folder (the Store, the state stores, the Adapters' folders),
    /// the repositories, the stand-ins' scripts and logs, and the Keeper's socket file,
    /// stale as a killed Keeper leaves it. The world may start another Keeper on them.
    /// Panics when no Keeper of this world runs.
    pub fn kill_everything(&mut self) {
        let keeper = self
            .keeper_pid()
            .expect("no Keeper of this world runs to kill everything under");
        // Frozen first, so that what the Keeper and its programs start meanwhile is found:
        // a process whose parent dies is taken in by the system and is no longer found
        // below that parent. The list is taken again, each time with what was started
        // since, until a look finds no one new.
        let mut doomed = vec![keeper];
        let mut frozen = 0;
        loop {
            for pid in &doomed[frozen..] {
                // SAFETY: a plain signal to a process of this world.
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGSTOP) };
            }
            frozen = doomed.len();
            for pid in descendants_of(keeper) {
                if !doomed.contains(&pid) {
                    doomed.push(pid);
                }
            }
            if doomed.len() == frozen {
                break;
            }
        }
        for pid in &doomed[1..] {
            // SAFETY: a plain signal to a process of this world.
            unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
        }
        assert!(self.end_keeper(), "the killed Keeper is still there");
        assert!(
            crate::wait_until(DEADLINE, || doomed.iter().all(|pid| !crate::is_alive(*pid))),
            "still running after the kill: {:?}",
            doomed
                .iter()
                .filter(|pid| crate::is_alive(**pid))
                .collect::<Vec<_>>()
        );
        for folder in ["run", "tmp"] {
            let folder = self.root.join(folder);
            for entry in fs::read_dir(&folder).expect("read a folder of the world") {
                let path = entry.expect("read an entry").path();
                // A link, a socket or a file goes as it is; only a real folder is entered.
                let result = if path.is_dir() && !path.is_symlink() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                result.unwrap_or_else(|why| panic!("empty {}: {why}", path.display()));
            }
        }
    }

    /// Starts the stand-in agent of `dialect` as a user would start it outside every
    /// terminal of the app, in `folder`: in a pseudo-terminal the world opens itself, with
    /// the world's environment for programs (`World::command`) and `YAAR_TERMINAL` unset. It
    /// reports over the Keeper's socket from no terminal the Keeper holds. Its script is
    /// `script`; its log is `agent_log(dialect.name())`.
    ///
    /// As Claude Code it gets `CLAUDE_CODE_PLUGIN_DIRS` naming the plugin folder the
    /// world's Keeper wrote, as a user who points Claude Code at the app's plugin by hand
    /// would.
    ///
    /// As pi it is the stand-in named `pi`, which finds its extension in pi's settings, as
    /// pi does, and reports through the program `YAAR_PI_REPORT` names. The world sets both
    /// by hand, where they are missing, as a user who points pi at the app's pieces by hand
    /// would: the variable names the reporting program the Keeper's shells name, and the
    /// settings ([`World::pi_settings`]) list the extension the world's Keeper wrote, which
    /// the Keeper's yes is not needed for. A settings file that lists it already, after
    /// [`World::allow_pi_extension`], is left as it is; one that does not has the entry
    /// added to its `extensions` and keeps its other keys; no file is made, with the folder
    /// it lies in, holding the entry alone.
    ///
    /// The file the world writes is compact JSON, not the layout the app keeps: a test that
    /// compares a user's settings byte for byte gives the Keeper's yes first.
    ///
    /// Panics when the world's Keeper wrote no plugin, or no extension, to point at: start
    /// the Keeper first.
    pub fn start_agent_outside(
        &self,
        dialect: Dialect,
        script: &crate::standin::Script,
        folder: &Path,
    ) -> OutsideAgent {
        let name = dialect.name();
        let agents = self
            .agents
            .as_ref()
            .expect("put the stand-ins on the search path first (World::agents_on_path)");
        let script_file = self.agent_script(name, script);
        // Started by its name, as a shell finds it, which the stand-in reads its dialect from.
        let mut command = self.command(&agents.join(name));
        command
            .env(SCRIPT_VARIABLE, script_file)
            .env(LOG_VARIABLE, self.agent_file(name, "log"))
            .current_dir(folder);
        match dialect {
            Dialect::ClaudeCode => {
                // Where the Adapter of Claude Code writes its plugin, once the Keeper runs.
                let plugin = self.data_dir().join("agents").join("claude").join("plugin");
                assert!(
                    plugin.join("hooks/hooks.json").exists(),
                    "the Keeper of this world wrote no plugin at {}: start it first",
                    plugin.display()
                );
                command.env("CLAUDE_CODE_PLUGIN_DIRS", plugin);
            }
            Dialect::Pi => {
                // Where the Adapter of pi writes its extension, once the Keeper runs.
                let extension = self.pi_extension();
                assert!(
                    extension.is_file(),
                    "the Keeper of this world wrote no extension of pi at {}: start it first",
                    extension.display()
                );
                self.list_in_pi_settings(&extension);
                // The reporting program lies beside the Keeper, where the Keeper names it.
                command.env("YAAR_PI_REPORT", crate::program("yaar-pi-report"));
            }
        }
        let (master, slave) = open_terminal();
        let stdio = |terminal: &OwnedFd| {
            Stdio::from(terminal.try_clone().expect("duplicate the terminal's end"))
        };
        let child = command
            .stdin(stdio(&slave))
            .stdout(stdio(&slave))
            .stderr(stdio(&slave))
            .spawn()
            .unwrap_or_else(|why| panic!("start the stand-in {name}: {why}"));
        // The program holds the slave end now; the world's copy would keep the terminal
        // open for ever.
        drop(slave);
        OutsideAgent {
            child,
            terminal: fs::File::from(master),
            output: Vec::new(),
            seen: 0,
        }
    }

    /// The settings of pi in this world's home, `<home>/.pi/agent/settings.json`: where pi
    /// looks for the extensions it loads, and where the Keeper writes the one entry after
    /// the user's yes. The file is there only when something wrote it.
    pub fn pi_settings(&self) -> PathBuf {
        self.pi_folder().join("settings.json")
    }

    /// Makes pi installed in this world, as pi's first run does: the folder
    /// `<home>/.pi/agent` and nothing in it, so that the world's Keeper asks the user's
    /// leave to write into pi's settings. Call it before [`World::start_keeper`]: a world
    /// that never calls it has no pi, and its home holds no `.pi` after a Keeper ran.
    pub fn install_pi(&self) {
        fs::create_dir_all(self.pi_folder()).expect("make the folder of pi's own files");
    }

    /// Gives the user's yes to the question of pi, as the Window's control does: sends
    /// `AnswerConsent` for `pi` through a client of the world, and waits until the Keeper
    /// lists the answer as yes, whence pi's settings list the extension. Call it after
    /// [`World::start_keeper`], and [`World::install_pi`] before that.
    ///
    /// Panics when the Keeper lists no question of pi (pi is not installed in this world),
    /// when it says what went wrong with pi's settings, and when the answer is not yes
    /// within [`DEADLINE`].
    pub fn allow_pi_extension(&self) {
        let mut client = self.client();
        let mut consents = client
            .answer_consent("pi", ConsentAnswer::Yes)
            .unwrap_or_else(|why| panic!("the Keeper refused the yes for pi: {why:?}"));
        let deadline = Instant::now() + DEADLINE;
        loop {
            let pi = consents
                .iter()
                .find(|consent| consent.agent == "pi")
                .unwrap_or_else(|| {
                    panic!(
                        "the Keeper asks no question of pi: call install_pi before start_keeper \
                         ({consents:?})"
                    )
                });
            assert_eq!(pi.problem, None, "pi's settings could not be changed");
            if pi.answer == ConsentAnswer::Yes {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the Keeper did not list the answer for pi as yes: {pi:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
            consents = client
                .consents()
                .unwrap_or_else(|why| panic!("the Keeper refused the list of questions: {why:?}"));
        }
    }

    fn pi_folder(&self) -> PathBuf {
        self.home().join(".pi").join("agent")
    }

    /// Where the Adapter of pi writes its extension, once the Keeper runs.
    fn pi_extension(&self) -> PathBuf {
        self.data_dir()
            .join("agents")
            .join("pi")
            .join("extension")
            .join("yaar.ts")
    }

    /// Makes pi's settings list `extension` among the top-level `extensions`, leaving them
    /// as they are when they do: what a user who points pi at the app's extension by hand
    /// does.
    fn list_in_pi_settings(&self, extension: &Path) {
        let entry = extension.to_str().expect("the extension's path is text");
        let settings = self.pi_settings();
        let mut text = match fs::read_to_string(&settings) {
            Ok(text) => serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|why| {
                panic!(
                    "the settings of pi at {} are not JSON: {why}",
                    settings.display()
                )
            }),
            Err(why) if why.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(why) => panic!("read {}: {why}", settings.display()),
        };
        let object = text.as_object_mut().unwrap_or_else(|| {
            panic!("the settings of pi at {} are no object", settings.display())
        });
        let listed = object
            .entry("extensions")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .unwrap_or_else(|| panic!("`extensions` of {} is no array", settings.display()));
        if listed.iter().any(|listed| listed.as_str() == Some(entry)) {
            return;
        }
        listed.push(serde_json::Value::String(entry.to_owned()));
        fs::create_dir_all(self.pi_folder()).expect("make the folder of pi's own files");
        fs::write(&settings, text.to_string()).expect("write the settings of pi");
    }

    /// Kills the Keeper this world started with the kill signal, as a crash or the system
    /// does, and returns once it is gone and collected; the world may then start another
    /// on the same folders. Nothing of the Keeper's own runs: no stop, no close of its
    /// database. Panics when no Keeper of this world runs.
    pub fn kill_keeper(&mut self) {
        assert!(
            self.keeper.is_some(),
            "no Keeper of this world runs to kill"
        );
        assert!(self.end_keeper(), "the killed Keeper is still there");
    }

    /// The process id of the Keeper this world started, while it runs: its own, also
    /// when it runs under a wrapper. A wrapper that ended before the Keeper does not end
    /// the Keeper's time as the world's, and a Keeper that ended under a wrapper that
    /// runs on does: the wrapper is ended then, and the world may start another Keeper.
    pub fn keeper_pid(&mut self) -> Option<u32> {
        let own = self.keeper_under_wrapper;
        let keeper = self.keeper.as_mut()?;
        let started = keeper.id();
        let wrapper_runs = matches!(keeper.try_wait(), Ok(None));
        match own {
            Some(own) if crate::is_alive(own) => Some(own),
            Some(_) => {
                self.end_keeper();
                None
            }
            None if wrapper_runs => Some(started),
            // It has ended and is collected: the world may start another.
            None => {
                self.keeper = None;
                None
            }
        }
    }

    /// Kills what the world started as its Keeper, the Keeper itself first when it runs
    /// under a wrapper, and collects it. Gives whether the Keeper's own process is gone.
    fn end_keeper(&mut self) -> bool {
        let Some(mut started) = self.keeper.take() else {
            return true;
        };
        let mut gone = true;
        if let Some(own) = self.keeper_under_wrapper.take()
            && crate::is_alive(own)
        {
            // SAFETY: a plain signal to a process of this world.
            unsafe { libc::kill(own as libc::pid_t, libc::SIGKILL) };
            gone = crate::wait_until(DEADLINE, || !crate::is_alive(own));
        }
        // It may have ended on its own already; the kill then finds nothing to do.
        let _ = started.kill();
        let _ = started.wait();
        gone
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
    /// profile, which may put other folders first. So the world's `.profile` puts the
    /// folder first again. Panics unless a login shell finds the stand-ins in that folder.
    pub fn agents_on_path(&mut self) {
        self.agents_on_path_with(|search| search);
    }

    /// [`World::agents_on_path`], with the profile it writes made of its own text by
    /// `profile`: where the check that a login shell finds the stand-ins first has to be
    /// seen to refuse a shell that does not.
    fn agents_on_path_with(&mut self, profile: impl FnOnce(String) -> String) {
        let folder = self.root.join("agents");
        fs::create_dir_all(&folder).expect("make the folder of the stand-ins");
        // A link to a copy of the stand-in that nothing replaces: a link to the file Cargo
        // built would lead nowhere while a build puts that file in its place again.
        let standin = crate::program("standin-agent");
        for name in AGENT_NAMES {
            let link = folder.join(name);
            let _ = fs::remove_file(&link);
            symlink(&standin, &link).expect("link a stand-in");
        }
        self.agents = Some(folder.clone());
        self.write_profile(&profile(self.profile()));
        self.check_agents_found(&folder);
    }

    /// Panics, naming the paths, unless a login shell of this world, as the app starts
    /// one, finds each stand-in in `folder`.
    fn check_agents_found(&self, folder: &Path) {
        let found = self
            .command(Path::new("/bin/sh"))
            .args(["-l", "-c"])
            .arg(format!(
                "for n in {}; do command -v $n; done",
                AGENT_NAMES.join(" ")
            ))
            .stdin(Stdio::null())
            .output()
            .expect("start a login shell");
        let wanted: Vec<_> = AGENT_NAMES.iter().map(|name| folder.join(name)).collect();
        assert_found_first(&String::from_utf8_lossy(&found.stdout), &wanted);
    }

    /// The folder of stand-ins, once [`World::agents_on_path`] made it.
    pub fn agents_dir(&self) -> Option<PathBuf> {
        self.agents.clone()
    }

    /// Writes `script` as the file `<name>.script` of the world and gives its path. A
    /// stand-in started with [`crate::standin::SCRIPT_VARIABLE`] naming it follows it; the
    /// script named `default` is the one every stand-in of the world follows when nothing
    /// else names one, as one started from a launcher. Its log is the file `<name>.log`
    /// beside it, which a stand-in started with [`crate::standin::LOG_VARIABLE`] naming
    /// that file writes.
    pub fn agent_script(&self, name: &str, script: &crate::standin::Script) -> PathBuf {
        let path = self.agent_file(name, "script");
        fs::write(&path, script.text()).expect("write the script of the stand-in");
        path
    }

    /// What reached the stand-in whose log is `<name>.log` of the world, as it wrote it
    /// down so far; every stand-in of the world writes to `default.log` when nothing else
    /// names its log.
    pub fn agent_log(&self, name: &str) -> crate::standin::StandinLog {
        crate::standin::StandinLog::read(&self.agent_file(name, "log"))
    }

    fn agent_file(&self, name: &str, extension: &str) -> PathBuf {
        self.root.join(format!("{name}.{extension}"))
    }

    /// A world for a test of an agent: the stand-in on its search path, the script
    /// `script` written as `<name>.script`, a repository `app` with one commit and an
    /// identity of its own in its configuration, since git on Linux guesses none, and the
    /// Keeper started. Name the script `default` for the stand-in that a launcher or a
    /// shell of the app starts to follow it.
    pub fn with_standin(name: &str, script: &crate::standin::Script) -> (World, Repo) {
        World::with_standin_as(crate::dialect::Dialect::ClaudeCode, name, script)
    }

    /// A world for a test of an agent as `dialect`, as [`World::with_standin`] makes it for
    /// the stand-in `name`. As pi, pi is installed in the world's home before the Keeper
    /// starts, so that the Keeper asks the question of pi, and the user's yes is given after
    /// it, so that the stand-in started as `pi` finds the app's extension in pi's settings,
    /// as pi does.
    pub fn with_standin_as(
        dialect: crate::dialect::Dialect,
        name: &str,
        script: &crate::standin::Script,
    ) -> (World, Repo) {
        let mut world = World::new();
        world.agents_on_path();
        if dialect == crate::dialect::Dialect::Pi {
            world.install_pi();
        }
        world.agent_script(name, script);
        let repo = world.repository("app");
        repo.git(&["config", "user.name", "A Test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        world.start_keeper();
        if dialect == crate::dialect::Dialect::Pi {
            world.allow_pi_extension();
        }
        (world, repo)
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

/// A stand-in agent that runs outside every terminal of the app, in a pseudo-terminal of
/// the world's own (`World::start_agent_outside`). Dropping it kills it and waits for it.
pub struct OutsideAgent {
    child: std::process::Child,
    /// The pseudo-terminal's end the test types into and reads.
    terminal: std::fs::File,
    /// Everything the agent wrote, as far as it was read.
    output: Vec<u8>,
    /// How far of `output` the prompts waited for so far reach.
    seen: usize,
}

impl OutsideAgent {
    /// Its process.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Types `text` and Enter into its terminal.
    pub fn type_line(&mut self, text: &str) {
        self.terminal
            .write_all(format!("{text}\r").as_bytes())
            .expect("type into the terminal of the agent");
    }

    /// Waits, until [`crate::DEADLINE`], for its prompt `standin> ` in what it wrote since
    /// it started, read in `poll` and never with a sleep between looks; panics, naming
    /// what it wrote, when it does not come. A prompt that an earlier wait found is not
    /// found again: each call waits for one the agent printed after the last.
    pub fn wait_for_prompt(&mut self) {
        let deadline = Instant::now() + DEADLINE;
        let prompt = PROMPT.as_bytes();
        loop {
            let unseen = &self.output[self.seen..];
            if let Some(at) = unseen.windows(prompt.len()).position(|w| w == prompt) {
                self.seen += at + prompt.len();
                return;
            }
            assert!(
                self.read_some(deadline),
                "the agent printed no prompt after byte {}; it wrote {:?}",
                self.seen,
                String::from_utf8_lossy(&self.output)
            );
        }
    }

    /// Reads what the agent has written, waiting in `poll` for it until `deadline`. Gives
    /// false when nothing came in time, or its terminal is closed.
    fn read_some(&mut self, deadline: Instant) -> bool {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            // Rounded up, so that a part of a millisecond left is a wait and not none.
            let wait = i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);
            let mut poll = libc::pollfd {
                fd: self.terminal.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid `pollfd`.
            let ready = unsafe { libc::poll(&mut poll, 1, wait) };
            if ready < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if ready <= 0 || poll.revents & libc::POLLIN == 0 {
                return false;
            }
            let mut buffer = [0u8; 8192];
            match self.terminal.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    self.output.extend_from_slice(&buffer[..n]);
                    return true;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // Closed: nothing more will come.
                _ => return false,
            }
        }
    }
}

/// A pseudo-terminal of the world's own: the end a test reads and types into, and the end
/// a program runs on. Neither is inherited by what the world starts, but for the one it
/// duplicates the slave into.
fn open_terminal() -> (OwnedFd, OwnedFd) {
    let (mut master, mut slave) = (0, 0);
    // SAFETY: both are valid places for a descriptor, and the rest may be null.
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0, "openpty: {}", io::Error::last_os_error());
    for fd in [master, slave] {
        // SAFETY: `fd` was just opened by `openpty`.
        let marked = unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        assert_eq!(marked, 0, "mark the terminal's end close-on-exec");
    }
    // SAFETY: both descriptors are open and owned by nobody else.
    unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

impl Drop for OutsideAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.end_keeper();
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
    /// Whether a Window sent the stop-everything frame. Set while the requests are held,
    /// so that a wait on `more` cannot miss it.
    stopped: AtomicBool,
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
        let deadline = Instant::now() + limit;
        let mut requests = self.seen.requests();
        while !self.seen.stopped.load(Ordering::SeqCst) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            requests = self
                .seen
                .more
                .wait_timeout(requests, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
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
/// copy of each frame that is a request, or the stop-everything frame, before the frame
/// goes on. A frame that cannot
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
        match read_message(&mut frame.as_slice()) {
            Ok(Message::Request { request, .. }) => {
                seen.requests().push(request);
                seen.more.notify_all();
            }
            Ok(Message::Shutdown) => {
                let _held = seen.requests();
                seen.stopped.store(true, Ordering::SeqCst);
                seen.more.notify_all();
            }
            _ => {}
        }
        to.write_all(&frame)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_shell_that_finds_another_claude_makes_the_world_panic_naming_both_paths() {
        let wanted: Vec<PathBuf> = AGENT_NAMES
            .iter()
            .map(|name| Path::new("/world/agents").join(name))
            .collect();
        let right: String = wanted
            .iter()
            .map(|p| format!("{}\n", p.display()))
            .collect();
        assert_found_first(&right, &wanted);

        let wrong = right.replacen("/world/agents/claude", "/usr/local/bin/claude", 1);
        let panic = std::panic::catch_unwind(|| assert_found_first(&wrong, &wanted))
            .expect_err("another claude first is refused");
        let said = panic
            .downcast_ref::<String>()
            .expect("the panic says what it found");
        assert!(said.contains("/usr/local/bin/claude"), "{said}");
        assert!(said.contains("/world/agents/claude"), "{said}");
    }

    #[test]
    fn a_world_whose_login_shell_finds_another_claude_first_panics_naming_both_paths() {
        let mut world = World::new();
        let elsewhere = world.home().join("elsewhere");
        fs::create_dir_all(&elsewhere).expect("make the folder");
        fs::write(elsewhere.join("claude"), "#!/bin/sh\n").expect("write a claude");
        fs::set_permissions(elsewhere.join("claude"), fs::Permissions::from_mode(0o755))
            .expect("make it runnable");

        // A profile that puts the other folder in front of the world's own, as a system's
        // profile may.
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.agents_on_path_with(|profile| {
                format!(
                    "{profile}PATH='{}':$PATH; export PATH\n",
                    elsewhere.display()
                )
            })
        }))
        .expect_err("another claude first is refused");
        let said = panic
            .downcast_ref::<String>()
            .expect("the panic says what it found");
        assert!(
            said.contains(&elsewhere.join("claude").display().to_string()),
            "{said}"
        );
        assert!(
            said.contains(&world.root.join("agents/claude").display().to_string()),
            "{said}"
        );
    }

    #[test]
    fn a_login_shell_that_finds_nothing_makes_the_world_panic_too() {
        let wanted = [PathBuf::from("/world/agents/claude")];
        assert!(std::panic::catch_unwind(|| assert_found_first("", &wanted)).is_err());
    }
}
