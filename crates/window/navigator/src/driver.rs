//! One language server: started from what its row says, spoken to in the Language Server
//! Protocol, and stopped. Nothing here names a language or a server (NFR-002): every
//! difference between servers is a field of a row, which [`Launch`] carries.
//!
//! # What a server is told
//!
//! The program starts with `args`, its working folder `root`, exactly the environment
//! [`Launch::environment`] (no variable of this process leaks in), its standard input and
//! output piped to the driver and its standard error written to [`Launch::log`]. On Linux
//! it is also asked to end when this process ends (`PR_SET_PDEATHSIG`), so that no server
//! outlives a Window that crashed; on macOS a server that reads its input to its end ends
//! with the Window.
//!
//! The driver sends `initialize` with `processId` this process, `rootUri` and one
//! workspace folder of `root`, `initializationOptions`, and these client capabilities,
//! with the row's merged over them: `general.positionEncodings` `["utf-16"]`;
//! `textDocument.hover.contentFormat` `["markdown", "plaintext"]`;
//! `textDocument.definition.linkSupport` true; `textDocument.references` `{}`;
//! `textDocument.synchronization` `{}`; `workspace.configuration` true;
//! `workspace.workspaceFolders` true; `workspace.didChangeWatchedFiles.dynamicRegistration`
//! false; `window.workDoneProgress` true. Then `initialized`, and
//! `workspace/didChangeConfiguration` with the row's `settings`.
//!
//! # What a server asks, and is answered (ADR-015's four)
//!
//! | Request | Answer |
//! | --- | --- |
//! | `workspace/configuration` | for each item, the value of the row's `settings` at its `section` (split at `.`), the whole table for an item with none, null where there is none |
//! | `client/registerCapability`, `client/unregisterCapability` | null: nothing is registered |
//! | `window/workDoneProgress/create` | null |
//! | `workspace/diagnostic/refresh`, and any other request | null, or for a method the driver does not know, the error `MethodNotFound` (-32601) |
//!
//! Every notification is fed to the server's [`crate::readiness::Readiness`]; the rest of
//! them, diagnostics among them, are read and dropped.
//!
//! # Threads
//!
//! [`Server::start`] spawns the process and one reading thread, which reads every frame,
//! answers the server's requests itself, keeps the readiness and hands each event to the
//! `events` callback it was given: [`ServerEvent::Ready`] once, each answer to a request,
//! and [`ServerEvent::Exited`] last. Writes go through one lock on the server's input; a
//! write never waits for an answer. Nothing here blocks the caller beyond a write.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::registry::Ready;

/// Everything a server is started with, made from its row and the place it serves, every
/// variable already expanded.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    /// The program, by its absolute path.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// The folder it serves, and its working folder.
    pub root: PathBuf,
    /// Its whole environment.
    pub environment: Vec<(OsString, OsString)>,
    /// Merged over the driver's own client capabilities.
    pub capabilities: Value,
    /// Sent as `initializationOptions`.
    pub initialization_options: Option<Value>,
    /// The answer to `workspace/configuration`.
    pub settings: Value,
    /// The readiness rule.
    pub ready: Ready,
    /// File extension, with no dot, to the LSP `languageId`.
    pub language_ids: Vec<(String, String)>,
    /// The file its standard error is written to; none drops it.
    pub log: Option<PathBuf>,
}

/// What the reading thread hands the `events` callback of [`Server::start`].
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    /// The readiness rule was met. Told once.
    Ready,
    /// The answer to a request [`Server::request`] sent: its `result`, or its `error`'s
    /// message.
    Answered {
        /// The request, by the number `request` gave.
        request: u64,
        /// The result, or the error's message.
        result: Result<Value, String>,
    },
    /// The server's process ended, or its `initialize` failed and it was killed: told
    /// last. Every request still waiting is answered with an error before it.
    Exited {
        /// Its exit code, if it ended by itself with one.
        code: Option<i32>,
    },
}

/// A running language server.
pub struct Server {
    pid: u32,
}

impl Server {
    /// Starts `launch` and initializes it as the module doc says, handing every
    /// [`ServerEvent`] to `events` on the reading thread. An error, as a sentence, when the
    /// program cannot be started; a server that starts and then fails `initialize` is
    /// started, and told as [`ServerEvent::Exited`].
    pub fn start(
        launch: Launch,
        events: Box<dyn Fn(ServerEvent) + Send + 'static>,
    ) -> Result<Server, String> {
        let _ = (launch, events);
        todo!()
    }

    /// Its process.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether its readiness rule was met.
    pub fn is_ready(&self) -> bool {
        todo!()
    }

    /// Tells the server the text of the file at `path` as it is now: `textDocument/didOpen`
    /// with version 1, or, for a file it holds open, `textDocument/didChange` with the whole
    /// text and the next version. The `languageId` is the one of the file's extension.
    pub fn open(&self, path: &Path, text: &str) -> Result<(), String> {
        let _ = (path, text);
        todo!()
    }

    /// Sends a request and gives its number; its answer comes as
    /// [`ServerEvent::Answered`].
    pub fn request(&self, method: &str, params: Value) -> Result<u64, String> {
        let _ = (method, params);
        todo!()
    }

    /// Cancels a request: `$/cancelRequest`. Its answer, if one still comes, is dropped
    /// and not told.
    pub fn cancel(&self, request: u64) {
        let _ = request;
        todo!()
    }

    /// Tells the server that these files changed on disk:
    /// `workspace/didChangeWatchedFiles`, each as changed, or as deleted where no file is
    /// there now. A file it holds open is told again by [`Server::open`] at its next ask.
    pub fn files_changed(&self, paths: &[PathBuf]) {
        let _ = paths;
        todo!()
    }

    /// Stops the server: `shutdown`, then `exit`, then waits for its process to end until
    /// `within`, and kills it after. Returns once the process is reaped, so that its
    /// process id names nothing of the server's afterwards.
    pub fn stop(self, within: Duration) {
        let _ = within;
        todo!()
    }
}
