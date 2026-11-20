//! pi adapter (`H-AGENT-PI`).
//!
//! Owns: the mapping of pi's extension events to the seven events; its delivery, reply handling, shell entries, launcher and resume command.
//!
//! Surface: [`Pi`], the [`yaar_agent::Adapter`] of pi, and the names it stands on: [`NAME`],
//! [`TITLE`], [`COMMAND`], [`WRAPPER`], [`BIN`], [`EXTENSION`], [`EXTENSION_FILE`],
//! [`REPORT_PROGRAM`], [`REPORT_VARIABLE`], [`SUBCOMMANDS`], [`REPLY_TOOL`],
//! [`REPLY_EVENT`] and [`INSTRUCTION`]; [`ToPi`], its delivery module; and what the
//! reporting program `yaar-pi-report` does ([`pass_on`], [`ReportPlace`],
//! [`ReportAnswer`], [`DEADLINE`], [`TOOL_DEADLINE`]).
//!
//! # How pi reaches the app
//!
//! pi reads no variable that names an extension. [`Adapter::install`] writes two files into
//! the Adapter's own folder of the app, from the sources of the companion folder
//! `companions/pi` of the repository, which this crate holds as text:
//!
//! - `<folder>/`[`BIN`]`/`[`WRAPPER`]: the wrapper, a POSIX `sh` script made from
//!   `companions/pi/pi.sh` ([`WRAPPER_SOURCE`]), with the absolute paths of the reporting
//!   program and of the extension put in, each single-quoted for `sh` (a `'` inside written
//!   `'\''`). It exports [`REPORT_VARIABLE`] naming the reporting program, finds the next
//!   program named `pi` on the search path after its own folder, and runs it with every
//!   argument it was given, after `-e` and the extension's path; when the first argument is
//!   one of [`SUBCOMMANDS`], with no `-e` at all, since pi would take the extension's path
//!   for part of the subcommand's arguments. It writes nothing and decides nothing else.
//!   With no other `pi` on the search path it says so on its standard error and ends with
//!   127, as a shell does for a program it cannot find.
//! - `<folder>/`[`EXTENSION`]`/`[`EXTENSION_FILE`]: the extension, `companions/pi/yaar.ts`
//!   ([`EXTENSION_SOURCE`]) as it is, TypeScript, which pi loads itself.
//!
//! It gives one [`yaar_agent::ShellEntry::SearchFirst`] with `<folder>/`[`BIN`], so that the
//! name `pi` typed in a shell of the app, or run by its launcher, finds the wrapper first.
//! Nothing is written into pi's own folder, `~/.pi`, or a worktree; a pi started outside the
//! app's shells finds no wrapper and loads nothing of the app.
//!
//! # What the extension reports
//!
//! The extension runs the reporting program [`REPORT_PROGRAM`], found by
//! [`REPORT_VARIABLE`], once for each event below, one at a time and in the order pi gave
//! them, with no shell between, so that the program's parent is pi itself. It writes the
//! report's payload on the program's standard input and reads its answer from its standard
//! output; the program passes the payload on, unread, to the Keeper as one
//! `yaar_link::Request::Report` of the Adapter [`NAME`] ([`pass_on`]). A pi with no
//! [`REPORT_VARIABLE`] in its environment, one started outside the app, reports nothing.
//!
//! A payload is one JSON object with the keys pi's own events and context give, in pi's
//! words, so that a recorded line of `system-tests/recorded/pi/` is a payload as it is (the
//! recordings hold more keys, which the Adapter does not read):
//!
//! | `event` | Keys besides `event`, `sessionId`, `sessionFile` and `cwd` |
//! | --- | --- |
//! | `session_start` | `reason`: `startup`, `new`, `resume`, `reload` or `fork` |
//! | `session_shutdown` | `reason`: `quit`, `new`, `resume`, `reload` or `fork` |
//! | `input` | `source` (`interactive`, `rpc` or `extension`), `streamingBehavior` (null, `steer` or `followUp`) |
//! | `agent_start` | none |
//! | `tool_call` | `toolName`, `toolCallId`, `input`: `{"path"}` for `read`, `edit` and `write`, `{"command"}` for `bash`, `{}` for any other tool |
//! | `tool_result` | as `tool_call`, and `isError`; for `edit`, `details` with `patch`, the unified diff pi made |
//! | `agent_settled` | `aborted` |
//! | [`REPLY_EVENT`] | `toolCallId`, `arguments`: `{"thread", "body", "addressed"}` as the agent called the tool |
//!
//! `sessionId` is `ctx.sessionManager.getSessionId()`, `sessionFile` its
//! `getSessionFile()` (a path, or null for a session pi keeps in no file), `cwd` is
//! `ctx.cwd`: every report names its session and its folder. A message's text, a file's
//! content and an edit's texts are not sent: the Adapter reads none of them.
//!
//! # What it is answered
//!
//! The program prints the bytes of the Keeper's `yaar_link::Reply::Answer`, which this
//! Adapter wrote, and nothing for any other reply or none; with `--tool` it prints a JSON
//! object in every case ([`pass_on`]). The extension reads:
//!
//! | Report | Answer | What the extension does |
//! | --- | --- | --- |
//! | `input` | `{"review": "<the rendered review>"}` | returns `{"action": "transform", "text": <the user's text>, a blank line, <the review>, "images": <the user's images>}` |
//! | `input` | nothing | returns `{"action": "continue"}` |
//! | [`REPLY_EVENT`] | `{"text": "recorded reply to <thread> (addressed\|open, <n> characters)"}` | gives the agent that text as the tool's result |
//! | [`REPLY_EVENT`] | `{"error": "<why>"}` | throws an error with `why`, which pi shows the agent as the tool's failure |
//! | any other | anything | nothing: the handler returns once the report is passed on |
//!
//! # What the events are
//!
//! A run of pi's agent, from the user's message to its last word, is a turn of the app; pi's
//! own `turn_start` and `turn_end`, one for each request to the model, are not.
//!
//! | Report | Event |
//! | --- | --- |
//! | `session_start`, `reason` `resume` | session started, resumed |
//! | `session_start`, any other `reason` but `reload` | session started, new (the Keeper takes a session id it holds for a resume, as after `pi --session`) |
//! | `session_shutdown`, any `reason` but `reload` | session ended |
//! | `input` while no run of the session is open | turn started |
//! | `agent_start` while no run of the session is open | turn started |
//! | `tool_call` of `bash` | command began |
//! | `tool_result` of `bash` | command ended |
//! | `tool_result` of `read`, not `isError` | file read, `input.path` |
//! | `tool_result` of `edit`, not `isError` | file written by the edit tool, with the lines of `details.patch`: for each hunk `@@ -a,b +c,d @@`, `c` to `c + d - 1`, or `c` alone when `d` is 0; none without a patch |
//! | `tool_result` of `write`, not `isError` | file written by the write tool, no lines |
//! | `agent_settled` | turn ended: interrupted when `aborted`, else finished |
//!
//! Every other report gives nothing: `session_*` of `reload`, `input` while a run is open
//! (a message pi queues, which goes with the run that is open: the app's account of the
//! turn is not split), any other tool, [`REPLY_EVENT`]. A path is given absolute: a relative
//! `input.path` is joined to `cwd`. Every event carries `cwd` as the session's folder and
//! `sessionFile` as [`yaar_agent::Reported::kept`]. pi asks no permission: no event is ever
//! [`yaar_agent::Event::Permission`], and its sessions never wait for the user's answer.
//! pi reports an interrupt itself (`agent_settled` with `aborted`): [`Adapter::marks`] is
//! empty. A read through an `@` mention or through a shell command runs no read tool and
//! gives no read event.
//!
//! What a report asks ([`Adapter::asked`]): an `input` whose `source` is not `extension` is
//! [`yaar_agent::Asks::Message`], whether pi is idle or works (pi queues it, and the review
//! goes with it); a [`REPLY_EVENT`] is [`yaar_agent::Asks::Reply`]; nothing else asks. A
//! message an extension sent, as one pi starts for a background task, carries no review.
//!
//! [`Adapter::install`]: yaar_agent::Adapter::install
//! [`Adapter::asked`]: yaar_agent::Adapter::asked
//! [`Adapter::marks`]: yaar_agent::Adapter::marks

mod deliver;
mod install;
mod report;
mod reporter;

pub use deliver::{INSTRUCTION, ToPi};
pub use reporter::{DEADLINE, ReportAnswer, ReportPlace, TOOL_DEADLINE, pass_on};

use std::io;
use std::path::Path;

use report::Remembered;
use yaar_agent::{
    Adapter, Answer, Asked, CliSession, Delivery, Report, ReportError, Reported, ShellEntry,
};

/// The name the app knows pi by.
pub const NAME: &str = "pi";

/// The name a person reads.
pub const TITLE: &str = "pi";

/// What a launcher runs in the user's shell: the name, which the wrapper answers first.
pub const COMMAND: &str = "pi";

/// The wrapper's name in [`BIN`]: pi's own, so that it is found in pi's place.
pub const WRAPPER: &str = "pi";

/// The folder, inside the Adapter's own folder of the app, that holds the wrapper and is put
/// first on the search path of every shell of the app.
pub const BIN: &str = "bin";

/// The folder, inside the Adapter's own folder of the app, that holds the extension.
pub const EXTENSION: &str = "extension";

/// The extension's file name in [`EXTENSION`].
pub const EXTENSION_FILE: &str = "yaar.ts";

/// The program of the app that pi's extension runs for each report, in the folder of the
/// app's programs beside the Keeper.
pub const REPORT_PROGRAM: &str = "yaar-pi-report";

/// The variable the wrapper exports, naming [`REPORT_PROGRAM`] by its absolute path: the
/// extension finds it there, and a pi with no such variable reports nothing.
pub const REPORT_VARIABLE: &str = "YAAR_PI_REPORT";

/// pi's subcommands, which it finds by the first argument alone: the wrapper runs pi with
/// no extension when the first argument is one of these, and leaves every argument as it
/// is. Read from pi 1.1.0; a subcommand a later pi adds and this list lacks would be run
/// with the extension's path among its arguments.
pub const SUBCOMMANDS: [&str; 8] = [
    "install",
    "remove",
    "uninstall",
    "update",
    "list",
    "config",
    "mcp",
    "auth",
];

/// The reply tool the extension registers with pi, as pi lists it to the agent.
pub const REPLY_TOOL: &str = "reply_to_thread";

/// The `event` of the payload of a call of [`REPLY_TOOL`].
pub const REPLY_EVENT: &str = "reply_to_thread";

/// The source of the extension, `companions/pi/yaar.ts`, which [`Adapter::install`] writes
/// as it is.
///
/// [`Adapter::install`]: yaar_agent::Adapter::install
pub const EXTENSION_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../companions/pi/yaar.ts"
));

/// The source of the wrapper, `companions/pi/pi.sh`: a POSIX `sh` script in which
/// [`Adapter::install`] puts `@REPORT@` in place of the reporting program's absolute path
/// and `@EXTENSION@` in place of the extension's, each quoted for `sh` as one word.
///
/// [`Adapter::install`]: yaar_agent::Adapter::install
pub const WRAPPER_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../companions/pi/pi.sh"
));

/// The Adapter of pi. It remembers, for each session, whether a run of its agent is open.
#[derive(Debug, Default)]
pub struct Pi {
    memory: Remembered,
}

impl Pi {
    /// The Adapter, knowing no session yet.
    pub fn new() -> Pi {
        Pi::default()
    }
}

impl Adapter for Pi {
    fn name(&self) -> &'static str {
        NAME
    }

    fn title(&self) -> &'static str {
        TITLE
    }

    fn command(&self) -> &'static str {
        COMMAND
    }

    /// Writes `folder/`[`BIN`]`/`[`WRAPPER`], which can be run, and
    /// `folder/`[`EXTENSION`]`/`[`EXTENSION_FILE`], as the crate's doc says, each through a
    /// temporary file of its folder renamed into place, and gives
    /// [`yaar_agent::ShellEntry::SearchFirst`] with `folder/`[`BIN`]. The reporting program
    /// is `companions/`[`REPORT_PROGRAM`] by its absolute path. A path that is not text is
    /// an error of kind `InvalidInput`, and nothing is written.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
        install::write(folder, companions)
    }

    /// None: pi reports an interrupt itself.
    fn marks(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        self.memory.report(report)
    }

    /// Nothing: pi's Adapter watches for no mark.
    fn marked(&mut self, terminal: &str, mark: usize) -> Vec<Reported> {
        let _ = (terminal, mark);
        Vec::new()
    }

    fn asked(&self, folder: &Path, report: &Report<'_>) -> Result<Option<Asked>, ReportError> {
        self.memory.asked(folder, report)
    }

    fn instruction(&self) -> &'static str {
        INSTRUCTION
    }

    /// [`ToPi`]: the whole review in the answer, at any length.
    fn delivery<'a>(&'a self, folder: &'a Path, asked: &'a Asked) -> Box<dyn Delivery + 'a> {
        let _ = (folder, asked);
        Box::new(ToPi)
    }

    fn answer(&self, asked: &Asked, answer: &Answer<'_>) -> Vec<u8> {
        self.memory.answer(asked, answer)
    }

    fn resume(&self, session: &CliSession, kept: Option<&Path>) -> String {
        deliver::resume(session, kept)
    }
}
