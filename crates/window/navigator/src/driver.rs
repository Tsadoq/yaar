//! On Linux a server is killed when the thread that started it ends: start servers from a
//! thread that lives as long as they do. On macOS a crashed Window leaves its servers running.
//! What a caller sends before the answer to `initialize` is held and sent after `initialized`.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::lsp::{file_uri, read_frame, write_frame};
use crate::readiness::{Readiness, Readying};
use crate::registry::Ready;

const INITIALIZE: u64 = 0;

/// A server that closed its output and runs on can be spoken to no more: it is killed after this.
const AFTER_OUTPUT_CLOSED: Duration = Duration::from_secs(10);

/// How long `Server::stop` waits, once the process is reaped, for the reading thread: a
/// descendant that kept the server's output open would hold it back for good.
const AFTER_REAPED: Duration = Duration::from_secs(2);

const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub root: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    /// Merged over the driver's own client capabilities.
    pub capabilities: Value,
    pub initialization_options: Option<Value>,
    pub settings: Value,
    pub ready: Ready,
    pub language_ids: Vec<(String, String)>,
    pub log: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    Ready,
    Answered {
        request: u64,
        result: Result<Value, String>,
    },
    /// Told last, after every request still waiting was answered with an error.
    Exited {
        code: Option<i32>,
    },
}

enum Outgoing {
    Frame(Vec<u8>),
    Close,
}

struct Gate {
    open: bool,
    held: Vec<Outgoing>,
}

struct Ends {
    /// Its exit code when the process was reaped, none for one killed by a signal.
    reaped: Option<Option<i32>>,
    reading_done: bool,
}

struct Shared {
    outbox: Sender<Outgoing>,
    gate: Mutex<Gate>,
    /// Set, under the child's lock, when the process was reaped by a wait that `Child` does not
    /// know of: a kill after it would reach a process id that may be another's.
    reaped_apart: AtomicBool,
    ready: AtomicBool,
    closed: AtomicBool,
    next_request: AtomicU64,
    /// None once the server ended, so that a later request is refused instead of left waiting.
    pending: Mutex<Option<BTreeSet<u64>>>,
    documents: Mutex<HashMap<PathBuf, i32>>,
    language_ids: Vec<(String, String)>,
    ends: Mutex<Ends>,
    ended: Condvar,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    fn send(&self, message: &Value) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("the server has ended".to_owned());
        }
        self.queue(Outgoing::Frame(frame_of(message)?))
    }

    fn queue(&self, outgoing: Outgoing) -> Result<(), String> {
        let mut gate = locked(&self.gate);
        if gate.open {
            self.outbox
                .send(outgoing)
                .map_err(|_| "the server's input is closed".to_owned())
        } else {
            gate.held.push(outgoing);
            Ok(())
        }
    }

    fn send_now(&self, message: &Value) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("the server has ended".to_owned());
        }
        self.outbox
            .send(Outgoing::Frame(frame_of(message)?))
            .map_err(|_| "the server's input is closed".to_owned())
    }

    fn open_gate(&self, first: &[Value]) {
        let mut gate = locked(&self.gate);
        for message in first {
            let _ = self.send_now(message);
        }
        for outgoing in gate.held.drain(..) {
            let _ = self.outbox.send(outgoing);
        }
        gate.open = true;
    }

    fn kill(&self, child: &Mutex<Child>) {
        let mut child = locked(child);
        if !self.reaped_apart.load(Ordering::SeqCst) {
            let _ = child.kill();
        }
    }

    fn wait_reaped(&self, within: Duration) -> Option<Option<i32>> {
        let (ends, _) = self
            .ended
            .wait_timeout_while(locked(&self.ends), within, |ends| ends.reaped.is_none())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ends.reaped
    }
}

fn frame_of(message: &Value) -> Result<Vec<u8>, String> {
    let mut frame = Vec::new();
    write_frame(&mut frame, message).map_err(|error| error.to_string())?;
    Ok(frame)
}

pub struct Server {
    pid: u32,
    shared: Arc<Shared>,
    child: Arc<Mutex<Child>>,
}

impl Server {
    pub fn start(
        launch: Launch,
        events: Box<dyn Fn(ServerEvent) + Send + 'static>,
    ) -> Result<Server, String> {
        let root = std::path::absolute(&launch.root)
            .map_err(|error| format!("cannot use {}: {error}", launch.root.display()))?;
        let root_uri = file_uri(&root);
        let folder = json!({
            "uri": root_uri,
            "name": root.file_name().map_or_else(|| "root".to_owned(), |n| n.to_string_lossy().into_owned()),
        });

        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .current_dir(&root)
            .env_clear()
            .envs(
                launch
                    .environment
                    .iter()
                    .map(|(n, v)| (n.as_os_str(), v.as_os_str())),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log_file(launch.log.as_deref()));
        end_with_the_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start {}: {error}", launch.program.display()))?;
        let pid = child.id();
        let stdin = child.stdin.take().expect("the input was piped");
        let stdout = child.stdout.take().expect("the output was piped");

        let (outbox, inbox) = channel();
        let shared = Arc::new(Shared {
            outbox,
            gate: Mutex::new(Gate {
                open: false,
                held: Vec::new(),
            }),
            reaped_apart: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            next_request: AtomicU64::new(INITIALIZE + 1),
            pending: Mutex::new(Some(BTreeSet::new())),
            documents: Mutex::new(HashMap::new()),
            language_ids: launch.language_ids.clone(),
            ends: Mutex::new(Ends {
                reaped: None,
                reading_done: false,
            }),
            ended: Condvar::new(),
        });
        let _ = shared.send_now(&request_message(
            INITIALIZE,
            "initialize",
            initialize_params(&launch, &root_uri, &folder),
        ));

        let child = Arc::new(Mutex::new(child));
        let reader = Reader {
            shared: Arc::clone(&shared),
            child: Arc::clone(&child),
            events,
            readiness: Readiness::new(&launch.ready),
            settings: launch.settings,
            folder,
            initialized: false,
            told_ready: false,
        };
        let started = start_thread("language-server-reader", move || reader.run(stdout))
            .and_then(|()| start_thread("language-server-writer", move || write_loop(stdin, inbox)))
            .and_then(|()| {
                let shared = Arc::clone(&shared);
                let child = Arc::clone(&child);
                start_thread("language-server-waiter", move || reap(pid, &child, &shared))
            });
        if let Err(error) = started {
            shared.kill(&child);
            return Err(error);
        }
        Ok(Server { pid, shared, child })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn is_ready(&self) -> bool {
        self.shared.ready.load(Ordering::SeqCst)
    }

    /// `didOpen` with version 1, or `didChange` with the whole text and the next version for a file
    /// it holds open. An extension the row does not name is an error, and nothing is sent.
    pub fn open(&self, path: &Path, text: &str) -> Result<(), String> {
        let extension = path.extension().and_then(|extension| extension.to_str());
        let language = self
            .shared
            .language_ids
            .iter()
            .find(|(known, _)| Some(known.as_str()) == extension)
            .map(|(_, language)| language.as_str())
            .ok_or_else(|| format!("no language is named for {}", path.display()))?;
        let path = std::path::absolute(path)
            .map_err(|error| format!("cannot use {}: {error}", path.display()))?;
        // Held while queueing, so that two asks about one file queue its versions in order.
        let mut documents = locked(&self.shared.documents);
        let version = documents.entry(path.clone()).or_insert(0);
        *version += 1;
        self.shared
            .send(&document_message(&path, language, *version, text))
    }

    pub fn request(&self, method: &str, params: Value) -> Result<u64, String> {
        let mut pending = locked(&self.shared.pending);
        let Some(waiting) = pending.as_mut() else {
            return Err("the server has ended".to_owned());
        };
        let request = self.shared.next_request.fetch_add(1, Ordering::SeqCst);
        // Noted before it is sent: the answer cannot come first.
        waiting.insert(request);
        let sent = self.shared.send(&request_message(request, method, params));
        if sent.is_err() {
            waiting.remove(&request);
        }
        sent.map(|()| request)
    }

    pub fn cancel(&self, request: u64) {
        let was_waiting = locked(&self.shared.pending)
            .as_mut()
            .is_some_and(|waiting| waiting.remove(&request));
        if was_waiting {
            let _ = self.shared.send(&notification_message(
                "$/cancelRequest",
                json!({"id": request}),
            ));
        }
    }

    pub fn files_changed(&self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let changes = watched_files(paths, |path| path.exists());
        let _ = self.shared.send(&notification_message(
            "workspace/didChangeWatchedFiles",
            json!({"changes": changes}),
        ));
    }

    pub fn stop(self, within: Duration) {
        let shutdown = self.shared.next_request.fetch_add(1, Ordering::SeqCst);
        let _ = self
            .shared
            .send(&request_message(shutdown, "shutdown", Value::Null));
        let _ = self.shared.send(&notification_message("exit", Value::Null));
        let _ = self.shared.queue(Outgoing::Close);

        if self.shared.wait_reaped(within).is_none() {
            self.shared.kill(&self.child);
            let mut ends = locked(&self.shared.ends);
            while ends.reaped.is_none() {
                ends = self
                    .shared
                    .ended
                    .wait(ends)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        let _ =
            self.shared
                .ended
                .wait_timeout_while(locked(&self.shared.ends), AFTER_REAPED, |ends| {
                    !ends.reading_done
                });
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.kill(&self.child);
    }
}

fn start_thread(name: &str, body: impl FnOnce() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map(drop)
        .map_err(|error| format!("cannot start a thread of the server's: {error}"))
}

fn log_file(log: Option<&Path>) -> Stdio {
    let Some(log) = log else {
        return Stdio::null();
    };
    if let Some(folder) = log.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

#[cfg(target_os = "linux")]
fn end_with_the_window(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: `getpid` has no preconditions.
    let parent = unsafe { libc::getpid() };
    // SAFETY: between the fork and the exec the closure calls `prctl`, `getppid` and `_exit`
    // only, which are safe there, and touches no memory of this process's.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have ended before the request took effect: its signal would never
            // come.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn end_with_the_window(command: &mut Command) {
    let _ = command;
}

fn write_loop(mut input: ChildStdin, queue: Receiver<Outgoing>) {
    while let Ok(Outgoing::Frame(frame)) = queue.recv() {
        if input
            .write_all(&frame)
            .and_then(|()| input.flush())
            .is_err()
        {
            return;
        }
    }
}

/// Waits for the process to end without reaping it, then reaps it, so that a kill by
/// `Server::stop` or the drop never reaches a process id the system has given to another.
fn reap(pid: u32, child: &Mutex<Child>, shared: &Shared) {
    let code = loop {
        // SAFETY: an all-zero `siginfo_t` is what the call expects to fill in.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waits for one child of this process, and reaps nothing.
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if waited != 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break locked(child).wait().ok().and_then(|status| status.code());
        }
        if ended(&info) {
            let status = locked(child).wait();
            break status.map_or_else(|_| exit_code(&info), |status| status.code());
        }
        // SAFETY: as above, and it may reap. macOS reports a stopped child to a wait for exits,
        // again at every call: take it, so that the next wait blocks. The child may end in between,
        // and then this is the call that reaps it.
        let mut taken: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // Under the child's lock, so that no kill is on its way to the process id when it is
        // reaped: `reaped_apart` is set before the lock is let go.
        let guard = locked(child);
        // SAFETY: as above.
        let took = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut taken,
                libc::WEXITED | libc::WNOHANG,
            )
        };
        if took == 0 && ended(&taken) {
            shared.reaped_apart.store(true, Ordering::SeqCst);
            drop(guard);
            break exit_code(&taken);
        }
        drop(guard);
    };
    let mut ends = locked(&shared.ends);
    ends.reaped = Some(code);
    shared.ended.notify_all();
}

fn ended(info: &libc::siginfo_t) -> bool {
    matches!(
        info.si_code,
        libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED
    )
}

fn exit_code(info: &libc::siginfo_t) -> Option<i32> {
    // SAFETY: `si_status` is meaningful for the exit a wait reports, and is only read here.
    (info.si_code == libc::CLD_EXITED).then(|| unsafe { info.si_status() })
}

struct Reader {
    shared: Arc<Shared>,
    child: Arc<Mutex<Child>>,
    events: Box<dyn Fn(ServerEvent) + Send + 'static>,
    readiness: Readiness,
    settings: Value,
    folder: Value,
    initialized: bool,
    told_ready: bool,
}

impl Reader {
    fn run(mut self, output: ChildStdout) {
        let mut input = BufReader::new(output);
        loop {
            match read_frame(&mut input) {
                Ok(Some(message)) => self.handle(&message),
                Ok(None) => break,
                Err(_) => {
                    self.shared.kill(&self.child);
                    break;
                }
            }
        }
        self.finish();
    }

    fn handle(&mut self, message: &Value) {
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").unwrap_or(&Value::Null);
        match (method, message.get("id")) {
            (Some(method), Some(id)) => self.requested(id, method, params),
            (Some(method), None) => {
                self.readiness.notification(method, params);
                self.tell_ready();
            }
            (None, Some(id)) => {
                let result = match message.get("error") {
                    Some(error) => Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("the server answered with an error")
                        .to_owned()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                self.answered(id, result);
            }
            (None, None) => {}
        }
    }

    fn requested(&self, id: &Value, method: &str, params: &Value) {
        let reply = match answer_request(method, params, &self.settings, &self.folder) {
            Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": METHOD_NOT_FOUND, "message": format!("the driver does not answer {method}")},
            }),
        };
        let _ = self.shared.send_now(&reply);
    }

    fn answered(&mut self, id: &Value, result: Result<Value, String>) {
        let Some(request) = id.as_u64() else {
            return;
        };
        if request == INITIALIZE {
            if result.is_err() {
                self.readiness.ended();
                self.shared.kill(&self.child);
                return;
            }
            self.shared.open_gate(&after_initialize(&self.settings));
            self.initialized = true;
            self.readiness.answered_initialize();
            self.tell_ready();
            return;
        }
        let waiting = locked(&self.shared.pending)
            .as_mut()
            .is_some_and(|waiting| waiting.remove(&request));
        if waiting {
            (self.events)(ServerEvent::Answered { request, result });
        }
    }

    fn tell_ready(&mut self) {
        if self.initialized && !self.told_ready && self.readiness.now() == Readying::Ready {
            self.told_ready = true;
            self.shared.ready.store(true, Ordering::SeqCst);
            (self.events)(ServerEvent::Ready);
        }
    }

    fn finish(mut self) {
        self.readiness.ended();
        self.shared.ready.store(false, Ordering::SeqCst);
        self.shared.closed.store(true, Ordering::SeqCst);
        let _ = self.shared.outbox.send(Outgoing::Close);
        let waiting = locked(&self.shared.pending).take().unwrap_or_default();
        for request in waiting {
            (self.events)(ServerEvent::Answered {
                request,
                result: Err("the server ended before it answered".to_owned()),
            });
        }
        let code = match self.shared.wait_reaped(AFTER_OUTPUT_CLOSED) {
            Some(code) => code,
            None => {
                self.shared.kill(&self.child);
                let mut ends = locked(&self.shared.ends);
                while ends.reaped.is_none() {
                    ends = self
                        .shared
                        .ended
                        .wait(ends)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                ends.reaped.flatten()
            }
        };
        (self.events)(ServerEvent::Exited { code });
        locked(&self.shared.ends).reading_done = true;
        self.shared.ended.notify_all();
    }
}

/// `params` is left out when null: some servers refuse a `null` where they expect none.
fn request_message(id: u64, method: &str, params: Value) -> Value {
    let mut message = json!({"jsonrpc": "2.0", "id": id, "method": method});
    if !params.is_null() {
        message["params"] = params;
    }
    message
}

fn notification_message(method: &str, params: Value) -> Value {
    let mut message = json!({"jsonrpc": "2.0", "method": method});
    if !params.is_null() {
        message["params"] = params;
    }
    message
}

fn initialize_params(launch: &Launch, root_uri: &str, folder: &Value) -> Value {
    let mut params = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "workspaceFolders": [folder],
        "capabilities": client_capabilities(&launch.capabilities),
    });
    if let Some(options) = &launch.initialization_options {
        params["initializationOptions"] = options.clone();
    }
    params
}

/// Objects are merged key by key; any other value of the row replaces the driver's.
fn client_capabilities(row: &Value) -> Value {
    fn merge(base: &mut Value, over: &Value) {
        match (base, over) {
            (Value::Object(base), Value::Object(over)) => {
                for (key, value) in over {
                    match base.get_mut(key) {
                        Some(existing) => merge(existing, value),
                        None => {
                            base.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
            (base, over) => *base = over.clone(),
        }
    }
    let mut capabilities = json!({
        "general": {"positionEncodings": ["utf-16"]},
        "textDocument": {
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "definition": {"linkSupport": true},
            "references": {},
            "synchronization": {},
        },
        "workspace": {
            "configuration": true,
            "workspaceFolders": true,
            "didChangeWatchedFiles": {"dynamicRegistration": false},
        },
        "window": {"workDoneProgress": true},
    });
    if !row.is_null() {
        merge(&mut capabilities, row);
    }
    capabilities
}

fn after_initialize(settings: &Value) -> [Value; 2] {
    [
        notification_message("initialized", json!({})),
        notification_message(
            "workspace/didChangeConfiguration",
            json!({"settings": settings}),
        ),
    ]
}

fn answer_request(method: &str, params: &Value, settings: &Value, folder: &Value) -> Option<Value> {
    match method {
        "workspace/configuration" => {
            let items = params.get("items").and_then(Value::as_array);
            Some(Value::Array(
                items
                    .into_iter()
                    .flatten()
                    .map(|item| setting_at(settings, item.get("section").and_then(Value::as_str)))
                    .collect(),
            ))
        }
        "workspace/workspaceFolders" => Some(json!([folder])),
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "window/showMessageRequest" => Some(Value::Null),
        "window/showDocument" => Some(json!({"success": false})),
        // Refused: a server never edits what the reviewer reads.
        "workspace/applyEdit" => Some(json!({"applied": false})),
        refresh
            if refresh
                .strip_prefix("workspace/")
                .and_then(|rest| rest.strip_suffix("/refresh"))
                .is_some_and(|kind| !kind.is_empty()) =>
        {
            Some(Value::Null)
        }
        _ => None,
    }
}

/// The parts of `section` are split at `.`; the whole table for none or an empty one, null where
/// there is nothing.
fn setting_at(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|section| !section.is_empty()) else {
        return settings.clone();
    };
    section
        .split('.')
        .try_fold(settings, |at, part| at.get(part))
        .cloned()
        .unwrap_or(Value::Null)
}

fn document_message(path: &Path, language_id: &str, version: i32, text: &str) -> Value {
    let uri = file_uri(path);
    if version == 1 {
        notification_message(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": version,
                "text": text,
            }}),
        )
    } else {
        notification_message(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": [{"text": text}],
            }),
        )
    }
}

/// Change type 2 is changed, 3 deleted (where `exists` says no file is there).
fn watched_files(paths: &[PathBuf], exists: impl Fn(&Path) -> bool) -> Vec<Map<String, Value>> {
    paths
        .iter()
        .filter_map(|path| {
            let path = std::path::absolute(path).ok()?;
            let mut change = Map::new();
            change.insert("uri".to_owned(), Value::String(file_uri(&path)));
            change.insert("type".to_owned(), json!(if exists(&path) { 2 } else { 3 }));
            Some(change)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_capabilities_are_merged_key_by_key_over_the_drivers_and_win_where_both_speak() {
        let merged = client_capabilities(&json!({
            "textDocument": {"hover": {"contentFormat": ["plaintext"]}},
            "workspace": {"configuration": false},
            "experimental": {"flag": true},
        }));
        assert_eq!(merged["experimental"]["flag"], true);
        assert_eq!(merged["general"]["positionEncodings"], json!(["utf-16"]));
        assert_eq!(
            merged["textDocument"]["hover"]["contentFormat"],
            json!(["plaintext"])
        );
        assert_eq!(merged["textDocument"]["definition"]["linkSupport"], true);
        assert_eq!(merged["workspace"]["configuration"], false);
        assert_eq!(merged["workspace"]["workspaceFolders"], true);
        assert_eq!(
            client_capabilities(&Value::Null),
            client_capabilities(&json!({}))
        );
        assert_eq!(
            client_capabilities(&json!({}))["window"]["workDoneProgress"],
            true
        );
    }

    #[test]
    fn a_setting_is_found_by_the_parts_of_its_section_and_the_whole_table_by_none() {
        let settings = json!({"a": {"b": {"c": 1}}, "list": [1, 2], "n": 3});
        assert_eq!(setting_at(&settings, Some("a.b")), json!({"c": 1}));
        assert_eq!(setting_at(&settings, Some("a.b.c")), json!(1));
        assert_eq!(setting_at(&settings, Some("a.x")), Value::Null);
        assert_eq!(setting_at(&settings, Some("n.deeper")), Value::Null);
        assert_eq!(setting_at(&settings, Some("list.0")), Value::Null);
        assert_eq!(setting_at(&settings, None), settings);
        assert_eq!(setting_at(&settings, Some("")), settings);
    }

    #[test]
    fn a_request_for_configuration_is_answered_item_by_item_and_with_none_when_there_are_no_items()
    {
        let settings = json!({"a": {"b": 1}});
        let folder = json!({"uri": "file:///x", "name": "x"});
        let answer = answer_request(
            "workspace/configuration",
            &json!({"items": [{"section": "a.b"}, {"section": "z"}, {}]}),
            &settings,
            &folder,
        );
        assert_eq!(answer, Some(json!([1, null, settings.clone()])));
        let answer = answer_request("workspace/configuration", &json!({}), &settings, &folder);
        assert_eq!(answer, Some(json!([])));
    }

    #[test]
    fn the_requests_a_server_may_make_are_answered_and_any_other_is_not_found() {
        let settings = json!({});
        let folder = json!({"uri": "file:///x", "name": "x"});
        let answer = |method: &str| answer_request(method, &Value::Null, &settings, &folder);
        for null in [
            "client/registerCapability",
            "client/unregisterCapability",
            "window/workDoneProgress/create",
            "window/showMessageRequest",
            "workspace/diagnostic/refresh",
            "workspace/inlayHint/refresh",
        ] {
            assert_eq!(answer(null), Some(Value::Null), "{null}");
        }
        assert_eq!(
            answer("workspace/workspaceFolders"),
            Some(json!([folder.clone()]))
        );
        assert_eq!(
            answer("window/showDocument"),
            Some(json!({"success": false}))
        );
        assert_eq!(
            answer("workspace/applyEdit"),
            Some(json!({"applied": false}))
        );
        for unknown in ["custom/thing", "workspace/refresh", "textDocument/hover"] {
            assert_eq!(answer(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn a_document_is_opened_at_version_1_and_changed_with_the_whole_text_at_the_next() {
        let path = Path::new("/p/a b.sil");
        let opened = document_message(path, "standin", 1, "one");
        assert_eq!(opened["method"], "textDocument/didOpen");
        assert_eq!(
            opened["params"]["textDocument"]["uri"],
            "file:///p/a%20b.sil"
        );
        assert_eq!(opened["params"]["textDocument"]["languageId"], "standin");
        assert_eq!(opened["params"]["textDocument"]["version"], 1);
        assert_eq!(opened["params"]["textDocument"]["text"], "one");
        for version in [2, 3] {
            let changed = document_message(path, "standin", version, "two");
            assert_eq!(changed["method"], "textDocument/didChange");
            assert_eq!(changed["params"]["textDocument"]["version"], version);
            assert_eq!(
                changed["params"]["contentChanges"],
                json!([{"text": "two"}])
            );
        }
    }

    #[test]
    fn a_file_still_there_changed_and_a_file_that_is_gone_deleted() {
        let paths = [PathBuf::from("/p/here.sil"), PathBuf::from("/p/gone.sil")];
        let changes = watched_files(&paths, |path| path.ends_with("here.sil"));
        assert_eq!(
            Value::from(changes),
            json!([
                {"uri": "file:///p/here.sil", "type": 2},
                {"uri": "file:///p/gone.sil", "type": 3},
            ])
        );
    }

    #[test]
    fn a_message_with_no_parameters_carries_none_and_initialized_comes_with_the_rows_settings() {
        assert!(
            request_message(3, "shutdown", Value::Null)
                .get("params")
                .is_none()
        );
        assert!(
            notification_message("exit", Value::Null)
                .get("params")
                .is_none()
        );
        assert_eq!(
            request_message(3, "x", json!({"a": 1}))["params"],
            json!({"a": 1})
        );
        let [initialized, configuration] = after_initialize(&json!({"a": {"b": 1}}));
        assert_eq!(initialized["method"], "initialized");
        assert_eq!(configuration["method"], "workspace/didChangeConfiguration");
        assert_eq!(configuration["params"]["settings"], json!({"a": {"b": 1}}));
    }
}
