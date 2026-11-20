//! Tests of the stand-in agent: the built `standin-agent`, run in a pseudo-terminal the
//! test opens, with a plugin folder of the test's own whose hooks write down what they
//! were given. No Keeper, no app: the stand-in knows only Claude Code's side of the plugin
//! contract.

use std::cell::RefCell;
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yaar_system_tests::standin::{
    Did, IGNORE_PIECES, INTERRUPT_MARK, LOG_VARIABLE, PROMPT, SCRIPT_VARIABLE, Script, StandinLog,
    Step,
};
use yaar_system_tests::{DEADLINE, Place, is_alive, program, wait_until};

/// The progress sequence of a turn that works, and the one that says no progress.
const WORKING: &[u8] = b"\x1b]9;4;3;\x07";
const NO_PROGRESS: &[u8] = b"\x1b]9;4;0;\x07";

/// The stand-in's own limit on a hook, 10 s, and a little room for starting and killing it.
const HOOK_LIMIT_AND_ROOM: Duration = Duration::from_secs(25);

/// The pseudo-terminal a stand-in runs in: the test is the terminal emulator, reading what
/// the program writes and typing what a user types.
struct Pty {
    /// Gone once the test closes the terminal.
    master: Option<OwnedFd>,
    /// Kept open, so that the settings of the terminal outlive the program.
    slave: OwnedFd,
    output: Vec<u8>,
}

impl Pty {
    fn open() -> Pty {
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
        assert_eq!(opened, 0, "openpty");
        for fd in [master, slave] {
            // The program must not inherit the master: the terminal would never close.
            // SAFETY: `fd` was just opened by `openpty`.
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
        }
        // SAFETY: both descriptors are open and owned by nobody else.
        unsafe {
            Pty {
                master: Some(OwnedFd::from_raw_fd(master)),
                slave: OwnedFd::from_raw_fd(slave),
                output: Vec::new(),
            }
        }
    }

    fn master(&self) -> i32 {
        self.master
            .as_ref()
            .expect("the terminal is open")
            .as_raw_fd()
    }

    /// Closes the terminal, as the app does when it ends the program's terminal.
    fn close(&mut self) {
        self.master = None;
    }

    /// Types these bytes, in one write.
    fn send(&self, bytes: &[u8]) {
        // SAFETY: the pointer and length are those of a live slice.
        let written = unsafe { libc::write(self.master(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize, "typing into the terminal");
    }

    /// Reads whatever the program has written so far.
    fn drain(&mut self) {
        loop {
            let mut poll = libc::pollfd {
                fd: self.master(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid `pollfd`.
            if unsafe { libc::poll(&mut poll, 1, 0) } <= 0 || poll.revents & libc::POLLIN == 0 {
                return;
            }
            let mut buffer = [0u8; 8192];
            // SAFETY: the buffer is live and its length is given.
            let n = unsafe { libc::read(self.master(), buffer.as_mut_ptr().cast(), buffer.len()) };
            if n <= 0 {
                return;
            }
            self.output.extend_from_slice(&buffer[..n as usize]);
        }
    }

    /// Reads one chunk of what the program has written, waiting for it in `poll` until
    /// `deadline` when there is none yet. Gives false when nothing came in time or the
    /// terminal is closed. The wait is the system's own: a loop that looks and sleeps hands
    /// over a chunk per sleep, which a slow machine stretches past any deadline.
    fn read_chunk(&mut self, deadline: Instant) -> bool {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            // Rounded up, so that a part of a millisecond left is not a look that waits not at all.
            let wait = i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);
            let mut poll = libc::pollfd {
                fd: self.master(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid `pollfd`.
            let ready = unsafe { libc::poll(&mut poll, 1, wait) };
            if ready < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            if ready <= 0 || poll.revents & libc::POLLIN == 0 {
                return false;
            }
            let mut buffer = [0u8; 8192];
            // SAFETY: the buffer is live and its length is given.
            let n = unsafe { libc::read(self.master(), buffer.as_mut_ptr().cast(), buffer.len()) };
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if n <= 0 {
                return false;
            }
            self.output.extend_from_slice(&buffer[..n as usize]);
            return true;
        }
    }

    /// Reads, one chunk at a time and no further than it has to, until `needle` is in the
    /// output after byte `from`, for at most [`DEADLINE`]; gives where it ends. What the
    /// program writes after that stays unread.
    fn read_to(&mut self, needle: &[u8], from: usize) -> Option<usize> {
        let deadline = Instant::now() + DEADLINE;
        // Only what came in since the last look is searched: the output may be a megabyte.
        let mut looked = from;
        loop {
            if let Some(at) = find(&self.output[looked..], needle) {
                return Some(looked + at + needle.len());
            }
            looked = self.output.len().saturating_sub(needle.len()).max(from);
            if Instant::now() >= deadline || !self.read_chunk(deadline) {
                return None;
            }
        }
    }

    /// Waits until `needle` is in the output after byte `from`; gives where it ends.
    fn wait_for(&mut self, needle: &[u8], from: usize) -> usize {
        let end = self.read_to(needle, from);
        self.drain();
        end.unwrap_or_else(|| {
            panic!(
                "never saw {:?} after byte {from}; the output was {:?}",
                String::from_utf8_lossy(needle),
                String::from_utf8_lossy(&self.output)
            )
        })
    }

    /// Reads until `needle` is in the output after byte `from`, and no further, so that
    /// what the program writes after that stays unread and it soon waits in a write again.
    fn read_until(&mut self, needle: &[u8], from: usize) {
        assert!(
            self.read_to(needle, from).is_some(),
            "never read as far as {:?}",
            String::from_utf8_lossy(needle)
        );
    }

    fn termios(&self) -> libc::termios {
        // SAFETY: an all-zero `termios` is a valid value to be filled in.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: the descriptor is open, and `termios` is a place for the settings.
        let got = unsafe { libc::tcgetattr(self.slave.as_raw_fd(), &mut termios) };
        assert_eq!(got, 0, "tcgetattr");
        termios
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// One group of a plugin's `hooks.json`.
struct Group {
    event: &'static str,
    matcher: Option<&'static str>,
    command: String,
}

fn group(event: &'static str, matcher: Option<&'static str>, command: String) -> Group {
    Group {
        event,
        matcher,
        command,
    }
}

/// A command that appends what it is given on its standard input, as a line, to `file`.
fn record(file: &Path) -> String {
    format!("cat >> '{0}'; echo >> '{0}'", file.display())
}

/// The folder of a place.
fn root_of(place: &Place) -> PathBuf {
    place.socket().parent().expect("a folder").to_owned()
}

/// The lines of a file of recorded hook inputs, each a JSON value; none for a file that is
/// not there.
fn events_in(file: &Path) -> Vec<Value> {
    fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("a hook input is JSON"))
        .collect()
}

/// One run of the stand-in: its folders, its terminal and its process.
struct Run {
    place: Place,
    work: PathBuf,
    log: PathBuf,
    pty: Pty,
    /// The settings of the terminal before any program ran in it.
    before: libc::termios,
    child: Child,
    gates: Vec<PathBuf>,
    sleepers: Vec<PathBuf>,
}

/// What a run is started with.
struct Start<'a> {
    place: Place,
    name: &'a str,
    script: Script,
    /// One plugin folder for each list of groups.
    plugins: Vec<Vec<Group>>,
    /// Folders named before the plugins in the list the stand-in is given.
    before: Vec<PathBuf>,
    args: Vec<String>,
    env: Vec<(&'a str, String)>,
    /// Whether the terminal is the program's controlling terminal, as it is in a shell of
    /// the app: closing it then sends the program a hangup.
    controlling: bool,
    /// Gates (FIFOs) hooks wait on, and files naming processes hooks start: ended when
    /// the test fails.
    gates: Vec<PathBuf>,
    sleepers: Vec<PathBuf>,
    /// Whether a plugin folder with a tool server of the test's own (`TOOL_SERVER`) is
    /// among the plugins.
    server: bool,
}

impl<'a> Start<'a> {
    fn new(place: Place, name: &'a str, script: Script) -> Self {
        Start {
            place,
            name,
            script,
            plugins: Vec::new(),
            before: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
            controlling: false,
            gates: Vec::new(),
            sleepers: Vec::new(),
            server: false,
        }
    }

    /// A last plugin folder whose `.mcp.json` names a tool server of the test's own.
    fn with_tool_server(mut self) -> Self {
        self.server = true;
        self
    }

    fn plugin(mut self, groups: Vec<Group>) -> Self {
        self.plugins.push(groups);
        self
    }

    /// Plugin hooks for these events, each recording its input into `file`.
    fn recording(self, events: &[&'static str], file: &Path) -> Self {
        self.plugin(
            events
                .iter()
                .map(|event| group(event, None, record(file)))
                .collect(),
        )
    }

    /// The terminal is the program's controlling terminal.
    fn controlling(mut self) -> Self {
        self.controlling = true;
        self
    }

    /// A gate a hook waits on, to be opened when the test fails.
    fn gate(mut self, at: &Path) -> Self {
        self.gates.push(at.to_owned());
        self
    }

    /// A file a hook writes a process id in, to be ended when the test fails.
    fn sleeper(mut self, at: &Path) -> Self {
        self.sleepers.push(at.to_owned());
        self
    }

    fn start(self) -> Run {
        let root = root_of(&self.place);
        let work = root.join("work");
        fs::create_dir_all(&work).expect("create the worktree");
        // The stand-in reports the folder it was started in as the system names it.
        let work = work.canonicalize().expect("resolve the worktree");
        let script = root.join("script");
        fs::write(&script, self.script.text()).expect("write the script");
        let log = root.join("standin.log");
        let mut dirs: Vec<String> = self
            .before
            .iter()
            .map(|folder| folder.display().to_string())
            .collect();
        for (n, groups) in self.plugins.iter().enumerate() {
            let folder = root.join(format!("plugin-{n}"));
            fs::create_dir_all(folder.join("hooks")).expect("create the plugin");
            let mut hooks = serde_json::Map::new();
            for group in groups {
                let mut entry = json!({"hooks": [{"type": "command", "command": group.command}]});
                if let Some(matcher) = group.matcher {
                    entry["matcher"] = json!(matcher);
                }
                hooks
                    .entry(group.event)
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .expect("a list")
                    .push(entry);
            }
            fs::write(
                folder.join("hooks/hooks.json"),
                json!({"hooks": hooks}).to_string(),
            )
            .expect("write the hooks");
            dirs.push(folder.display().to_string());
        }
        if self.server {
            let folder = root.join("plugin-server");
            fs::create_dir_all(&folder).expect("create the plugin of the tool server");
            let script = folder.join("server.sh");
            fs::write(&script, tool_server_script(&root)).expect("write the tool server");
            let server = json!({"mcpServers": {"yaar": {
                "command": "/bin/sh",
                "args": [script],
            }}});
            fs::write(folder.join(".mcp.json"), server.to_string()).expect("write .mcp.json");
            dirs.push(folder.display().to_string());
        }
        let pty = Pty::open();
        // Read here, not after the start: the program sets the terminal raw as it begins, and
        // a read made later may come after that.
        let before = pty.termios();
        let stdio = |pty: &Pty| Stdio::from(pty.slave.try_clone().expect("duplicate the slave"));
        // A copy of the program, not the file Cargo built: Cargo puts that file in its place
        // again at every build, and tests that start their own builds run beside this one.
        let mut command = self.place.command(&program("standin-agent"));
        // The program is told its name the way a shell tells it: by what it was typed as.
        command
            .arg0(self.name)
            .args(&self.args)
            .current_dir(&work)
            .env(SCRIPT_VARIABLE, &script)
            .env(LOG_VARIABLE, &log)
            .env("CLAUDE_CODE_PLUGIN_DIRS", dirs.join(":"))
            .stdin(stdio(&pty))
            .stdout(stdio(&pty))
            .stderr(stdio(&pty));
        for (name, value) in &self.env {
            command.env(name, value);
        }
        if self.controlling {
            // SAFETY: only async-signal-safe calls between the fork and the exec. The
            // standard input is already the terminal by then.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = command.spawn().expect("start the stand-in");
        drop(command);
        Run {
            place: self.place,
            work,
            log,
            pty,
            before,
            child,
            gates: self.gates,
            sleepers: self.sleepers,
        }
    }
}

impl Run {
    fn home(&self) -> PathBuf {
        root_of(&self.place).join("home")
    }

    fn log(&self) -> StandinLog {
        StandinLog::read(&self.log)
    }

    /// Waits until the log satisfies `done`, and gives it.
    fn wait_log(&self, what: &str, done: impl Fn(&StandinLog) -> bool) -> StandinLog {
        let mut last = StandinLog::default();
        let seen = wait_until(DEADLINE, || {
            last = self.log();
            done(&last)
        });
        assert!(seen, "the log never showed {what}: {last:?}");
        last
    }

    /// Waits for the prompt after byte `from` of the output, then types the line. Gives
    /// where the prompt ended.
    fn say(&mut self, from: usize, line: &str) -> usize {
        let at = self.pty.wait_for(PROMPT.as_bytes(), from);
        self.pty.send(format!("{line}\r").as_bytes());
        at
    }

    /// Waits for the program to end.
    fn wait_exit(&mut self) -> ExitStatus {
        let mut status = None;
        let ended = wait_until(DEADLINE, || {
            status = self.child.try_wait().expect("poll the stand-in");
            status.is_some()
        });
        assert!(ended, "the stand-in did not end: {:?}", self.log());
        status.expect("ended")
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // A test that failed leaves the program running, and the hooks it started: end
        // them and collect the program.
        if std::thread::panicking() {
            free_hooks(&self.gates, &self.sleepers);
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn names(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["hook_event_name"].as_str().expect("an event name"))
        .collect()
}

/// The log's list of what it did, as short words, to compare in one assertion.
fn story(log: &StandinLog) -> Vec<String> {
    log.did
        .iter()
        .map(|did| match did {
            Did::Began { step, received } => format!("began {step} at {received}"),
            Did::Done { step, received } => format!("done {step} at {received}"),
            Did::Hook { event, killed, .. } => {
                format!("hook {event}{}", if *killed { " killed" } else { "" })
            }
            Did::Interrupted { step, received } => format!("interrupted {step} at {received}"),
            Did::Flooded { step, .. } => format!("flooded {step}"),
            Did::ScriptDone { received } => format!("script done at {received}"),
            Did::Exited { status } => format!("exited {status}"),
            Did::Tool { received, .. } => format!("tool at {received}"),
        })
        .collect()
}

fn path(text: &str) -> PathBuf {
    PathBuf::from(text)
}

fn make_fifo(at: &Path) {
    let name = CString::new(at.to_str().expect("a path")).expect("no NUL");
    // SAFETY: a valid C string.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0, "mkfifo");
}

/// The command of a hook that has the gate open, says it is there by making `started`, and
/// waits for a line on the gate. The hook opens the gate for reading and writing, so that
/// its open never waits for the test: a FIFO opened for reading alone waits for a writer,
/// and a writer that comes and goes before the reader has looked loses it, which left the
/// hook waiting for ever on a loaded machine.
fn gated(started: &Path, gate: &Path) -> String {
    format!(
        "exec 3<> '{}'; : > '{}'; read line <&3",
        gate.display(),
        started.display()
    )
}

/// Lets the hook that waits on a gate, a FIFO the test made, end: writes the line it waits
/// for, once the hook has the gate open (the test has seen its `started` file by then). A
/// run the gate is named to ([`Start::gate`]) lets the hook end by itself when the test
/// failed, so that no hook is left behind.
fn open_gate(at: &Path) {
    let mut gate = fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(at)
        .expect("open the gate: the hook has it open already");
    gate.write_all(b"\n").expect("write to the gate");
}

/// Ends, for a test that failed, what a hook started: the hook waiting at a gate, and the
/// process a file names.
fn free_hooks(gates: &[PathBuf], sleepers: &[PathBuf]) {
    for gate in gates {
        // Opening without waiting fails until the hook has the gate open: try until it does.
        let _ = wait_until(Duration::from_secs(3), || {
            fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(gate)
                .and_then(|mut gate| gate.write_all(b"\n"))
                .is_ok()
        });
    }
    for file in sleepers {
        let pid = fs::read_to_string(file)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        if let Some(pid) = pid.filter(|pid| is_alive(*pid)) {
            // SAFETY: a process the test's own hook started and that is still running.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

/// The tool server of the tests of a reply: a `sh` script that answers the messages of MCP
/// one line at a time, as the app's own server does, and writes down what it was started
/// with and every line it read (`server.started`, `server.read` of the run's folder). A call
/// whose line holds `REFUSE` is answered with an error result.
fn tool_server_script(root: &Path) -> String {
    format!(
        r#"echo "$$ $PPID $CLAUDE_CODE_SESSION_ID $(pwd -P)" >> '{started}'
n=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> '{read}'
  id=$(printf '%s' "$line" | sed -n 's/^{{[^{{}}]*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"2025-11-25","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"yaar","version":"0"}}}}}}\n' "$id" ;;
    *'"method":"tools/list"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[]}}}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      n=$((n + 1))
      case "$line" in
        *REFUSE*)
          printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"text","text":"refused: no such thread"}}],"isError":true}}}}\n' "$id" ;;
        *)
          printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"text","text":"recorded %s"}}]}}}}\n' "$id" "$n" ;;
      esac ;;
  esac
done
"#,
        started = root.join("server.started").display(),
        read = root.join("server.read").display(),
    )
}

/// The review a canned hook answers with: an instruction, then a heading and a comment for
/// each thread. The words `Thread` and `##` also stand in the instruction, away from the
/// start of a line, as in the app's own, and each thread's code, fenced as the engine fences
/// it, holds a line that looks like the next heading.
fn review_of(ids: &[&str]) -> String {
    let mut text = String::from(
        "This is a review. For each thread, reply to the id written after \"Thread\" \
         in its heading, as in ## Thread x.\n\n",
    );
    for id in ids {
        text.push_str(&format!(
            "## Thread {id}\n\nsrc/lib.rs, line 3\n\n```\n## Thread in-code-of-{id}\n```\n\nComment:\nwhy this?\n\n"
        ));
    }
    text
}

/// A hook command that answers `UserPromptSubmit` as the app's does when a review rides with
/// the message: with `additionalContext` and the line shown under the prompt; for a prompt
/// holding `only_for` when one is given, with nothing for any other.
fn review_hook(review: &str, only_for: Option<&str>) -> String {
    let answer = json!({
        "hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": review},
        "systemMessage": "yaar: review round 1 attached",
    })
    .to_string()
    .replace('\'', r"'\''");
    match only_for {
        None => format!("cat > /dev/null; printf '%s' '{answer}'"),
        Some(words) => format!("case \"$(cat)\" in *'{words}'*) printf '%s' '{answer}' ;; esac"),
    }
}

/// The lines the tool server read, each a JSON message.
fn server_read(root: &Path) -> Vec<Value> {
    events_in(&root.join("server.read"))
}

/// The tools/call messages among what the tool server read.
fn calls(read: &[Value]) -> Vec<&Value> {
    read.iter()
        .filter(|message| message["method"] == "tools/call")
        .collect()
}

/// The reply tool as Claude Code lists it to the agent.
const LISTED_REPLY_TOOL: &str = "mcp__plugin_yaar-review_yaar__reply_to_thread";

// The script's text.

#[test]
fn a_script_comes_back_from_its_text_as_it_was() {
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Edit(path("dir/b.txt"), "two words\nsecond line".into()),
        Step::Write(path("c.txt"), "back\\slash and \\n kept".into()),
        Step::Write(path("empty.txt"), String::new()),
        Step::Shell("echo 'x'; echo \"y\"\nexit 1".into()),
        Step::Ask(path("d.txt"), " leading and trailing  ".into()),
        Step::Hold(path("a folder/go")),
        Step::Flood(7),
        Step::End,
        Step::Mark,
        Step::Exit,
    ]);
    assert_eq!(Script::parse(&script.text()), script);
    // The file is one keyword and its words to a line, a line end inside a text written
    // as `\n`.
    let text = script.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 12);
    assert_eq!(lines[0], "turn");
    assert_eq!(lines[2], "edit dir/b.txt two words\\nsecond line");
    assert_eq!(lines[5], r#"shell echo 'x'; echo "y"\nexit 1"#);
    assert_eq!(lines[8], "flood 7");
}

#[test]
fn a_text_with_a_carriage_return_comes_back_from_the_script_as_it_was() {
    let script = Script(vec![
        Step::Write(path("a.txt"), "ends in one\r".into()),
        Step::Shell("printf 'x\\r\\n'\r\r".into()),
        Step::Edit(path("b.txt"), "\r\nand\\r".into()),
    ]);
    let text = script.text();
    assert_eq!(text.lines().count(), 3, "a carriage return made a line");
    assert!(!text.contains('\r'), "a carriage return is written raw");
    assert_eq!(Script::parse(&text), script);
}

#[test]
fn a_script_typed_by_hand_is_read_with_its_blank_lines_left_out() {
    let script = Script::parse("turn\n\nedit p.txt hello there\nend\nexit\n");
    assert_eq!(
        script,
        Script(vec![
            Step::Turn,
            Step::Edit(path("p.txt"), "hello there".into()),
            Step::End,
            Step::Exit,
        ])
    );
}

#[test]
#[should_panic(expected = "line 3")]
fn a_script_line_that_cannot_be_read_is_named_by_its_number() {
    let _ = Script::parse("turn\nend\nfrobnicate now\n");
}

#[test]
#[should_panic(expected = "line 2")]
fn a_flood_of_no_number_is_named_by_its_line() {
    let _ = Script::parse("turn\nflood many\n");
}

// A scripted turn.

#[test]
fn a_scripted_turn_gives_the_hooks_of_claude_code_in_its_order_with_their_inputs() {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Edit(path("a.txt"), "three".into()),
        Step::Write(path("new.txt"), "hello\nworld".into()),
        Step::Write(path("a.txt"), "replaced".into()),
        Step::Shell("echo made > made.txt".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "Stop",
                "SessionEnd",
            ],
            &events,
        )
        .start();
    fs::write(run.work.join("a.txt"), "one\ntwo\n").expect("seed a file");

    let from = run.say(0, "do it");
    assert!(run.wait_exit().success());
    let all = events_in(&events);

    let log = run.log();
    let session = log.session.clone();
    assert_eq!(session.len(), 36, "a UUID-shaped session id");
    let work = run.work.display().to_string();
    let file = |name: &str| format!("{work}/{name}");
    let with = |event: &str, extra: Value| {
        let mut input = json!({"session_id": session, "hook_event_name": event, "cwd": work});
        for (key, value) in extra.as_object().expect("an object") {
            input[key] = value.clone();
        }
        input
    };
    let edit_input = json!({
        "file_path": file("a.txt"), "old_string": "two", "new_string": "two\nthree"
    });
    let write_input = json!({"file_path": file("new.txt"), "content": "hello\nworld"});
    let replace_input = json!({"file_path": file("a.txt"), "content": "replaced"});
    let bash_input = json!({"command": "echo made > made.txt"});
    let read_input = json!({"file_path": file("a.txt")});
    let expected = vec![
        with("SessionStart", json!({"source": "startup"})),
        with("UserPromptSubmit", json!({"prompt": "do it"})),
        with(
            "PreToolUse",
            json!({"tool_name": "Read", "tool_input": read_input}),
        ),
        with(
            "PostToolUse",
            json!({"tool_name": "Read", "tool_input": read_input, "tool_response": {}}),
        ),
        with(
            "PreToolUse",
            json!({"tool_name": "Edit", "tool_input": edit_input}),
        ),
        with(
            "PostToolUse",
            json!({"tool_name": "Edit", "tool_input": edit_input, "tool_response": {
                "filePath": file("a.txt"),
                "structuredPatch": [{
                    "oldStart": 2, "oldLines": 0, "newStart": 3, "newLines": 1,
                    "lines": ["+three"]
                }]
            }}),
        ),
        with(
            "PreToolUse",
            json!({"tool_name": "Write", "tool_input": write_input}),
        ),
        with(
            "PostToolUse",
            json!({"tool_name": "Write", "tool_input": write_input, "tool_response": {
                "filePath": file("new.txt"), "type": "create", "structuredPatch": []
            }}),
        ),
        with(
            "PreToolUse",
            json!({"tool_name": "Write", "tool_input": replace_input}),
        ),
        with(
            "PostToolUse",
            json!({"tool_name": "Write", "tool_input": replace_input, "tool_response": {
                "filePath": file("a.txt"), "type": "update", "structuredPatch": []
            }}),
        ),
        with(
            "PreToolUse",
            json!({"tool_name": "Bash", "tool_input": bash_input}),
        ),
        with(
            "PostToolUse",
            json!({"tool_name": "Bash", "tool_input": bash_input, "tool_response": {}}),
        ),
        with("Stop", json!({"stop_hook_active": false})),
        with("SessionEnd", json!({"reason": "prompt_input_exit"})),
    ];
    // The transcript path lies in the world's home, is named for the session and is never
    // written.
    let home = run.home().display().to_string();
    let without_transcript: Vec<Value> = all
        .iter()
        .map(|event| {
            let mut event = event.clone();
            let at = event
                .as_object_mut()
                .expect("an object")
                .remove("transcript_path")
                .expect("a transcript path");
            let at = at.as_str().expect("a path");
            assert!(
                at.starts_with(&home) && at.ends_with(&format!("{session}.jsonl")),
                "{at}"
            );
            assert!(!Path::new(at).exists());
            event
        })
        .collect();
    assert_eq!(without_transcript, expected);

    // The tools really acted.
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "replaced"
    );
    assert_eq!(
        fs::read_to_string(run.work.join("new.txt")).unwrap(),
        "hello\nworld"
    );
    assert_eq!(
        fs::read_to_string(run.work.join("made.txt")).unwrap(),
        "made\n"
    );

    // What it printed, in order, each line starting at the left of the terminal.
    run.pty.drain();
    let output = run.pty.output.clone();
    let mut at = from;
    for part in [
        &b"\r\n"[..],
        WORKING,
        "● Read a.txt\r\n".as_bytes(),
        "● Edit a.txt\r\n".as_bytes(),
        "● Write new.txt\r\n".as_bytes(),
        "● Write a.txt\r\n".as_bytes(),
        "● Bash echo made > made.txt\r\n".as_bytes(),
        "● Done\r\n".as_bytes(),
        NO_PROGRESS,
    ] {
        let found = find(&output[at..], part).unwrap_or_else(|| {
            panic!(
                "{:?} missing in {:?}",
                String::from_utf8_lossy(part),
                String::from_utf8_lossy(&output[at..])
            )
        });
        at += found + part.len();
    }

    // What it wrote down.
    assert_eq!(log.pid, run.child.id());
    assert!(log.args.is_empty());
    assert_eq!(log.received, b"do it\r");
    let mut wanted: Vec<String> = [
        "hook SessionStart",
        "began 0 at 0",
        "hook UserPromptSubmit",
        "done 0 at 6",
    ]
    .map(str::to_owned)
    .into();
    for step in 1..=5 {
        wanted.extend([
            format!("began {step} at 6"),
            "hook PreToolUse".to_owned(),
            "hook PostToolUse".to_owned(),
            format!("done {step} at 6"),
        ]);
    }
    wanted.extend(
        [
            "began 6 at 6",
            "hook Stop",
            "done 6 at 6",
            "began 7 at 6",
            "hook SessionEnd",
            "done 7 at 6",
            "exited 0",
        ]
        .map(str::to_owned),
    );
    assert_eq!(story(&log), wanted);
}

#[test]
fn a_shell_command_that_fails_gives_post_tool_use_failure() {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let script = Script(vec![
        Step::Turn,
        Step::Shell("touch ran.txt; exit 3".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(
            &["PreToolUse", "PostToolUse", "PostToolUseFailure"],
            &events,
        )
        .start();
    run.say(0, "go");
    assert!(run.wait_exit().success());
    let seen = events_in(&events);
    assert_eq!(names(&seen), ["PreToolUse", "PostToolUseFailure"]);
    assert_eq!(seen[1]["tool_name"], "Bash");
    assert_eq!(
        seen[1]["tool_input"],
        json!({"command": "touch ran.txt; exit 3"})
    );
    assert_eq!(seen[1]["tool_response"], json!({}));
    assert_eq!(seen[1]["error"], "Exit code 3");
    assert!(
        run.work.join("ran.txt").exists(),
        "the command ran in the worktree"
    );
}

#[test]
fn a_group_with_a_matcher_runs_for_its_tools_and_every_plugin_folder_runs_in_order() {
    let place = Place::new();
    let root = root_of(&place);
    let (all, edits, longer, stops, order) = (
        root.join("all"),
        root.join("edits"),
        root.join("longer"),
        root.join("stops"),
        root.join("order"),
    );
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Edit(path("a.txt"), "x".into()),
        Step::Write(path("b.txt"), "y".into()),
        Step::Shell("true".into()),
        Step::End,
        Step::Exit,
    ]);
    let empty = root.join("empty-plugin");
    fs::create_dir_all(empty.join(".claude-plugin")).expect("a plugin with no hooks");
    let mut start = Start::new(place, "claude", script)
        .plugin(vec![
            group("PreToolUse", None, record(&all)),
            group("PreToolUse", Some("Edit|Write"), record(&edits)),
            // A matcher is the tool's name or one of its alternatives, not a text that
            // holds the name.
            group("PreToolUse", Some("Reader|Writer|Bashful"), record(&longer)),
            // An event that has no tool has nothing for a matcher to match: it runs.
            group("Stop", Some("Bash"), record(&stops)),
            group("SessionStart", None, "echo hello-answer".into()),
            group(
                "UserPromptSubmit",
                None,
                format!("echo one >> '{}'", order.display()),
            ),
        ])
        .plugin(vec![group(
            "UserPromptSubmit",
            None,
            format!("echo two >> '{}'", order.display()),
        )]);
    // A folder that holds no hooks and one that is not there are no fault.
    start.before = vec![root.join("not-there"), empty];
    let mut run = start.start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    run.say(0, "go");
    assert!(run.wait_exit().success());
    let tool = |file: &Path| -> Vec<String> {
        events_in(file)
            .iter()
            .map(|event| event["tool_name"].as_str().unwrap_or("-").to_owned())
            .collect()
    };
    assert_eq!(tool(&all), ["Read", "Edit", "Write", "Bash"]);
    assert_eq!(tool(&edits), ["Edit", "Write"]);
    assert!(
        !longer.exists(),
        "a matcher that only holds a tool's name ran"
    );
    assert_eq!(tool(&stops), ["-"]);
    assert_eq!(fs::read_to_string(&order).expect("both ran"), "one\ntwo\n");
    let answer = run.log().did.iter().find_map(|did| match did {
        Did::Hook { event, answer, .. } if event == "SessionStart" => Some(answer.clone()),
        _ => None,
    });
    assert_eq!(answer, Some(b"hello-answer\n".to_vec()));
    // The answer is kept, not shown: the program ended, so what it wrote is all here.
    run.pty.drain();
    assert!(
        find(&run.pty.output, b"hello-answer").is_none(),
        "a hook's answer reached the terminal: {:?}",
        String::from_utf8_lossy(&run.pty.output)
    );
}

#[test]
fn a_hooks_parent_is_the_stand_in_and_it_runs_in_the_folder_the_stand_in_started_in() {
    let place = Place::new();
    let root = root_of(&place);
    let out = root.join("who");
    // A lone `exec`: `sh -c` runs the program in its own place, whichever `sh` it is, as it
    // does for the app's hook program, whose parent has to be the agent.
    let hook = root.join("who.sh");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\ncat > /dev/null\necho \"$PPID $CLAUDE_PID $CLAUDECODE $CLAUDE_PROJECT_DIR $(pwd -P) $YAAR_TERMINAL\" >> '{}'\n",
            out.display()
        ),
    )
    .expect("write the hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("make it run");
    let command = format!("exec '{}'", hook.display());
    let mut start = Start::new(
        place,
        "claude",
        Script(vec![Step::Turn, Step::End, Step::Exit]),
    )
    .plugin(vec![
        group("SessionStart", None, command.clone()),
        group("Stop", None, command),
    ]);
    // What the shell of the app gives the agent reaches its hooks: they run with the
    // agent's own environment.
    start.env.push(("YAAR_TERMINAL", "probe-7".to_owned()));
    let mut run = start.start();
    run.say(0, "go");
    assert!(run.wait_exit().success());
    let pid = run.log().pid.to_string();
    let work = run.work.display().to_string();
    let lines: Vec<String> = fs::read_to_string(&out)
        .expect("the hook wrote")
        .lines()
        .map(str::to_owned)
        .collect();
    let expected = format!("{pid} {pid} 1 {work} {work} probe-7");
    assert_eq!(lines, [expected.clone(), expected]);
}

#[test]
fn a_hook_and_a_shell_command_do_not_ignore_a_hangup_the_stand_in_ignores() {
    let place = Place::new();
    let root = root_of(&place);
    let (from_hook, from_shell) = (root.join("from-hook"), root.join("from-shell"));
    // A program that ignores a hangup passes that on to what it starts, but Claude Code's
    // hooks and commands do not ignore it: each one hangs itself up and must not go on.
    let hook = format!("kill -HUP $$; echo alive >> '{}'", from_hook.display());
    let shell = format!("kill -HUP $$; echo alive >> '{}'", from_shell.display());
    let script = Script(vec![Step::Turn, Step::Shell(shell), Step::End, Step::Exit]);
    let mut run = Start::new(place, "claude", script)
        .plugin(vec![group("UserPromptSubmit", None, hook)])
        .start();
    run.say(0, "go");
    assert!(run.wait_exit().success());
    assert!(!from_hook.exists(), "the hook ignored a hangup");
    assert!(!from_shell.exists(), "the command ignored a hangup");
}

// The keys.

#[test]
fn esc_during_a_hold_prints_the_mark_and_runs_no_hook_and_the_script_goes_on_after_end() {
    let place = Place::new();
    let root = root_of(&place);
    let events = root.join("events");
    let script = Script(vec![
        Step::Turn,
        Step::Hold(root.join("never")),
        Step::Read(path("a.txt")),
        Step::End,
        Step::Turn,
        Step::Edit(path("a.txt"), "after".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
                "SessionEnd",
            ],
            &events,
        )
        .start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    let at = run.say(0, "first");
    let held = run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"\x1b");
    let marked = run.pty.wait_for(INTERRUPT_MARK, held);
    run.say(marked, "second");
    assert!(run.wait_exit().success());

    // The first turn gave a prompt submission and then nothing: no read, no stop.
    let seen = events_in(&events);
    assert_eq!(
        names(&seen),
        [
            "SessionStart",
            "UserPromptSubmit",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "SessionEnd"
        ]
    );
    assert_eq!(seen[1]["prompt"], "first");
    assert_eq!(seen[2]["prompt"], "second");
    let story = story(&run.log());
    let interrupted = story
        .iter()
        .position(|did| did.starts_with("interrupted"))
        .expect("the interrupt is written down");
    assert_eq!(story[interrupted], "interrupted 1 at 7");
    // Nothing between the interrupt and the second turn: not a hook, and not the steps
    // that were skipped, `read` and the `end` of the first turn.
    assert_eq!(story[interrupted + 1], "began 4 at 7");
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "seed\nafter\n"
    );
}

#[test]
fn esc_read_between_the_steps_of_a_turn_ends_it_with_the_mark_and_the_script_goes_on_after_end() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, started, gate) = (root.join("events"), root.join("started"), root.join("gate"));
    make_fifo(&gate);
    // The first read tool waits at its gate, and the key is typed meanwhile: the program
    // reads it when that step is over, between two steps.
    let block = gated(&started, &gate);
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Edit(path("a.txt"), "never".into()),
        Step::End,
        Step::Turn,
        Step::Edit(path("a.txt"), "after".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(
            &[
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
                "SessionEnd",
            ],
            &events,
        )
        .plugin(vec![group("PreToolUse", Some("Read"), block)])
        .gate(&gate)
        .start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    let at = run.say(0, "first");
    assert!(
        wait_until(DEADLINE, || started.exists()),
        "the read tool never began"
    );
    run.pty.send(b"\x1b");
    open_gate(&gate);
    let marked = run.pty.wait_for(INTERRUPT_MARK, at);
    run.say(marked, "second");
    assert!(run.wait_exit().success());

    // The tool that was running ended as it would; then nothing: no edit, no stop.
    let seen = events_in(&events);
    assert_eq!(
        names(&seen),
        [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "SessionEnd"
        ]
    );
    assert_eq!(seen[3]["prompt"], "second");
    assert_eq!(seen[1]["tool_name"], "Read");
    assert_eq!(seen[4]["tool_name"], "Edit");
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "seed\nafter\n"
    );
    let story = story(&run.log());
    let interrupted = story
        .iter()
        .position(|did| did.starts_with("interrupted"))
        .expect("the interrupt is written down");
    assert_eq!(story[interrupted], "interrupted 2 at 7");
    assert_eq!(story[interrupted + 1], "began 4 at 7");
}

#[test]
fn a_hold_waits_for_its_file_and_the_turn_goes_on_when_it_is_there() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::Read(path("a.txt")),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["PreToolUse"], &events)
        .start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    // The program reads keys while it holds, so a byte it reads after the banner is a
    // barrier: a hold that did not hold would be past the read tool by then.
    run.pty.send(b"k");
    run.wait_log("the probe byte", |log| log.received.ends_with(b"k"));
    assert!(
        !events.exists(),
        "the turn went on before the file was there"
    );
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    assert_eq!(names(&events_in(&events)), ["PreToolUse"]);
}

#[test]
fn end_runs_the_stop_hooks_and_waits_for_them_before_it_prints_the_mark() {
    let place = Place::new();
    let root = root_of(&place);
    let (started, done) = (root.join("started"), root.join("done"));
    let gate = root.join("gate");
    make_fifo(&gate);
    let hook = format!("{}; : > '{}'", gated(&started, &gate), done.display());
    let mut run = Start::new(
        place,
        "claude",
        Script(vec![Step::Turn, Step::End, Step::Exit]),
    )
    .plugin(vec![group("Stop", None, hook)])
    .gate(&gate)
    .start();
    run.say(0, "go");
    assert!(
        wait_until(DEADLINE, || started.exists()),
        "the Stop hook never ran"
    );
    // The hook is running and holds the program: what was printed before it is here, and
    // the mark of the end of the turn, which comes after it, is not.
    run.pty.drain();
    assert!(find(&run.pty.output, "● Done\r\n".as_bytes()).is_some());
    assert!(find(&run.pty.output, NO_PROGRESS).is_none());
    assert!(!done.exists());
    // Opening the gate lets the hook end.
    open_gate(&gate);
    run.pty.wait_for(NO_PROGRESS, 0);
    assert!(
        done.exists(),
        "the mark was printed before the Stop hook ended"
    );
    assert!(run.wait_exit().success());
}

/// The prompts the `UserPromptSubmit` hooks were given, in order.
fn prompts_in(events: &Path) -> Vec<String> {
    events_in(events)
        .iter()
        .filter(|event| event["hook_event_name"] == "UserPromptSubmit")
        .map(|event| event["prompt"].as_str().expect("a prompt").to_owned())
        .collect()
}

/// How many times `needle` is in `haystack`.
fn count_of(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

#[test]
fn a_line_typed_during_a_hold_is_submitted_at_once_as_claude_code_queues_it() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    // No second `turn`: the queued message starts none, so a stand-in that held it for
    // one would wait at it for ever.
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"second\r");
    // The hook runs for the line while the hold goes on: a repeated look at a fact that
    // only becomes true.
    let log = run.wait_log("the hook run for the queued line", |log| {
        log.did
            .iter()
            .filter(|did| matches!(did, Did::Hook { event, .. } if event == "UserPromptSubmit"))
            .count()
            == 2
    });
    assert!(!go.exists());
    assert_eq!(log.received, b"first\rsecond\r");
    assert!(
        log.did
            .iter()
            .all(|did| !matches!(did, Did::Done { step: 1, .. })),
        "the hold was over before the queued line was submitted: {:?}",
        story(&log)
    );
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    assert_eq!(prompts_in(&events), ["first", "second"]);
    // One turn: the progress mark of a turn that works was printed once, and so was the
    // prompt; the queued line was neither asked for nor echoed.
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output, PROMPT.as_bytes()), 1);
    assert_eq!(count_of(&run.pty.output, WORKING), 1);
    assert_eq!(count_of(&run.pty.output, b"second"), 0);
}

#[test]
fn a_line_typed_between_the_steps_of_a_turn_is_held_for_the_next_turn() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let (started, gate) = (root.join("started"), root.join("gate"));
    make_fifo(&gate);
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Hold(go.clone()),
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .plugin(vec![group(
            "PreToolUse",
            Some("Read"),
            gated(&started, &gate),
        )])
        .gate(&gate)
        .start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    let at = run.say(0, "first");
    assert!(
        wait_until(DEADLINE, || started.exists()),
        "the hook never ran"
    );
    // Typed while the read tool's hook runs: no hold is open, so it is no queued message.
    run.pty.send(b"second\r");
    open_gate(&gate);
    run.pty.wait_for("● Hold ".as_bytes(), at);
    // The line was read before the hold began, and the hold is not given what came
    // before it. A key typed in the hold is the barrier: a hold that took the line as
    // queued would have run its hook before it read this key.
    run.pty.send(b"k");
    let log = run.wait_log("the probe key", |log| log.received.ends_with(b"k"));
    assert!(log.did.contains(&Did::Began {
        step: 2,
        received: 13
    }));
    assert_eq!(prompts_in(&events), ["first"]);
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    // It is the next turn's message, taken once the first turn ended, and no prompt was
    // asked for for it.
    assert_eq!(prompts_in(&events), ["first", "second"]);
    let story = story(&run.log());
    let held = story.iter().position(|did| did == "done 2 at 14");
    let submitted = story.iter().rposition(|did| did == "hook UserPromptSubmit");
    assert!(
        held.is_some() && held < submitted,
        "the message was submitted before the hold ended: {story:?}"
    );
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output, PROMPT.as_bytes()), 1);
}

#[test]
fn a_message_queued_before_a_dialog_is_up_does_not_answer_it() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::Ask(path("p.txt"), "content".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    // A message with an `n` in it, which would deny the dialog if the dialog read it.
    run.pty.send(b"nope\r");
    run.wait_log("the queued line", |log| log.received == b"first\rnope\r");
    fs::write(&go, "").expect("make the file");
    run.pty
        .wait_for(b"Do you want to create p.txt? (y/n)\r\n", at);
    // A key typed after the dialog is the barrier: it is read after anything the program
    // made of the bytes before it.
    run.pty.send(b"q");
    let log = run.wait_log("the key after the dialog", |log| {
        log.received.ends_with(b"nope\rq")
    });
    assert!(
        log.did
            .iter()
            .all(|did| !matches!(did, Did::Interrupted { .. })),
        "the queued line answered the dialog: {:?}",
        story(&log)
    );
    assert!(!run.work.join("p.txt").exists());
    run.pty.send(b"y");
    assert!(run.wait_exit().success());
    assert_eq!(
        fs::read_to_string(run.work.join("p.txt")).unwrap(),
        "content"
    );
    assert_eq!(prompts_in(&events), ["first", "nope"]);
}

#[test]
fn a_line_held_for_the_next_turn_does_not_answer_a_dialog_either() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let (started, gate) = (root.join("started"), root.join("gate"));
    make_fifo(&gate);
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Hold(go.clone()),
        Step::Ask(path("p.txt"), "content".into()),
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .plugin(vec![group(
            "PreToolUse",
            Some("Read"),
            gated(&started, &gate),
        )])
        .gate(&gate)
        .start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    let at = run.say(0, "first");
    assert!(
        wait_until(DEADLINE, || started.exists()),
        "the hook never ran"
    );
    // For the next turn: it holds an `n`, which would deny the dialog if it were read as
    // an answer.
    run.pty.send(b"any\r");
    open_gate(&gate);
    run.pty.wait_for("● Hold ".as_bytes(), at);
    fs::write(&go, "").expect("make the file");
    let asked = run
        .pty
        .wait_for(b"Do you want to create p.txt? (y/n)\r\n", at);
    run.pty.send(b"q");
    let log = run.wait_log("the key after the dialog", |log| {
        log.received.ends_with(b"any\rq")
    });
    assert!(
        log.did
            .iter()
            .all(|did| !matches!(did, Did::Interrupted { .. })),
        "the held line answered the dialog: {:?}",
        story(&log)
    );
    assert!(!run.work.join("p.txt").exists());
    run.pty.send(b"y");
    assert!(run.wait_exit().success());
    assert_eq!(
        fs::read_to_string(run.work.join("p.txt")).unwrap(),
        "content"
    );
    assert_eq!(prompts_in(&events), ["first", "any"]);
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output[asked..], PROMPT.as_bytes()), 0);
}

#[test]
fn a_self_started_turn_runs_the_hook_with_a_task_notice_and_reads_no_byte() {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let script = Script(vec![
        Step::SelfTurn,
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit", "Stop"], &events)
        .start();
    // The prompt is for the next turn, and comes after the first one ended.
    let prompt = run.pty.wait_for(PROMPT.as_bytes(), 0);
    let log = run.log();
    assert!(log.received.is_empty(), "it read {:?}", log.received);
    let seen = events_in(&events);
    assert_eq!(names(&seen), ["UserPromptSubmit", "Stop"]);
    let notice = seen[0]["prompt"].as_str().expect("a prompt");
    assert!(notice.starts_with("<task-notification>"), "{notice:?}");
    // The turn that works came with it, and was shown to have ended; no prompt was
    // printed before that.
    assert_eq!(find(&run.pty.output, WORKING), Some(0));
    assert!(find(&run.pty.output, NO_PROGRESS).is_some_and(|at| at < prompt));
    assert_eq!(count_of(&run.pty.output, PROMPT.as_bytes()), 1);

    // The message the user types after it is a message of its own.
    run.pty.send(b"typed\r");
    assert!(run.wait_exit().success());
    assert_eq!(prompts_in(&events)[1], "typed");
    assert_eq!(run.log().received, b"typed\r");
}

#[test]
fn a_hold_outside_a_turn_gates_the_steps_after_it_and_leaves_the_keys_typed_during_it() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Hold(go.clone()),
        Step::SelfTurn,
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    run.pty.wait_for("● Hold ".as_bytes(), 0);
    // Keys with no Enter, and an Esc: with no turn open the Esc interrupts nothing. Each
    // is read before the next is typed, so the Esc is a read of its own, as a key is.
    run.pty.send(b"abc");
    run.wait_log("abc", |log| log.received.ends_with(b"abc"));
    run.pty.send(b"\x1b");
    run.wait_log("the Esc", |log| log.received.ends_with(b"abc\x1b"));
    run.pty.send(b"k");
    let log = run.wait_log("the keys", |log| log.received.ends_with(b"abc\x1bk"));
    // Read in the hold, which has not ended: the hook of the step after it has not run,
    // and nothing printed the mark of an interrupt.
    assert!(
        log.did
            .iter()
            .all(|did| !matches!(did, Did::Done { step: 0, .. } | Did::Interrupted { .. })),
        "{:?}",
        story(&log)
    );
    assert!(!events.exists());
    fs::write(&go, "").expect("make the file");
    // The turn the stand-in started itself ended, and asks for a message: the keys are
    // still its own, so that the Enter typed now ends the line they began.
    let prompt = run.pty.wait_for(PROMPT.as_bytes(), 0);
    run.pty.send(b"\r");
    assert!(run.wait_exit().success());
    let prompts = prompts_in(&events);
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[0].starts_with("<task-notification>"));
    assert_eq!(prompts[1], "abck");
    // The Esc read in the hold interrupted neither the turn the stand-in started nor
    // anything else.
    let log = run.log();
    assert!(
        log.did
            .iter()
            .all(|did| !matches!(did, Did::Interrupted { .. })),
        "{:?}",
        story(&log)
    );
    run.pty.drain();
    assert!(find(&run.pty.output, INTERRUPT_MARK).is_none());
    assert!(find(&run.pty.output, WORKING).is_some_and(|at| at < prompt));
}

#[test]
fn a_line_typed_before_a_self_started_turn_is_the_next_turns_message_and_the_self_turn_takes_none_of_it()
 {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Hold(go.clone()),
        Step::SelfTurn,
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    run.pty.wait_for("● Hold ".as_bytes(), 0);
    run.pty.send(b"typed\r");
    run.wait_log("the line", |log| log.received == b"typed\r");
    assert!(!events.exists());
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    // The notice is the self-started turn's message, and the line is the next turn's, which
    // had it at hand and asked for nothing.
    let prompts = prompts_in(&events);
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[0].starts_with("<task-notification>"));
    assert_eq!(prompts[1], "typed");
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output, PROMPT.as_bytes()), 0);
}

#[test]
fn an_esc_typed_after_the_line_of_the_next_turn_interrupts_that_turn_as_it_would_claude_code() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Hold(go.clone()),
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Edit(path("a.txt"), "never".into()),
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    run.pty.wait_for("● Hold ".as_bytes(), 0);
    // A whole line, and then an Esc, each read before the next is typed. With no turn
    // open the line is not submitted, as the hold has not ended; Claude Code would have
    // started its turn at the Enter, and the Esc is then that turn's.
    run.pty.send(b"first\r");
    run.wait_log("the line", |log| log.received == b"first\r");
    run.pty.send(b"\x1b");
    run.wait_log("the Esc", |log| log.received == b"first\r\x1b");
    fs::write(&go, "").expect("make the file");
    let marked = run.pty.wait_for(INTERRUPT_MARK, 0);
    run.say(marked, "second");
    assert!(run.wait_exit().success());
    assert_eq!(prompts_in(&events), ["first", "second"]);
    // The turn the line began was interrupted before its edit.
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "seed\n"
    );
    let story = story(&run.log());
    assert!(
        story.iter().any(|did| did.starts_with("interrupted")),
        "{story:?}"
    );
}

#[test]
fn a_dialog_after_a_hold_outside_a_turn_sets_aside_what_was_typed_during_it() {
    let place = Place::new();
    let go = root_of(&place).join("go");
    let script = Script(vec![
        Step::Hold(go.clone()),
        Step::Ask(path("p.txt"), "content".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script).start();
    run.pty.wait_for("● Hold ".as_bytes(), 0);
    // With an `n`, which would deny the dialog if it were read as an answer to it.
    run.pty.send(b"abcn");
    run.wait_log("the keys", |log| log.received == b"abcn");
    fs::write(&go, "").expect("make the file");
    run.pty
        .wait_for(b"Do you want to create p.txt? (y/n)\r\n", 0);
    run.pty.send(b"q");
    let log = run.wait_log("the key after the dialog", |log| {
        log.received.ends_with(b"abcnq")
    });
    assert!(
        log.did
            .iter()
            .all(|did| !matches!(did, Did::Interrupted { .. })),
        "what was typed in the hold answered the dialog: {:?}",
        story(&log)
    );
    assert!(!run.work.join("p.txt").exists());
    run.pty.send(b"y");
    assert!(run.wait_exit().success());
    assert_eq!(
        fs::read_to_string(run.work.join("p.txt")).unwrap(),
        "content"
    );
}

// A reply through the plugin's tool server.

/// A run of a turn that gets the review `review` makes, given the run's folder, with its
/// message, for a prompt holding `only_for` when one is given, and replies as `script` says,
/// through a tool server of the test's own. Gives the run, its folder and the file its hooks
/// record their inputs in.
fn replying(
    review: impl FnOnce(&Path) -> String,
    only_for: Option<&str>,
    script: Script,
) -> (Run, PathBuf, PathBuf) {
    let place = Place::new();
    let root = root_of(&place);
    let review = review(&root);
    let events = root.join("events");
    let run = Start::new(place, "claude", script)
        .plugin(vec![group(
            "UserPromptSubmit",
            None,
            review_hook(&review, only_for),
        )])
        .recording(
            &[
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
            ],
            &events,
        )
        .with_tool_server()
        .start();
    (run, root, events)
}

/// Bodies a reply may carry: quotes, backticks, line ends, shell syntax, a backslash, a
/// line that looks like a message of the protocol, and what is no ASCII.
const AWKWARD_BODY: &str = "one \"quoted\" and `ticked` and $(echo hi); rm -rf / && \\ done\n\
    {\"id\":7,\"method\":\"tools/list\"}\n\tTabbed \u{fc}ber \u{2713}\n";

#[test]
fn a_reply_goes_to_the_thread_of_the_review_through_a_tool_server_started_once() {
    let review = review_of(&["t-aaa", "t-bbb", "t-ccc"]);
    let script = Script(vec![
        Step::Turn,
        Step::Reply {
            thread: 2,
            addressed: true,
            body: AWKWARD_BODY.into(),
        },
        Step::Reply {
            thread: 1,
            addressed: false,
            body: "short".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    let (mut run, root, events) = replying(|_| review, None, script);
    let at = run.say(0, "please do it");
    run.pty.wait_for("● Reply t-bbb\r\n".as_bytes(), at);
    assert!(run.wait_exit().success());

    // What the server read: what Claude Code says before it calls a tool, and the calls.
    let read = server_read(&root);
    let methods: Vec<&str> = read
        .iter()
        .map(|message| message["method"].as_str().expect("a method"))
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/call",
            "tools/call"
        ]
    );
    assert!(read[1].get("id").is_none(), "a notification has no id");
    let ids: Vec<u64> = [0, 2, 3, 4]
        .iter()
        .map(|at| read[*at]["id"].as_u64().expect("an id"))
        .collect();
    assert_eq!(ids, [0, 1, 2, 3]);
    let sent = calls(&read);
    assert_eq!(sent[0]["params"]["name"], "reply_to_thread");
    // The second heading's id, the body byte for byte, addressed; then the first's, open.
    let arguments = |call: &Value| call["params"]["arguments"].clone();
    assert_eq!(
        arguments(sent[0]),
        json!({"thread": "t-bbb", "body": AWKWARD_BODY, "addressed": true})
    );
    assert_eq!(
        arguments(sent[1]),
        json!({"thread": "t-aaa", "body": "short", "addressed": false})
    );

    // One server, a child of the stand-in, in its folder, told the session the stand-in
    // gave itself.
    let log = run.log();
    let started = fs::read_to_string(root.join("server.started")).expect("the server started");
    let started: Vec<&str> = started.lines().collect();
    assert_eq!(started.len(), 1, "started more than once: {started:?}");
    let fields: Vec<&str> = started[0].splitn(4, ' ').collect();
    assert_eq!(fields[1], log.pid.to_string(), "its parent");
    assert_eq!(fields[2], log.session);
    assert_eq!(fields[3], run.work.display().to_string());

    // What the stand-in wrote down: the call and the answer, between the hooks around it,
    // and the line in its terminal.
    let tools: Vec<&Did> = log
        .did
        .iter()
        .filter(|did| matches!(did, Did::Tool { .. }))
        .collect();
    assert_eq!(tools.len(), 2);
    let Did::Tool {
        arguments: logged,
        answer,
        ..
    } = tools[0]
    else {
        unreachable!()
    };
    assert_eq!(*logged, arguments(sent[0]));
    assert_eq!(
        *answer,
        json!({"content": [{"type": "text", "text": "recorded 1"}]})
    );
    let story = story(&log);
    let first = story
        .iter()
        .position(|did| did.starts_with("tool"))
        .expect("a tool");
    assert_eq!(story[first - 1], "hook PreToolUse");
    assert!(story[first + 1..].starts_with(&["hook PostToolUse".to_owned()]));

    let seen = events_in(&events);
    let tool_events: Vec<&Value> = seen
        .iter()
        .filter(|event| event["tool_name"] == LISTED_REPLY_TOOL)
        .collect();
    assert_eq!(tool_events.len(), 4);
    assert_eq!(tool_events[0]["hook_event_name"], "PreToolUse");
    assert_eq!(tool_events[0]["tool_input"], arguments(sent[0]));
    assert_eq!(
        tool_events[0]["mcp_server"]["name"],
        "plugin:yaar-review:yaar"
    );
    assert_eq!(tool_events[1]["hook_event_name"], "PostToolUse");
    assert_eq!(
        tool_events[1]["tool_response"],
        json!([{"type": "text", "text": "recorded 1"}])
    );

    // The server is the stand-in's own: it is gone with it.
    let server: u32 = fields[0].parse().expect("the server's id");
    assert!(
        wait_until(DEADLINE, || !is_alive(server)),
        "the tool server outlived the stand-in"
    );
}

#[test]
fn a_review_delivered_as_a_file_names_its_threads_in_that_file() {
    let script = Script(vec![
        Step::Turn,
        Step::Reply {
            thread: 3,
            addressed: true,
            body: "done".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    let (mut run, root, _) = replying(
        |root| {
            let folder = root.join("reviews");
            fs::create_dir_all(&folder).expect("make the folder of reviews");
            let file = folder.join("r-1.md");
            fs::write(&file, review_of(&["f-one", "f-two", "f-three"])).expect("write the review");
            // The pointer names a folder and a file that is not there as well as the file,
            // and ends its sentence on the path.
            format!(
                "Your review is long. Read all of {} first, not {}/missing.md, and answer: {}.",
                folder.display(),
                folder.display(),
                file.display()
            )
        },
        None,
        script,
    );
    run.say(0, "go");
    assert!(run.wait_exit().success());
    let read = server_read(&root);
    assert_eq!(
        calls(&read)[0]["params"]["arguments"],
        json!({"thread": "f-three", "body": "done", "addressed": true})
    );
}

#[test]
fn a_review_that_rides_with_a_queued_message_is_the_one_a_reply_names() {
    let script = Script(vec![
        Step::Turn,
        Step::Hold(path("go")),
        Step::Reply {
            thread: 2,
            addressed: true,
            body: "queued".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    // Only the second message gets a review: the first has none to reply to.
    let (mut run, root, _) = replying(
        |_| review_of(&["q-one", "q-two"]),
        Some("with a review"),
        script,
    );
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"with a review\r");
    run.wait_log("the review in the answer to the queued message", |log| {
        log.did.iter().any(
            |did| matches!(did, Did::Hook { event, answer, .. } if event == "UserPromptSubmit" && !answer.is_empty()),
        )
    });
    assert!(!root.join("server.started").exists());
    fs::write(run.work.join("go"), "").expect("make the file");
    assert!(run.wait_exit().success());
    let read = server_read(&root);
    assert_eq!(
        calls(&read)[0]["params"]["arguments"],
        json!({"thread": "q-two", "body": "queued", "addressed": true})
    );
}

#[test]
fn a_reply_the_server_refuses_is_a_failed_tool_with_the_servers_reason() {
    let script = Script(vec![
        Step::Turn,
        Step::Reply {
            thread: 1,
            addressed: true,
            body: "REFUSE".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    let (mut run, _, events) = replying(|_| review_of(&["r-one"]), None, script);
    run.say(0, "go");
    assert!(run.wait_exit().success());
    let log = run.log();
    let answer = log.did.iter().find_map(|did| match did {
        Did::Tool { answer, .. } => Some(answer.clone()),
        _ => None,
    });
    assert_eq!(
        answer,
        Some(json!({
            "content": [{"type": "text", "text": "refused: no such thread"}],
            "isError": true
        }))
    );
    let after: Vec<Value> = events_in(&events)
        .into_iter()
        .filter(|event| event["tool_name"] == LISTED_REPLY_TOOL)
        .collect();
    assert_eq!(names(&after), ["PreToolUse", "PostToolUseFailure"]);
    assert_eq!(after[1]["error"], "refused: no such thread");
    assert!(after[1].get("tool_response").is_none());
}

#[test]
fn a_reply_to_a_thread_the_review_does_not_have_stops_the_stand_in_and_says_which() {
    let reply = |thread| {
        Script(vec![
            Step::Turn,
            Step::Reply {
                thread,
                addressed: true,
                body: "x".into(),
            },
            Step::End,
            Step::Exit,
        ])
    };
    // Past the last thread of the review it got.
    let (mut run, root, _) = replying(|_| review_of(&["only"]), None, reply(2));
    run.say(0, "go");
    assert!(!run.wait_exit().success());
    run.pty.drain();
    let shown = String::from_utf8_lossy(&run.pty.output).into_owned();
    assert!(
        shown.contains("no thread 2") && shown.contains("only"),
        "{shown:?}"
    );
    assert!(
        !root.join("server.started").exists(),
        "the server was started"
    );

    // And with no review at all: the hook answers nothing for any prompt.
    let (mut run, _, _) = replying(|_| review_of(&["only"]), Some("never typed"), reply(1));
    run.say(0, "go");
    assert!(!run.wait_exit().success());
    run.pty.drain();
    let shown = String::from_utf8_lossy(&run.pty.output).into_owned();
    assert!(shown.contains("no thread 1"), "{shown:?}");
}

#[test]
fn ask_ignores_any_byte_but_y_n_and_esc_and_y_writes_the_file() {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let script = Script(vec![
        Step::Turn,
        Step::Ask(path("p.txt"), "content".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(
            &["PreToolUse", "PermissionRequest", "PostToolUse", "Stop"],
            &events,
        )
        .start();
    let at = run.say(0, "go");
    run.pty
        .wait_for(b"Do you want to create p.txt? (y/n)\r\n", at);
    // Capital letters, Enter, Ctrl-C, other letters and digits: none of them answers.
    run.pty.send(b"YN\r\x03q1");
    run.wait_log("the ignored bytes", |log| {
        log.received.ends_with(b"YN\r\x03q1")
    });
    assert!(!run.work.join("p.txt").exists());
    run.pty.send(b"y");
    assert!(run.wait_exit().success());
    assert_eq!(
        fs::read_to_string(run.work.join("p.txt")).unwrap(),
        "content"
    );
    let seen = events_in(&events);
    assert_eq!(
        names(&seen),
        ["PreToolUse", "PermissionRequest", "PostToolUse", "Stop"]
    );
    let input = json!({"file_path": format!("{}/p.txt", run.work.display()), "content": "content"});
    assert_eq!(seen[1]["tool_name"], "Write");
    assert_eq!(seen[1]["tool_input"], input);
    assert_eq!(seen[2]["tool_response"]["type"], "create");
    assert!(seen[1].get("tool_response").is_none());
    // The file came after the `y`, and not after a byte before it: the post hook saw
    // `go\r`, the six ignored bytes and the `y`.
    let post = run.log().did.iter().find_map(|did| match did {
        Did::Hook {
            event, received, ..
        } if event == "PostToolUse" => Some(*received),
        _ => None,
    });
    assert_eq!(post, Some(3 + 6 + 1));
}

/// A dialog denied with this key: the mark, no event after the request, no file, the turn
/// over as interrupted, and the script goes on after its `end`.
fn denied_with(key: &[u8]) {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let script = Script(vec![
        Step::Turn,
        Step::Ask(path("p.txt"), "content".into()),
        Step::Edit(path("p.txt"), "never".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(
            &[
                "PreToolUse",
                "PermissionRequest",
                "PostToolUse",
                "Stop",
                "SessionEnd",
            ],
            &events,
        )
        .start();
    let at = run.say(0, "go");
    let asked = run
        .pty
        .wait_for(b"Do you want to create p.txt? (y/n)\r\n", at);
    run.pty.send(key);
    run.pty.wait_for(INTERRUPT_MARK, asked);
    assert!(run.wait_exit().success());
    assert!(!run.work.join("p.txt").exists());
    assert_eq!(
        names(&events_in(&events)),
        ["PreToolUse", "PermissionRequest", "SessionEnd"]
    );
    let story = story(&run.log());
    let interrupted = story
        .iter()
        .position(|did| did.starts_with("interrupted"))
        .expect("the denial is written down");
    let received = 3 + key.len();
    assert_eq!(story[interrupted], format!("interrupted 1 at {received}"));
    assert_eq!(story[interrupted + 1], format!("began 4 at {received}"));
}

#[test]
fn n_denies_the_dialog_and_ends_the_turn_as_interrupted() {
    denied_with(b"n");
}

#[test]
fn esc_denies_the_dialog_and_ends_the_turn_as_interrupted() {
    denied_with(b"\x1b");
}

// The end of a script, and the other names.

#[test]
fn a_script_run_out_reads_on_until_the_terminal_closes_then_ends_with_no_session_end() {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let mut run = Start::new(place, "claude", Script(vec![Step::Turn, Step::End]))
        .recording(
            &["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"],
            &events,
        )
        .controlling()
        .start();
    run.say(0, "go");
    run.wait_log("the end of the script", |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::ScriptDone { .. }))
    });
    // Everything typed from here on reaches its log, and the program goes on.
    run.pty.send(b"left over");
    run.wait_log("the typed bytes", |log| log.received == b"go\rleft over");
    assert!(
        run.child.try_wait().expect("poll").is_none(),
        "it ended on its own"
    );
    run.pty.close();
    assert!(run.wait_exit().success());
    let story = story(&run.log());
    assert_eq!(story[story.len() - 2..], ["script done at 3", "exited 0"]);
    assert_eq!(
        names(&events_in(&events)),
        ["SessionStart", "UserPromptSubmit", "Stop"]
    );
}

#[test]
fn otheragent_edits_its_files_and_runs_no_hook() {
    no_hook_run("otheragent", None);
}

#[test]
fn a_stand_in_started_around_the_apps_pieces_runs_no_hook() {
    no_hook_run("claude", Some((IGNORE_PIECES, "1")));
}

fn no_hook_run(name: &str, variable: Option<(&str, &str)>) {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let script = Script(vec![
        Step::Turn,
        Step::Edit(path("a.txt"), "edited".into()),
        Step::Mark,
        Step::End,
        Step::Exit,
    ]);
    let mut start = Start::new(place, name, script).recording(
        &[
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "SessionEnd",
        ],
        &events,
    );
    if let Some((key, value)) = variable {
        start.env.push((key, value.to_owned()));
    }
    let mut run = start.start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    run.say(0, "go");
    assert!(run.wait_exit().success());
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "seed\nedited\n"
    );
    assert!(!events.exists(), "a hook ran");
    let log = run.log();
    assert!(
        log.did.iter().all(|did| !matches!(did, Did::Hook { .. })),
        "{log:?}"
    );
    // It still prints what such a CLI prints, the mark among it.
    run.pty.drain();
    assert!(find(&run.pty.output, "● Edit a.txt\r\n".as_bytes()).is_some());
    assert!(find(&run.pty.output, NO_PROGRESS).is_some());
}

// The stand-in as pi.

/// What a test as pi has: an extension file, the reporting program of the test's own, which
/// records what it is given and prints an answer the test chose, and the files they live in.
/// Each test gives pi a home of its own, the folder of its place, so that nothing under the
/// real `~/.pi` is read or written.
struct PiKit {
    root: PathBuf,
    extension: PathBuf,
    program: PathBuf,
    reports: PathBuf,
    /// The bytes the test last wrote into pi's settings, none while it wrote none.
    settings_written: RefCell<Option<Vec<u8>>>,
}

/// One run of the reporting program, as the test's program wrote it down.
struct Report {
    /// The program's parent: the process that ran it.
    parent: u32,
    /// Whether it was run with `--tool`.
    tool: bool,
    /// Whether `s.txt` was in the folder it ran in when it ran.
    s_there: bool,
    /// What it was given on its standard input.
    payload: Value,
}

impl Report {
    fn event(&self) -> &str {
        self.payload["event"].as_str().expect("an event")
    }
}

impl PiKit {
    fn new(place: &Place) -> PiKit {
        let root = root_of(place);
        let extension = root.join("ext/yaar.ts");
        fs::create_dir_all(extension.parent().expect("a folder")).expect("make the folder");
        fs::write(&extension, "// stands for the extension\n").expect("write the extension");
        // The program lies in a folder whose name a shell would split and expand: run
        // through `sh -c`, it is not found, so a stand-in that puts a shell between itself
        // and the program fails every report.
        let folder = root.join("report $x");
        fs::create_dir_all(&folder).expect("make the folder");
        let (program, reports) = (folder.join("report.sh"), root.join("reports"));
        // An answer for a steered message comes before one for any message, and each
        // answer is a file of the test's, there or not.
        fs::write(
            &program,
            format!(
                r#"#!/bin/sh
tool=no
[ "$1" = "--tool" ] && tool=yes
payload=$(cat)
there=no
[ -e s.txt ] && there=yes
printf '%s %s %s %s\n' "$PPID" "$tool" "$there" "$payload" >> '{reports}'
case "$payload" in
  *'"event":"input"'*)
    if [ -f '{root}/answer-steer' ] && case "$payload" in *'"streamingBehavior":"steer"'*) true ;; *) false ;; esac; then
      cat '{root}/answer-steer'
    elif [ -f '{root}/answer-input' ]; then
      cat '{root}/answer-input'
    fi ;;
  *'"event":"reply_to_thread"'*)
    [ -f '{root}/answer-tool' ] && cat '{root}/answer-tool' ;;
esac
exit 0
"#,
                reports = reports.display(),
                root = root.display(),
            ),
        )
        .expect("write the program");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("make it run");
        PiKit {
            root,
            extension,
            program,
            reports,
            settings_written: RefCell::new(None),
        }
    }

    /// The text the program prints for a report: `input` for any message, `steer` for a
    /// message typed while a tool runs (which comes first), `tool` for the reply tool.
    fn answer(&self, which: &str, text: &str) {
        fs::write(self.root.join(format!("answer-{which}")), text).expect("write the answer");
    }

    /// pi's settings in the home of the test.
    fn settings(&self) -> PathBuf {
        self.root.join("home/.pi/agent/settings.json")
    }

    fn write_settings(&self, text: &str) {
        self.write_settings_bytes(text.as_bytes());
    }

    fn write_settings_bytes(&self, bytes: &[u8]) {
        fs::create_dir_all(self.settings().parent().expect("a folder")).expect("make the folder");
        fs::write(self.settings(), bytes).expect("write the settings");
        *self.settings_written.borrow_mut() = Some(bytes.to_vec());
    }

    /// Settings that list the extension, with other keys beside it, in a layout no JSON
    /// writer produces (keys out of order, mixed indentation, a space before one colon, a
    /// closing line and a blank one), so that a stand-in that parsed and wrote them back
    /// would change bytes.
    fn list_extension(&self) {
        let path = serde_json::to_string(&self.extension).expect("JSON");
        self.write_settings(&format!(
            "{{\n\t\"zoom\" : 1,\n  \"extensions\":[ {path} ] ,\n\t  \"theme\":\"dark\"\n}}\n\n"
        ));
    }

    /// Checks that pi's settings are the bytes the test wrote, or still not there: pi, and
    /// the stand-in as pi, never writes them. Called after the stand-in has ended.
    fn assert_settings_kept(&self) {
        assert_eq!(
            fs::read(self.settings()).ok(),
            *self.settings_written.borrow(),
            "the stand-in changed pi's settings"
        );
    }

    /// The stand-in as `pi`, in a shell that names the program.
    fn start<'a>(&self, place: Place, script: Script) -> Start<'a> {
        let mut start = Start::new(place, "pi", script);
        start
            .env
            .push(("YAAR_PI_REPORT", self.program.display().to_string()));
        start
    }

    /// What the program was run for, in order.
    fn reports(&self) -> Vec<Report> {
        fs::read_to_string(&self.reports)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let words: Vec<&str> = line.splitn(4, ' ').collect();
                Report {
                    parent: words[0].parse().expect("a process id"),
                    tool: words[1] == "yes",
                    s_there: words[2] == "yes",
                    payload: serde_json::from_str(words[3]).expect("a payload is JSON"),
                }
            })
            .collect()
    }

    /// Waits until the program has been run `n` times.
    fn wait_reports(&self, n: usize) -> Vec<Report> {
        let mut seen = Vec::new();
        let enough = wait_until(DEADLINE, || {
            seen = self.reports();
            seen.len() >= n
        });
        let events: Vec<&str> = seen.iter().map(Report::event).collect();
        assert!(enough, "waited for {n} reports; the program got {events:?}");
        seen
    }
}

fn events_of(reports: &[Report]) -> Vec<&str> {
    reports.iter().map(Report::event).collect()
}

/// The words of every `Did::Hook` the stand-in wrote down, as `event`.
fn hooked(log: &StandinLog) -> Vec<&str> {
    log.did
        .iter()
        .filter_map(|did| match did {
            Did::Hook { event, .. } => Some(event.as_str()),
            _ => None,
        })
        .collect()
}

/// Checks that each needle is in the output, one after the other.
fn in_order(output: &[u8], needles: &[&str]) {
    let mut from = 0;
    for needle in needles {
        let at = find(&output[from..], needle.as_bytes()).unwrap_or_else(|| {
            panic!(
                "no {needle:?} after byte {from} of {:?}",
                String::from_utf8_lossy(output)
            )
        });
        from += at + needle.len();
    }
}

/// The keys a payload of this event holds, which the crate doc of the pi Adapter gives.
fn keys_of_event(report: &Report) -> Vec<&'static str> {
    let mut keys = vec!["cwd", "event", "sessionFile", "sessionId"];
    keys.extend(match report.event() {
        "session_start" | "session_shutdown" => vec!["reason"],
        "input" => vec!["source", "streamingBehavior"],
        "agent_start" => vec![],
        "tool_call" => vec!["input", "toolCallId", "toolName"],
        "tool_result" if report.payload["toolName"] == "edit" => {
            vec!["details", "input", "isError", "toolCallId", "toolName"]
        }
        "tool_result" => vec!["input", "isError", "toolCallId", "toolName"],
        "agent_settled" => vec!["aborted"],
        "reply_to_thread" => vec!["arguments", "toolCallId"],
        other => panic!("an event the crate doc does not give: {other}"),
    });
    keys.sort_unstable();
    keys
}

fn keys_of_payload(report: &Report) -> Vec<&str> {
    let mut keys: Vec<&str> = report
        .payload
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

#[test]
fn pi_with_the_extension_reports_each_moment_of_its_script_in_order_with_the_payloads_the_adapter_reads()
 {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    // What the program answers to a message is what the log holds as the hook's answer.
    kit.answer("input", r#"{"review": "a review"}"#);
    let script = Script(vec![
        Step::Turn,
        Step::Read(path("a.txt")),
        Step::Edit(path("a.txt"), "more".into()),
        Step::Write(path("n.txt"), "hi".into()),
        Step::Shell("echo x > s.txt".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = kit.start(place, script).start();
    fs::write(run.work.join("a.txt"), "one\ntwo\nthree\n").expect("seed");
    run.say(0, "hello");
    assert!(run.wait_exit().success());

    let reports = kit.reports();
    let log = run.log();
    let moments = [
        "session_start",
        "input",
        "agent_start",
        "tool_call",
        "tool_result",
        "tool_call",
        "tool_result",
        "tool_call",
        "tool_result",
        "tool_call",
        "tool_result",
        "agent_settled",
        "session_shutdown",
    ];
    assert_eq!(events_of(&reports), moments);

    // Every payload is one object with the keys the crate doc gives its event, naming the
    // session, its file and the folder.
    let file = kit
        .root
        .join("home/.pi/agent/sessions/standin")
        .join(format!("{}.jsonl", log.session));
    for report in &reports {
        assert_eq!(
            keys_of_payload(report),
            keys_of_event(report),
            "{:?}",
            report.payload
        );
        assert_eq!(report.payload["sessionId"], log.session);
        assert_eq!(report.payload["sessionFile"], file.display().to_string());
        assert_eq!(report.payload["cwd"], run.work.display().to_string());
        // The program is run by the stand-in itself, with no shell between, and not with
        // `--tool`.
        assert_eq!(report.parent, log.pid);
        assert!(!report.tool);
    }
    let payload = |at: usize| &reports[at].payload;
    assert_eq!(payload(0)["reason"], "startup");
    assert_eq!(payload(1)["source"], "interactive");
    assert!(payload(1)["streamingBehavior"].is_null());
    let tools: Vec<(&str, &Value)> = [3, 5, 7, 9]
        .iter()
        .map(|at| {
            (
                payload(*at)["toolName"].as_str().expect("a tool"),
                &payload(*at)["input"],
            )
        })
        .collect();
    assert_eq!(
        tools,
        [
            ("read", &json!({"path": "a.txt"})),
            ("edit", &json!({"path": "a.txt"})),
            ("write", &json!({"path": "n.txt"})),
            ("bash", &json!({"command": "echo x > s.txt"})),
        ]
    );
    for at in [3, 5, 7, 9] {
        // A result names the call it answers, and none failed.
        assert_eq!(payload(at)["toolCallId"], payload(at + 1)["toolCallId"]);
        assert_eq!(payload(at)["toolName"], payload(at + 1)["toolName"]);
        assert_eq!(payload(at + 1)["isError"], false);
    }
    let ids: Vec<&Value> = [3, 5, 7, 9]
        .iter()
        .map(|at| &payload(*at)["toolCallId"])
        .collect();
    assert!(ids.windows(2).all(|pair| pair[0] != pair[1]), "{ids:?}");

    // The edit's patch: one hunk, whose added range starts at the line after the last of
    // the three lines the file had, with the line it added and no line of context.
    let patch = payload(6)["details"]["patch"].as_str().expect("a patch");
    let lines: Vec<&str> = patch.lines().collect();
    assert_eq!(
        lines,
        ["--- a.txt", "+++ a.txt", "@@ -3,0 +4,1 @@", "+more"]
    );

    assert_eq!(payload(11)["aborted"], false);
    assert_eq!(payload(12)["reason"], "quit");

    // The shell command runs between its two reports: not yet there when the call was
    // reported, there when the result was.
    assert!(!reports[9].s_there && reports[10].s_there);

    // What the stand-in wrote down: a hook of each report, under pi's own event name, with
    // what the program printed.
    assert_eq!(hooked(&log), moments);
    let answers: Vec<&[u8]> = log
        .did
        .iter()
        .filter_map(|did| match did {
            Did::Hook { answer, killed, .. } => {
                assert!(!killed);
                Some(answer.as_slice())
            }
            _ => None,
        })
        .collect();
    assert_eq!(answers[1], br#"{"review": "a review"}"#);
    assert!(
        answers
            .iter()
            .enumerate()
            .all(|(at, answer)| at == 1 || answer.is_empty())
    );

    // The steps acted, as the lines of the terminal show, and pi prints no progress
    // sequence and no mark.
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "one\ntwo\nthree\nmore\n"
    );
    assert_eq!(fs::read_to_string(run.work.join("n.txt")).unwrap(), "hi");
    assert!(run.work.join("s.txt").exists());
    run.pty.drain();
    let shown = String::from_utf8_lossy(&run.pty.output).into_owned();
    in_order(
        shown.as_bytes(),
        &[
            "● Read a.txt",
            "● Edit a.txt",
            "● Write n.txt",
            "● Bash echo x > s.txt",
            "● Done",
        ],
    );
    assert_eq!(count_of(&run.pty.output, WORKING), 0);
    assert_eq!(count_of(&run.pty.output, NO_PROGRESS), 0);
    assert_eq!(count_of(&run.pty.output, INTERRUPT_MARK), 0);

    // pi's own files: the settings as they were, no session file, nothing else made.
    assert_eq!(log.extensions, std::slice::from_ref(&kit.extension));
    kit.assert_settings_kept();
    assert!(!file.exists());
    let made: Vec<String> = fs::read_dir(kit.root.join("home/.pi/agent"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(made, ["settings.json"]);
}

#[test]
fn pi_with_no_extension_in_its_settings_reports_nothing_and_with_one_reports_through_the_program_the_shell_names()
 {
    // No settings at all: pi runs, finds no extension, and the program is never run.
    let place = Place::new();
    let kit = PiKit::new(&place);
    let mut run = kit
        .start(place, Script(vec![Step::Turn, Step::Exit]))
        .start();
    run.say(0, "hello");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let log = run.log();
    assert!(log.extensions.is_empty());
    assert!(hooked(&log).is_empty(), "{log:?}");
    assert!(!kit.reports.exists(), "the program was run");
    // It ran its script all the same: the turn and the exit are written down.
    assert!(
        log.did
            .iter()
            .any(|did| matches!(did, Did::Done { step: 1, .. }))
    );

    // With the extension listed, the same script is reported, through the program the
    // shell names.
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let mut run = kit
        .start(place, Script(vec![Step::Turn, Step::Exit]))
        .start();
    run.say(0, "hello");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    assert_eq!(
        events_of(&kit.reports()),
        ["session_start", "input", "agent_start", "session_shutdown"]
    );
}

#[test]
fn an_extension_given_with_dash_e_is_found_where_the_settings_list_none_and_after_those_that_do() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    let mut start = kit.start(place, Script(vec![Step::Exit]));
    start.args = vec!["-e".into(), kit.extension.display().to_string()];
    let mut run = start.start();
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let log = run.log();
    assert_eq!(log.extensions, std::slice::from_ref(&kit.extension));
    assert_eq!(log.args, ["-e", &kit.extension.display().to_string()]);
    assert_eq!(
        events_of(&kit.reports()),
        ["session_start", "session_shutdown"]
    );

    // Both: what the settings list comes first, then the long form of the flag; a file
    // named twice is not told apart, as pi's own list is not.
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let other = kit.root.join("ext/other.ts");
    fs::write(&other, "").expect("write another");
    let mut start = kit.start(place, Script(vec![Step::Exit]));
    start.args = vec!["--extension".into(), other.display().to_string()];
    let mut run = start.start();
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    assert_eq!(run.log().extensions, [kit.extension.clone(), other]);

    // A file named in the settings and again with the flag is listed twice, as pi's own
    // list has it.
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let mut start = kit.start(place, Script(vec![Step::Exit]));
    start.args = vec!["-e".into(), kit.extension.display().to_string()];
    let mut run = start.start();
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    assert_eq!(
        run.log().extensions,
        [kit.extension.clone(), kit.extension.clone()]
    );
}

#[test]
fn settings_that_start_with_a_byte_order_mark_are_read_as_pi_reads_them() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    let path = serde_json::to_string(&kit.extension).expect("JSON");
    kit.write_settings(&format!("\u{feff}{{\"extensions\": [{path}]}}\n"));
    let mut run = kit.start(place, Script(vec![Step::Exit])).start();
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    assert_eq!(run.log().extensions, std::slice::from_ref(&kit.extension));
    assert_eq!(
        events_of(&kit.reports()),
        ["session_start", "session_shutdown"]
    );
}

/// A run as pi with the script `turn`, `edit`, `end`, `exit` in which no report is made:
/// the program the test names was not run, and the steps acted.
fn pi_reports_nothing(set_up: impl FnOnce(&PiKit, &mut Start<'_>), found: &[&str]) {
    let place = Place::new();
    let kit = PiKit::new(&place);
    let script = Script(vec![
        Step::Turn,
        Step::Edit(path("a.txt"), "edited".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut start = kit.start(place, script);
    set_up(&kit, &mut start);
    let mut run = start.start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    run.say(0, "go");
    assert!(run.wait_exit().success());
    // The stand-in has ended, so a report that would have been run was run: the barrier of
    // a clause that says nothing happened.
    assert!(!kit.reports.exists(), "the program was run");
    let log = run.log();
    assert!(hooked(&log).is_empty(), "{log:?}");
    let found: Vec<PathBuf> = found.iter().map(|name| kit.root.join(name)).collect();
    assert_eq!(log.extensions, found);
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "seed\nedited\n"
    );
    run.pty.drain();
    in_order(&run.pty.output, &["● Edit a.txt", "● Done"]);
    // pi's settings are the same bytes, or still not there.
    kit.assert_settings_kept();
}

#[test]
fn pi_with_no_settings_file_reports_nothing_and_its_steps_still_act() {
    pi_reports_nothing(|_, _| {}, &[]);
}

#[test]
fn pi_with_settings_that_list_no_extensions_reports_nothing() {
    pi_reports_nothing(|kit, _| kit.write_settings(r#"{"theme": "dark"}"#), &[]);
}

#[test]
fn pi_with_entries_that_name_no_file_reports_nothing() {
    // A missing file, a folder and a value that is no string.
    pi_reports_nothing(
        |kit, _| {
            let listed = json!({"extensions": [
                kit.root.join("ext/missing.ts"),
                kit.root.join("ext"),
                5,
                null
            ]});
            kit.write_settings(&listed.to_string());
        },
        &[],
    );
}

#[test]
fn pi_given_an_extension_with_dash_e_that_names_no_file_reports_nothing() {
    // No settings, and a flag whose path is no file: one that is not there, and a folder.
    pi_reports_nothing(
        |kit, start| {
            start.args = vec![
                "-e".into(),
                kit.root.join("ext/missing.ts").display().to_string(),
            ];
        },
        &[],
    );
    pi_reports_nothing(
        |kit, start| {
            start.args = vec![
                "--extension".into(),
                kit.root.join("ext").display().to_string(),
            ];
        },
        &[],
    );
}

#[test]
fn pi_with_settings_that_are_a_json_array_or_no_json_reports_nothing() {
    pi_reports_nothing(
        |kit, _| kit.write_settings(&json!([kit.extension]).to_string()),
        &[],
    );
    pi_reports_nothing(
        |kit, _| kit.write_settings(&format!(r#"{{"extensions": ["{}"#, kit.extension.display())),
        &[],
    );
    // An `extensions` that is no array.
    pi_reports_nothing(
        |kit, _| {
            kit.write_settings(&json!({"extensions": kit.extension}).to_string());
        },
        &[],
    );
}

#[test]
fn pi_with_the_extension_found_and_no_program_named_reports_nothing() {
    pi_reports_nothing(
        |kit, start| {
            kit.list_extension();
            start.env.clear();
        },
        &["ext/yaar.ts"],
    );
}

#[test]
fn pi_started_around_the_apps_pieces_reports_nothing_with_the_extension_listed() {
    pi_reports_nothing(
        |kit, start| {
            kit.list_extension();
            start.env.push((IGNORE_PIECES, "1".to_owned()));
        },
        &["ext/yaar.ts"],
    );
}

#[test]
fn a_reporting_program_that_cannot_be_started_is_a_report_with_no_answer_and_the_script_goes_on() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let mut start = kit.start(
        place,
        Script(vec![
            Step::Turn,
            Step::Edit(path("a.txt"), "edited".into()),
            Step::End,
            Step::Exit,
        ]),
    );
    start.env.clear();
    start.env.push((
        "YAAR_PI_REPORT",
        kit.root.join("no-such-program").display().to_string(),
    ));
    let mut run = start.start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    run.say(0, "go");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let log = run.log();
    assert_eq!(
        hooked(&log),
        [
            "session_start",
            "input",
            "agent_start",
            "tool_call",
            "tool_result",
            "agent_settled",
            "session_shutdown"
        ]
    );
    assert!(log.did.iter().all(|did| match did {
        Did::Hook { answer, killed, .. } => answer.is_empty() && !killed,
        _ => true,
    }));
    assert_eq!(
        fs::read_to_string(run.work.join("a.txt")).unwrap(),
        "seed\nedited\n"
    );
}

#[test]
fn a_self_started_turn_is_an_input_from_the_extension_with_no_prompt_and_no_key_read() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let script = Script(vec![Step::SelfTurn, Step::End, Step::Exit]);
    let mut run = kit.start(place, script).start();
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let reports = kit.reports();
    assert_eq!(
        events_of(&reports),
        [
            "session_start",
            "input",
            "agent_start",
            "agent_settled",
            "session_shutdown"
        ]
    );
    assert_eq!(reports[1].payload["source"], "extension");
    assert!(reports[1].payload["streamingBehavior"].is_null());
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output, PROMPT.as_bytes()), 0);
    assert!(run.log().received.is_empty());
}

#[test]
fn a_line_typed_during_a_hold_is_an_input_that_steers_at_its_enter_and_starts_no_run() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let go = kit.root.join("go");
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = kit.start(place, script).start();
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"second\r");
    // The report is made at the Enter, while the hold goes on: the file it waits for is
    // not there yet when the report is.
    let seen = kit.wait_reports(4);
    assert_eq!(
        events_of(&seen),
        ["session_start", "input", "agent_start", "input"]
    );
    assert_eq!(seen[3].payload["source"], "interactive");
    assert_eq!(seen[3].payload["streamingBehavior"], "steer");
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    // The message went with the run that was open: one start, one end.
    assert_eq!(
        events_of(&kit.reports()),
        [
            "session_start",
            "input",
            "agent_start",
            "input",
            "agent_settled",
            "session_shutdown"
        ]
    );
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output, PROMPT.as_bytes()), 1);
}

#[test]
fn esc_during_a_turn_reports_the_run_aborted_prints_no_mark_and_the_script_goes_on_after_end() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let script = Script(vec![
        Step::Turn,
        Step::Hold(kit.root.join("never")),
        Step::Read(path("a.txt")),
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = kit.start(place, script).start();
    fs::write(run.work.join("a.txt"), "seed\n").expect("seed");
    let at = run.say(0, "first");
    let held = run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"\x1b");
    run.wait_log("the interrupt", |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::Interrupted { .. }))
    });
    run.say(held, "second");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();

    // The first run was aborted, and the read and the end of it that the script still had
    // were skipped; the second run is whole.
    let reports = kit.reports();
    assert_eq!(
        events_of(&reports),
        [
            "session_start",
            "input",
            "agent_start",
            "agent_settled",
            "input",
            "agent_start",
            "agent_settled",
            "session_shutdown"
        ]
    );
    assert_eq!(reports[3].payload["aborted"], true);
    assert_eq!(reports[6].payload["aborted"], false);
    // After the program ended, so that nothing printed later is missed: no mark, no
    // progress sequence, and no `Done` for the run that was aborted.
    run.pty.drain();
    assert_eq!(count_of(&run.pty.output, INTERRUPT_MARK), 0);
    assert_eq!(count_of(&run.pty.output, b"Interrupted"), 0);
    assert_eq!(count_of(&run.pty.output, NO_PROGRESS), 0);
    assert_eq!(count_of(&run.pty.output, "● Done".as_bytes()), 1);
    assert_eq!(count_of(&run.pty.output, "● Read".as_bytes()), 0);
}

#[test]
fn a_reply_runs_the_program_with_tool_and_the_arguments_of_the_thread_byte_for_byte() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    kit.answer(
        "input",
        &json!({"review": review_of(&["T1", "T2"])}).to_string(),
    );
    let answer = json!({"text": "recorded reply to T2 (addressed, 5 characters)"});
    kit.answer("tool", &answer.to_string());
    let script = Script(vec![
        Step::Turn,
        Step::Reply {
            thread: 2,
            addressed: true,
            body: AWKWARD_BODY.into(),
        },
        Step::Reply {
            thread: 1,
            addressed: false,
            body: "short".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    let mut run = kit.start(place, script).start();
    run.say(0, "please do it");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();

    let reports = kit.reports();
    assert_eq!(
        events_of(&reports),
        [
            "session_start",
            "input",
            "agent_start",
            "reply_to_thread",
            "reply_to_thread",
            "agent_settled",
            "session_shutdown"
        ]
    );
    let tools: Vec<bool> = reports.iter().map(|report| report.tool).collect();
    assert_eq!(tools, [false, false, false, true, true, false, false]);
    for at in [3, 4] {
        assert_eq!(
            keys_of_payload(&reports[at]),
            keys_of_event(&reports[at]),
            "{:?}",
            reports[at].payload
        );
        assert_eq!(reports[at].parent, run.log().pid);
    }
    let sent = |at: usize| reports[at].payload["arguments"].clone();
    assert_eq!(
        sent(3),
        json!({"thread": "T2", "body": AWKWARD_BODY, "addressed": true})
    );
    assert_eq!(
        sent(4),
        json!({"thread": "T1", "body": "short", "addressed": false})
    );
    assert_ne!(
        reports[3].payload["toolCallId"],
        reports[4].payload["toolCallId"]
    );

    // Written down as the call and what the program printed.
    let tools: Vec<(Value, Value)> = run
        .log()
        .did
        .iter()
        .filter_map(|did| match did {
            Did::Tool {
                arguments, answer, ..
            } => Some((arguments.clone(), answer.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0], (sent(3), answer));
    assert_eq!(tools[1].0, sent(4));
    // Neither is a report of a tool pi runs: no `Hook` of the reply.
    assert_eq!(
        hooked(&run.log()),
        [
            "session_start",
            "input",
            "agent_start",
            "agent_settled",
            "session_shutdown"
        ]
    );
    run.pty.drain();
    in_order(&run.pty.output, &["● Reply T2", "● Reply T1"]);
}

#[test]
fn a_review_that_rides_with_a_queued_message_is_the_one_a_reply_names_as_pi() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    // The first message is answered with no review; the one typed while the tool runs is.
    kit.answer(
        "steer",
        &json!({"review": review_of(&["queued-1"])}).to_string(),
    );
    let go = kit.root.join("go");
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::Reply {
            thread: 1,
            addressed: true,
            body: "done".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    let mut run = kit.start(place, script).start();
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"second\r");
    kit.wait_reports(4);
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let reports = kit.reports();
    let reply = reports
        .iter()
        .find(|report| report.event() == "reply_to_thread")
        .expect("the reply was reported");
    assert_eq!(reply.payload["arguments"]["thread"], "queued-1");
}

#[test]
fn a_session_told_by_session_and_session_dir_keeps_that_id_and_names_its_file_in_that_folder() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let folder = kit.root.join("sessions-of-the-test");
    let mut start = kit.start(place, Script(vec![Step::Exit]));
    start.args = vec![
        "--session".into(),
        "id-1".into(),
        "--session-dir".into(),
        folder.display().to_string(),
    ];
    let mut run = start.start();
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let reports = kit.reports();
    assert_eq!(events_of(&reports), ["session_start", "session_shutdown"]);
    for report in &reports {
        assert_eq!(report.payload["sessionId"], "id-1");
        assert_eq!(
            report.payload["sessionFile"],
            folder.join("id-1.jsonl").display().to_string()
        );
    }
    // pi starts a session it was told to take up as any other: the Adapter reads the
    // resume from the id it holds.
    assert_eq!(reports[0].payload["reason"], "startup");
    assert_eq!(run.log().session, "id-1");
    assert!(!folder.exists(), "the stand-in made the folder");
}

#[test]
fn a_script_with_ask_is_refused_at_its_line_and_nothing_is_reported() {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let script = Script(vec![
        Step::Turn,
        Step::Ask(path("a.txt"), "text".into()),
        Step::End,
    ]);
    let mut run = kit.start(place, script).start();
    let status = run.wait_exit();
    kit.assert_settings_kept();
    assert_eq!(status.code(), Some(2));
    run.pty.drain();
    let shown = String::from_utf8_lossy(&run.pty.output).into_owned();
    assert!(
        shown.contains("script line 2") && shown.contains("`ask`"),
        "{shown:?}"
    );
    assert!(!kit.reports.exists());
}

#[test]
fn a_shell_command_that_fails_is_a_result_with_is_error_and_an_edit_of_an_empty_file_has_a_hunk_from_line_one()
 {
    let place = Place::new();
    let kit = PiKit::new(&place);
    kit.list_extension();
    let script = Script(vec![
        Step::Turn,
        Step::Shell("exit 3".into()),
        Step::Edit(path("empty.txt"), "first\nsecond".into()),
        Step::End,
        Step::Exit,
    ]);
    let mut run = kit.start(place, script).start();
    fs::write(run.work.join("empty.txt"), "").expect("seed");
    run.say(0, "go");
    assert!(run.wait_exit().success());
    kit.assert_settings_kept();
    let reports = kit.reports();
    let results: Vec<&Report> = reports
        .iter()
        .filter(|report| report.event() == "tool_result")
        .collect();
    assert_eq!(results[0].payload["toolName"], "bash");
    assert_eq!(results[0].payload["isError"], true);
    assert_eq!(results[1].payload["isError"], false);
    let patch = results[1].payload["details"]["patch"]
        .as_str()
        .expect("a patch");
    assert_eq!(
        patch.lines().collect::<Vec<_>>(),
        [
            "--- empty.txt",
            "+++ empty.txt",
            "@@ -0,0 +1,2 @@",
            "+first",
            "+second"
        ]
    );
    assert_eq!(
        fs::read_to_string(run.work.join("empty.txt")).unwrap(),
        "first\nsecond\n"
    );
}

// Sessions, the terminal, the log.

#[test]
fn a_resume_argument_names_the_session_and_its_source() {
    let place = Place::new();
    let events = root_of(&place).join("events");
    let id = "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d";
    let mut start =
        Start::new(place, "claude", Script(vec![Step::Exit])).recording(&["SessionStart"], &events);
    start.args = vec!["--resume".into(), id.into()];
    let mut run = start.start();
    assert!(run.wait_exit().success());
    let seen = events_in(&events);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["session_id"], id);
    assert_eq!(seen[0]["source"], "resume");
    let log = run.log();
    assert_eq!(log.session, id);
    assert_eq!(log.args, ["--resume", id]);
}

#[test]
fn a_new_session_has_a_uuid_shaped_id_of_its_own_every_time() {
    let mut ids = Vec::new();
    for _ in 0..2 {
        let mut run = Start::new(Place::new(), "claude", Script(vec![Step::Exit])).start();
        assert!(run.wait_exit().success());
        ids.push(run.log().session);
    }
    for id in &ids {
        let parts: Vec<&str> = id.split('-').collect();
        let lengths: Vec<usize> = parts.iter().map(|part| part.len()).collect();
        assert_eq!(lengths, [8, 4, 4, 4, 12], "{id}");
        assert!(
            id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()),
            "{id}"
        );
        assert!(parts[2].starts_with('4'), "a version 4 UUID: {id}");
    }
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn the_terminal_is_raw_while_the_stand_in_runs_and_as_it_was_after_it_exits() {
    let mut run = Start::new(Place::new(), "claude", Script(vec![Step::Turn, Step::Exit])).start();
    let before = run.before;
    assert_ne!(
        before.c_lflag & (libc::ECHO | libc::ICANON | libc::ISIG),
        0,
        "a new terminal echoes"
    );
    // Once the prompt is up the terminal is set.
    run.say(0, "go");
    let during = run.pty.termios();
    assert_eq!(during.c_lflag & (libc::ECHO | libc::ICANON | libc::ISIG), 0);
    assert_eq!(during.c_iflag & (libc::IXON | libc::ICRNL), 0);
    assert_ne!(
        during.c_oflag & libc::OPOST,
        0,
        "output processing stays on"
    );
    assert!(run.wait_exit().success());
    let after = run.pty.termios();
    // The kernel marks input it has to retype after a change of mode; that is not a setting.
    assert_eq!(
        after.c_lflag & !libc::PENDIN,
        before.c_lflag & !libc::PENDIN
    );
    assert_eq!(after.c_iflag, before.c_iflag);
    assert_eq!(after.c_oflag, before.c_oflag);
}

#[test]
fn a_flood_writes_its_numbered_lines_in_order_and_writes_down_its_longest_write() {
    let script = Script(vec![Step::Turn, Step::Flood(3000), Step::End, Step::Exit]);
    let mut run = Start::new(Place::new(), "claude", script).start();
    let at = run.say(0, "go");
    run.pty.wait_for(b"line 3000\r\n", at);
    assert!(run.wait_exit().success());
    run.pty.drain();
    let expected: String = (1..=3000).map(|k| format!("line {k}\r\n")).collect();
    assert!(
        find(&run.pty.output[at..], expected.as_bytes()).is_some(),
        "the lines are not whole and in order"
    );
    let story = story(&run.log());
    let began = story
        .iter()
        .position(|did| did == "began 1 at 3")
        .expect("began");
    assert_eq!(
        story[began..began + 3],
        ["began 1 at 3", "flooded 1", "done 1 at 3"]
    );
    // The terminal was read all the while: no write waited anywhere near the time the next
    // test holds the terminal for.
    let longest = longest_write(&run.log());
    assert!(
        longest < FLOOD_HOLD / 2,
        "a write waited {longest:?} though the terminal was read all the time"
    );
}

/// How long the test that does not read a flood holds its terminal.
const FLOOD_HOLD: Duration = Duration::from_millis(1200);

/// The longest write the log says a flood made.
fn longest_write(log: &StandinLog) -> Duration {
    log.did
        .iter()
        .find_map(|did| match did {
            Did::Flooded { longest_us, .. } => Some(Duration::from_micros(*longest_us)),
            _ => None,
        })
        .expect("the flood is written down")
}

#[test]
fn a_flood_into_a_terminal_nobody_reads_writes_down_the_wait_of_its_longest_write() {
    // More lines than any terminal buffers, a megabyte or so: from the start of the flood
    // the program waits in one write until the test reads. The test holds the terminal
    // twice, with a read between that stops half way, and measures both spans itself. The
    // longest write waited one span; the sum of the waits would be both.
    let lines = 100_000;
    let script = Script(vec![Step::Turn, Step::Flood(lines), Step::End, Step::Exit]);
    let mut run = Start::new(Place::new(), "claude", script).start();
    let at = run.say(0, "go");
    run.wait_log("the flood", |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::Began { step: 1, .. }))
    });
    let first = Instant::now();
    wait_until(FLOOD_HOLD, || first.elapsed() >= FLOOD_HOLD);
    let first = first.elapsed();
    run.pty
        .read_until(format!("line {}\r\n", lines / 2).as_bytes(), at);
    let second = Instant::now();
    wait_until(FLOOD_HOLD, || second.elapsed() >= FLOOD_HOLD);
    let second = second.elapsed();
    run.pty.wait_for(format!("line {lines}\r\n").as_bytes(), at);
    assert!(run.wait_exit().success());
    let span = first.max(second);
    let longest = longest_write(&run.log());
    assert!(
        longest >= span * 3 / 4,
        "the longest write waited {longest:?}, the terminal was not read for {span:?} at a time"
    );
    assert!(
        longest < span * 3 / 2,
        "the longest write waited {longest:?}: the waits of {first:?} and {second:?} were added"
    );
}

#[test]
fn a_hook_that_does_not_end_is_killed_with_what_it_started_after_ten_seconds() {
    let place = Place::new();
    let root = root_of(&place);
    let (sleeper, late) = (root.join("sleeper"), root.join("late"));
    // The hook parts a child from itself: the whole group has to go, or the program would
    // wait on the pipe the child holds.
    let hook = format!(
        "sleep 60 & echo $! > '{}'; wait; echo late > '{}'",
        sleeper.display(),
        late.display()
    );
    let began = Instant::now();
    let mut run = Start::new(place, "claude", Script(vec![Step::Exit]))
        .plugin(vec![group("SessionStart", None, hook)])
        .sleeper(&sleeper)
        .start();
    let ended = wait_until(HOOK_LIMIT_AND_ROOM, || {
        run.child.try_wait().expect("poll").is_some()
    });
    assert!(ended, "the stand-in waited on the hook for ever");
    // The hook got its ten seconds: the program started it after `began`, and waited for it.
    assert!(
        began.elapsed() >= Duration::from_secs(10),
        "the hook was killed after {:?}",
        began.elapsed()
    );
    assert_eq!(story(&run.log())[0], "hook SessionStart killed");
    let pid: u32 = fs::read_to_string(&sleeper)
        .expect("the hook started a child")
        .trim()
        .parse()
        .expect("a pid");
    assert!(
        wait_until(DEADLINE, || !is_alive(pid)),
        "the hook's child is still running"
    );
    assert!(!late.exists(), "the hook went on after it was killed");
}

#[test]
fn a_log_read_while_the_program_writes_it_ignores_a_line_not_yet_whole() {
    let mut run = Start::new(Place::new(), "claude", Script(vec![Step::Turn, Step::Exit])).start();
    run.say(0, "go");
    assert!(run.wait_exit().success());
    let whole = run.log();
    assert_eq!(whole.did.last(), Some(&Did::Exited { status: 0 }));
    let mut text = fs::read(&run.log).expect("the log");
    text.extend_from_slice(b"{\"did\":\"exi");
    fs::write(&run.log, text).expect("append a part of a line");
    assert_eq!(run.log(), whole);
    // A log that is not there yet is an empty one.
    let missing = root_of(&run.place).join("no-such-log");
    assert_eq!(StandinLog::read(&missing), StandinLog::default());
}

#[test]
fn a_terminal_that_closes_during_a_tool_stops_the_script_before_its_next_step() {
    let place = Place::new();
    let root = root_of(&place);
    let (gate, after) = (root.join("gate"), root.join("after"));
    // The shell command blocks the program until the test lets it go: the terminal closes
    // while the program is not reading it, and the step after the command must not run.
    let wait = format!("while [ ! -e '{}' ]; do sleep 0.05; done", gate.display());
    let script = Script(vec![
        Step::Turn,
        Step::Shell(wait),
        Step::Write(after.clone(), "late".to_owned()),
        Step::End,
    ]);
    let mut run = Start::new(place, "claude", script).controlling().start();
    run.say(0, "go");
    run.wait_log("the command", |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::Began { step: 1, .. }))
    });
    run.pty.close();
    fs::write(&gate, "").expect("open the gate");
    assert!(run.wait_exit().success());
    assert!(!after.exists(), "a step ran after the terminal closed");
    let story = story(&run.log());
    assert!(
        !story.iter().any(|did| did.starts_with("began 2")),
        "a step began after the terminal closed: {story:?}"
    );
    assert_eq!(story.last().map(String::as_str), Some("exited 0"));
}

#[test]
fn a_terminal_that_closes_at_the_prompt_or_during_a_hold_ends_the_stand_in_with_no_session_end() {
    for (steps, waits_for) in [
        (vec![Step::Turn], PROMPT),
        (vec![Step::Turn, Step::Hold(path("/never"))], "● Hold "),
    ] {
        let place = Place::new();
        let events = root_of(&place).join("events");
        let mut run = Start::new(place, "claude", Script(steps))
            .recording(&["SessionEnd"], &events)
            .controlling()
            .start();
        let at = run.pty.wait_for(PROMPT.as_bytes(), 0);
        if waits_for != PROMPT {
            run.pty.send(b"go\r");
            run.pty.wait_for(waits_for.as_bytes(), at);
        }
        run.pty.close();
        assert!(run.wait_exit().success());
        assert_eq!(run.log().did.last(), Some(&Did::Exited { status: 0 }));
        assert!(!events.exists(), "a session end was reported");
    }
}
