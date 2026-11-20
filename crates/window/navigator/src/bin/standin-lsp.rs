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
//! | `--log <file>` | appends one JSON object a line to `<file>`, below |
//!
//! # Its log
//!
//! Each line one object with `event` and what goes with it: `start` with `pid`, `cwd`,
//! `args` (every argument but the first) and `env` (the names and values of `HOME`,
//! `TMPDIR`, `XDG_CACHE_HOME` and every variable whose name starts with `STANDIN_`, where
//! set); `initialize` with `rootUri`, `initializationOptions` and `capabilities` as
//! received; `configuration` with the answer received; `request` with `method` and `id`
//! for every request received; `notification` with `method` for every notification
//! received; `ready`; `exit` with its code. A line is written before what it tells is
//! answered, so that a test that reads the log after an answer finds the line.
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
//!   under the root, in order of path, then line, as `Location`s whose range starts at the
//!   name; before it is ready, `[]`.
//! - `textDocument/references` once ready: every whole-word occurrence of the name in
//!   every `.sil` file under the root, by path, then line, then column; before, `[]`.
//! - `shutdown`: null; `exit`: it ends with 0 after a `shutdown`, else with 1. At the end
//!   of its input it ends with 0.
//! - Any other request: the error -32601. Positions are counted in UTF-16 units.
//!
//! Files under the root are read as they are on disk; folders whose name starts with `.`
//! are not entered.

fn main() {
    todo!()
}
