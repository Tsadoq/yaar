//! The stand-in language server of the tests (ADR-033 § 3): a server for an invented
//! language, the stand-in language, whose files end in `.sil`. It sits with the Navigator's
//! component tests and is started by them, by the review pane's and the Window's tests
//! through a registry row of their own, and by nothing the app ships.
//!
//! It follows what its command line says and decides nothing. It speaks the Language
//! Server Protocol on its standard input and output, as `yaar_navigator::lsp` frames it,
//! and knows the stand-in language only as this: a name is a run of letters, digits and
//! `_`; a line `def <name>` defines `<name>`.
//!
//! # Its command line
//!
//! | Argument | What it does |
//! | --- | --- |
//! | `--version` | prints `standin-lsp 1` and ends with 0, or with `--fail-trial` prints `standin-lsp: broken` on its standard error and ends with 3 |
//! | `--fail-trial` | see `--version`; alone, nothing |
//! | `--ready <answer\|progress\|notification>` | which readiness it shows, for a row of that rule: `answer` (the default) needs nothing; `progress` sends `window/workDoneProgress/create` for the token `standin`, then `$/progress` `begin`, then, once ready, `end`; `notification` sends `standin/status` with `{"ready": false}`, then, once ready, `{"ready": true}` |
//! | `--ready-when <file>` | it is ready only once `<file>` exists, looked for every 10 ms after `initialized`; without it, at `initialized` |
//! | `--fail-initialize` | answers `initialize` with the error -32603 and ends with 1 |
//! | `--exit-after-ready` | ends with 2 right after it became ready, as a server that dies |
//! | `--ask-configuration <section>` | after `initialized`, asks `workspace/configuration` for `section` and logs the answer |
//! | `--log <file>` | appends one JSON object a line to `<file>`, below; `--version` writes none |
//!
//! Any other argument, a missing value, or a value a row above does not have, ends with 64 and
//! a sentence on its standard error: a mistyped command line of a test does not pass as
//! another one.
//! # Its log
//!
//! Each line one object with `event` and what goes with it: `start` with `pid`, `cwd`,
//! `args` (every argument but the first) and `env` (the names and values of `HOME`,
//! `TMPDIR`, `XDG_CACHE_HOME` and every variable whose name starts with `STANDIN_`, where
//! set); `initialize` with `rootUri`, `initializationOptions` and `capabilities` as
//! received; `configuration` with `result`, the answer received, as the client sent it;
//! `request` with `method` and `id`
//! for every request received; `notification` with `method` for every notification
//! received, and with `changes`, as received, for `workspace/didChangeWatchedFiles`; `ready`;
//! `exit` with its code. A line is written before what it tells is answered, so that a test
//! that reads the log after an answer finds the line.
//!
//! # What it answers
//!
//! - `initialize`: `{"capabilities": {"hoverProvider": true, "definitionProvider": true,
//!   "referencesProvider": true, "textDocumentSync": 1}, "serverInfo": {"name":
//!   "standin-lsp"}}`.
//! - `textDocument/hover` before it is ready: null. Once ready: the name at the position,
//!   in the text the client last opened or changed for that document, else the file on
//!   disk: `{"contents": {"kind": "plaintext", "value": "standin: <name>"}}`; null where
//!   no name is at the position.
//! - `textDocument/definition` once ready: every line `def <name>` of every `.sil` file
//!   under the root, in order of path, then line, for the name at the position, as `Location`s
//!   whose range is the name; before it is ready, `[]`.
//! - `textDocument/references` once ready: every whole-word occurrence of the name in
//!   every `.sil` file under the root, by path, then line, then column; before, `[]`.
//! - `shutdown`: null; `exit`: it ends with 0 after a `shutdown`, else with 1. At the end
//!   of its input it ends with 0.
//! - Any other request: the error -32601. Positions are counted in UTF-16 units.
//!
//! The name at a position is read from the same text as hover's, whichever of the three asks
//! it is. Files under the root are read as they are on disk; folders whose name starts with
//! `.` are not entered. A line `def <name>` is a line whose first word, after any white
//! space, is `def`, followed by white space and the name; what follows the name is not read.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use yaar_navigator::lsp::{
    byte_column, file_uri, path_of_uri, read_frame, utf16_column, write_frame,
};

/// How often `--ready-when` looks for its file.
const LOOK_EVERY: Duration = Duration::from_millis(10);

/// The id of the request the stand-in makes for `--ready progress`.
const PROGRESS_REQUEST: u64 = 1;

/// The id of the request the stand-in makes for `--ask-configuration`.
const CONFIGURATION_REQUEST: u64 = 2;

/// Which readiness it shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Readiness {
    Answer,
    Progress,
    Notification,
}

/// What the command line says.
struct Options {
    version: bool,
    fail_trial: bool,
    ready: Readiness,
    ready_when: Option<PathBuf>,
    fail_initialize: bool,
    exit_after_ready: bool,
    ask_configuration: Option<String>,
    log: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        version: false,
        fail_trial: false,
        ready: Readiness::Answer,
        ready_when: None,
        fail_initialize: false,
        exit_after_ready: false,
        ask_configuration: None,
        log: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |what: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs {what}"))
        };
        match arg.as_str() {
            "--version" => options.version = true,
            "--fail-trial" => options.fail_trial = true,
            "--ready" => {
                options.ready = match value("answer, progress or notification")?.as_str() {
                    "answer" => Readiness::Answer,
                    "progress" => Readiness::Progress,
                    "notification" => Readiness::Notification,
                    other => {
                        return Err(format!(
                            "--ready is answer, progress or notification, not {other}"
                        ));
                    }
                }
            }
            "--ready-when" => options.ready_when = Some(value("a file")?.into()),
            "--fail-initialize" => options.fail_initialize = true,
            "--exit-after-ready" => options.exit_after_ready = true,
            "--ask-configuration" => options.ask_configuration = Some(value("a section")?),
            "--log" => options.log = Some(value("a file")?.into()),
            other => return Err(format!("{other} is not an argument of the stand-in")),
        }
    }
    Ok(options)
}

/// What the process shares between the thread that reads and the one that looks for the
/// file of `--ready-when`: the one place frames are written from, the log, and whether it
/// is ready.
struct Server {
    options: Options,
    out: Mutex<io::Stdout>,
    log: Option<Mutex<File>>,
    /// Set by the first thread to become ready, before it logs it.
    becoming_ready: AtomicBool,
    /// Set once the log says so: what answers reads this.
    ready: AtomicBool,
}

impl Server {
    /// Appends one line to the log, when there is one.
    fn log(&self, line: Value) {
        if let Some(log) = &self.log {
            let mut text = line.to_string();
            text.push('\n');
            let mut file = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            // One write for the line, so that two threads never interleave one.
            let _ = file.write_all(text.as_bytes());
        }
    }

    /// Writes one frame. A client that is gone is found out by the end of its input.
    fn send(&self, message: Value) {
        let mut out = self
            .out
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = write_frame(&mut *out, &message);
    }

    fn respond(&self, id: &Value, result: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn fail(&self, id: &Value, code: i64, message: &str) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request(&self, id: u64, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    /// Logs the end and ends.
    fn finish(&self, code: i32) -> ! {
        self.log(json!({"event": "exit", "code": code}));
        exit(code)
    }

    /// Becomes ready, once: logs it, answers from now on, shows it as the command line
    /// says, and dies if it was asked to.
    fn become_ready(&self) {
        if self.becoming_ready.swap(true, Ordering::SeqCst) {
            return;
        }
        // The line first: whoever is answered from now on finds it in the log.
        self.log(json!({"event": "ready"}));
        self.ready.store(true, Ordering::SeqCst);
        match self.options.ready {
            Readiness::Answer => {}
            Readiness::Progress => self.notify(
                "$/progress",
                json!({"token": "standin", "value": {"kind": "end"}}),
            ),
            Readiness::Notification => self.notify("standin/status", json!({"ready": true})),
        }
        if self.options.exit_after_ready {
            self.finish(2);
        }
    }
}

/// A name is a run of letters, digits and `_`.
fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The byte ranges of the names of a line.
fn names(line: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = None;
    for (at, character) in line.char_indices() {
        match (is_word(character), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                found.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        found.push((from, line.len()));
    }
    found
}

/// The byte range of the name a line `def <name>` defines.
fn definition(line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("def")?;
    let after = rest.trim_start();
    if after.len() == rest.len() {
        return None;
    }
    let from = line.len() - after.len();
    names(after)
        .first()
        .filter(|(start, _)| *start == 0)
        .map(|(_, end)| (from, from + end))
}

/// Every `.sil` file under `folder`, by path; a folder whose name starts with `.` is not
/// entered.
fn sil_files(folder: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                if !hidden {
                    sil_files(&path, into);
                }
            }
            Ok(_) if path.extension().is_some_and(|extension| extension == "sil") => {
                into.push(path)
            }
            _ => {}
        }
    }
}

/// The state of the one reading thread: what the client told it.
struct Session {
    server: Arc<Server>,
    root: PathBuf,
    documents: HashMap<PathBuf, String>,
    shut_down: bool,
}

impl Session {
    /// The text of a document: what the client last opened or changed, else the file.
    fn text_of(&self, path: &Path) -> Option<String> {
        self.documents
            .get(path)
            .cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    /// The name at the position a text document request names.
    fn name_at(&self, params: &Value) -> Option<String> {
        let path = path_of_uri(params.get("textDocument")?.get("uri")?.as_str()?)?;
        let position = params.get("position")?;
        let line = usize::try_from(position.get("line")?.as_u64()?).ok()?;
        let character = u32::try_from(position.get("character")?.as_u64()?).ok()?;
        let text = self.text_of(&path)?;
        let line = text.lines().nth(line)?;
        let byte = byte_column(line, character) as usize;
        names(line)
            .into_iter()
            .find(|(start, end)| (*start..*end).contains(&byte))
            .map(|(start, end)| line[start..end].to_owned())
    }

    /// The places of `name` in every `.sil` file under the root, as locations: where `pick`
    /// finds it in a line.
    fn places(&self, name: &str, pick: impl Fn(&str) -> Vec<(usize, usize)>) -> Vec<Value> {
        let mut files = Vec::new();
        sil_files(&self.root, &mut files);
        files.sort();
        let mut places = Vec::new();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (number, line) in text.lines().enumerate() {
                for (start, end) in pick(line) {
                    if &line[start..end] == name {
                        places.push(json!({
                            "uri": file_uri(&file),
                            "range": {
                                "start": {"line": number, "character": utf16_column(line, start as u32)},
                                "end": {"line": number, "character": utf16_column(line, end as u32)},
                            },
                        }));
                    }
                }
            }
        }
        places
    }

    fn request(&mut self, method: &str, id: &Value, params: &Value) {
        let server = Arc::clone(&self.server);
        server.log(json!({"event": "request", "method": method, "id": id}));
        match method {
            "initialize" => {
                server.log(json!({
                    "event": "initialize",
                    "rootUri": params.get("rootUri").cloned().unwrap_or(Value::Null),
                    "initializationOptions": params.get("initializationOptions").cloned().unwrap_or(Value::Null),
                    "capabilities": params.get("capabilities").cloned().unwrap_or(Value::Null),
                }));
                if let Some(root) = root_of(params) {
                    self.root = root;
                }
                if server.options.fail_initialize {
                    server.fail(id, -32603, "the stand-in was told to fail initialize");
                    server.finish(1);
                }
                server.respond(
                    id,
                    json!({
                        "capabilities": {
                            "hoverProvider": true,
                            "definitionProvider": true,
                            "referencesProvider": true,
                            "textDocumentSync": 1,
                        },
                        "serverInfo": {"name": "standin-lsp"},
                    }),
                );
            }
            "shutdown" => {
                self.shut_down = true;
                server.respond(id, Value::Null);
            }
            "textDocument/hover" => {
                let answer = match self.name_at(params) {
                    Some(name) if server.ready.load(Ordering::SeqCst) => json!({
                        "contents": {"kind": "plaintext", "value": format!("standin: {name}")},
                    }),
                    _ => Value::Null,
                };
                server.respond(id, answer);
            }
            "textDocument/definition" | "textDocument/references" => {
                let places = match self.name_at(params) {
                    Some(name) if server.ready.load(Ordering::SeqCst) => {
                        if method == "textDocument/definition" {
                            self.places(&name, |line| definition(line).into_iter().collect())
                        } else {
                            self.places(&name, names)
                        }
                    }
                    _ => Vec::new(),
                };
                server.respond(id, Value::Array(places));
            }
            _ => server.fail(id, -32601, "the stand-in does not answer this request"),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) {
        let server = Arc::clone(&self.server);
        let mut line = json!({"event": "notification", "method": method});
        if method == "workspace/didChangeWatchedFiles" {
            line["changes"] = params.get("changes").cloned().unwrap_or(Value::Null);
        }
        server.log(line);
        let document = params.get("textDocument");
        let path = document
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
            .and_then(path_of_uri);
        match (method, path) {
            ("textDocument/didOpen", Some(path)) => {
                if let Some(text) = document
                    .and_then(|document| document.get("text"))
                    .and_then(Value::as_str)
                {
                    self.documents.insert(path, text.to_owned());
                }
            }
            ("textDocument/didChange", Some(path)) => {
                let last = params
                    .get("contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str);
                if let Some(text) = last {
                    self.documents.insert(path, text.to_owned());
                }
            }
            ("textDocument/didClose", Some(path)) => {
                self.documents.remove(&path);
            }
            ("initialized", _) => self.initialized(),
            ("exit", _) => server.finish(if self.shut_down { 0 } else { 1 }),
            _ => {}
        }
    }

    fn initialized(&self) {
        let server = &self.server;
        match server.options.ready {
            Readiness::Answer => {}
            Readiness::Progress => {
                server.request(
                    PROGRESS_REQUEST,
                    "window/workDoneProgress/create",
                    json!({"token": "standin"}),
                );
                server.notify(
                    "$/progress",
                    json!({"token": "standin", "value": {"kind": "begin", "title": "standin"}}),
                );
            }
            Readiness::Notification => {
                server.notify("standin/status", json!({"ready": false}));
            }
        }
        if let Some(section) = &server.options.ask_configuration {
            server.request(
                CONFIGURATION_REQUEST,
                "workspace/configuration",
                json!({"items": [{"section": section}]}),
            );
        }
        match &server.options.ready_when {
            None => server.become_ready(),
            Some(file) => {
                let file = file.clone();
                let server = Arc::clone(server);
                std::thread::spawn(move || {
                    while !file.exists() {
                        std::thread::sleep(LOOK_EVERY);
                    }
                    server.become_ready();
                });
            }
        }
    }

    /// What the client answered to a request of the stand-in's.
    fn response(&self, id: &Value, result: &Value) {
        if *id == json!(CONFIGURATION_REQUEST) && self.server.options.ask_configuration.is_some() {
            self.server
                .log(json!({"event": "configuration", "result": result}));
        }
    }
}

/// The folder the client says it serves.
fn root_of(params: &Value) -> Option<PathBuf> {
    let uri = params.get("rootUri").and_then(Value::as_str).or_else(|| {
        params
            .get("workspaceFolders")?
            .as_array()?
            .first()?
            .get("uri")?
            .as_str()
    });
    uri.and_then(path_of_uri).or_else(|| {
        params
            .get("rootPath")
            .and_then(Value::as_str)
            .map(PathBuf::from)
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = parse(&args).unwrap_or_else(|reason| {
        eprintln!("standin-lsp: {reason}");
        exit(64)
    });
    if options.version {
        if options.fail_trial {
            eprintln!("standin-lsp: broken");
            exit(3);
        }
        println!("standin-lsp 1");
        exit(0);
    }

    let log = options.log.as_ref().map(|path| {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|error| {
                eprintln!(
                    "standin-lsp: cannot open the log {}: {error}",
                    path.display()
                );
                exit(70)
            });
        Mutex::new(file)
    });
    let server = Arc::new(Server {
        options,
        out: Mutex::new(io::stdout()),
        log,
        becoming_ready: AtomicBool::new(false),
        ready: AtomicBool::new(false),
    });

    let environment: serde_json::Map<String, Value> = std::env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .filter(|(name, _)| {
            matches!(name.as_str(), "HOME" | "TMPDIR" | "XDG_CACHE_HOME")
                || name.starts_with("STANDIN_")
        })
        .map(|(name, value)| (name, Value::String(value)))
        .collect();
    server.log(json!({
        "event": "start",
        "pid": std::process::id(),
        "cwd": std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default(),
        "args": args,
        "env": environment,
    }));

    let mut session = Session {
        server: Arc::clone(&server),
        root: std::env::current_dir().unwrap_or_default(),
        documents: HashMap::new(),
        shut_down: false,
    };
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    loop {
        let message = match read_frame(&mut input) {
            Ok(Some(message)) => message,
            Ok(None) => server.finish(0),
            Err(error) => {
                eprintln!("standin-lsp: {error}");
                server.finish(1)
            }
        };
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").unwrap_or(&Value::Null);
        match (method, message.get("id")) {
            (Some(method), Some(id)) => session.request(method, id, params),
            (Some(method), None) => session.notification(method, params),
            (None, Some(id)) => session.response(id, message.get("result").unwrap_or(&Value::Null)),
            (None, None) => {}
        }
    }
}
