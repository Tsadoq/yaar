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

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{WalkBuilder, WalkState};

/// One match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The file, by its absolute path: the folder searched, made absolute if it was given
    /// relative, joined with the file's path in it.
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
        if word.is_empty() || word.contains(['\n', '\r']) {
            return Err(SearchError::NoWord);
        }
        let matcher = RegexMatcherBuilder::new()
            .word(true)
            .build(&escape(word))
            .map_err(|error| SearchError::Folder(format!("the word is no pattern: {error}")))?;
        let metadata =
            std::fs::metadata(folder).map_err(|error| SearchError::Folder(error.to_string()))?;
        if !metadata.is_dir() {
            return Err(SearchError::Folder("it is not a folder".to_owned()));
        }

        // The folder as an absolute path, so that every match says where its file is.
        let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
        let walker = walker(&folder, self.threads);
        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            walker.run(|| {
                let matcher = matcher.clone();
                let sender = sender.clone();
                let mut searcher = SearcherBuilder::new()
                    .binary_detection(BinaryDetection::quit(0))
                    .line_number(true)
                    .build();
                Box::new(move |entry| {
                    let Ok(entry) = entry else {
                        return WalkState::Continue;
                    };
                    if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                        return WalkState::Continue;
                    }
                    if search_file(&mut searcher, &matcher, entry.path(), &sender) {
                        WalkState::Continue
                    } else {
                        // The receiver is gone: nobody wants the rest.
                        WalkState::Quit
                    }
                })
            });
        });
        Ok(receiver)
    }

    /// Every match of `word` in `folder`, ordered by path, then line, then column: the
    /// whole list, once every file was searched.
    pub fn all(&self, folder: &Path, word: &str) -> Result<Vec<Found>, SearchError> {
        let mut list: Vec<Found> = self.search(folder, word)?.iter().collect();
        list.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
        Ok(list)
    }
}

/// Whether git's ignore rules, as [`TextSearch::search`] reads them for `folder`, leave out
/// `path`, a file inside `folder`, or any folder above it inside `folder`. A path outside
/// `folder` is not ignored by them.
pub fn is_ignored(folder: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(folder) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    if components.iter().any(|part| part.as_os_str() == GIT_FOLDER) {
        return true;
    }

    // The rules in the order the walk weighs them: every `.gitignore`, the deepest folder's
    // first, then every repository's own exclude file, the deepest first, then the user's
    // global file.
    let (global, _) = Gitignore::global();
    let mut gitignores: Vec<Gitignore> = Vec::new();
    let mut excludes: Vec<Gitignore> = Vec::new();

    // A file is out when it is, or when a folder above it is: the first folder that is
    // decides, as the walk never goes below it.
    let mut folder_now = folder.to_path_buf();
    for (depth, part) in components.iter().enumerate() {
        let mut builder = GitignoreBuilder::new(&folder_now);
        let _ = builder.add(folder_now.join(".gitignore"));
        gitignores.push(builder.build().unwrap_or_else(|_| Gitignore::empty()));
        // The folder searched and any folder below it may be a repository of its own, and
        // its exclude file counts for what lies below it.
        if let Some(file) = exclude_file(&folder_now) {
            let mut builder = GitignoreBuilder::new(&folder_now);
            // No such file is the usual case; it holds no rules then.
            let _ = builder.add(file);
            excludes.push(builder.build().unwrap_or_else(|_| Gitignore::empty()));
        }

        let here = folder_now.join(part);
        let is_folder = depth + 1 < components.len() || here.is_dir();
        let verdict = gitignores
            .iter()
            .rev()
            .map(|rules| rules.matched(&here, is_folder))
            .chain(
                excludes
                    .iter()
                    .rev()
                    .map(|rules| rules.matched(&here, is_folder)),
            )
            .chain([global.matched(&here, is_folder)])
            .find(|verdict| !verdict.is_none());
        if verdict.is_some_and(|verdict| verdict.is_ignore()) {
            return true;
        }
        folder_now = here;
    }
    false
}

/// The exclude file of the repository whose `.git` is in `dir`, found as the walk finds it.
/// A `.git` folder holds it; a `.git` that is a file, as a linked worktree's is, names the
/// folder git keeps the worktree in (`gitdir:`), and the exclude file is the one of the
/// repository that folder belongs to (its `commondir`). No such `.git` gives none, and a
/// `.git` file the walk cannot follow gives none either: the walk then applies no exclude
/// rules there.
fn exclude_file(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(GIT_FOLDER);
    let kind = dot_git.metadata().ok()?;
    if !kind.is_file() {
        return Some(dot_git.join("info").join("exclude"));
    }
    // The walk reads the path as written, which is absolute where git wrote it.
    let first_line = |file: &Path| -> Option<String> {
        let text = std::fs::read_to_string(file).ok()?;
        text.lines().next().map(str::to_owned)
    };
    let line = first_line(&dot_git)?;
    let git_dir = PathBuf::from(line.strip_prefix("gitdir: ")?);
    let common = first_line(&git_dir.join("commondir"))?;
    let common = if common.starts_with('.') {
        git_dir.join(common)
    } else {
        PathBuf::from(common)
    };
    Some(common.join("info").join("exclude"))
}

/// The name of git's own folder, which is never searched.
const GIT_FOLDER: &str = ".git";

/// The walk of `folder` by git's rules and on `threads` threads, as `search` and
/// `is_ignored` both read them.
fn walker(folder: &Path, threads: usize) -> ignore::WalkParallel {
    WalkBuilder::new(folder)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        // The other tools' files, `.ignore` and `.rgignore`, are not git's rules.
        .ignore(false)
        .hidden(false)
        // The rules of the folders above are not this folder's: it may be a worktree inside
        // another project, or an export in a cache.
        .parents(false)
        // An export of an old side is no repository and its `.gitignore` files count.
        .require_git(false)
        .threads(threads)
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != GIT_FOLDER)
        .build_parallel()
}

/// The word as a regular expression that matches exactly it: every ASCII punctuation
/// character is escaped, but `<` and `>`, which the pattern syntax does not let one
/// escape (`\<` and `\>` are word boundaries) and which mean themselves unescaped.
fn escape(word: &str) -> String {
    let mut pattern = String::with_capacity(word.len() * 2);
    for c in word.chars() {
        if c.is_ascii_punctuation() && c != '<' && c != '>' {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

/// Searches one file and sends each match on `sender`. False when the receiver is gone.
/// A file that cannot be read, or that holds a NUL byte, gives nothing: the matches of a
/// binary file found before the byte are held back until the whole file is known to be
/// text.
fn search_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    path: &Path,
    sender: &Sender<Found>,
) -> bool {
    let mut collect = Collect {
        matcher,
        path,
        found: Vec::new(),
        binary: false,
    };
    if searcher.search_path(matcher, path, &mut collect).is_err() || collect.binary {
        return true;
    }
    collect
        .found
        .into_iter()
        .all(|found| sender.send(found).is_ok())
}

/// The matches of one file, as the searcher finds the lines that hold them.
struct Collect<'a> {
    matcher: &'a RegexMatcher,
    path: &'a Path,
    found: Vec<Found>,
    binary: bool,
}

impl Sink for Collect<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let mut line = found.bytes();
        if let Some(rest) = line.strip_suffix(b"\n") {
            line = rest.strip_suffix(b"\r").unwrap_or(rest);
        }
        let number = u32::try_from(found.line_number().unwrap_or(0)).unwrap_or(u32::MAX);
        let text = String::from_utf8_lossy(line).into_owned();
        let (path, found) = (self.path, &mut self.found);
        // Never fails: the matcher's error type is that of no error.
        let _ = self.matcher.find_iter(line, |at| {
            found.push(Found {
                path: path.to_path_buf(),
                line: number,
                column: u32::try_from(at.start()).unwrap_or(u32::MAX),
                text: text.clone(),
            });
            true
        });
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> Result<bool, io::Error> {
        self.binary = true;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_threads_are_half_the_cores_and_at_least_one() {
        // At most half the cores, rounded down, and never below one: the number is that of
        // the machine the test runs on, so the rule is stated, not a figure.
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert_eq!(
            TextSearch::half_the_cores(),
            (cores / 2).max(1),
            "half of {cores} cores, rounded down, at least one"
        );
        assert_eq!(TextSearch::new(0).threads(), 1);
        assert_eq!(TextSearch::new(5).threads(), 5);
    }

    #[test]
    fn each_punctuation_character_of_a_word_matches_only_itself() {
        for c in (0u8..128)
            .map(char::from)
            .filter(char::is_ascii_punctuation)
        {
            let word = format!("x{c}y");
            let matcher = RegexMatcherBuilder::new()
                .build(&escape(&word))
                .unwrap_or_else(|error| panic!("{word:?} is no pattern: {error}"));
            assert!(
                matcher.is_match(word.as_bytes()).expect("a match"),
                "{word:?} matches itself"
            );
            // The wildcard, the alternation and the repetitions would match these.
            for other in ["xy", "xxy", "xzy", "x", "y"] {
                assert!(
                    !matcher.is_match(other.as_bytes()).expect("a match"),
                    "{word:?} matches {other:?}"
                );
            }
        }
    }
}
