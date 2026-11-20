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
//! answered ([`server_gone`]), has the ask answered by text match, marked.
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
//! each once, to every new-side server of the workspace that is ready
//! (`Server::files_changed`): a path told twice, and one that no longer exists, are the
//! driver's to tell as changed or deleted. A server that is not ready reads the files when
//! it loads, and is sent a file's text at each ask.
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

use crate::lifecycle::{ensure, ready_server};
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
    /// The answer of a server to one of its requests.
    Server { key: ServerKey, request: u64 },
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
                Some(request) => {
                    let how = How::Server { key, request };
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
/// the text of `file` as it is now first, then the request. Gives its number. None when no
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
) -> Option<u64> {
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
    server.request(question.method(), params).ok()
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
/// ignored by git, or anything else. `yaar_text_search::is_ignored` is called once, for
/// the first workspace folder that holds the file, and reads git's configuration each time
/// it is called.
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
    if let How::Server { key, request } = pending.how
        && let Some(server) = ready_server(core, &key)
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
        if let Some(server) = ready_server(core, &key) {
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
    let waiting = core.answers.waiting.iter().find_map(|(id, pending)| {
        matches!(&pending.how, How::Server { key: k, request: r } if k == key && *r == request)
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
