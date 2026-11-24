//! pi adapter: maps pi's extension events to the app's events, delivers reviews, and writes one
//! entry into pi's settings after the user's yes.
//!
//! The extension (`companions/pi/yaar.ts`, [`EXTENSION_SOURCE`]) runs [`REPORT_PROGRAM`] once per
//! event, in pi's order, with no shell between so that the program's parent is pi; payload on its
//! standard input, answer on its standard output. The handlers await the program: pi runs a tool
//! only after its `tool_call` handlers return, which is what credits a command's writes. A failed
//! report returns as if nothing were answered.
//!
//! A payload is one JSON object in pi's own words (extra keys are not read) with `event`,
//! `sessionId`, `sessionFile` (a path, or null) and `cwd`.
//!
//! | `event` | Further keys |
//! | --- | --- |
//! | `session_start` | `reason`: `startup`, `new`, `resume`, `reload` or `fork` |
//! | `session_shutdown` | `reason`: `quit`, `new`, `resume`, `reload` or `fork` |
//! | `input` | `source` (`interactive`, `rpc`, `extension`), `streamingBehavior` |
//! | `agent_start` | none |
//! | `tool_call` | `toolName`, `toolCallId`, `input`: `{"path"}` (files), `{"command"}` (bash) |
//! | `tool_result` | as `tool_call`, and `isError`; for `edit`, `details.patch`, the unified diff |
//! | `agent_settled` | `aborted` |
//! | [`REPLY_EVENT`] | `toolCallId`, `arguments`: `{"thread", "body", "addressed"}` |
//!
//! The program prints the bytes of the Keeper's `yaar_link::Reply::Answer`, nothing for any other
//! reply; with `--tool` a JSON object in every case. The extension reads:
//!
//! | Report | Answer | The extension |
//! | --- | --- | --- |
//! | `input` | `{"review": "<text>"}` | `transform`: the user's text, a blank line, the review |
//! | `input` | nothing | `{"action": "continue"}` |
//! | [`REPLY_EVENT`] | `{"text": "..."}` | gives the agent that text as the tool's result |
//! | [`REPLY_EVENT`] | `{"error": "<why>"}` | throws: pi shows the agent a failed tool |
//!
//! A run of pi's agent, from the user's message to its last word, is one turn; pi's `turn_start`
//! and `turn_end` are not. A turn begins at the first `input` or `agent_start` while no run of the
//! session is open, and ends at `agent_settled`: interrupted when `aborted`, else finished.
//!
//! A report that starts or ends no session, turn, command or file access gives nothing, `input`
//! while a run is open included (pi queues it into the open turn). A read through an `@` mention
//! or a shell command gives no event. pi asks no permission, so no event is
//! [`yaar_agent::Event::Permission`].

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

pub const NAME: &str = "pi";

pub const TITLE: &str = "pi";

pub const COMMAND: &str = "pi";

/// The folder, in the Adapter's own folder of the app, that holds [`EXTENSION_FILE`].
pub const EXTENSION: &str = "extension";

pub const EXTENSION_FILE: &str = "yaar.ts";

/// The program pi's extension runs for each report, in the folder of the app's programs
/// beside the Keeper.
pub const REPORT_PROGRAM: &str = "yaar-pi-report";

/// Set in every shell of the app; names [`REPORT_PROGRAM`] by its absolute path.
pub const REPORT_VARIABLE: &str = "YAAR_PI_REPORT";

/// `PI_CODING_AGENT_DIR` names another folder for pi, which the app does not follow.
pub const AGENT_FOLDER: &str = ".pi/agent";

pub const SETTINGS_FILE: &str = "settings.json";

pub const SETTINGS_KEY: &str = "extensions";

/// `yes` or `no` and a line feed.
pub const CONSENT_FILE: &str = "consent";

pub const REPLY_TOOL: &str = "reply_to_thread";

/// The `event` of the payload of a call of [`REPLY_TOOL`] (same string, different role).
pub const REPLY_EVENT: &str = "reply_to_thread";

pub const EXTENSION_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../companions/pi/yaar.ts"
));

#[derive(Debug, Default)]
pub struct Pi {
    memory: Remembered,
    home: Option<PathBuf>,
}

impl Pi {
    /// No home: asks nothing and writes nothing of pi's.
    pub fn new() -> Pi {
        Pi::default()
    }

    /// The Adapter never reads the home from its environment.
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

    fn install(&self, folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
        install::write(folder, companions)
    }

    fn consent(&self, folder: &Path) -> Option<Consent> {
        let home = self.home.as_deref()?;
        settings::consent(home, folder)
    }

    fn answer_consent(&self, folder: &Path, answer: ConsentAnswer) -> Option<Consent> {
        let home = self.home.as_deref()?;
        settings::answer(home, folder, answer)
    }

    /// pi reports an interrupt itself.
    fn marks(&self) -> Vec<Vec<u8>> {
        Vec::new()
    }

    fn report(&mut self, report: &Report<'_>) -> Result<Vec<Reported>, ReportError> {
        self.memory.report(report)
    }

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
