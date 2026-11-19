//! The stand-in agent CLI of the system tests: the program `standin-agent`, and the script
//! a test hands it.
//!
//! It stands in for the agent CLI itself, and for nothing of the app. A test puts it on
//! the search path of its world under the names `claude`, `pi` and `otheragent`, and starts
//! it as a user starts a CLI: from a launcher of the app, or by its name typed in a shell
//! of the app. It runs in the app's terminal, reads the keys typed there, and writes there
//! what such a CLI writes.
//!
//! Started as `claude` it behaves as Claude Code does towards the app's pieces: it reads
//! the plugin folders `CLAUDE_CODE_PLUGIN_DIRS` names, and for each event of its script
//! runs every command `hooks/hooks.json` of those folders gives for that event, through
//! `/bin/sh -c` in the folder it was started in, with the event's input on its standard
//! input, as the shapes recorded from the real Claude Code have it, and waits for it. The
//! file is Claude Code's plugin format, as the crate `yaar-adapter-claude` shows it:
//! `{"hooks": {"<event>": [{"matcher": "<tools parted by |>", "hooks": [{"type":
//! "command", "command": "<command>"}]}]}}`, a group with no `matcher` being for every
//! tool. The hook's environment is the stand-in's own, with `CLAUDE_PID` (its process
//! id), `CLAUDECODE=1` and `CLAUDE_PROJECT_DIR` (the folder it was started in) added, as
//! the recordings show Claude Code's hooks get. A hook that has not ended within 10 s is
//! killed and written down so. That command is the app's own hook
//! program, which reports to the Keeper over its socket: the stand-in knows neither the
//! Keeper, nor the Link, nor the socket. As `otheragent` it runs no hook at all: a CLI the
//! app does not support, that only edits files. As `pi` it refuses to start in slice 4,
//! saying the pi dialect comes in slice 7. With [`IGNORE_PIECES`] set it runs no hook
//! either, as a CLI started around the app's pieces.
//!
//! Its script is a file, named by [`SCRIPT_VARIABLE`], of [`Step`]s, one to a line. It
//! decides nothing: it follows the script and the keys. What reached it, every byte read
//! from its terminal, every hook's answer and its own arguments, it writes down in the
//! file [`LOG_VARIABLE`] names: the agent's side of a test, read with [`StandinLog`].
//!
//! Its terminal. It turns echo, line editing, the signals of keys, flow control and the
//! change of a carriage return into a line feed off, so that Esc and Ctrl-C reach it as
//! bytes. Output processing stays on but for the change of a line feed into a carriage
//! return and a line feed: the stand-in ends each line with `\r\n` itself, so that each
//! line starts at the left and the bytes it writes, [`INTERRUPT_MARK`] among them, are the
//! bytes the terminal passes on. Every step writes a line of its own in the terminal, after the
//! hooks before it and before the hooks after it: `● Read <path>`, `● Edit <path>`,
//! `● Write <path>`, `● Bash <command>`, `● Hold <path>`, and `● Done` at the end of a
//! turn. A test that reads the screen looks for those lines and for [`PROMPT`].

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The variable that names the stand-in's script.
pub const SCRIPT_VARIABLE: &str = "STANDIN_SCRIPT";

/// The variable that names the file the stand-in writes down what reached it in.
pub const LOG_VARIABLE: &str = "STANDIN_LOG";

/// The variable that, set to `1`, makes the stand-in run none of the app's pieces.
pub const IGNORE_PIECES: &str = "STANDIN_IGNORE_PIECES";

/// What the stand-in prints when it is ready for the user's next message, at the start
/// of a line: a test that types a message waits for it first, and looks for the stand-in's
/// own words, never for a shell's prompt.
pub const PROMPT: &str = "standin> ";

/// One step of a stand-in's script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Prints [`PROMPT`] and waits for a line typed at it, ended by Enter: the user's
    /// message. Its submission starts a turn: `UserPromptSubmit`, and then the progress
    /// sequence of a turn that works, `ESC ] 9 ; 4 ; 3 ; BEL`, as Claude Code writes it. A
    /// line typed while a turn runs is held until the turn ends, and then starts the next
    /// without a prompt printed, as both CLIs do.
    Turn,
    /// Reads a file of the worktree with its read tool: `PreToolUse` and `PostToolUse` of
    /// `Read`, the file really read in between.
    Read(PathBuf),
    /// Adds a line to a file with its edit tool: `PreToolUse` and `PostToolUse` of
    /// `Edit`, the file really changed in between.
    Edit(PathBuf, String),
    /// Writes a file whole with its write tool, a new file among them: `PreToolUse` and
    /// `PostToolUse` of `Write`.
    Write(PathBuf, String),
    /// Runs a shell command in the worktree, through `/bin/sh -c`: `PreToolUse` of
    /// `Bash`, the command, `PostToolUse` of `Bash`.
    Shell(String),
    /// Asks for permission to write a file whole: `PreToolUse` and `PermissionRequest` of
    /// `Write`, then the line `Do you want to create <path>? (y/n)` printed in the
    /// terminal, and a wait for one key. `y` writes the file and gives `PostToolUse`; `n`
    /// or Esc denies it, prints the mark Claude Code prints, [`INTERRUPT_MARK`], gives no
    /// event, and ends the turn as interrupted. Any other byte is written down and
    /// ignored, and the wait goes on.
    Ask(PathBuf, String),
    /// Holds the turn open until the file exists: a test that wants to act while the
    /// agent works makes the file when it is done.
    Hold(PathBuf),
    /// Writes this many numbered lines into its terminal as fast as it can, and writes
    /// down in its log the longest a single write waited.
    Flood(u32),
    /// Ends the turn: prints `● Done`, runs the `Stop` hooks and waits for them to return,
    /// and only then prints the progress sequence that says no progress,
    /// `ESC ] 9 ; 4 ; 0 ; BEL`, as Claude Code does. Esc pressed during a turn, before
    /// this step, prints [`INTERRUPT_MARK`], gives no event, and ends the turn as
    /// interrupted: the script goes on after the next `end`.
    End,
    /// Prints the progress sequence that says no progress, `ESC ] 9 ; 4 ; 0 ; BEL`, alone,
    /// and runs no hook: what any program may print, as a test's `otheragent` does to show
    /// that such a sequence in a terminal with no session ends no turn.
    Mark,
    /// Ends the session, `SessionEnd` with `reason` `prompt_input_exit`, puts the
    /// terminal back as it found it, and ends the program with 0.
    Exit,
}

/// The mark Claude Code prints when a turn stops: the progress sequence that says no
/// progress, as the recordings hold it, then the words it prints on an interrupt. Claude
/// Code wraps those words in moves of the cursor and colours, in a write of their own;
/// the stand-in leaves those out, since the Keeper reads the sequence alone.
pub const INTERRUPT_MARK: &[u8] =
    b"\x1b]9;4;0;\x07Interrupted \xc2\xb7 What should Claude do instead?\r\n";

/// A stand-in's script.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script(pub Vec<Step>);

impl Script {
    /// The script as the file the stand-in reads: one step to a line, a keyword and its
    /// words, a text to the end of the line with `\n` for a line end inside it.
    pub fn text(&self) -> String {
        self.0.iter().map(|step| step.line() + "\n").collect()
    }

    /// Reads a script from the text [`Script::text`] writes. Panics, naming the line, on
    /// one it cannot read.
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
    /// The step as a line of a script file.
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
            Step::Flood(lines) => format!("flood {lines}"),
            Step::End => "end".into(),
            Step::Mark => "mark".into(),
            Step::Exit => "exit".into(),
        }
    }

    /// The step a line of a script file stands for, or why it stands for none.
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
            "flood" => rest
                .parse()
                .map(Step::Flood)
                .map_err(|_| "`flood` needs a number of lines".to_owned()),
            "end" => alone(Step::End),
            "mark" => alone(Step::Mark),
            "exit" => alone(Step::Exit),
            _ => Err(format!("no step is called `{keyword}`")),
        }
    }
}

/// A text as one line of a script: `\` for a backslash, `\n` for a line end and `\r` for
/// a carriage return, which reading a file by lines would otherwise take for a line end.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// The text a line of a script holds, undoing [`escape`].
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

/// One thing a stand-in did, in the order it did it, with how many bytes it had read
/// from its terminal by then: a test asserts on positions in this list, and reads what
/// reached the stand-in between two of them from `StandinLog::received`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    /// Step `step` of the script, by its index, began.
    Began {
        /// The step's index in the script.
        step: usize,
        /// The bytes read so far.
        received: usize,
    },
    /// Step `step` is done.
    Done {
        /// The step's index in the script.
        step: usize,
        /// The bytes read so far.
        received: usize,
    },
    /// A hook ran, and what it printed.
    Hook {
        /// The hook's event, as `PermissionRequest`.
        event: String,
        /// What it printed on its standard output.
        answer: Vec<u8>,
        /// Whether it was killed for not ending within 10 s.
        killed: bool,
        /// The bytes read so far.
        received: usize,
    },
    /// The turn was interrupted by Esc, or a dialog denied, during step `step`.
    Interrupted {
        /// The step's index in the script.
        step: usize,
        /// The bytes read so far.
        received: usize,
    },
    /// A [`Step::Flood`] ended: the longest a single write of it waited.
    Flooded {
        /// The step's index in the script.
        step: usize,
        /// The longest write, in microseconds.
        longest_us: u64,
    },
    /// The script ran out with no `exit`: the stand-in reads its terminal on until it
    /// closes.
    ScriptDone {
        /// The bytes read so far.
        received: usize,
    },
    /// The program ends, with this status.
    Exited {
        /// Its exit status.
        status: i32,
    },
}

/// What reached a stand-in, as it wrote it down.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StandinLog {
    /// Its process id.
    pub pid: u32,
    /// Its arguments, without its own name.
    pub args: Vec<String>,
    /// The CLI's own session id it gave itself, or the one it was told to resume.
    pub session: String,
    /// Every byte it read from its terminal, in order.
    pub received: Vec<u8>,
    /// What it did, in order.
    pub did: Vec<Did>,
}

impl StandinLog {
    /// Reads the log the stand-in wrote at `path`; an empty log when the file is not
    /// there yet. A last line the stand-in has not finished writing is left out.
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

    /// Adds what one line of the log says.
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

/// The longest a hook may take before the stand-in kills it.
const HOOK_LIMIT: Duration = Duration::from_secs(10);

/// How long a wait for a key looks at the file it also waits for, in milliseconds.
const KEY_POLL_MS: i32 = 20;

/// The progress sequence of a turn that works, and the one that says no progress.
const WORKING: &[u8] = b"\x1b]9;4;3;\x07";
const NO_PROGRESS: &[u8] = b"\x1b]9;4;0;\x07";

/// The stand-in's program: what `standin-agent` runs, given its name as it was started,
/// its arguments, and its environment through the process. Gives its exit status.
///
/// When its script runs out with no [`Step::Exit`], it runs no hook and ends no session:
/// it reads its terminal on, writing down every byte, until the terminal closes, and then
/// ends with 0. Only `exit` ends the session; a stand-in whose terminal ends, or that is
/// killed, reports no end, as Claude Code does not.
///
/// A name other than `pi` and `otheragent` acts as `claude`: the stand-in is run under that
/// name where a test does not say.
pub fn run(name: &str, args: &[String]) -> i32 {
    // In a shell of the app the terminal is the stand-in's controlling terminal, and its
    // closing sends a hangup: that is a close to read and end on, not a death.
    // SAFETY: ignoring a signal is always allowed.
    unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
    if name == "pi" {
        eprintln!("the pi dialect of the stand-in comes with slice 7");
        return 2;
    }
    let script = match std::env::var_os(SCRIPT_VARIABLE) {
        Some(file) => {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|why| panic!("the script {}: {why}", Path::new(&file).display()));
            Script::parse(&text)
        }
        None => Script::default(),
    };
    let pieces =
        name != "otheragent" && std::env::var(IGNORE_PIECES).map_or(true, |value| value != "1");
    let mut standin = Standin::start(args, pieces);
    let status = standin.go(&script);
    standin.restore();
    status
}

/// One key read from the terminal. Esc is a read of the single byte `0x1b`: a longer
/// read that holds it is a sequence of keys, and its bytes are bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Byte(u8),
    Esc,
}

/// What the stand-in reads from its terminal, and what it has read and not yet used.
#[derive(Default)]
struct Input {
    /// How many bytes have been read in all.
    received: usize,
    pending: VecDeque<Key>,
    /// Whether the terminal has closed.
    closed: bool,
}

impl Input {
    /// Reads what the terminal has, waiting at most `wait_ms` (`-1` for as long as it
    /// takes), and writes it to the log. Whether bytes came.
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

    /// Takes the first Esc read and not used, if any.
    fn take_esc(&mut self) -> bool {
        match self.pending.iter().position(|key| *key == Key::Esc) {
            Some(at) => {
                self.pending.remove(at);
                true
            }
            None => false,
        }
    }

    /// Takes the keys up to and with the first Enter as a line, without the Esc among
    /// them, when a whole line has been read.
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

/// The file the stand-in writes down what it did in, a JSON value to a line, each line
/// written whole at once.
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

/// How a step ended.
enum Ended {
    Done,
    /// Esc, or a dialog denied: the turn is over, interrupted.
    Interrupted,
    /// The terminal closed.
    Closed,
    /// The session was ended by the script.
    Exit,
}

/// What the user answered to a dialog.
enum Answer {
    Yes,
    No,
    /// The terminal closed with no answer.
    Closed,
}

/// A stand-in agent in the middle of its script.
struct Standin {
    log: Log,
    input: Input,
    /// The terminal's settings as they were, kept to put them back.
    saved: Option<libc::termios>,
    /// The `hooks` object of each plugin folder's `hooks/hooks.json`, in folder order.
    plugins: Vec<Value>,
    session: String,
    resumed: bool,
    cwd: PathBuf,
    transcript: PathBuf,
    /// Whether a turn is open: a message was taken and `end` has not come.
    in_turn: bool,
}

impl Standin {
    fn start(args: &[String], pieces: bool) -> Standin {
        let saved = raw_terminal();
        let log = match std::env::var_os(LOG_VARIABLE) {
            Some(file) => Log(Some(File::create(&file).unwrap_or_else(|why| {
                panic!("the log {}: {why}", Path::new(&file).display())
            }))),
            None => Log(None),
        };
        let resumed_id = args
            .iter()
            .position(|arg| arg == "--resume")
            .and_then(|at| args.get(at + 1));
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
        log.line(&json!({"session": session}));
        let plugins = if pieces { read_plugins() } else { Vec::new() };
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
        }
    }

    fn restore(&self) {
        if let Some(saved) = &self.saved {
            // SAFETY: standard input and settings `tcgetattr` gave.
            unsafe { libc::tcsetattr(0, libc::TCSANOW, saved) };
        }
    }

    /// Runs the script, and what follows it. Gives the exit status.
    fn go(&mut self, script: &Script) -> i32 {
        let source = if self.resumed { "resume" } else { "startup" };
        self.hook("SessionStart", None, json!({"source": source}));
        let steps = &script.0;
        // After an interruption the steps up to and with the next `end` are skipped.
        let mut skipping = false;
        let mut at = 0;
        while at < steps.len() {
            if self.input.closed {
                return self.closed();
            }
            let step = &steps[at];
            self.input.fill(&self.log, 0);
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

    /// The program's end when its terminal closed.
    fn closed(&self) -> i32 {
        self.log.did("exited", json!({"status": 0}));
        0
    }

    fn interrupt(&mut self, step: usize) {
        out(INTERRUPT_MARK);
        self.log.did(
            "interrupted",
            json!({"step": step, "received": self.input.received}),
        );
        self.in_turn = false;
    }

    fn step(&mut self, at: usize, step: &Step) -> Ended {
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
        }
    }

    /// Asks for a message when none was typed ahead, and starts the turn with it.
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
        self.in_turn = true;
        self.hook("UserPromptSubmit", None, json!({"prompt": line}));
        out(WORKING);
        Ended::Done
    }

    /// A tool of the agent: `PreToolUse`, its line, the action, and `PostToolUse` of what
    /// it gave, or `PostToolUseFailure` when it names an error.
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

    /// Adds a line to the file, the last line of it the text the edit replaces.
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
            let mut after = before.clone();
            if !after.is_empty() && !after.ends_with('\n') {
                after.push('\n');
            }
            after.push_str(text);
            after.push('\n');
            std::fs::write(&file, after)
                .unwrap_or_else(|why| panic!("edit {}: {why}", file.display()));
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
                let status = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(command)
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

    /// Asks for permission to write a file whole, and waits for the user's key.
    fn ask(&mut self, at: usize, path: &Path, text: &str) -> Ended {
        let file = self.absolute(path);
        let input = json!({"file_path": file, "content": text});
        let fields = json!({"tool_name": "Write", "tool_input": input});
        self.hook("PreToolUse", Some("Write"), fields.clone());
        self.hook("PermissionRequest", Some("Write"), fields);
        out(format!("Do you want to create {}? (y/n)\r\n", path.display()).as_bytes());
        // Only keys read after the dialog is up answer it. What was typed before is not an
        // answer to a question not yet asked: a line typed during the turn is the next
        // turn's message, and it stays for it.
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

    /// Reads keys until `y`, `n` or Esc; any other byte is ignored.
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

    /// Holds the turn open until the file is there, with Esc to interrupt.
    fn hold(&mut self, at: usize, path: &Path) -> Ended {
        out(format!("● Hold {}\r\n", path.display()).as_bytes());
        let file = self.absolute(path);
        while !file.exists() {
            if self.input.take_esc() {
                self.interrupt(at);
                return Ended::Interrupted;
            }
            if self.input.closed {
                return Ended::Closed;
            }
            self.input.fill(&self.log, KEY_POLL_MS);
        }
        Ended::Done
    }

    /// Writes numbered lines as fast as the terminal takes them.
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

    fn absolute(&self, path: &Path) -> PathBuf {
        self.cwd.join(path)
    }

    /// Runs every command the plugins give for the event, each to its end, and writes
    /// down what each printed. `tool` is the tool's name for an event that has one.
    fn hook(&self, event: &str, tool: Option<&str>, fields: Value) {
        let commands = self.commands(event, tool);
        if commands.is_empty() {
            return;
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
        }
    }

    /// The commands the plugins give for an event, in the order of the folders. A group
    /// with a `matcher` is for the tools it names, parted by `|`; an event that has no
    /// tool has no name to match, and every group runs.
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

/// Writes bytes to the terminal as they are, and lets the terminal have them at once. A
/// terminal that is gone is not a fault of the stand-in: what it could not write is lost.
fn out(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(bytes);
    let _ = stdout.flush();
}

/// Writes a file whole, its folders made, and gives the tool response of it.
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

/// Sets the terminal as the stand-in wants it: no echo, no line editing, no signals from
/// keys, no flow control and no change of Enter into a line feed, so that Esc, Ctrl-C and
/// the rest reach it as the bytes they are. Output processing stays on; only the change of
/// `\n` into `\r\n` goes, and the stand-in writes `\r\n` itself, so that what it writes
/// reaches the terminal byte for byte. Gives the settings as they were, or none when its
/// standard input is no terminal.
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

/// A session id as Claude Code makes one: 16 random bytes as a version 4 UUID.
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

/// The `hooks` object of `hooks/hooks.json` of each folder `CLAUDE_CODE_PLUGIN_DIRS`
/// names, in order. A folder with no such file has no hooks.
fn read_plugins() -> Vec<Value> {
    let Some(dirs) = std::env::var_os("CLAUDE_CODE_PLUGIN_DIRS") else {
        return Vec::new();
    };
    let dirs = dirs.to_string_lossy().into_owned();
    let mut plugins = Vec::new();
    for dir in dirs.split(':').filter(|dir| !dir.is_empty()) {
        let file = Path::new(dir).join("hooks/hooks.json");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|why| panic!("the hooks {}: {why}", file.display()));
        plugins.push(parsed["hooks"].clone());
    }
    plugins
}

/// Runs one hook command as Claude Code does: through `/bin/sh -c` in `cwd`, with the
/// input on its standard input, its standard output kept. A hook still running after
/// [`HOOK_LIMIT`] is killed with everything it started. Gives what it printed and whether
/// it was killed.
fn run_hook(command: &str, input: &[u8], cwd: &Path) -> (Vec<u8>, bool) {
    use std::io::Read;
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("CLAUDE_PID", std::process::id().to_string())
        .env("CLAUDECODE", "1")
        .env("CLAUDE_PROJECT_DIR", cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A group of its own, so that the kill below reaches what the hook started.
        .process_group(0)
        .spawn()
        .unwrap_or_else(|why| panic!("run the hook {command:?}: {why}"));
    let pid = child.id();
    let mut stdin = child.stdin.take().expect("a piped standard input");
    let mut stdout = child.stdout.take().expect("a piped standard output");
    let input = input.to_vec();
    // A hook that reads none of its input must not hold the stand-in up: the input goes
    // in from a thread of its own, and a closed pipe is no fault.
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
    (reader.join().unwrap_or_default(), killed)
}
