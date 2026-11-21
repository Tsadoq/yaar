//! The answers: hover, definition and references for a place, a file opened through one,
//! and text match while no server is ready (FR-051, FR-052, FR-054, FR-055, FR-068).
//!
//! # Which server answers a place
//!
//! The row is `Core::registry.for_path(place.path)`, by the file's extension; a file of a
//! language no row knows is always answered by text match over the worktree (DEC-047). By
//! the place's tree:
//!
//! | Tree | Key | Root, and the file asked about |
//! | --- | --- | --- |
//! | `Worktree`, a relative path | the row's new side | the worktree's folder; the file on disk |
//! | `Worktree`, an absolute path: a file outside the worktree, or ignored in it, opened through an answer | the row's new side | the worktree's folder; the file at that path, as the server is asked about it. Text match runs over the worktree |
//! | `Version(v)` | the row's old side of `v` | the export of `v`, once `old_side::root` gives it. While it is being written, the ask is answered at once by text match over the worktree, marked: no server is ready for the file (FR-068) |
//! | `Past { version, text }` | the row's new side | the worktree's folder. The line is carried from `text` to the file on disk through the diff of the two (`yaar_diff::FileDiff`): a line no hunk touches is asked at its line on disk; any other line, and a file no longer on disk, by text match over the worktree, marked (OLD-07, ADR-016 point 6) |
//!
//! `lifecycle::ensure` makes sure the key is looked for; `lifecycle::ready_server` says
//! whether a server of the key runs and is ready, and that alone decides: a server ready
//! answers even while the state shown is the question of an earlier candidate
//! ([`crate::State::Asking`]), since the later candidates run meanwhile. With none ready,
//! the place is answered by text match over the root ([`found`]) and the answer marked
//! guessed. When one is ready, the file's text as the root holds it now is handed to the server
//! (`Server::open`), and the request sent at the place: `textDocument/hover`,
//! `textDocument/definition`, or `textDocument/references` with `includeDeclaration` true;
//! line less one, column in UTF-16 units of the line read from the file
//! (`lsp::utf16_column`). The server's answer ([`server_answered`]) is told unmarked:
//! `lsp::hover_text` for a hover; for the others each `lsp::spots`, as a
//! [`crate::Location`] whose line is the spot's plus one, whose column is in bytes of that
//! line as its file holds it (`lsp::byte_column`), and whose text is that line; references
//! sorted by path, then line. A server that answers with an error, or ends before it
//! answered ([`server_gone`]), has the ask answered by text match, marked. So does a ready
//! server the Navigator ends itself when its key is stopped or searched for again:
//! `lifecycle.rs` tells [`server_gone`] after each such end. An ask is held for the server's
//! process as well as for its request number, which each process counts from the start, so
//! that none takes the answer of a later process; an ask held for a process that is no longer
//! the key's ready one is answered by text match at the next ask or answer, whatever ended the
//! process. At its own end the Navigator answers no ask: `lifecycle::close` has
//! [`forget_the_waiting`] drop the asks still waiting before it stops the servers, and no
//! text search starts at a quit.
//!
//! # Text match
//!
//! `yaar_text_search::TextSearch::search` of the place's name over the root, on a thread of
//! its own, which sends `Event::Found`. The matches are ordered: those of the file under
//! the pointer first, then those of the files of `place.shown` in that order, then the rest
//! by path; each file's by line. A hover by text match is the first three matches that are
//! not the place's own line, each written `<path relative to the root>:<line>: <the line,
//! trimmed>`, joined by line feeds; none when there is none. A definition and a reference
//! list by text match are every match, in that order. Every one of them is marked guessed.
//!
//! # Cancelling, and files that change
//!
//! [`cancel`] cancels the server's request (`Server::cancel`) or drops the search's result;
//! a cancelled ask is never answered. [`files_changed`] hands the paths, made absolute and
//! each once, to every running new-side server of the workspace, starting or ready
//! (`Server::files_changed`; `lifecycle::running_server`): a path told twice, and one that no
//! longer exists, are the driver's to tell as changed or deleted, and the driver holds what a
//! server is sent before it answered `initialize`. A file's text is sent at each ask.
//!
//! # Opening a file
//!
//! [`open`] reads the location's file whole as UTF-8 and tells [`crate::Told::Opened`]
//! with its kind: [`crate::FileKind::Version`] when it lies in an export
//! (`old_side::version_of`); [`crate::FileKind::Worktree`] when it lies in the folder of a
//! workspace, its links resolved, and `yaar_text_search::is_ignored` says git does not
//! ignore it; else [`crate::FileKind::Outside`]. A file that cannot be read, or is not
//! UTF-8, is told as the reason.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_diff::FileDiff;
use yaar_link::WorkspaceId;
use yaar_text_search::{Found, TextSearch, is_ignored};

use crate::driver::Server;
use crate::lifecycle::{ensure, ready_server, running_server};
use crate::lsp::{Spot, byte_column, file_uri, hover_text, spots, utf16_column};
use crate::navigator::{Core, Event, ServerKey, Side};
use crate::old_side;
use crate::port::{
    Answered, AskId, FileKind, Location, OpenedFile, Place, Told, Tree, WorkspaceRoot,
};

/// How many lines a hover by text match shows.
const HOVER_LINES: usize = 3;

/// What a place is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Question {
    /// `textDocument/hover`.
    Hover,
    /// `textDocument/definition`.
    Definition,
    /// `textDocument/references`.
    References,
}

impl Question {
    /// The request that asks it of a server.
    fn method(self) -> &'static str {
        match self {
            Question::Hover => "textDocument/hover",
            Question::Definition => "textDocument/definition",
            Question::References => "textDocument/references",
        }
    }
}

/// What this module keeps in [`Core::answers`]: the asks that wait, for a server's answer,
/// a search's or an export.
#[derive(Default)]
pub(crate) struct Answers {
    /// Every ask that was neither answered nor cancelled.
    waiting: HashMap<AskId, Pending>,
    /// The old sides asked about while their export was written, each with the row asked:
    /// workspace, version, row. The server of each is ensured when the export is ready.
    old: BTreeSet<(WorkspaceId, String, String)>,
}

/// One ask that waits.
struct Pending {
    /// What was asked.
    question: Question,
    /// Where.
    place: Place,
    /// The folder text match runs over, if it has to.
    search: PathBuf,
    /// The line of the place in the files of `search`, when the place's own line is one of
    /// them: a hover by text match leaves it out.
    own_line: Option<u32>,
    /// What it waits for.
    how: How,
}

/// What an ask waits for.
enum How {
    /// The answer of a server to one of its requests. The request's number is the server's
    /// own, a count that starts again with each process, so the process is named too: a
    /// number held for a server that has since ended is not the next one's.
    Server {
        key: ServerKey,
        pid: u32,
        request: u64,
    },
    /// A text search.
    Search,
}

/// How a place is answered.
enum Plan {
    /// By text match over `search`.
    Text {
        search: PathBuf,
        own_line: Option<u32>,
    },
    /// By the server of `key`, when it is ready, asked about `file`; else by text match
    /// over `root`.
    Server {
        key: ServerKey,
        root: PathBuf,
        file: PathBuf,
        own_line: Option<u32>,
        /// For the new side of a past change, that file's text then: the place's line is
        /// carried from it to the file on disk.
        past: Option<std::sync::Arc<str>>,
    },
}

/// What an answer holds, before the question decides which part is told.
#[derive(Default)]
struct Reply {
    hover: Option<String>,
    places: Vec<Location>,
}

/// [`crate::Ask::Hover`], [`crate::Ask::Definition`] and [`crate::Ask::References`].
pub(crate) fn ask(core: &mut Core, id: AskId, question: Question, place: Place) {
    answer_the_stale(core);
    // With no worktree there is nothing to ask or to search: an empty answer, marked.
    let Some(worktree) = core
        .folder_of(&place.workspace)
        .map(|folder| absolute(&folder))
    else {
        return tell(core, id, question, Reply::default(), true);
    };
    let plan = plan(core, &place, &worktree);
    let pending = |search: PathBuf, own_line: Option<u32>, how: How| Pending {
        question,
        place: place.clone(),
        search,
        own_line,
        how,
    };
    match plan {
        Plan::Text { search, own_line } => {
            by_text(core, id, pending(search, own_line, How::Search));
        }
        Plan::Server {
            key,
            root,
            file,
            own_line,
            past,
        } => {
            ensure(core, &key, &root);
            match send(core, &key, &file, &place, question, past.as_deref()) {
                Some((pid, request)) => {
                    let how = How::Server { key, pid, request };
                    core.answers
                        .waiting
                        .insert(id, pending(root, own_line, how));
                }
                None => by_text(core, id, pending(root, own_line, How::Search)),
            }
        }
    }
}

/// Which server answers `place`, and where it is asked, as the module doc's table says.
fn plan(core: &mut Core, place: &Place, worktree: &Path) -> Plan {
    let row = core
        .registry
        .for_path(&place.path)
        .map(|row| row.name.clone());
    let key = |language: String, side: Side| ServerKey {
        workspace: place.workspace.clone(),
        language,
        side,
    };
    let in_worktree = Plan::Text {
        search: worktree.to_path_buf(),
        own_line: matches!(place.tree, Tree::Worktree).then_some(place.line),
    };
    // A file of a language no row knows has no server, whatever its side.
    let Some(row) = row else {
        return in_worktree;
    };
    match &place.tree {
        Tree::Worktree => Plan::Server {
            key: key(row, Side::New),
            root: worktree.to_path_buf(),
            file: worktree.join(&place.path),
            own_line: Some(place.line),
            past: None,
        },
        Tree::Version(version) => match old_side::root(core, &place.workspace, version) {
            Some(export) => {
                let export = absolute(&export);
                Plan::Server {
                    key: key(
                        row,
                        Side::Old {
                            version: version.clone(),
                        },
                    ),
                    file: export.join(&place.path),
                    root: export,
                    own_line: Some(place.line),
                    past: None,
                }
            }
            // Being written: text match over the worktree meanwhile, and the old side's
            // server when `export_ready` says so.
            None => {
                core.answers
                    .old
                    .insert((place.workspace.clone(), version.clone(), row));
                Plan::Text {
                    search: worktree.to_path_buf(),
                    own_line: None,
                }
            }
        },
        Tree::Past { text, .. } => Plan::Server {
            key: key(row, Side::New),
            root: worktree.to_path_buf(),
            file: worktree.join(&place.path),
            own_line: None,
            past: Some(text.clone()),
        },
    }
}

/// Sends the request of `question` about `place` to the server of `key`, when it is ready:
/// the text of `file` as it is now first, then the request. Gives the server's process id and
/// the request's number. None when no
/// server of the key is ready, or the file cannot be read as text, or has no such line (a
/// past line that changed since, or a file gone), or the server refuses: the ask is then
/// answered by text match.
fn send(
    core: &Core,
    key: &ServerKey,
    file: &Path,
    place: &Place,
    question: Question,
    past: Option<&str>,
) -> Option<(u32, u64)> {
    // An address is made of an absolute path only.
    if !file.is_absolute() {
        return None;
    }
    let server = ready_server(core, key)?;
    let text = std::fs::read_to_string(file).ok()?;
    let line = place.line.checked_sub(1)?;
    let line = match past {
        Some(past) => carry(past, &text, line)?,
        None => line,
    };
    let character = utf16_column(text.lines().nth(line as usize)?, place.column);
    server.open(file, &text).ok()?;
    let mut params = json!({
        "textDocument": {"uri": file_uri(file)},
        "position": {"line": line, "character": character},
    });
    if question == Question::References {
        params["context"] = json!({"includeDeclaration": true});
    }
    let request = server.request(question.method(), params).ok()?;
    Some((server.pid(), request))
}

/// The line of `now` that line `line`, from 0, of `past` has become, when no change
/// between them touches it; none when one does, or the line is not there. The two are
/// diffed as the app diffs everything (`yaar_diff`), and a line no hunk touches moves by the
/// lines the hunks before it added or removed. A block moved elsewhere is not followed: a
/// line in a hunk is a changed line (ADR-013's first rule).
fn carry(past: &str, now: &str, line: u32) -> Option<u32> {
    let diff = FileDiff::new(past.as_bytes(), now.as_bytes());
    if line >= diff.line_count(yaar_diff::Side::Old) {
        return None;
    }
    let mut moved = i64::from(line);
    for hunk in diff.hunks() {
        if hunk.old.end <= line {
            moved += i64::from(hunk.new.end - hunk.new.start);
            moved -= i64::from(hunk.old.end - hunk.old.start);
        } else if hunk.old.start <= line {
            return None;
        } else {
            break;
        }
    }
    u32::try_from(moved).ok()
}

/// [`crate::Ask::Open`].
pub(crate) fn open(core: &mut Core, id: AskId, location: Location) {
    let file = opened(core, &location.path);
    core.tell(Told::Opened { id, file });
}

/// The file at `path`, read whole as UTF-8, with what it is. The reason, as a sentence,
/// when it cannot be read as text.
fn opened(core: &Core, path: &Path) -> Result<OpenedFile, String> {
    if !path.is_absolute() {
        return Err(format!("{} is not an absolute path", path.display()));
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8 text", path.display()))?;
    Ok(OpenedFile {
        path: path.to_path_buf(),
        kind: kind_of(core, path),
        text,
    })
}

/// What the file at `path` is, for the comments on it: in an export, in a worktree and not
/// ignored by git, or anything else. `yaar_text_search::is_ignored` is called once for each
/// workspace folder that holds the file, until one of them does not ignore it (so twice for
/// a folder nested in another), and reads git's configuration each time it is called.
fn kind_of(core: &Core, path: &Path) -> FileKind {
    if let Some((workspace, version, relative)) = old_side::version_of(core, path) {
        return FileKind::Version {
            workspace,
            version,
            relative,
        };
    }
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let folders: Vec<(WorkspaceId, PathBuf)> = core
        .workspaces
        .iter()
        .filter_map(|root: &WorkspaceRoot| {
            let folder = root.folder.as_ref()?;
            let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| absolute(folder));
            Some((root.id.clone(), folder))
        })
        .collect();
    worktree_kind(&resolved, &folders, is_ignored)
}

/// [`FileKind::Worktree`] for the first of `folders` that holds `resolved` and whose git
/// rules (`ignored`) do not leave it out; else [`FileKind::Outside`].
fn worktree_kind(
    resolved: &Path,
    folders: &[(WorkspaceId, PathBuf)],
    ignored: impl Fn(&Path, &Path) -> bool,
) -> FileKind {
    for (workspace, folder) in folders {
        if let Ok(relative) = resolved.strip_prefix(folder)
            && !ignored(folder, resolved)
        {
            return FileKind::Worktree {
                workspace: workspace.clone(),
                relative: relative.to_path_buf(),
            };
        }
    }
    FileKind::Outside
}

/// [`crate::Ask::Cancel`].
pub(crate) fn cancel(core: &mut Core, id: AskId) {
    let Some(pending) = core.answers.waiting.remove(&id) else {
        return;
    };
    // Only the server the request was sent to is told: a later process of the key counts its
    // requests again from the start.
    if let How::Server { key, pid, request } = pending.how
        && let Some(server) = ready_server(core, &key)
        && server.pid() == pid
    {
        server.cancel(request);
    }
}

/// [`crate::Ask::FilesChanged`].
pub(crate) fn files_changed(core: &mut Core, workspace: &WorkspaceId, paths: &[PathBuf]) {
    let Some(folder) = core.folder_of(workspace) else {
        return;
    };
    let changed = joined(&absolute(&folder), paths);
    if changed.is_empty() {
        return;
    }
    for row in core.registry.rows() {
        let key = ServerKey {
            workspace: workspace.clone(),
            language: row.name.clone(),
            side: Side::New,
        };
        if let Some(server) = running_server(core, &key) {
            server.files_changed(&changed);
        }
    }
}

/// `paths` joined with `folder` (a path that is absolute stays as it is), each once, in the
/// order they first come.
fn joined(folder: &Path, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .iter()
        .map(|path| folder.join(path))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// A text search ended: its ask is answered, marked, unless it was cancelled.
pub(crate) fn found(core: &mut Core, id: AskId, found: Result<Vec<Found>, String>) {
    // What waits under this id is a search's, or the id is of an ask already answered or
    // cancelled.
    if !matches!(
        core.answers.waiting.get(&id),
        Some(Pending {
            how: How::Search,
            ..
        })
    ) {
        return;
    }
    let Some(pending) = core.answers.waiting.remove(&id) else {
        return;
    };
    let reply = match found {
        Ok(list) => by_match(
            pending.question,
            &pending.place,
            &pending.search,
            pending.own_line,
            list,
        ),
        Err(reason) => {
            eprintln!("text match failed: {reason}");
            Reply::default()
        }
    };
    tell(core, id, pending.question, reply, true);
}

/// A server of `key` answered its request `request`.
pub(crate) fn server_answered(
    core: &mut Core,
    key: &ServerKey,
    request: u64,
    result: Result<serde_json::Value, String>,
) {
    // A server answers only after it was ready, and the answer is its current process's: an
    // ask held for another process of the key is answered first, by text match.
    answer_the_stale(core);
    let waiting = core.answers.waiting.iter().find_map(|(id, pending)| {
        matches!(&pending.how, How::Server { key: k, request: r, .. } if k == key && *r == request)
            .then_some(*id)
    });
    // None: the ask was cancelled, and the answer comes after.
    let Some(id) = waiting else {
        return;
    };
    let Some(pending) = core.answers.waiting.remove(&id) else {
        return;
    };
    match result {
        Ok(value) => {
            let reply = reply_of(pending.question, &value);
            tell(core, id, pending.question, reply, false);
        }
        Err(_) => by_text(core, id, pending),
    }
}

/// Every ask held for a server process that is no longer the key's ready one is answered by
/// text match, marked. Every end of a ready server is told to [`server_gone`], so this is
/// the second line: it keeps an ask from ever waiting for a process that is not the key's
/// ready one, or taking the answer of a later process that counts its requests again.
fn answer_the_stale(core: &mut Core) {
    let stale: Vec<AskId> = core
        .answers
        .waiting
        .iter()
        .filter(|(_, pending)| match &pending.how {
            How::Server { key, pid, .. } => ready_server(core, key).map(Server::pid) != Some(*pid),
            How::Search => false,
        })
        .map(|(id, _)| *id)
        .collect();
    for id in stale {
        if let Some(pending) = core.answers.waiting.remove(&id) {
            by_text(core, id, pending);
        }
    }
}

/// The server of `key` ended: every ask still waiting for it is answered by text match.
pub(crate) fn server_gone(core: &mut Core, key: &ServerKey) {
    let ids: Vec<AskId> = core
        .answers
        .waiting
        .iter()
        .filter(|(_, pending)| matches!(&pending.how, How::Server { key: k, .. } if k == key))
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if let Some(pending) = core.answers.waiting.remove(&id) {
            by_text(core, id, pending);
        }
    }
}

/// The Navigator ends: every ask still waiting is forgotten, not answered. Nobody is left to
/// read an answer, and an answer by text match would start a search over a worktree that
/// nothing waits for, so none starts at a quit. Called before the servers stop, since the end
/// of a server is what answers the asks waiting at it.
pub(crate) fn forget_the_waiting(core: &mut Core) {
    core.answers.waiting.clear();
}

/// The export of `version` of `workspace` is written, at `root`: every row asked about on
/// that old side while it was written has its old side's key ensured
/// (`lifecycle::ensure`, the export as its root). No ask waits for an export: each was
/// answered by text match when it came.
pub(crate) fn export_ready(core: &mut Core, workspace: &WorkspaceId, version: &str, root: &Path) {
    for row in forget_old(core, workspace, version) {
        let key = ServerKey {
            workspace: workspace.clone(),
            language: row,
            side: Side::Old {
                version: version.to_owned(),
            },
        };
        ensure(core, &key, &absolute(root));
    }
}

/// The export of `version` of `workspace` could not be written: nothing waits for it, and
/// the rows asked about are forgotten; the next ask about that old side asks for the
/// export again.
pub(crate) fn export_failed(core: &mut Core, workspace: &WorkspaceId, version: &str) {
    forget_old(core, workspace, version);
}

/// Takes out of the old sides asked about those of `version` of `workspace`; gives their
/// rows.
fn forget_old(core: &mut Core, workspace: &WorkspaceId, version: &str) -> Vec<String> {
    let rows: Vec<(WorkspaceId, String, String)> = core
        .answers
        .old
        .iter()
        .filter(|(w, v, _)| w == workspace && v == version)
        .cloned()
        .collect();
    for entry in &rows {
        core.answers.old.remove(entry);
    }
    rows.into_iter().map(|(_, _, row)| row).collect()
}

/// Answers `pending` by text match over its folder: the search runs on a thread of its own
/// and sends [`Event::Found`].
fn by_text(core: &mut Core, id: AskId, mut pending: Pending) {
    pending.how = How::Search;
    let events = core.events.clone();
    let folder = pending.search.clone();
    let name = pending.place.name.clone();
    let started = std::thread::Builder::new()
        .name("navigator-text-search".to_owned())
        .spawn(move || {
            let found = TextSearch::new(TextSearch::half_the_cores())
                .all(&folder, &name)
                .map_err(|error| error.to_string());
            // A Navigator that ended hears nothing.
            let _ = events.send(Event::Found { id, found });
        });
    match started {
        Ok(_) => {
            core.answers.waiting.insert(id, pending);
        }
        Err(error) => {
            eprintln!("text match failed: cannot start a search: {error}");
            tell(core, id, pending.question, Reply::default(), true);
        }
    }
}

/// Tells the answer to `question` as [`Told`] says it.
fn tell(core: &Core, id: AskId, question: Question, reply: Reply, guessed: bool) {
    let told = match question {
        Question::Hover => Told::Hover {
            id,
            answer: Answered {
                value: reply.hover,
                guessed,
            },
        },
        Question::Definition => Told::Definition {
            id,
            answer: Answered {
                value: reply.places,
                guessed,
            },
        },
        Question::References => Told::References {
            id,
            answer: Answered {
                value: reply.places,
                guessed,
            },
        },
    };
    core.tell(told);
}

/// What a server's `result` to `question` says.
fn reply_of(question: Question, result: &Value) -> Reply {
    match question {
        Question::Hover => Reply {
            hover: hover_text(result),
            places: Vec::new(),
        },
        Question::Definition | Question::References => {
            // Read once for each file, whatever the number of places in it.
            let mut files: HashMap<PathBuf, Option<String>> = HashMap::new();
            let mut line_of = |path: &Path, line: u32| {
                files
                    .entry(path.to_path_buf())
                    .or_insert_with(|| std::fs::read_to_string(path).ok())
                    .as_deref()
                    .and_then(|text| text.lines().nth(line as usize))
                    .map(str::to_owned)
            };
            Reply {
                hover: None,
                places: locations(
                    &spots(result),
                    &mut line_of,
                    question == Question::References,
                ),
            }
        }
    }
}

/// The [`Location`]s of `spots`: the line from 1, the column in bytes of the line as
/// `line_of` gives it (the spot's own when it gives none), the text of that line. With
/// `sorted`, by path, then line, then column: a server's references come in any order.
fn locations(
    spots: &[Spot],
    line_of: &mut impl FnMut(&Path, u32) -> Option<String>,
    sorted: bool,
) -> Vec<Location> {
    let mut places: Vec<Location> = spots
        .iter()
        .map(|spot| {
            let text = line_of(&spot.path, spot.line);
            Location {
                path: spot.path.clone(),
                line: spot.line + 1,
                column: text
                    .as_deref()
                    .map_or(spot.character, |line| byte_column(line, spot.character)),
                text: text.unwrap_or_default(),
            }
        })
        .collect();
    if sorted {
        places.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    }
    places
}

/// The answer by text match: `found` ordered as the module doc says, and the hover's three
/// lines, or every match as a place.
fn by_match(
    question: Question,
    place: &Place,
    root: &Path,
    own_line: Option<u32>,
    mut found: Vec<Found>,
) -> Reply {
    let pointer = root.join(&place.path);
    let shown: Vec<PathBuf> = place.shown.iter().map(|path| root.join(path)).collect();
    found.sort_by(|a, b| {
        let key = |found: &Found| {
            (
                rank(&found.path, &pointer, &shown),
                found.path.clone(),
                found.line,
                found.column,
            )
        };
        key(a).cmp(&key(b))
    });
    match question {
        Question::Hover => {
            let lines: Vec<String> = found
                .iter()
                .filter(|found| {
                    !(found.path == pointer && own_line.is_some_and(|line| line == found.line))
                })
                .take(HOVER_LINES)
                .map(|found| {
                    let path = found.path.strip_prefix(root).unwrap_or(&found.path);
                    format!("{}:{}: {}", path.display(), found.line, found.text.trim())
                })
                .collect();
            Reply {
                hover: (!lines.is_empty()).then(|| lines.join("\n")),
                places: Vec::new(),
            }
        }
        Question::Definition | Question::References => Reply {
            hover: None,
            places: found
                .into_iter()
                .map(|found| Location {
                    path: found.path,
                    line: found.line,
                    column: found.column,
                    text: found.text,
                })
                .collect(),
        },
    }
}

/// Where a file's matches come in the list: the file under the pointer first, the files of
/// the change on screen next in their order, the rest after.
fn rank(path: &Path, pointer: &Path, shown: &[PathBuf]) -> usize {
    if path == pointer {
        0
    } else {
        shown
            .iter()
            .position(|shown| shown == path)
            .map_or(shown.len() + 1, |at| at + 1)
    }
}

/// `path` as an absolute path: the working folder joined to a relative one.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use super::*;
    use crate::port::RepositoryConsent;
    use crate::registry::Registry;
    use crate::{NavigatorConfig, SystemClock, fetch, lifecycle};
    use yaar_link::RepositoryId;

    fn hit(path: &str, line: u32, text: &str) -> Found {
        Found {
            path: PathBuf::from(path),
            line,
            column: 4,
            text: text.to_owned(),
        }
    }

    fn place(path: &str, line: u32, shown: &[&str]) -> Place {
        Place {
            workspace: WorkspaceId::new("1".repeat(32)),
            tree: Tree::Worktree,
            path: PathBuf::from(path),
            line,
            column: 0,
            name: "name".to_owned(),
            shown: shown.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn a_line_no_hunk_touches_moves_with_the_lines_added_or_removed_before_it() {
        // A line added in front, `b` changed, `c` unchanged after both.
        let (past, now) = ("a\nb\nc\n", "x\na\nb2\nc\n");
        assert_eq!(carry(past, now, 0), Some(1));
        assert_eq!(carry(past, now, 1), None, "a changed line is not carried");
        assert_eq!(carry(past, now, 2), Some(3));
        // A line removed in front moves the ones after it up.
        assert_eq!(carry("a\nb\n", "b\n", 0), None);
        assert_eq!(carry("a\nb\n", "b\n", 1), Some(0));
        // A line added after it moves nothing.
        assert_eq!(carry("a\n", "a\nb\n", 0), Some(0));
        // A file that did not change.
        assert_eq!(carry("a\nb\n", "a\nb\n", 1), Some(1));
        // A line the past text does not have.
        assert_eq!(carry("a\n", "a\n", 1), None);
        assert_eq!(carry("", "a\n", 0), None);
    }

    #[test]
    fn matches_come_by_the_file_under_the_pointer_then_the_files_on_screen_then_path_and_line() {
        let list = vec![
            hit("/w/z.sil", 1, "z"),
            hit("/w/b.sil", 7, "b7"),
            hit("/w/a.sil", 2, "a2"),
            hit("/w/b.sil", 3, "b3"),
            hit("/w/pointer.sil", 9, "p9"),
            hit("/w/c.sil", 1, "c"),
        ];
        let reply = by_match(
            Question::References,
            &place("pointer.sil", 9, &["z.sil", "b.sil"]),
            Path::new("/w"),
            Some(9),
            list,
        );
        let order: Vec<(String, u32)> = reply
            .places
            .iter()
            .map(|place| (place.path.display().to_string(), place.line))
            .collect();
        assert_eq!(
            order,
            [
                ("/w/pointer.sil".to_owned(), 9),
                ("/w/z.sil".to_owned(), 1),
                ("/w/b.sil".to_owned(), 3),
                ("/w/b.sil".to_owned(), 7),
                ("/w/a.sil".to_owned(), 2),
                ("/w/c.sil".to_owned(), 1),
            ]
        );
        // The pointer's own line stays in a list of places: every match is one.
        assert_eq!(reply.places[0].text, "p9");
    }

    #[test]
    fn a_hover_by_text_match_is_three_lines_other_than_the_places_own() {
        let list = vec![
            hit("/w/main.sil", 2, "  use total  "),
            hit("/w/lib.sil", 1, "def total"),
            hit("/w/lib.sil", 5, "total()"),
            hit("/w/zeta.sil", 3, "use total"),
            hit("/w/zeta.sil", 4, "use total again"),
        ];
        let at = place("main.sil", 2, &[]);
        let root = Path::new("/w");
        let hover = by_match(Question::Hover, &at, root, Some(2), list.clone()).hover;
        assert_eq!(
            hover.as_deref(),
            Some("lib.sil:1: def total\nlib.sil:5: total()\nzeta.sil:3: use total")
        );
        // A place whose line is not a line of the files searched leaves nothing out, and a
        // line is trimmed.
        let hover = by_match(Question::Hover, &at, root, None, list).hover;
        assert_eq!(
            hover.as_deref(),
            Some("main.sil:2: use total\nlib.sil:1: def total\nlib.sil:5: total()")
        );
        // Only the place's own line: nothing to show.
        let only = vec![hit("/w/main.sil", 2, "use total")];
        assert_eq!(
            by_match(Question::Hover, &at, root, Some(2), only).hover,
            None
        );
        assert_eq!(
            by_match(Question::Hover, &at, root, Some(2), vec![]).hover,
            None
        );
    }

    #[test]
    fn places_are_told_in_bytes_from_the_line_and_references_are_sorted() {
        // `é` is two bytes and one unit, the clef four bytes and two.
        let lines = ["use é 𝄞 name", "name"];
        let mut line_of = |path: &Path, line: u32| {
            (path == Path::new("/w/a.sil"))
                .then(|| lines.get(line as usize).map(|text| (*text).to_owned()))
                .flatten()
        };
        let spots = vec![
            Spot {
                path: PathBuf::from("/w/b.sil"),
                line: 0,
                character: 2,
            },
            Spot {
                path: PathBuf::from("/w/a.sil"),
                line: 1,
                character: 0,
            },
            Spot {
                path: PathBuf::from("/w/a.sil"),
                line: 0,
                character: 9,
            },
        ];
        let sorted = locations(&spots, &mut line_of, true);
        let shown: Vec<(&str, u32, u32, &str)> = sorted
            .iter()
            .map(|at| {
                (
                    at.path.to_str().unwrap(),
                    at.line,
                    at.column,
                    at.text.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                // 4 + 2 (é) + 1 + 4 (the clef) + 1 = 12 bytes before `name`.
                ("/w/a.sil", 1, 12, "use é 𝄞 name"),
                ("/w/a.sil", 2, 0, "name"),
                // A file that cannot be read: the spot's own column, no text.
                ("/w/b.sil", 1, 2, ""),
            ]
        );
        // A definition keeps the server's order.
        let kept = locations(&spots, &mut line_of, false);
        assert_eq!(kept[0].path, PathBuf::from("/w/b.sil"));
    }

    #[test]
    fn a_file_is_the_worktrees_unless_git_ignores_it_and_is_outside_in_any_other_case() {
        let (one, two) = (
            WorkspaceId::new("1".repeat(32)),
            WorkspaceId::new("2".repeat(32)),
        );
        let folders = vec![
            (one.clone(), PathBuf::from("/w/one")),
            (two.clone(), PathBuf::from("/w/one/nested")),
        ];
        let ignored = |_: &Path, path: &Path| path.starts_with("/w/one/vendor");
        assert_eq!(
            worktree_kind(Path::new("/w/one/src/a.rs"), &folders, ignored),
            FileKind::Worktree {
                workspace: one.clone(),
                relative: PathBuf::from("src/a.rs")
            }
        );
        assert_eq!(
            worktree_kind(Path::new("/w/one/vendor/a.rs"), &folders, ignored),
            FileKind::Outside
        );
        // A folder inside another is the other's too: the first one that holds the file
        // and does not ignore it.
        let nested_ignored = |folder: &Path, _: &Path| folder == Path::new("/w/one");
        assert_eq!(
            worktree_kind(Path::new("/w/one/nested/a.rs"), &folders, nested_ignored),
            FileKind::Worktree {
                workspace: two,
                relative: PathBuf::from("a.rs")
            }
        );
        assert_eq!(
            worktree_kind(Path::new("/elsewhere/a.rs"), &folders, ignored),
            FileKind::Outside
        );
        // A path that is only named like the folder, not inside it.
        assert_eq!(
            worktree_kind(Path::new("/w/one-more/a.rs"), &folders, ignored),
            FileKind::Outside
        );
    }

    #[test]
    fn changed_paths_are_joined_with_the_folder_and_told_once_each() {
        let paths = vec![
            PathBuf::from("a.rs"),
            PathBuf::from("/elsewhere/b.rs"),
            PathBuf::from("a.rs"),
            PathBuf::from("/w/a.rs"),
        ];
        assert_eq!(
            joined(Path::new("/w"), &paths),
            [PathBuf::from("/w/a.rs"), PathBuf::from("/elsewhere/b.rs")]
        );
        assert_eq!(joined(Path::new("/w"), &[]), Vec::<PathBuf>::new());
    }

    /// A Navigator's core with no row and one workspace whose folder is not there, and the
    /// ends of its channels.
    fn core() -> (Core, Receiver<Event>, async_channel::Receiver<Told>) {
        let (events, inbox) = channel();
        let (told, heard) = async_channel::unbounded();
        let workspace = WorkspaceRoot {
            id: WorkspaceId::new("1".repeat(32)),
            repository: RepositoryId::new("1".repeat(32)),
            folder: Some(PathBuf::from("/no-such-folder-of-the-answers-tests")),
        };
        let core = Core {
            config: NavigatorConfig {
                builtin: String::new(),
                user_file: PathBuf::new(),
                servers: PathBuf::new(),
                cache: PathBuf::new(),
                search_path: Vec::new(),
                environment: Vec::new(),
                platform: None,
                stop_after: Duration::from_secs(1),
                clock: Arc::new(SystemClock),
            },
            registry: Registry::default(),
            told,
            events,
            workspaces: vec![workspace],
            on_screen: None,
            consents: Vec::<RepositoryConsent>::new(),
            own_cache: PathBuf::new(),
            life: lifecycle::Life::default(),
            fetch: fetch::Fetches::default(),
            answers: Answers::default(),
            old: crate::old_side::OldSides::default(),
        };
        (core, inbox, heard)
    }

    /// The outcome of the text search an ask started.
    fn searched(inbox: &Receiver<Event>) -> (AskId, Result<Vec<Found>, String>) {
        match inbox
            .recv_timeout(Duration::from_secs(10))
            .expect("the search ends")
        {
            Event::Found { id, found } => (id, found),
            _ => panic!("the search tells its outcome as an event of its own"),
        }
    }

    #[test]
    fn a_cancelled_ask_is_never_answered_and_one_whose_search_failed_is_answered_empty_and_marked()
    {
        let (mut core, inbox, heard) = core();
        let at = place("notes.txt", 1, &[]);
        ask(&mut core, AskId(1), Question::Hover, at.clone());
        ask(&mut core, AskId(2), Question::Hover, at);

        // The first is cancelled while its search runs; its outcome comes after.
        cancel(&mut core, AskId(1));
        let mut outcomes = [searched(&inbox), searched(&inbox)];
        outcomes.sort_by_key(|(id, _)| *id);
        for (id, outcome) in outcomes {
            found(&mut core, id, outcome);
        }
        // The folder is not there: the second is answered with nothing, marked.
        match heard.try_recv().expect("the second ask is answered") {
            Told::Hover { id, answer } => {
                assert_eq!(id, AskId(2));
                assert_eq!(
                    answer,
                    Answered {
                        value: None,
                        guessed: true
                    }
                );
            }
            other => panic!("told {other:?}"),
        }
        assert!(heard.try_recv().is_err(), "the cancelled ask was answered");
        // An outcome for an ask that was answered is dropped.
        found(&mut core, AskId(2), Ok(Vec::new()));
        assert!(heard.try_recv().is_err());
    }

    #[test]
    fn an_ask_about_a_workspace_with_no_worktree_is_answered_at_once_with_nothing_marked() {
        let (mut core, _inbox, heard) = core();
        core.workspaces[0].folder = None;
        ask(
            &mut core,
            AskId(3),
            Question::References,
            place("a.sil", 1, &[]),
        );
        match heard.try_recv().expect("answered at once") {
            Told::References { id, answer } => {
                assert_eq!(id, AskId(3));
                assert_eq!(
                    answer,
                    Answered {
                        value: Vec::new(),
                        guessed: true
                    }
                );
            }
            other => panic!("told {other:?}"),
        }
    }

    #[test]
    fn an_old_side_asked_about_while_its_export_was_written_is_ensured_when_it_is_ready() {
        let (mut core, _inbox, _heard) = core();
        let workspace = WorkspaceId::new("1".repeat(32));
        let key = |version: &str| ServerKey {
            workspace: workspace.clone(),
            language: "standin".to_owned(),
            side: Side::Old {
                version: version.to_owned(),
            },
        };
        for version in ["v1", "v2", "v3"] {
            core.answers
                .old
                .insert((workspace.clone(), version.to_owned(), "standin".to_owned()));
        }

        // Nothing runs for an old side before its export is ready.
        assert_eq!(lifecycle::state_of(&core, &key("v1")), None);
        export_ready(&mut core, &workspace, "v1", Path::new("/export/1"));
        assert!(lifecycle::state_of(&core, &key("v1")).is_some());
        assert_eq!(lifecycle::state_of(&core, &key("v2")), None);

        // An export that failed is forgotten: its row is ensured by no later export.
        export_failed(&mut core, &workspace, "v2");
        export_ready(&mut core, &workspace, "v2", Path::new("/export/2"));
        assert_eq!(lifecycle::state_of(&core, &key("v2")), None);
        // The one not heard of yet still waits.
        export_ready(&mut core, &workspace, "v3", Path::new("/export/3"));
        assert!(lifecycle::state_of(&core, &key("v3")).is_some());
    }
}
