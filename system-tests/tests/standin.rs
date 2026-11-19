//! Tests of the stand-in agent: the built `standin-agent`, run in a pseudo-terminal the
//! test opens, with a plugin folder of the test's own whose hooks write down what they
//! were given. No Keeper, no app: the stand-in knows only Claude Code's side of the plugin
//! contract.

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

    /// Waits until `needle` is in the output after byte `from`; gives where it ends.
    fn wait_for(&mut self, needle: &[u8], from: usize) -> usize {
        let mut end = None;
        let seen = wait_until(DEADLINE, || {
            self.drain();
            end = find(&self.output[from..], needle).map(|at| from + at + needle.len());
            end.is_some()
        });
        assert!(
            seen,
            "never saw {:?} after byte {from}; the output was {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&self.output)
        );
        end.expect("seen")
    }

    /// Reads, one chunk at a time and no further than it has to, until `needle` is in the
    /// output after byte `from`. What the program writes after that stays unread, so that it
    /// soon waits in a write again.
    fn read_until(&mut self, needle: &[u8], from: usize) {
        // Only what came in since the last look is searched: the output is a megabyte.
        let mut looked = from;
        let seen = wait_until(DEADLINE, || {
            loop {
                if find(&self.output[looked..], needle).is_some() {
                    return true;
                }
                looked = self.output.len().saturating_sub(needle.len()).max(from);
                let mut poll = libc::pollfd {
                    fd: self.master(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: one valid `pollfd`.
                if unsafe { libc::poll(&mut poll, 1, 0) } <= 0 || poll.revents & libc::POLLIN == 0 {
                    return false;
                }
                let mut buffer = [0u8; 8192];
                // SAFETY: the buffer is live and its length is given.
                let n =
                    unsafe { libc::read(self.master(), buffer.as_mut_ptr().cast(), buffer.len()) };
                if n <= 0 {
                    return false;
                }
                self.output.extend_from_slice(&buffer[..n as usize]);
            }
        });
        assert!(
            seen,
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
        }
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
        let pty = Pty::open();
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
    // A lone command: `sh -c` runs it in its own place, as it does for the app's hook
    // program, whose parent has to be the agent.
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
    let command = format!("'{}'", hook.display());
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

#[test]
fn a_line_typed_during_a_turn_is_the_next_turns_message_and_no_prompt_is_printed_for_it() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    run.pty.send(b"second\r");
    run.wait_log("the queued line", |log| log.received == b"first\rsecond\r");
    fs::write(&go, "").expect("make the file");
    assert!(run.wait_exit().success());
    let prompts: Vec<String> = events_in(&events)
        .iter()
        .map(|event| event["prompt"].as_str().expect("a prompt").to_owned())
        .collect();
    assert_eq!(prompts, ["first", "second"]);
    run.pty.drain();
    let shown = run
        .pty
        .output
        .windows(PROMPT.len())
        .filter(|window| *window == PROMPT.as_bytes())
        .count();
    assert_eq!(shown, 1, "only the first message was asked for");
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

#[test]
fn a_dialog_is_answered_by_keys_typed_after_it_and_a_line_typed_before_it_waits_for_its_turn() {
    let place = Place::new();
    let root = root_of(&place);
    let (events, go) = (root.join("events"), root.join("go"));
    let script = Script(vec![
        Step::Turn,
        Step::Hold(go.clone()),
        Step::Ask(path("p.txt"), "content".into()),
        Step::End,
        Step::Turn,
        Step::End,
        Step::Exit,
    ]);
    let mut run = Start::new(place, "claude", script)
        .recording(&["UserPromptSubmit"], &events)
        .start();
    let at = run.say(0, "first");
    run.pty.wait_for("● Hold ".as_bytes(), at);
    // A message for the next turn, typed while this one works: it holds an `n` and no
    // `y`, and is read before the dialog is up.
    run.pty.send(b"any\r");
    run.wait_log("the queued line", |log| log.received == b"first\rany\r");
    fs::write(&go, "").expect("make the file");
    let asked = run
        .pty
        .wait_for(b"Do you want to create p.txt? (y/n)\r\n", at);
    // A key typed after the dialog is the barrier: it is read after anything the program
    // made of the bytes before it.
    run.pty.send(b"q");
    let log = run.wait_log("the key after the dialog", |log| {
        log.received.ends_with(b"any\rq")
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
    // The queued line is the next turn's message, and no prompt was printed for it.
    let prompts: Vec<String> = events_in(&events)
        .iter()
        .map(|event| event["prompt"].as_str().expect("a prompt").to_owned())
        .collect();
    assert_eq!(prompts, ["first", "any"]);
    run.pty.drain();
    let shown = run.pty.output[asked..]
        .windows(PROMPT.len())
        .filter(|window| *window == PROMPT.as_bytes())
        .count();
    assert_eq!(shown, 0, "a prompt was printed for the queued line");
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

#[test]
fn pi_refuses_to_start_and_says_why() {
    let mut run = Start::new(Place::new(), "pi", Script(vec![Step::Turn, Step::Exit])).start();
    let status = run.wait_exit();
    assert_eq!(status.code(), Some(2));
    run.pty.drain();
    let shown = String::from_utf8_lossy(&run.pty.output).into_owned();
    assert!(
        shown.contains("the pi dialect of the stand-in comes with slice 7"),
        "{shown:?}"
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
    let before = run.pty.termios();
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
