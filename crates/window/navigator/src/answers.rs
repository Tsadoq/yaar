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
//! a cancelled ask is never answered. [`files_changed`] hands the paths, made absolute, to
//! every running new-side server of the workspace (`Server::files_changed`).
//!
//! # Opening a file
//!
//! [`open`] reads the location's file whole as UTF-8 and tells [`crate::Told::Opened`]
//! with its kind: [`crate::FileKind::Version`] when it lies in an export
//! (`old_side::version_of`); [`crate::FileKind::Worktree`] when it lies in the folder of a
//! workspace, its links resolved, and `yaar_text_search::is_ignored` says git does not
//! ignore it; else [`crate::FileKind::Outside`]. A file that cannot be read, or is not
//! UTF-8, is told as the reason.

use std::path::{Path, PathBuf};

use yaar_link::WorkspaceId;
use yaar_text_search::Found;

use crate::navigator::{Core, ServerKey};
use crate::port::{AskId, Location, Place};

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

/// What this module keeps in [`Core::answers`]: the asks that wait, for a server's answer,
/// a search's or an export.
#[derive(Default)]
pub(crate) struct Answers {}

/// [`crate::Ask::Hover`], [`crate::Ask::Definition`] and [`crate::Ask::References`].
pub(crate) fn ask(core: &mut Core, id: AskId, question: Question, place: Place) {
    let _ = (core, id, question, place);
    todo!()
}

/// [`crate::Ask::Open`].
pub(crate) fn open(core: &mut Core, id: AskId, location: Location) {
    let _ = (core, id, location);
    todo!()
}

/// [`crate::Ask::Cancel`].
pub(crate) fn cancel(core: &mut Core, id: AskId) {
    let _ = (core, id);
    todo!()
}

/// [`crate::Ask::FilesChanged`].
pub(crate) fn files_changed(core: &mut Core, workspace: &WorkspaceId, paths: &[PathBuf]) {
    let _ = (core, workspace, paths);
    todo!()
}

/// A text search ended: its ask is answered, marked, unless it was cancelled.
pub(crate) fn found(core: &mut Core, id: AskId, found: Result<Vec<Found>, String>) {
    let _ = (core, id, found);
    todo!()
}

/// A server of `key` answered its request `request`.
pub(crate) fn server_answered(
    core: &mut Core,
    key: &ServerKey,
    request: u64,
    result: Result<serde_json::Value, String>,
) {
    let _ = (core, key, request, result);
    todo!()
}

/// The server of `key` ended: every ask still waiting for it is answered by text match.
pub(crate) fn server_gone(core: &mut Core, key: &ServerKey) {
    let _ = (core, key);
    todo!()
}

/// The export of `version` of `workspace` is written, at `root`: every row asked about on
/// that old side while it was written has its old side's key ensured
/// (`lifecycle::ensure`, the export as its root). No ask waits for an export: each was
/// answered by text match when it came.
pub(crate) fn export_ready(core: &mut Core, workspace: &WorkspaceId, version: &str, root: &Path) {
    let _ = (core, workspace, version, root);
    todo!()
}

/// The export of `version` of `workspace` could not be written: nothing waits for it, and
/// the rows asked about are forgotten; the next ask about that old side asks for the
/// export again.
pub(crate) fn export_failed(core: &mut Core, workspace: &WorkspaceId, version: &str) {
    let _ = (core, workspace, version);
    todo!()
}
