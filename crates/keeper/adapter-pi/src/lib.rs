//! pi adapter (`H-AGENT-PI`).
//!
//! Owns: the mapping of pi's extension events to the seven events; its delivery, reply handling, shell entries, launcher and resume command; and the one entry it writes into pi's own settings, after the user's yes.
//!
//! Surface: [`Pi`], the [`yaar_agent::Adapter`] of pi, and the names it stands on: [`NAME`],
//! [`TITLE`], [`COMMAND`], [`EXTENSION`], [`EXTENSION_FILE`], [`REPORT_PROGRAM`],
//! [`REPORT_VARIABLE`], [`AGENT_FOLDER`], [`SETTINGS_FILE`], [`SETTINGS_KEY`],
//! [`CONSENT_FILE`], [`REPLY_TOOL`], [`REPLY_EVENT`] and [`INSTRUCTION`]; [`ToPi`], its
//! delivery module; and what the reporting program `yaar-pi-report` does ([`pass_on`],
//! [`ReportPlace`], [`ReportAnswer`], [`DEADLINE`], [`TOOL_DEADLINE`]).
//!
//! # How pi reaches the app
//!
//! pi reads no variable that names an extension; it loads the extensions its own settings
//! list. The app asks the user once, and only a yes puts the app's extension there.
//!
//! [`Adapter::install`] writes one file into the Adapter's own folder of the app, at every
//! start of the Keeper, from the source of the companion folder `companions/pi` of the
//! repository, which this crate holds as text:
//! `<folder>/`[`EXTENSION`]`/`[`EXTENSION_FILE`], the extension, `companions/pi/yaar.ts`
//! ([`EXTENSION_SOURCE`]) as it is, TypeScript, which pi loads itself. The path is the same
//! after an update of the app; only the file's text changes. It gives one
//! [`yaar_agent::ShellEntry::Variable`]: [`REPORT_VARIABLE`] naming the reporting program
//! [`REPORT_PROGRAM`] by its absolute path, so that every shell of the app, and so every
//! pi typed in one or started by a launcher, tells the extension that it runs inside the
//! app and where to report. Nothing of pi's is written by it.
//!
//! pi's own settings are `<home>/`[`AGENT_FOLDER`]`/`[`SETTINGS_FILE`], `home` being the
//! user's home the Keeper hands [`Pi::in_home`] at its start (a test hands a folder of its
//! own; [`Pi::new`] knows no home and asks nothing). pi is installed for the app when
//! `<home>/`[`AGENT_FOLDER`] is a folder: pi makes it at its first run. Until then the
//! Adapter asks nothing ([`Adapter::consent`] gives none) and pi in the app is any program.
//!
//! The question ([`Adapter::consent`]) says, in plain words, that the app would add the
//! extension's absolute path to the list [`SETTINGS_KEY`] of that file, that pi would then
//! load the extension in every pi the user starts, where outside the app it does nothing,
//! and that the answer can be changed. The answer is kept in `<folder>/`[`CONSENT_FILE`]:
//! `yes` or `no` and a line feed; no file is "not asked yet". A kept yes whose entry is no
//! longer in pi's settings (the user took it out by hand) stands as not asked: the app asks
//! again and writes nothing back by itself.
//!
//! What a yes writes ([`Adapter::answer_consent`]): the one entry, the extension's absolute
//! path as a JSON string, at the end of the top-level array [`SETTINGS_KEY`], and nothing
//! else. Every other byte of the file stays as it was, its layout and the order of its keys
//! among them: the entry is put into the text, not by writing the parsed settings back.
//! Where the array is missing, the key and an array of the one entry are added as the
//! object's last member; where the file is missing, it is written as pi would, the object
//! with that one key, indented by two spaces. An entry already there is not added twice.
//! A no, and an ask again, take that one string out of the array with the comma that parted
//! it, and nothing else: an array left empty stays, and so does a file the app made.
//!
//! A file is changed safely: the new text is written to a temporary file in the folder of
//! the settings, which is then renamed over them, so pi never reads a half-written file;
//! the file's permissions are kept, and a settings file that is a symbolic link is changed
//! where it points. While it changes the file the Adapter holds pi's own lock beside it,
//! the folder `<settings file>.lock`, which pi takes before it writes its settings, and
//! gives up, changing nothing, when pi holds it past a short wait.
//!
//! A settings file the app cannot read as pi writes it is left as it was, and the
//! [`yaar_agent::Consent`] says so in `problem`, naming the file and why: not text, not
//! JSON, not a JSON object, a top-level [`SETTINGS_KEY`] that is no array of strings, or
//! two of them. pi reads its settings with `JSON.parse` (after a byte-order mark, which is
//! kept): what it cannot read, the app does not change.
//!
//! # What the extension reports
//!
//! The extension runs the reporting program [`REPORT_PROGRAM`], found by
//! [`REPORT_VARIABLE`], once for each event below, one at a time and in the order pi gave
//! them, with no shell between, so that the program's parent is pi itself. It writes the
//! report's payload on the program's standard input and reads its answer from its standard
//! output; the program passes the payload on, unread, to the Keeper as one
//! `yaar_link::Request::Report` of the Adapter [`NAME`] ([`pass_on`]).
//!
//! Outside the app the extension does nothing and says nothing. Once the user said yes, pi
//! loads it in every pi the user starts, in another terminal program too; there no
//! [`REPORT_VARIABLE`] is set, and the extension, which looks for it when pi loads it,
//! registers no handler and no tool, reports nothing and prints nothing. Inside the app
//! every shell sets it. A loaded extension reads the variable again at each report: in pi
//! the two are the same, and the extension's test takes it away between two reports of
//! one process, after which the extension reports nothing.
//!
//! Every handler awaits the end of its own report, the program's exit, before it returns,
//! as Claude Code's hooks are run to their end before Claude Code goes on. pi runs a tool
//! only once the `tool_call` handlers have returned: so the app is told that a shell
//! command begins before it runs, and a file it writes at once is credited to its session,
//! not to no agent. A report that fails (no program, no Keeper, a deadline) still ends the
//! handler, which then returns as if nothing were answered.
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
//! `ctx.cwd`: every report names its session and its folder. Not sent: the message's text
//! and images, a write's `content`, and an edit's `edits` (the texts it replaced and put
//! in), none of which the Adapter reads. An edit's `details.patch` is sent: its hunk
//! headers give the lines the Adapter reports, and its lines hold what the edit took out
//! and put in.
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
//! | any other | anything | nothing: the handler returns once the program has ended |
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
//! `sessionFile` as [`yaar_agent::Reported::cli_record`]. pi asks no permission: no event is ever
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
//! [`Adapter::consent`]: yaar_agent::Adapter::consent
//! [`Adapter::answer_consent`]: yaar_agent::Adapter::answer_consent
//! [`Adapter::asked`]: yaar_agent::Adapter::asked
//! [`Adapter::marks`]: yaar_agent::Adapter::marks

mod deliver;
mod install;
mod report;
mod reporter;
mod settings;

pub use deliver::{INSTRUCTION, ToPi};
pub use reporter::{DEADLINE, ReportAnswer, ReportPlace, TOOL_DEADLINE, pass_on};

use std::io;
use std::path::{Path, PathBuf};

use report::Remembered;
use yaar_agent::{
    Adapter, Answer, Asked, CliSession, Consent, ConsentAnswer, Delivery, Report, ReportError,
    Reported, ShellEntry,
};

/// The name the app knows pi by.
pub const NAME: &str = "pi";

/// The name a person reads.
pub const TITLE: &str = "pi";

/// What a launcher runs in the user's shell: pi itself, as the user types it.
pub const COMMAND: &str = "pi";

/// The folder, inside the Adapter's own folder of the app, that holds the extension.
pub const EXTENSION: &str = "extension";

/// The extension's file name in [`EXTENSION`].
pub const EXTENSION_FILE: &str = "yaar.ts";

/// The program of the app that pi's extension runs for each report, in the folder of the
/// app's programs beside the Keeper.
pub const REPORT_PROGRAM: &str = "yaar-pi-report";

/// The variable every shell of the app sets, naming [`REPORT_PROGRAM`] by its absolute
/// path: the extension finds it there, and an extension loaded where it is not set, in a pi
/// started outside the app, registers nothing, reports nothing and prints nothing.
pub const REPORT_VARIABLE: &str = "YAAR_PI_REPORT";

/// pi's own folder of settings, under the user's home: `PI_CODING_AGENT_DIR` names another
/// for pi, which the app does not follow.
pub const AGENT_FOLDER: &str = ".pi/agent";

/// pi's settings file in [`AGENT_FOLDER`].
pub const SETTINGS_FILE: &str = "settings.json";

/// The top-level key of [`SETTINGS_FILE`] whose array of paths lists the extensions pi
/// loads, besides those of its own `extensions` folder and `-e`.
pub const SETTINGS_KEY: &str = "extensions";

/// The file, in the Adapter's own folder of the app, that keeps the user's answer: `yes`
/// or `no` and a line feed.
pub const CONSENT_FILE: &str = "consent";

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

/// The Adapter of pi. It remembers, for each session, whether a run of its agent is open,
/// and knows the user's home, where pi keeps its settings.
#[derive(Debug, Default)]
pub struct Pi {
    memory: Remembered,
    // The user's home, under which pi keeps its settings; none for an Adapter that asks
    // nothing and writes nothing of pi's.
    home: Option<PathBuf>,
}

impl Pi {
    /// The Adapter, knowing no session yet and no home: it asks the user nothing and
    /// writes nothing of pi's, and pi in the app is any program.
    pub fn new() -> Pi {
        Pi::default()
    }

    /// The Adapter, knowing no session yet, for the user whose home is `home`: pi's
    /// settings are `home/`[`AGENT_FOLDER`]`/`[`SETTINGS_FILE`]. The Keeper hands it the
    /// home it was started with, and a test a folder of its own; the Adapter never reads
    /// one from its environment.
    pub fn in_home(home: &Path) -> Pi {
        Pi {
            memory: Remembered::default(),
            home: Some(home.to_owned()),
        }
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

    /// Writes `folder/`[`EXTENSION`]`/`[`EXTENSION_FILE`], as the crate's doc says, through
    /// a temporary file of its folder renamed into place, and gives
    /// [`yaar_agent::ShellEntry::Variable`] with [`REPORT_VARIABLE`] naming
    /// `companions/`[`REPORT_PROGRAM`] by its absolute path. Nothing of pi's is written.
    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
        install::write(folder, companions)
    }

    /// pi's question, as the crate's doc says, once pi is installed under the home this
    /// Adapter knows; none without a home or before pi's folder exists.
    fn consent(&self, folder: &Path) -> Option<Consent> {
        let home = self.home.as_deref()?;
        settings::consent(home, folder)
    }

    /// Keeps the answer and writes or takes out the one entry, as the crate's doc says.
    fn answer_consent(&self, folder: &Path, answer: ConsentAnswer) -> Option<Consent> {
        let home = self.home.as_deref()?;
        settings::answer(home, folder, answer)
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

    fn resume(&self, session: &CliSession, cli_record: Option<&Path>) -> String {
        deliver::resume(session, cli_record)
    }
}
