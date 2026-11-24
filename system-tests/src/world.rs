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
use crate::standin::{LOG_VARIABLE, PROMPT, SCRIPT_VARIABLE, Script};

const LINGER_MS: &str = "3600000";

const AGENT_NAMES: [&str; 3] = ["claude", "pi", "otheragent"];

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

fn assert_found_first(printed: &str, wanted: &[PathBuf]) {
    let found: Vec<_> = printed.lines().map(PathBuf::from).collect();
    assert!(
        found == wanted,
        "a login shell of this world finds {found:?} for {AGENT_NAMES:?}, not {wanted:?}"
    );
}

pub struct World {
    root: PathBuf,
    keeper: Option<Child>,
    keeper_under_wrapper: Option<u32>,
    shell: PathBuf,
    agents: Option<PathBuf>,
}

impl World {
    #[allow(clippy::new_without_default)]
    pub fn new() -> World {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS. The Keeper names a worktree by its resolved path, and a test
        // compares that with its world's paths.
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

    /// The user's `.profile`: the stand-ins first, then the shell's path and the machine's own
    /// folders, which a system profile may drop (Ubuntu's sets the path). A folder holding `claude`
    /// or `pi` is left out, since a system profile may add one (macOS's `/etc/paths`).
    fn profile(&self) -> String {
        let quoted = |text: &OsStr| format!("'{}'", text.to_string_lossy().replace('\'', r"'\''"));
        let machine = std::env::var_os("PATH").map(|search| without_agent_clis(&search));
        let agents = self
            .agents
            .as_deref()
            .map(|folder| quoted(folder.as_os_str()));
        // A subshell, so that its options and variables are not the login shell's; the opening
        // bracket of the pattern keeps macOS's bash 3 from ending the subshell at its closing one.
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

    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub fn data_dir(&self) -> PathBuf {
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

    pub fn socket(&self) -> PathBuf {
        self.root.join("keeper.sock")
    }

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

    /// macOS's login `/bin/sh` closes inherited descriptors, hiding a leak from the Keeper; the
    /// world's script ignores `-l` and becomes an interactive `/bin/sh`.
    pub fn keep_descriptors_in_terminals(&mut self) {
        let script = self.root.join("keeping-shell");
        fs::write(&script, "#!/bin/sh\nexec /bin/sh -i\n").expect("write the script of the shell");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("make the shell runnable");
        self.shell = script;
    }

    pub fn start_keeper(&mut self) -> u32 {
        self.launch(&[])
    }

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
        // Kept before the assert below can panic, so the world's drop ends the Keeper in every
        // case.
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

    pub fn kill_everything(&mut self) {
        let keeper = self
            .keeper_pid()
            .expect("no Keeper of this world runs to kill everything under");
        // Frozen first, so what is started meanwhile is found: a process whose parent dies is no
        // longer found below it.
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
                let result = if path.is_dir() && !path.is_symlink() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                result.unwrap_or_else(|why| panic!("empty {}: {why}", path.display()));
            }
        }
    }

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
        // Started by its name: the stand-in reads its dialect from it.
        let mut command = self.command(&agents.join(name));
        command
            .env(SCRIPT_VARIABLE, script_file)
            .env(LOG_VARIABLE, self.agent_file(name, "log"))
            .current_dir(folder);
        match dialect {
            Dialect::ClaudeCode => {
                let plugin = self.data_dir().join("agents").join("claude").join("plugin");
                assert!(
                    plugin.join("hooks/hooks.json").exists(),
                    "the Keeper of this world wrote no plugin at {}: start it first",
                    plugin.display()
                );
                command.env("CLAUDE_CODE_PLUGIN_DIRS", plugin);
            }
            Dialect::Pi => {
                let extension = self.pi_extension();
                assert!(
                    extension.is_file(),
                    "the Keeper of this world wrote no extension of pi at {}: start it first",
                    extension.display()
                );
                self.list_in_pi_settings(&extension);
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
        // The program holds the slave end now; the world's copy would keep the terminal open for
        // ever.
        drop(slave);
        OutsideAgent {
            child,
            terminal: fs::File::from(master),
            output: Vec::new(),
            seen: 0,
        }
    }

    pub fn pi_settings(&self) -> PathBuf {
        self.pi_folder().join("settings.json")
    }

    pub fn install_pi(&self) {
        fs::create_dir_all(self.pi_folder()).expect("make the folder of pi's own files");
    }

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

    pub fn pi_extension(&self) -> PathBuf {
        self.data_dir()
            .join("agents")
            .join("pi")
            .join("extension")
            .join("yaar.ts")
    }

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

    pub fn kill_keeper(&mut self) {
        assert!(
            self.keeper.is_some(),
            "no Keeper of this world runs to kill"
        );
        assert!(self.end_keeper(), "the killed Keeper is still there");
    }

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
            None => {
                self.keeper = None;
                None
            }
        }
    }

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
        let _ = started.kill();
        let _ = started.wait();
        gone
    }

    /// A script that, given a file, sets its terminal raw and appends every byte it reads to it.
    /// The file appears once raw mode is on, so a test that waits for it knows that what it types
    /// from then on is recorded as typed.
    pub fn key_recorder(&self) -> PathBuf {
        let script = self.root.join("key-recorder");
        fs::write(&script, "#!/bin/sh\nstty raw -echo\nexec cat >> \"$1\"\n")
            .expect("write the script of the key recorder");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("make the key recorder runnable");
        script
    }

    /// Puts the stand-ins `claude`, `pi` and `otheragent` first on the path of what this world
    /// starts next; call it before [`World::start_keeper`]. The world's `.profile` puts them first
    /// again after a login shell's system profile.
    pub fn agents_on_path(&mut self) {
        self.agents_on_path_with(|search| search);
    }

    fn agents_on_path_with(&mut self, profile: impl FnOnce(String) -> String) {
        let folder = self.root.join("agents");
        fs::create_dir_all(&folder).expect("make the folder of the stand-ins");
        // A link to a copy that nothing replaces: a link to the file Cargo built would lead nowhere
        // while a build puts it back.
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

    pub fn agents_dir(&self) -> Option<PathBuf> {
        self.agents.clone()
    }

    pub fn agent_script(&self, name: &str, script: &crate::standin::Script) -> PathBuf {
        let path = self.agent_file(name, "script");
        fs::write(&path, script.text()).expect("write the script of the stand-in");
        path
    }

    pub fn agent_log(&self, name: &str) -> crate::standin::StandinLog {
        crate::standin::StandinLog::read(&self.agent_file(name, "log"))
    }

    fn agent_file(&self, name: &str, extension: &str) -> PathBuf {
        self.root.join(format!("{name}.{extension}"))
    }

    pub fn with_standin(name: &str, script: &crate::standin::Script) -> (World, Repo) {
        World::with_standin_as(Dialect::ClaudeCode, name, script)
    }

    pub fn with_standin_as(dialect: Dialect, name: &str, script: &Script) -> (World, Repo) {
        World::with_standin_where(dialect == Dialect::Pi, name, script)
    }

    /// Installs and allows pi whatever the dialect: a launcher that wrongly starts pi in a Claude
    /// Code run then fails at the sentence naming the CLI, not at a wait that runs out.
    pub fn with_standin_and_pi(name: &str, script: &Script) -> (World, Repo) {
        World::with_standin_where(true, name, script)
    }

    fn with_standin_where(pi: bool, name: &str, script: &Script) -> (World, Repo) {
        let mut world = World::new();
        world.agents_on_path();
        if pi {
            world.install_pi();
        }
        world.agent_script(name, script);
        let repo = world.repository("app");
        repo.git(&["config", "user.name", "A Test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        world.start_keeper();
        if pi {
            world.allow_pi_extension();
        }
        (world, repo)
    }

    pub fn client(&self) -> Client {
        Client::attach(&self.socket())
    }

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

pub struct OutsideAgent {
    child: std::process::Child,
    terminal: std::fs::File,
    output: Vec<u8>,
    seen: usize,
}

impl OutsideAgent {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn type_line(&mut self, text: &str) {
        self.terminal
            .write_all(format!("{text}\r").as_bytes())
            .expect("type into the terminal of the agent");
    }

    /// Each call waits for a prompt printed after the one the previous call found.
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

    fn read_some(&mut self, deadline: Instant) -> bool {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
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
                _ => return false,
            }
        }
    }
}

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

impl World {
    pub fn app_files_listed(&self) -> Vec<(PathBuf, String)> {
        let mut named = Vec::new();
        for worktree in self.worktrees() {
            for command in LISTINGS {
                let listing =
                    git_in(&self.root, &worktree, true, command).unwrap_or_else(|failed| {
                        panic!("git could not list the files of a worktree: {failed}")
                    });
                let is_status = command[0] == "status";
                named.extend(
                    listing
                        .lines()
                        .filter(|line| {
                            let path = if is_status {
                                path_of_status_line(line)
                            } else {
                                line
                            };
                            names_an_app_file(path)
                        })
                        .map(|line| (worktree.clone(), line.to_owned())),
                );
            }
        }
        named
    }

    fn worktrees(&self) -> Vec<PathBuf> {
        let mut repositories: Vec<_> = fs::read_dir(self.root.join("repos"))
            .into_iter()
            .flatten()
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|folder| folder.join(".git").exists())
            .collect();
        repositories.sort();
        let mut found: Vec<PathBuf> = Vec::new();
        for repository in repositories {
            let listed = git_in(
                &self.root,
                &repository,
                true,
                &["worktree", "list", "--porcelain"],
            )
            .unwrap_or_else(|failed| {
                panic!("git could not list the worktrees of a repository: {failed}")
            });
            for record in listed.split("\n\n") {
                let is_bare = record.lines().any(|line| line == "bare");
                let folder = record
                    .lines()
                    .find_map(|line| line.strip_prefix("worktree "))
                    .map(PathBuf::from);
                if let Some(folder) = folder
                    && !is_bare
                    && folder.exists()
                    && !found.contains(&folder)
                {
                    found.push(folder);
                }
            }
        }
        found
    }
}

const LISTINGS: [&[&str]; 3] = [
    &[
        "status",
        "--porcelain=v2",
        "--ignored",
        "--untracked-files=all",
    ],
    &["ls-files", "--cached", "--others", "--exclude-standard"],
    &["ls-files", "--others", "--ignored", "--exclude-standard"],
];

fn git_in(world: &Path, folder: &Path, read_only: bool, args: &[&str]) -> Result<String, String> {
    let mut git = Command::new("git");
    git.env_clear();
    if let Some(search) = std::env::var_os("PATH") {
        git.env("PATH", search);
    }
    if read_only {
        git.arg("--no-optional-locks");
    }
    let output = git
        .current_dir(folder)
        .args(args)
        .env("HOME", world.join("home"))
        .env("TMPDIR", world.join("tmp"))
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
    if !output.status.success() {
        return Err(format!(
            "git {args:?} in {} failed: {}{}",
            folder.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let printed = String::from_utf8_lossy(&output.stdout);
    Ok(printed.strip_suffix('\n').unwrap_or(&printed).to_owned())
}

fn path_of_status_line(line: &str) -> &str {
    let fields = match line.chars().next() {
        Some('1') => 8,
        Some('2') => 9,
        Some('u') => 10,
        _ => 1,
    };
    let mut rest = line;
    for _ in 0..fields {
        rest = rest.split_once(' ').map_or("", |(_, rest)| rest);
    }
    rest
}

/// Git quotes non-ASCII paths; a rename's old path follows a tab and is read too.
fn names_an_app_file(path: &str) -> bool {
    path.split('\t')
        .flat_map(|path| path.trim_matches('"').split('/'))
        .any(|part| {
            let part = part.trim_matches('"').to_lowercase();
            APP_FILE_NAMES.iter().any(|name| part == *name)
                || APP_FILE_STARTS.iter().any(|start| part.starts_with(start))
                || is_review_file(&part)
        })
}

fn is_review_file(name: &str) -> bool {
    let Some((id, number)) = name
        .strip_suffix(".md")
        .and_then(|stem| stem.split_once('-'))
    else {
        return false;
    };
    id.len() == 32
        && id.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
}

pub const APP_FILE_NAMES: [&str; 5] = [
    "ade-id",
    ".mcp.json",
    ".claude-plugin",
    "yaar.ts",
    "language-servers.toml",
];

pub const APP_FILE_STARTS: [&str; 2] = ["yaar-", ".yaar"];

impl Drop for World {
    fn drop(&mut self) {
        // With stand-ins on the path, app files are listed before the Keeper ends, so a file the
        // app removes when it dies is still seen; their panic waits until the folder is gone.
        // Skipped while panicking: a panic in a drop while unwinding aborts.
        let left = if self.agents.is_some() && !std::thread::panicking() {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.app_files_listed()))
        } else {
            Ok(Vec::new())
        };
        self.end_keeper();
        // A process the Keeper started (a commit's git after a kill) may still be writing into the
        // folder, so its removal is retried until the folder is gone.
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
        match left {
            Err(failed) => std::panic::resume_unwind(failed),
            Ok(left) if !left.is_empty() => {
                let lines: String = left
                    .iter()
                    .map(|(worktree, line)| format!("\n  {}: {line}", worktree.display()))
                    .collect();
                panic!("the test ended with files of the app in a worktree:{lines}");
            }
            Ok(_) => {}
        }
    }
}

pub struct Repo {
    root: PathBuf,
    world: PathBuf,
}

impl Repo {
    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn git(&self, args: &[&str]) -> String {
        git_in(&self.world, &self.root, false, args).unwrap_or_else(|failed| panic!("{failed}"))
    }

    pub fn write(&self, path: &str, content: &str) {
        let file = self.root.join(path);
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).expect("make the folders above the file");
        }
        fs::write(&file, content).expect("write the file");
    }

    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }
}

#[derive(Default)]
struct Seen {
    requests: Mutex<Vec<Request>>,
    stopped: AtomicBool,
    more: Condvar,
}

impl Seen {
    fn requests(&self) -> MutexGuard<'_, Vec<Request>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct Tap {
    socket: PathBuf,
    read: usize,
    seen: Arc<Seen>,
    stop: Arc<AtomicBool>,
    accepting: Option<JoinHandle<()>>,
}

impl Tap {
    pub fn socket(&self) -> PathBuf {
        self.socket.clone()
    }

    /// A request is listed before it is handed on, so one the Keeper answered is never missing.
    pub fn requests(&self) -> Vec<Request> {
        self.seen.requests().clone()
    }

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
        self.stop.store(true, Ordering::Relaxed);
        // A connection wakes the accepting thread out of `accept`, so it sees that it is to stop.
        let _ = UnixStream::connect(&self.socket);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

fn join(window: UnixStream, keeper: UnixStream, seen: &Arc<Seen>) -> io::Result<()> {
    let (mut from_window, mut to_keeper) = (window.try_clone()?, keeper.try_clone()?);
    let seen = Arc::clone(seen);
    std::thread::spawn(move || {
        let _ = pass_requests(&mut from_window, &mut to_keeper, &seen);
        let _ = to_keeper.shutdown(Shutdown::Write);
    });
    let (mut from_keeper, mut to_window) = (keeper, window);
    std::thread::spawn(move || {
        let _ = io::copy(&mut from_keeper, &mut to_window);
        let _ = to_window.shutdown(Shutdown::Write);
    });
    Ok(())
}

fn pass_requests(from: &mut UnixStream, to: &mut UnixStream, seen: &Seen) -> io::Result<()> {
    loop {
        let mut frame = vec![0u8; 4];
        match from.read(&mut frame[..1])? {
            0 => return Ok(()),
            _ => from.read_exact(&mut frame[1..])?,
        }
        let length = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
        if length > MAX_FRAME {
            to.write_all(&frame)?;
            io::copy(&mut Read::take(&mut *from, u64::from(length)), to)?;
            continue;
        }
        frame.resize(4 + length as usize, 0);
        from.read_exact(&mut frame[4..])?;
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

    #[test]
    fn a_path_names_a_file_of_the_app_by_a_part_of_it_case_aside() {
        for named in [
            "ade-id",
            "sub/ade-id",
            "a/b/.mcp.json",
            ".claude-plugin/plugin.json",
            "src/yaar.ts",
            "language-servers.toml",
            "yaar-note.txt",
            "docs/Yaar-Notes.md",
            ".yaar/state",
            ".YAAR",
            "dir/0123456789abcdef0123456789ABCDEF-12.md",
            "\"\\303\\251/.mcp.json\"",
            "\"yaar-\\303\\251.txt\"",
            "old name\tyaar-new",
        ] {
            assert!(names_an_app_file(named), "{named}");
        }
    }

    #[test]
    fn a_path_that_only_looks_like_a_file_of_the_app_names_none() {
        for fine in [
            "README.md",
            "ade-ids.txt",
            "the-yaar-way.txt",
            "src/yaar.tsx",
            "language-servers.toml.bak",
            "mcp.json",
            "0123456789abcdef0123456789abcde-1.md",
            "0123456789abcdef0123456789abcdef0-1.md",
            "0123456789abcdef0123456789abcdeg-1.md",
            "0123456789abcdef0123456789abcdef-x.md",
            "0123456789abcdef0123456789abcdef-.md",
            "0123456789abcdef0123456789abcdef-1.txt",
            "0123456789abcdef0123456789abcdef.md",
        ] {
            assert!(!names_an_app_file(fine), "{fine}");
        }
    }

    #[test]
    fn the_path_of_a_status_line_comes_after_its_fixed_fields() {
        let hash = "1".repeat(40);
        let changed = format!("1 .M N... 100644 100644 100644 {hash} {hash} a b/c.txt");
        let renamed =
            format!("2 R. N... 100644 100644 100644 {hash} {hash} R100 new name\told name");
        let unmerged = format!("u UU N... 100644 100644 100644 100644 {hash} {hash} {hash} x/y");
        assert_eq!(path_of_status_line(&changed), "a b/c.txt");
        assert_eq!(path_of_status_line(&renamed), "new name\told name");
        assert_eq!(path_of_status_line(&unmerged), "x/y");
        assert_eq!(path_of_status_line("? sub/ade-id"), "sub/ade-id");
        assert_eq!(path_of_status_line("! yaar-x"), "yaar-x");
    }
}
