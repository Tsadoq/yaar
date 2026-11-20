//! Text search (`W-TEXT`).
//!
//! Owns: whole-word search of the worktree while no language server is ready.
//!
//! A name is found by searching the files of a folder for it as a whole word, with no
//! knowledge of any language (FR-068, ADR-017). The Navigator answers by it while no
//! language server is ready for a file, and marks every such answer as possibly wrong;
//! nothing here marks anything.
//!
//! Surface:
//! - [`TextSearch`], made once with the number of threads it searches on, at most half the
//!   machine's cores ([`TextSearch::half_the_cores`]);
//! - [`TextSearch::search`], one word and one folder in, every match out, as [`Found`]s
//!   sent on a channel as they are found, and [`TextSearch::all`], which waits for them;
//! - [`is_ignored`], whether git's ignore rules leave a file of a folder out.
//!
//! What is searched: every file under the folder, at any depth, by git's ignore rules
//! (`.gitignore` at every level, `.git/info/exclude`, and the user's global excludes file as
//! git finds it, `core.excludesFile` of the user's git configuration or else
//! `$XDG_CONFIG_HOME/git/ignore`), tracked or not:
//! a file the agent just created and git does not track yet is searched (TEXT-01); a file
//! git ignores is not. Hidden files and folders are searched unless an ignore rule leaves
//! them out, `.git` never. A file that holds a NUL byte is binary and not searched. The
//! `ignore` crate's own `.ignore` and `.rgignore` files are not read: only git's rules
//! count, also in a folder that is no repository, as an export of an old side, whose
//! `.gitignore` files are read all the same. No index is kept: each search reads the files
//! again (DEC-093).
//!
//! A word matches where it stands whole: the bytes before and after it are not letters,
//! digits or `_` (`grep_regex`'s word boundaries, on the word with every character of a
//! regular expression escaped). Lines are counted from 1, columns in bytes from 0.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

/// One match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The file, by its absolute path: the folder searched joined with the file's path in
    /// it.
    pub path: PathBuf,
    /// The line, counted from 1.
    pub line: u32,
    /// Where the word starts in the line: the offset of its first byte, from 0.
    pub column: u32,
    /// The line's text, with no line end, lossily made UTF-8.
    pub text: String,
}

/// Why a search found nothing it could give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// The word is empty, or holds a line end.
    NoWord,
    /// The folder cannot be read: the reason.
    Folder(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWord => f.write_str("there is no word to search for"),
            Self::Folder(reason) => write!(f, "the folder cannot be searched: {reason}"),
        }
    }
}

impl std::error::Error for SearchError {}

/// The search, with the number of threads it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSearch {
    threads: usize,
}

impl TextSearch {
    /// A search on `threads` threads, at least one.
    pub fn new(threads: usize) -> TextSearch {
        TextSearch {
            threads: threads.max(1),
        }
    }

    /// The number of threads of ADR-017: half the cores the machine says it has, rounded
    /// down, and at least one.
    pub fn half_the_cores() -> usize {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        (cores / 2).max(1)
    }

    /// The number of threads it searches on.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Searches `folder` for `word`, on threads of its own: each match is sent on the
    /// channel as it is found, in no fixed order, and the channel is closed when every
    /// file was searched. Dropping the receiver ends the search early. An error, before
    /// anything is searched, for an empty word or one that holds a line end, and for a
    /// folder that is not one.
    pub fn search(&self, folder: &Path, word: &str) -> Result<Receiver<Found>, SearchError> {
        let _ = (folder, word);
        todo!()
    }

    /// Every match of `word` in `folder`, ordered by path, then line, then column: the
    /// whole list, once every file was searched.
    pub fn all(&self, folder: &Path, word: &str) -> Result<Vec<Found>, SearchError> {
        let _ = (folder, word);
        todo!()
    }
}

/// Whether git's ignore rules, as [`TextSearch::search`] reads them for `folder`, leave out
/// `path`, a file inside `folder`, or any folder above it inside `folder`. A path outside
/// `folder` is not ignored by them.
pub fn is_ignored(folder: &Path, path: &Path) -> bool {
    let _ = (folder, path);
    todo!()
}
