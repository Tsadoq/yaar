//! The stand-in agent CLI, run as `claude`, `pi` or `otheragent`: it follows the script file
//! [`SCRIPT_VARIABLE`] names and logs what reached it to the file [`LOG_VARIABLE`] names.
//! Tests read its screen for each step's line (`● Read <path>` ... `● Done`) and for [`PROMPT`].

use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::mem::MaybeUninit;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::dialect::Dialect;

pub const SCRIPT_VARIABLE: &str = "STANDIN_SCRIPT";

pub const LOG_VARIABLE: &str = "STANDIN_LOG";

pub const IGNORE_PIECES: &str = "STANDIN_IGNORE_PIECES";

pub const PROMPT: &str = "standin> ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Prints [`PROMPT`] and waits for a line ended by Enter, the user's message, which starts a
    /// turn. A line typed earlier, outside a [`Step::Hold`], is taken without a prompt; an Esc read
    /// after its Enter interrupts the turn.
    Turn,
    SelfTurn,
    Read(PathBuf),
    Edit(PathBuf, String),
    Write(PathBuf, String),
    Shell(String),
    /// Asks permission to write a file with one key: `y` writes it, `n` or Esc denies and ends the
    /// turn as interrupted.
    Ask(PathBuf, String),
    /// Holds the turn open until the file exists; outside a turn it is a gate. In a turn, Esc
    /// interrupts and a line ended by Enter is queued.
    Hold(PathBuf),
    HoldAtStart(PathBuf),
    Flood(u32),
    /// Ends the turn with `● Done`. An Esc earlier in the turn interrupts it instead, and the
    /// script goes on after the next `end`.
    End,
    /// Prints the no-progress sequence alone and runs no hook, as any program may: such a sequence
    /// without a session ends no turn.
    Mark,
    Exit,
    Reply {
        /// Counted from 1, in the order the last review received names its threads.
        thread: u32,
        addressed: bool,
        body: String,
    },
}

pub const INTERRUPT_MARK: &[u8] =
    b"\x1b]9;4;0;\x07Interrupted \xc2\xb7 What should Claude do instead?\r\n";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script(pub Vec<Step>);

impl Script {
    pub fn text(&self) -> String {
        self.0.iter().map(|step| step.line() + "\n").collect()
    }

    pub fn parse(text: &str) -> Script {
        let steps = text
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.is_empty())
            .map(|(n, line)| {
                Step::from_line(line)
                    .unwrap_or_else(|why| panic!("script line {}: {why}: {line:?}", n + 1))
            })
            .collect();
        Script(steps)
    }
}

impl Step {
    fn line(&self) -> String {
        let word = |path: &Path| {
            let word = path.to_str().expect("a path of a script is text");
            assert!(
                !word.is_empty() && !word.contains(char::is_whitespace),
                "a path that is the first word of a step has no white space: {word:?}"
            );
            word.to_owned()
        };
        let whole = |path: &Path| {
            let text = path.to_str().expect("a path of a script is text");
            assert!(!text.contains('\n'), "a path of a script holds no line end");
            text.to_owned()
        };
        match self {
            Step::Turn => "turn".into(),
            Step::Read(path) => format!("read {}", whole(path)),
            Step::Edit(path, text) => format!("edit {} {}", word(path), escape(text)),
            Step::Write(path, text) => format!("write {} {}", word(path), escape(text)),
            Step::Shell(command) => format!("shell {}", escape(command)),
            Step::Ask(path, text) => format!("ask {} {}", word(path), escape(text)),
            Step::Hold(path) => format!("hold {}", whole(path)),
            Step::HoldAtStart(path) => format!("hold-at-start {}", whole(path)),
            Step::Flood(lines) => format!("flood {lines}"),
            Step::End => "end".into(),
            Step::Mark => "mark".into(),
            Step::Exit => "exit".into(),
            Step::SelfTurn => "self-turn".into(),
            Step::Reply {
                thread,
                addressed,
                body,
            } => format!(
                "reply {thread} {} {}",
                if *addressed { "addressed" } else { "open" },
                escape(body)
            ),
        }
    }

    fn from_line(line: &str) -> Result<Step, String> {
        let (keyword, rest) = line.split_once(' ').unwrap_or((line, ""));
        let alone = |step: Step| {
            if rest.is_empty() {
                Ok(step)
            } else {
                Err(format!("`{keyword}` takes no words"))
            }
        };
        let path_and_text = || -> Result<(PathBuf, String), String> {
            let (path, text) = rest.split_once(' ').unwrap_or((rest, ""));
            if path.is_empty() {
                return Err(format!("`{keyword}` needs a path"));
            }
            Ok((PathBuf::from(path), unescape(text)?))
        };
        let path = || {
            if rest.is_empty() {
                Err(format!("`{keyword}` needs a path"))
            } else {
                Ok(PathBuf::from(rest))
            }
        };
        match keyword {
            "turn" => alone(Step::Turn),
            "read" => path().map(Step::Read),
            "edit" => path_and_text().map(|(path, text)| Step::Edit(path, text)),
            "write" => path_and_text().map(|(path, text)| Step::Write(path, text)),
            "shell" => unescape(rest).map(Step::Shell),
            "ask" => path_and_text().map(|(path, text)| Step::Ask(path, text)),
            "hold" => path().map(Step::Hold),
            "hold-at-start" => path().map(Step::HoldAtStart),
            "flood" => rest
                .parse()
                .map(Step::Flood)
                .map_err(|_| "`flood` needs a number of lines".to_owned()),
            "end" => alone(Step::End),
            "mark" => alone(Step::Mark),
            "exit" => alone(Step::Exit),
            "self-turn" => alone(Step::SelfTurn),
            "reply" => {
                let mut words = rest.splitn(3, ' ');
                let thread = words
                    .next()
                    .and_then(|word| word.parse().ok())
                    .filter(|thread| *thread >= 1)
                    .ok_or_else(|| "`reply` needs a thread counted from 1".to_owned())?;
                let addressed = match words.next() {
                    Some("addressed") => true,
                    Some("open") => false,
                    _ => return Err("`reply` needs `addressed` or `open`".to_owned()),
                };
                let body = unescape(words.next().unwrap_or(""))?;
                Ok(Step::Reply {
                    thread,
                    addressed,
                    body,
                })
            }
            _ => Err(format!("no step is called `{keyword}`")),
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape(line: &str) -> Result<String, String> {
    let mut text = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => text.push('\\'),
            Some('n') => text.push('\n'),
            Some('r') => text.push('\r'),
            other => return Err(format!("`\\` followed by {other:?}")),
        }
    }
    Ok(text)
}

/// `received` counts the bytes the stand-in had read by then, so a test reads what arrived between
/// two things from `StandinLog::received`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    Began {
        step: usize,
        received: usize,
    },
    Done {
        step: usize,
        received: usize,
    },
    Hook {
        event: String,
        answer: Vec<u8>,
        killed: bool,
        received: usize,
    },
    Interrupted {
        step: usize,
        received: usize,
    },
    Flooded {
        step: usize,
        longest_us: u64,
    },
    ScriptDone {
        received: usize,
    },
    Exited {
        status: i32,
    },
    /// The reply tool's answer: the `result` of the call, or its `error`; `{"message": <why>}` when
    /// the server answered nothing in time or ended.
    Tool {
        arguments: Value,
        answer: Value,
        received: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StandinLog {
    pub pid: u32,
    pub args: Vec<String>,
    pub extensions: Vec<PathBuf>,
    pub session: String,
    pub received: Vec<u8>,
    pub did: Vec<Did>,
}

impl StandinLog {
    /// An empty log when the file is not there yet; a last line not yet finished is left out.
    pub fn read(path: &Path) -> StandinLog {
        let mut log = StandinLog::default();
        let Ok(bytes) = std::fs::read(path) else {
            return log;
        };
        let text = String::from_utf8_lossy(&bytes);
        let whole = text.rfind('\n').map_or("", |end| &text[..end]);
        for line in whole.lines() {
            let Ok(entry) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            log.take(&entry);
        }
        log
    }

    fn take(&mut self, entry: &Value) {
        let number = |key: &str| entry[key].as_u64().unwrap_or(0);
        let received = number("received") as usize;
        let step = number("step") as usize;
        if let Some(pid) = entry["pid"].as_u64() {
            self.pid = pid as u32;
        } else if let Some(args) = entry["args"].as_array() {
            self.args = args
                .iter()
                .filter_map(|arg| arg.as_str().map(str::to_owned))
                .collect();
        } else if let Some(list) = entry["extensions"].as_array() {
            self.extensions = list
                .iter()
                .filter_map(|path| path.as_str().map(PathBuf::from))
                .collect();
        } else if let Some(session) = entry["session"].as_str() {
            self.session = session.to_owned();
        } else if let Some(bytes) = entry["recv"].as_str() {
            self.received.extend(unhex(bytes));
        } else if let Some(did) = entry["did"].as_str() {
            self.did.push(match did {
                "began" => Did::Began { step, received },
                "done" => Did::Done { step, received },
                "hook" => Did::Hook {
                    event: entry["event"].as_str().unwrap_or_default().to_owned(),
                    answer: unhex(entry["answer"].as_str().unwrap_or_default()),
                    killed: entry["killed"].as_bool().unwrap_or(false),
                    received,
                },
                "interrupted" => Did::Interrupted { step, received },
                "flooded" => Did::Flooded {
                    step,
                    longest_us: number("longest_us"),
                },
                "script-done" => Did::ScriptDone { received },
                "exited" => Did::Exited {
                    status: entry["status"].as_i64().unwrap_or(0) as i32,
                },
                "tool" => Did::Tool {
                    arguments: entry["arguments"].clone(),
                    answer: entry["answer"].clone(),
                    received,
                },
                _ => return,
            });
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

const HOOK_LIMIT: Duration = Duration::from_secs(10);

const KEY_POLL_MS: i32 = 20;

const WORKING: &[u8] = b"\x1b]9;4;3;\x07";
const NO_PROGRESS: &[u8] = b"\x1b]9;4;0;\x07";

const LISTED_REPLY_TOOL: &str = "mcp__plugin_yaar-review_yaar__reply_to_thread";
const REPLY_TOOL: &str = "reply_to_thread";

const TOOL_SERVER_NAME: &str = "yaar";
const TOOL_SERVER_LISTED: &str = "plugin:yaar-review:yaar";

/// Claude Code waits for a tool as long as it takes; the app's own server answers within ten
/// seconds, so one silent by then is stuck.
const TOOL_WAIT: Duration = Duration::from_secs(15);

const REPORT_VARIABLE: &str = "YAAR_PI_REPORT";

const PI_SETTINGS: &str = ".pi/agent/settings.json";

pub const PI_SESSIONS: &str = ".pi/agent/sessions/standin";

const TASK_NOTIFICATION: &str = "<task-notification>\n<task-id>standin-task</task-id>\n\
<status>completed</status>\n<summary>The background task has ended.</summary>\n\
</task-notification>";

/// When the script runs out with no [`Step::Exit`], the stand-in ends no session: it reads its
/// terminal until that closes, then gives 0. As `pi` a script holding `ask`, which pi has no
/// counterpart of, is refused with status 2; any name but `pi` and `otheragent` acts as `claude`.
pub fn run(name: &str, args: &[String]) -> i32 {
    // In a shell of the app the terminal is the stand-in's controlling terminal, and its
    // closing sends a hangup: that is a close to read and end on, not a death.
    // SAFETY: ignoring a signal is always allowed.
    unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
    let script = match std::env::var_os(SCRIPT_VARIABLE) {
        Some(file) => {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|why| panic!("the script {}: {why}", Path::new(&file).display()));
            if name == "pi"
                && let Some(line) = text
                    .lines()
                    .position(|line| line.split(' ').next() == Some("ask"))
            {
                eprintln!(
                    "script line {}: `ask` is no step of pi's, which asks no permission",
                    line + 1
                );
                return 2;
            }
            Script::parse(&text)
        }
        None => Script::default(),
    };
    let pieces =
        name != "otheragent" && std::env::var(IGNORE_PIECES).map_or(true, |value| value != "1");
    let mut standin = Standin::start(args, pieces, name == "pi");
    let status = standin.go(&script);
    standin.restore();
    status
}

/// Esc is a read of the single byte `0x1b`: a longer read that holds it is a sequence of keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Byte(u8),
    Esc,
}

#[derive(Default)]
struct Input {
    received: usize,
    pending: VecDeque<Key>,
    closed: bool,
}

impl Input {
    fn fill(&mut self, log: &Log, wait_ms: i32) -> bool {
        if self.closed {
            return false;
        }
        let mut poll = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid `pollfd`.
        if unsafe { libc::poll(&mut poll, 1, wait_ms) } <= 0 {
            return false;
        }
        if poll.revents & libc::POLLIN == 0 {
            self.closed = poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
            return false;
        }
        let mut buffer = [0u8; 4096];
        // SAFETY: the buffer is live and its length is given.
        let n = unsafe { libc::read(0, buffer.as_mut_ptr().cast(), buffer.len()) };
        if n < 0 {
            let error = std::io::Error::last_os_error().kind();
            self.closed = !matches!(
                error,
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            );
            return false;
        }
        if n == 0 {
            self.closed = true;
            return false;
        }
        let bytes = &buffer[..n as usize];
        self.received += bytes.len();
        log.line(&json!({"recv": hex(bytes)}));
        if bytes == [0x1b] {
            self.pending.push_back(Key::Esc);
        } else {
            self.pending
                .extend(bytes.iter().map(|byte| Key::Byte(*byte)));
        }
        true
    }

    fn take_esc(&mut self) -> bool {
        match self.pending.iter().position(|key| *key == Key::Esc) {
            Some(at) => {
                self.pending.remove(at);
                true
            }
            None => false,
        }
    }

    fn forget_esc(&mut self) {
        self.pending.retain(|key| *key != Key::Esc);
    }

    fn take_line(&mut self) -> Option<String> {
        let end = self
            .pending
            .iter()
            .position(|key| *key == Key::Byte(b'\r'))?;
        let line: Vec<u8> = self
            .pending
            .drain(..=end)
            .filter_map(|key| match key {
                Key::Byte(b'\r') | Key::Esc => None,
                Key::Byte(byte) => Some(byte),
            })
            .collect();
        Some(String::from_utf8_lossy(&line).into_owned())
    }
}

struct Log(Option<File>);

impl Log {
    fn line(&self, entry: &Value) {
        if let Some(mut file) = self.0.as_ref() {
            let _ = file.write_all(format!("{entry}\n").as_bytes());
        }
    }

    fn did(&self, did: &str, fields: Value) {
        let mut entry = json!({"did": did});
        for (key, value) in fields.as_object().expect("an object") {
            entry[key] = value.clone();
        }
        self.line(&entry);
    }
}

enum Ended {
    Done,
    Interrupted,
    Closed,
    Exit,
}

enum Answer {
    Yes,
    No,
    Closed,
}

struct Standin {
    log: Log,
    input: Input,
    saved: Option<libc::termios>,
    plugins: Vec<Value>,
    session: String,
    resumed: bool,
    cwd: PathBuf,
    transcript: PathBuf,
    in_turn: bool,
    plugin_dirs: Vec<PathBuf>,
    review: Option<String>,
    server: Option<ToolServer>,
    pi: Option<PiSide>,
}

struct PiSide {
    program: Option<PathBuf>,
    session_file: PathBuf,
    calls: Cell<u32>,
}

impl Standin {
    fn start(args: &[String], pieces: bool, as_pi: bool) -> Standin {
        let saved = raw_terminal();
        let log = match std::env::var_os(LOG_VARIABLE) {
            Some(file) => Log(Some(File::create(&file).unwrap_or_else(|why| {
                panic!("the log {}: {why}", Path::new(&file).display())
            }))),
            None => Log(None),
        };
        let resumed_id = value_of(args, &[if as_pi { "--session" } else { "--resume" }]);
        let session = resumed_id.cloned().unwrap_or_else(new_session_id);
        let cwd = std::env::current_dir().expect("the folder the stand-in runs in");
        let home =
            std::env::var_os("HOME").map_or_else(|| PathBuf::from("/nonexistent"), PathBuf::from);
        let transcript = home
            .join(".claude/projects")
            .join(cwd.to_string_lossy().replace('/', "-"))
            .join(format!("{session}.jsonl"));
        log.line(&json!({"pid": std::process::id()}));
        log.line(&json!({"args": args}));
        let pi = as_pi.then(|| {
            let extensions = extensions_found(&home, args);
            log.line(&json!({"extensions": extensions}));
            let folder = value_of(args, &["--session-dir"])
                .map_or_else(|| home.join(PI_SESSIONS), PathBuf::from);
            PiSide {
                program: std::env::var_os(REPORT_VARIABLE)
                    .filter(|program| pieces && !extensions.is_empty() && !program.is_empty())
                    .map(PathBuf::from),
                session_file: folder.join(format!("{session}.jsonl")),
                calls: Cell::new(0),
            }
        });
        log.line(&json!({"session": session}));
        let plugin_dirs = if pieces && !as_pi {
            plugin_dirs()
        } else {
            Vec::new()
        };
        let plugins = read_plugins(&plugin_dirs);
        Standin {
            log,
            input: Input::default(),
            saved,
            plugins,
            session,
            resumed: resumed_id.is_some(),
            cwd,
            transcript,
            in_turn: false,
            plugin_dirs,
            review: None,
            server: None,
            pi,
        }
    }

    fn restore(&self) {
        if let Some(saved) = &self.saved {
            // SAFETY: standard input and settings `tcgetattr` gave.
            unsafe { libc::tcsetattr(0, libc::TCSANOW, saved) };
        }
    }

    fn go(&mut self, script: &Script) -> i32 {
        let steps = &script.0;
        let mut at = 0;
        while let Some(Step::HoldAtStart(path)) = steps.get(at) {
            self.log.did(
                "began",
                json!({"step": at, "received": self.input.received}),
            );
            if matches!(self.hold(at, path), Ended::Closed) {
                return self.closed();
            }
            self.log
                .did("done", json!({"step": at, "received": self.input.received}));
            at += 1;
        }
        if self.pi.is_some() {
            // pi reports `startup` even when resuming with `--session`: the Keeper already holds
            // the session id it expects.
            self.pi_report("session_start", json!({"reason": "startup"}));
        } else {
            let source = if self.resumed { "resume" } else { "startup" };
            self.hook("SessionStart", None, json!({"source": source}));
        }
        let mut skipping = false;
        while at < steps.len() {
            if self.input.closed {
                return self.closed();
            }
            let step = &steps[at];
            self.input.fill(&self.log, 0);
            if self.input.closed {
                return self.closed();
            }
            if self.in_turn && self.input.take_esc() {
                self.interrupt(at);
                skipping = true;
            }
            if skipping {
                skipping = !matches!(step, Step::End);
                at += 1;
                continue;
            }
            self.log.did(
                "began",
                json!({"step": at, "received": self.input.received}),
            );
            match self.step(at, step) {
                Ended::Done => self
                    .log
                    .did("done", json!({"step": at, "received": self.input.received})),
                Ended::Interrupted => skipping = true,
                Ended::Closed => return self.closed(),
                Ended::Exit => {
                    self.log
                        .did("done", json!({"step": at, "received": self.input.received}));
                    self.log.did("exited", json!({"status": 0}));
                    return 0;
                }
            }
            at += 1;
        }
        self.log
            .did("script-done", json!({"received": self.input.received}));
        while !self.input.closed {
            self.input.fill(&self.log, -1);
        }
        self.closed()
    }

    fn closed(&self) -> i32 {
        self.log.did("exited", json!({"status": 0}));
        0
    }

    fn interrupt(&mut self, step: usize) {
        if self.pi.is_some() {
            self.pi_report("agent_settled", json!({"aborted": true}));
        } else {
            out(INTERRUPT_MARK);
        }
        self.log.did(
            "interrupted",
            json!({"step": step, "received": self.input.received}),
        );
        self.in_turn = false;
    }

    fn step(&mut self, at: usize, step: &Step) -> Ended {
        if self.pi.is_some() {
            return self.pi_step(at, step);
        }
        match step {
            Step::Turn => self.turn(),
            Step::Read(path) => {
                let file = self.absolute(path);
                self.tool(
                    "Read",
                    json!({"file_path": file}),
                    &format!("Read {}", path.display()),
                    || {
                        std::fs::read(&file)
                            .unwrap_or_else(|why| panic!("read {}: {why}", file.display()));
                        (json!({}), None)
                    },
                );
                Ended::Done
            }
            Step::Edit(path, text) => {
                self.edit(path, text);
                Ended::Done
            }
            Step::Write(path, text) => {
                let file = self.absolute(path);
                let input = json!({"file_path": file, "content": text});
                self.tool("Write", input, &format!("Write {}", path.display()), || {
                    (write_whole(&file, text), None)
                });
                Ended::Done
            }
            Step::Shell(command) => {
                self.shell(command);
                Ended::Done
            }
            Step::Ask(path, text) => self.ask(at, path, text),
            Step::Hold(path) => self.hold(at, path),
            Step::HoldAtStart(_) => panic!("`hold-at-start` is a step at the head of a script"),
            Step::Flood(lines) => {
                self.flood(at, *lines);
                Ended::Done
            }
            Step::End => {
                self.in_turn = false;
                out(b"\xe2\x97\x8f Done\r\n");
                self.hook("Stop", None, json!({"stop_hook_active": false}));
                out(NO_PROGRESS);
                Ended::Done
            }
            Step::Mark => {
                out(NO_PROGRESS);
                Ended::Done
            }
            Step::Exit => {
                self.hook("SessionEnd", None, json!({"reason": "prompt_input_exit"}));
                self.restore();
                Ended::Exit
            }
            Step::SelfTurn => {
                self.self_turn();
                Ended::Done
            }
            Step::Reply {
                thread,
                addressed,
                body,
            } => {
                self.reply(*thread, *addressed, body);
                Ended::Done
            }
        }
    }

    fn turn(&mut self) -> Ended {
        let line = match self.input.take_line() {
            Some(line) => line,
            None => {
                out(PROMPT.as_bytes());
                loop {
                    if let Some(line) = self.input.take_line() {
                        break line;
                    }
                    if self.input.closed {
                        return Ended::Closed;
                    }
                    self.input.fill(&self.log, -1);
                }
            }
        };
        out(b"\r\n");
        // An Esc read after this line's Enter stays pending: Claude Code starts the turn at the
        // Enter, and the Esc interrupts it.
        self.in_turn = true;
        if self.pi.is_some() {
            self.pi_message("interactive", None);
            self.pi_report("agent_start", json!({}));
        } else {
            self.submit(&line);
            out(WORKING);
        }
        Ended::Done
    }

    fn self_turn(&mut self) {
        self.input.forget_esc();
        self.in_turn = true;
        if self.pi.is_some() {
            self.pi_message("extension", None);
            self.pi_report("agent_start", json!({}));
        } else {
            self.submit(TASK_NOTIFICATION);
            out(WORKING);
        }
    }

    fn queue(&mut self, line: &str) {
        if self.pi.is_some() {
            self.pi_message("interactive", Some("steer"));
        } else {
            self.submit(line);
        }
    }

    fn submit(&mut self, message: &str) {
        for answer in self.hook("UserPromptSubmit", None, json!({"prompt": message})) {
            if let Some(review) = review_in(&answer) {
                self.review = Some(review);
            }
        }
    }

    fn reply(&mut self, thread: u32, addressed: bool, body: &str) {
        let ids = self.review.as_deref().map(thread_ids).unwrap_or_default();
        let Some(id) = thread
            .checked_sub(1)
            .and_then(|index| ids.get(index as usize))
        else {
            panic!("no thread {thread} in the last review received, which names {ids:?}");
        };
        let arguments = json!({"thread": id, "body": body, "addressed": addressed});
        if self.pi.is_some() {
            self.pi_reply(id, &arguments);
            return;
        }
        let mut fields = json!({
            "tool_name": LISTED_REPLY_TOOL,
            "mcp_server": {"name": TOOL_SERVER_LISTED, "source": "plugin"},
            "tool_input": arguments,
        });
        self.hook("PreToolUse", Some(LISTED_REPLY_TOOL), fields.clone());
        out(format!("● Reply {id}\r\n").as_bytes());
        let server = self
            .server
            .get_or_insert_with(|| ToolServer::start(&self.plugin_dirs, &self.cwd, &self.session));
        let call = json!({"name": REPLY_TOOL, "arguments": arguments});
        let (answer, failed) = match server.request("tools/call", call) {
            Ok(message) => match message.get("error") {
                Some(error) => (error.clone(), true),
                None => {
                    let result = message["result"].clone();
                    let failed = result["isError"] == true;
                    (result, failed)
                }
            },
            Err(why) => (json!({"message": why}), true),
        };
        self.log.did(
            "tool",
            json!({"arguments": arguments, "answer": answer, "received": self.input.received}),
        );
        let event = if failed {
            let why = answer["content"][0]["text"]
                .as_str()
                .or_else(|| answer["message"].as_str())
                .map_or_else(|| answer.to_string(), str::to_owned);
            fields["error"] = json!(why);
            "PostToolUseFailure"
        } else {
            fields["tool_response"] = answer["content"].clone();
            "PostToolUse"
        };
        self.hook(event, Some(LISTED_REPLY_TOOL), fields);
    }

    fn tool(
        &self,
        name: &str,
        input: Value,
        banner: &str,
        act: impl FnOnce() -> (Value, Option<String>),
    ) {
        self.hook(
            "PreToolUse",
            Some(name),
            json!({"tool_name": name, "tool_input": input}),
        );
        out(format!("● {banner}\r\n").as_bytes());
        let (response, error) = act();
        let mut fields = json!({"tool_name": name, "tool_input": input, "tool_response": response});
        let event = match error {
            Some(error) => {
                fields["error"] = json!(error);
                "PostToolUseFailure"
            }
            None => "PostToolUse",
        };
        self.hook(event, Some(name), fields);
    }

    fn edit(&self, path: &Path, text: &str) {
        let file = self.absolute(path);
        let before = std::fs::read_to_string(&file)
            .unwrap_or_else(|why| panic!("edit {}: {why}", file.display()));
        let lines = before.lines().count();
        let last = before.lines().last().unwrap_or_default();
        let (old, new) = if lines == 0 {
            (String::new(), text.to_owned())
        } else {
            (last.to_owned(), format!("{last}\n{text}"))
        };
        let input = json!({"file_path": file, "old_string": old, "new_string": new});
        self.tool("Edit", input, &format!("Edit {}", path.display()), || {
            add_line(&file, &before, text);
            let added: Vec<String> = text.split('\n').map(|line| format!("+{line}")).collect();
            let patch = json!({
                "oldStart": lines, "oldLines": 0,
                "newStart": lines + 1, "newLines": added.len(),
                "lines": added,
            });
            (json!({"filePath": file, "structuredPatch": [patch]}), None)
        });
    }

    fn shell(&self, command: &str) {
        let cwd = self.cwd.clone();
        self.tool(
            "Bash",
            json!({"command": command}),
            &format!("Bash {command}"),
            || {
                let status = sh(command)
                    .current_dir(cwd)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .unwrap_or_else(|why| panic!("run {command:?}: {why}"));
                let error = (!status.success())
                    .then(|| format!("Exit code {}", status.code().unwrap_or(-1)));
                (json!({}), error)
            },
        );
    }

    fn ask(&mut self, at: usize, path: &Path, text: &str) -> Ended {
        let file = self.absolute(path);
        let input = json!({"file_path": file, "content": text});
        let fields = json!({"tool_name": "Write", "tool_input": input});
        self.hook("PreToolUse", Some("Write"), fields.clone());
        self.hook("PermissionRequest", Some("Write"), fields);
        out(format!("Do you want to create {}? (y/n)\r\n", path.display()).as_bytes());
        let earlier = std::mem::take(&mut self.input.pending);
        let answer = self.answer();
        let later = std::mem::replace(&mut self.input.pending, earlier);
        self.input.pending.extend(later);
        match answer {
            Answer::Yes => {}
            Answer::No => {
                self.interrupt(at);
                return Ended::Interrupted;
            }
            Answer::Closed => return Ended::Closed,
        }
        let response = write_whole(&file, text);
        self.hook(
            "PostToolUse",
            Some("Write"),
            json!({"tool_name": "Write", "tool_input": input, "tool_response": response}),
        );
        Ended::Done
    }

    fn answer(&mut self) -> Answer {
        loop {
            match self.input.pending.pop_front() {
                Some(Key::Byte(b'y')) => return Answer::Yes,
                Some(Key::Byte(b'n') | Key::Esc) => return Answer::No,
                Some(Key::Byte(_)) => {}
                None if self.input.closed => return Answer::Closed,
                None => {
                    self.input.fill(&self.log, -1);
                }
            }
        }
    }

    fn hold(&mut self, at: usize, path: &Path) -> Ended {
        out(format!("● Hold {}\r\n", path.display()).as_bytes());
        let file = self.absolute(path);
        let in_turn = self.in_turn;
        let earlier = if in_turn {
            std::mem::take(&mut self.input.pending)
        } else {
            VecDeque::new()
        };
        let ended = loop {
            if file.exists() {
                break Ended::Done;
            }
            if in_turn {
                if self.input.take_esc() {
                    self.interrupt(at);
                    break Ended::Interrupted;
                }
                if let Some(line) = self.input.take_line() {
                    self.queue(&line);
                    continue;
                }
            }
            if self.input.closed {
                break Ended::Closed;
            }
            self.input.fill(&self.log, KEY_POLL_MS);
        };
        if in_turn {
            let later = std::mem::replace(&mut self.input.pending, earlier);
            self.input.pending.extend(later);
        }
        ended
    }

    fn flood(&self, at: usize, lines: u32) {
        let mut longest = Duration::ZERO;
        for k in 1..=lines {
            let began = Instant::now();
            out(format!("line {k}\r\n").as_bytes());
            longest = longest.max(began.elapsed());
        }
        self.log.did(
            "flooded",
            json!({"step": at, "longest_us": longest.as_micros() as u64}),
        );
    }

    fn pi_step(&mut self, at: usize, step: &Step) -> Ended {
        match step {
            Step::Turn => return self.turn(),
            Step::Read(path) => {
                let file = self.absolute(path);
                self.pi_tool(
                    "read",
                    json!({"path": path}),
                    &format!("Read {}", path.display()),
                    || {
                        std::fs::read(&file)
                            .unwrap_or_else(|why| panic!("read {}: {why}", file.display()));
                        (Value::Null, false)
                    },
                );
            }
            Step::Edit(path, text) => self.pi_edit(path, text),
            Step::Write(path, text) => {
                let file = self.absolute(path);
                let banner = format!("Write {}", path.display());
                self.pi_tool("write", json!({"path": path}), &banner, || {
                    write_whole(&file, text);
                    (Value::Null, false)
                });
            }
            Step::Shell(command) => {
                let cwd = self.cwd.clone();
                let banner = format!("Bash {command}");
                self.pi_tool("bash", json!({"command": command}), &banner, || {
                    let status = sh(command)
                        .current_dir(cwd)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .unwrap_or_else(|why| panic!("run {command:?}: {why}"));
                    (Value::Null, !status.success())
                });
            }
            Step::Ask(..) => unreachable!("a script with `ask` is refused before pi starts"),
            Step::Hold(path) => return self.hold(at, path),
            Step::HoldAtStart(_) => panic!("`hold-at-start` is a step at the head of a script"),
            Step::Flood(lines) => self.flood(at, *lines),
            Step::End => {
                self.in_turn = false;
                out(b"\xe2\x97\x8f Done\r\n");
                self.pi_report("agent_settled", json!({"aborted": false}));
            }
            Step::Mark => out(NO_PROGRESS),
            Step::Exit => {
                self.pi_report("session_shutdown", json!({"reason": "quit"}));
                self.restore();
                return Ended::Exit;
            }
            Step::SelfTurn => self.self_turn(),
            Step::Reply {
                thread,
                addressed,
                body,
            } => self.reply(*thread, *addressed, body),
        }
        Ended::Done
    }

    fn pi_message(&mut self, source: &str, streaming: Option<&str>) {
        let fields = json!({"source": source, "streamingBehavior": streaming});
        if let Some(answer) = self.pi_report("input", fields)
            && let Some(review) = Dialect::Pi.review_in(&answer)
        {
            self.review = Some(review);
        }
    }

    fn pi_tool(&self, name: &str, input: Value, banner: &str, act: impl FnOnce() -> (Value, bool)) {
        let call = self.pi_call_id();
        let shared = json!({"toolName": name, "toolCallId": call, "input": input});
        self.pi_report("tool_call", shared.clone());
        out(format!("\u{25cf} {banner}\r\n").as_bytes());
        let (details, failed) = act();
        let mut done = shared;
        done["isError"] = json!(failed);
        if !details.is_null() {
            done["details"] = details;
        }
        self.pi_report("tool_result", done);
    }

    /// Adds a line as pi's edit tool does and reports it as one hunk with no context; pi's own
    /// patch carries up to four lines of context, so pi names a wider range.
    fn pi_edit(&self, path: &Path, text: &str) {
        let file = self.absolute(path);
        let before = std::fs::read_to_string(&file)
            .unwrap_or_else(|why| panic!("edit {}: {why}", file.display()));
        let lines = before.lines().count();
        let added: Vec<String> = text.split('\n').map(|line| format!("+{line}")).collect();
        let name = path.display();
        let patch = format!(
            "--- {name}\n+++ {name}\n@@ -{lines},0 +{},{} @@\n{}\n",
            lines + 1,
            added.len(),
            added.join("\n")
        );
        let banner = format!("Edit {name}");
        self.pi_tool("edit", json!({"path": path}), &banner, || {
            add_line(&file, &before, text);
            (json!({"patch": patch}), false)
        });
    }

    fn pi_reply(&self, thread: &str, arguments: &Value) {
        out(format!("\u{25cf} Reply {thread}\r\n").as_bytes());
        let payload = json!({"toolCallId": self.pi_call_id(), "arguments": arguments});
        let (printed, killed) = self
            .pi_run("reply_to_thread", payload, true)
            .expect("a review was received, so a program was run");
        let answer = serde_json::from_slice(&printed).unwrap_or_else(|_| {
            let why = if killed {
                "the program did not end in time"
            } else {
                "the program printed no JSON"
            };
            json!({"message": why})
        });
        self.log.did(
            "tool",
            json!({"arguments": arguments, "answer": answer, "received": self.input.received}),
        );
    }

    fn pi_call_id(&self) -> String {
        let n = self.pi.as_ref().map_or(0, |pi| pi.calls.get()) + 1;
        if let Some(pi) = &self.pi {
            pi.calls.set(n);
        }
        format!("call_{n:04}")
    }

    fn pi_report(&self, event: &str, fields: Value) -> Option<Vec<u8>> {
        let (answer, killed) = self.pi_run(event, fields, false)?;
        self.log.did(
            "hook",
            json!({
                "event": event,
                "answer": hex(&answer),
                "killed": killed,
                "received": self.input.received,
            }),
        );
        Some(answer)
    }

    /// No shell between, so that the program's parent is the stand-in, as it is pi for the
    /// extension.
    fn pi_run(&self, event: &str, fields: Value, tool: bool) -> Option<(Vec<u8>, bool)> {
        let pi = self.pi.as_ref()?;
        let program = pi.program.as_ref()?;
        let mut payload = json!({
            "event": event,
            "sessionId": self.session,
            "sessionFile": pi.session_file,
            "cwd": self.cwd,
        });
        for (key, value) in fields.as_object().expect("an object") {
            payload[key] = value.clone();
        }
        let mut command = Command::new(program);
        if tool {
            command.arg("--tool");
        }
        command.current_dir(&self.cwd);
        default_hangup(&mut command);
        Some(run_process(command, payload.to_string().as_bytes()).unwrap_or_default())
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        self.cwd.join(path)
    }

    fn hook(&self, event: &str, tool: Option<&str>, fields: Value) -> Vec<Vec<u8>> {
        let commands = self.commands(event, tool);
        let mut answers = Vec::new();
        if commands.is_empty() {
            return answers;
        }
        let mut input = json!({
            "session_id": self.session,
            "hook_event_name": event,
            "cwd": self.cwd,
            "transcript_path": self.transcript,
        });
        for (key, value) in fields.as_object().expect("an object") {
            input[key] = value.clone();
        }
        let input = input.to_string();
        for command in commands {
            let (answer, killed) = run_hook(&command, input.as_bytes(), &self.cwd);
            self.log.did(
                "hook",
                json!({
                    "event": event,
                    "answer": hex(&answer),
                    "killed": killed,
                    "received": self.input.received,
                }),
            );
            answers.push(answer);
        }
        answers
    }

    fn commands(&self, event: &str, tool: Option<&str>) -> Vec<String> {
        let mut commands = Vec::new();
        for plugin in &self.plugins {
            let Some(groups) = plugin[event].as_array() else {
                continue;
            };
            for group in groups {
                let matches = match (group["matcher"].as_str(), tool) {
                    (Some(matcher), Some(tool)) if !matcher.is_empty() && matcher != "*" => {
                        matcher.split('|').any(|name| name == tool)
                    }
                    _ => true,
                };
                if !matches {
                    continue;
                }
                for hook in group["hooks"].as_array().into_iter().flatten() {
                    if hook["type"] == "command"
                        && let Some(command) = hook["command"].as_str()
                    {
                        commands.push(command.to_owned());
                    }
                }
            }
        }
        commands
    }
}

fn add_line(file: &Path, before: &str, text: &str) {
    let mut after = before.to_owned();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str(text);
    after.push('\n');
    std::fs::write(file, after).unwrap_or_else(|why| panic!("edit {}: {why}", file.display()));
}

fn value_of<'a>(args: &'a [String], flags: &[&str]) -> Option<&'a String> {
    let at = args.iter().position(|arg| flags.contains(&arg.as_str()))?;
    args.get(at + 1)
}

fn extensions_found(home: &Path, args: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(text) = std::fs::read_to_string(home.join(PI_SETTINGS))
        && let Ok(Value::Object(settings)) =
            serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
        && let Some(listed) = settings.get("extensions").and_then(Value::as_array)
    {
        found.extend(listed.iter().filter_map(Value::as_str).map(PathBuf::from));
    }
    for (at, arg) in args.iter().enumerate() {
        if matches!(arg.as_str(), "-e" | "--extension")
            && let Some(path) = args.get(at + 1)
        {
            found.push(PathBuf::from(path));
        }
    }
    found.retain(|path| path.is_file());
    found
}

fn out(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(bytes);
    let _ = stdout.flush();
}

fn write_whole(file: &Path, text: &str) -> Value {
    let existed = file.exists();
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)
            .unwrap_or_else(|why| panic!("write {}: {why}", file.display()));
    }
    std::fs::write(file, text).unwrap_or_else(|why| panic!("write {}: {why}", file.display()));
    json!({
        "filePath": file,
        "type": if existed { "update" } else { "create" },
        "structuredPatch": [],
    })
}

/// No echo, line editing, key signals or flow control, so that Esc and Ctrl-C reach it as bytes.
/// Output processing stays on but for `\n` to `\r\n`: the stand-in writes `\r\n` itself.
fn raw_terminal() -> Option<libc::termios> {
    let mut settings = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: a place for the settings, and descriptor 0, whatever it is.
    if unsafe { libc::tcgetattr(0, settings.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `tcgetattr` filled it in.
    let saved = unsafe { settings.assume_init() };
    let mut raw = saved;
    raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG | libc::IEXTEN);
    raw.c_iflag &= !(libc::IXON | libc::ICRNL);
    raw.c_oflag &= !libc::ONLCR;
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    // SAFETY: standard input and settings `tcgetattr` gave, changed in flags.
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) };
    Some(saved)
}

fn new_session_id() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .expect("read /dev/urandom");
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let digits = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &digits[..8],
        &digits[8..12],
        &digits[12..16],
        &digits[16..20],
        &digits[20..]
    )
}

fn plugin_dirs() -> Vec<PathBuf> {
    let Some(dirs) = std::env::var_os("CLAUDE_CODE_PLUGIN_DIRS") else {
        return Vec::new();
    };
    dirs.to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn read_plugins(dirs: &[PathBuf]) -> Vec<Value> {
    let mut plugins = Vec::new();
    for dir in dirs {
        let file = dir.join("hooks/hooks.json");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|why| panic!("the hooks {}: {why}", file.display()));
        plugins.push(parsed["hooks"].clone());
    }
    plugins
}

fn review_in(answer: &[u8]) -> Option<String> {
    let answer: Value = serde_json::from_slice(answer).ok()?;
    let output = &answer["hookSpecificOutput"];
    if output["hookEventName"] != "UserPromptSubmit" {
        return None;
    }
    let mut text = output["additionalContext"].as_str()?.to_owned();
    let delivered = text.clone();
    for word in delivered.split_whitespace() {
        let bare = word.trim_matches(|c: char| ".,;:!?)(\"'`".contains(c));
        for candidate in [word, bare] {
            if Path::new(candidate).is_absolute()
                && let Ok(content) = std::fs::read_to_string(candidate)
            {
                text.push('\n');
                text.push_str(&content);
                break;
            }
        }
    }
    Some(text)
}

/// The ids of the threads a review names, in order: the word after `## Thread ` at the start of a
/// line outside a fenced block, where the review puts code that may look like a heading.
fn thread_ids(review: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut fence: Option<usize> = None;
    for line in review.lines() {
        let ticks = line.len() - line.trim_start_matches('`').len();
        if ticks >= 3 {
            match fence {
                None => {
                    fence = Some(ticks);
                    continue;
                }
                // Only a run at least as long as the opening one, with nothing after it, closes.
                Some(open) if ticks >= open && line.trim_end().len() == ticks => {
                    fence = None;
                    continue;
                }
                Some(_) => {}
            }
        }
        if fence.is_some() {
            continue;
        }
        if let Some(id) = line
            .strip_prefix("## Thread ")
            .and_then(|rest| rest.split_whitespace().next())
        {
            ids.push(id.to_owned());
        }
    }
    ids
}

struct ToolServer {
    child: Child,
    input: ChildStdin,
    /// Filled by a reader thread, so a wait for an answer is a channel wait with a deadline.
    messages: mpsc::Receiver<Value>,
    next_id: u64,
}

impl ToolServer {
    fn start(dirs: &[PathBuf], cwd: &Path, session: &str) -> ToolServer {
        let (dir, command, args) = dirs
            .iter()
            .find_map(|dir| {
                let text = std::fs::read_to_string(dir.join(".mcp.json")).ok()?;
                let file: Value = serde_json::from_str(&text).ok()?;
                let server = &file["mcpServers"][TOOL_SERVER_NAME];
                let args = server["args"].as_array().into_iter().flatten();
                Some((
                    dir,
                    server["command"].as_str()?.to_owned(),
                    args.filter_map(|arg| arg.as_str().map(str::to_owned))
                        .collect::<Vec<_>>(),
                ))
            })
            .unwrap_or_else(|| {
                panic!("no plugin folder of {dirs:?} has a .mcp.json naming a server `{TOOL_SERVER_NAME}`")
            });
        let root = dir.to_string_lossy();
        let expand = |text: &str| text.replace("${CLAUDE_PLUGIN_ROOT}", &root);
        let mut server = Command::new(expand(&command));
        server
            .args(args.iter().map(|arg| expand(arg)))
            .current_dir(cwd)
            .env("CLAUDE_CODE_SESSION_ID", session)
            .env("CLAUDECODE", "1")
            .env("CLAUDE_PROJECT_DIR", cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // As for a hook: an ignored signal stays ignored across `exec`.
        // SAFETY: `signal` is async-signal-safe, and this runs between `fork` and `exec`.
        unsafe {
            server.pre_exec(|| {
                libc::signal(libc::SIGHUP, libc::SIG_DFL);
                Ok(())
            })
        };
        let mut child = server
            .spawn()
            .unwrap_or_else(|why| panic!("start the tool server {command:?}: {why}"));
        let input = child.stdin.take().expect("a piped standard input");
        let output = child.stdout.take().expect("a piped standard output");
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { return };
                if let Ok(message) = serde_json::from_str::<Value>(&line)
                    && sender.send(message).is_err()
                {
                    return;
                }
            }
        });
        let mut server = ToolServer {
            child,
            input,
            messages,
            next_id: 0,
        };
        let initialize = json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"roots": {"listChanged": true}},
            "clientInfo": {"name": "claude-code", "title": "Claude Code", "version": "0"},
        });
        for (method, params) in [("initialize", initialize), ("tools/list", json!({}))] {
            let answer = server
                .request(method, params)
                .unwrap_or_else(|why| panic!("the tool server did not answer `{method}`: {why}"));
            assert!(
                answer.get("error").is_none(),
                "the tool server refused `{method}`: {answer}"
            );
            if method == "initialize" {
                server.notify("notifications/initialized");
            }
        }
        server
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        writeln!(self.input, "{message}")
            .and_then(|()| self.input.flush())
            .map_err(|why| format!("the tool server takes no message: {why}"))
    }

    fn notify(&mut self, method: &str) {
        let _ = self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + TOOL_WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self.messages.recv_timeout(left).map_err(|why| match why {
                mpsc::RecvTimeoutError::Timeout => {
                    format!("the tool server said nothing in {TOOL_WAIT:?}")
                }
                mpsc::RecvTimeoutError::Disconnected => "the tool server ended".to_owned(),
            })?;
            if message["id"] == id {
                return Ok(message);
            }
        }
    }
}

impl Drop for ToolServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn sh(command: &str) -> Command {
    let mut sh = Command::new("/bin/sh");
    sh.arg("-c").arg(command);
    default_hangup(&mut sh);
    sh
}

fn default_hangup(command: &mut Command) {
    // SAFETY: `signal` is async-signal-safe, and this runs between `fork` and `exec`.
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            Ok(())
        })
    };
}

fn run_hook(command: &str, input: &[u8], cwd: &Path) -> (Vec<u8>, bool) {
    let mut hook = sh(command);
    hook.current_dir(cwd)
        .env("CLAUDE_PID", std::process::id().to_string())
        .env("CLAUDECODE", "1")
        .env("CLAUDE_PROJECT_DIR", cwd);
    run_process(hook, input).unwrap_or_else(|why| panic!("run the hook {command:?}: {why}"))
}

fn run_process(mut command: Command, input: &[u8]) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A group of its own, so that the kill below reaches what the command started.
        .process_group(0)
        .spawn()?;
    let pid = child.id();
    let mut stdin = child.stdin.take().expect("a piped standard input");
    let mut stdout = child.stdout.take().expect("a piped standard output");
    let input = input.to_vec();
    // A hook that reads none of its input must not hold the stand-in up: the input goes in from a
    // thread of its own.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let reader = std::thread::spawn(move || {
        let mut answer = Vec::new();
        let _ = stdout.read_to_end(&mut answer);
        answer
    });
    let (ended, wait) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let _ = child.wait();
        let _ = ended.send(());
    });
    let killed = wait.recv_timeout(HOOK_LIMIT).is_err();
    if killed {
        // SAFETY: a negative process id names the group the child leads.
        unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
    }
    let _ = waiter.join();
    let _ = writer.join();
    Ok((reader.join().unwrap_or_default(), killed))
}
