//! Whole-word text search of a folder by git's ignore rules only (not `.ignore`, `.rgignore`), used
//! while no language server is ready. Lines count from 1, columns in bytes from 0.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{WalkBuilder, WalkState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Absolute, also when the searched folder was given relative.
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    /// The line without its line end, lossily UTF-8.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    NoWord,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSearch {
    threads: usize,
}

impl TextSearch {
    pub fn new(threads: usize) -> TextSearch {
        TextSearch {
            threads: threads.max(1),
        }
    }

    pub fn half_the_cores() -> usize {
        half_of(std::thread::available_parallelism().map_or(1, |n| n.get()))
    }

    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Matches arrive in no fixed order; the channel closes when every file was searched, and
    /// dropping the receiver ends the search early.
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
                        WalkState::Quit
                    }
                })
            });
        });
        Ok(receiver)
    }

    pub fn all(&self, folder: &Path, word: &str) -> Result<Vec<Found>, SearchError> {
        let mut list: Vec<Found> = self.search(folder, word)?.iter().collect();
        list.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
        Ok(list)
    }
}

fn half_of(cores: usize) -> usize {
    (cores / 2).max(1)
}

/// A path outside `folder` is not ignored.
pub fn is_ignored(folder: &Path, path: &Path) -> bool {
    let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let folder = folder.as_path();
    let Ok(relative) = path.strip_prefix(folder) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    if components.iter().any(|part| part.as_os_str() == GIT_FOLDER) {
        return true;
    }

    // The order the walk weighs rules: every `.gitignore`, deepest first, then the exclude files,
    // deepest first, then the global file.
    let (global, _) = Gitignore::global();
    let mut gitignores: Vec<Gitignore> = Vec::new();
    let mut excludes: Vec<Gitignore> = Vec::new();

    let mut folder_now = folder.to_path_buf();
    for (depth, part) in components.iter().enumerate() {
        let mut builder = GitignoreBuilder::new(&folder_now);
        let _ = builder.add(folder_now.join(".gitignore"));
        gitignores.push(builder.build().unwrap_or_else(|_| Gitignore::empty()));
        if let Some(file) = exclude_file(&folder_now) {
            let mut builder = GitignoreBuilder::new(&folder_now);
            let _ = builder.add(file);
            excludes.push(builder.build().unwrap_or_else(|_| Gitignore::empty()));
        }

        let here = folder_now.join(part);
        // The walk follows no link and git counts none as a folder.
        let is_folder = depth + 1 < components.len()
            || std::fs::symlink_metadata(&here).is_ok_and(|kind| kind.is_dir());
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

/// The exclude file of the repository whose `.git` is in `dir`: `.git/info/exclude`, or for a
/// `.git` file (linked worktree) the one of the repository its `gitdir:` folder's `commondir`
/// names. None without a `.git`, or when the file cannot be followed.
fn exclude_file(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(GIT_FOLDER);
    let kind = dot_git.metadata().ok()?;
    if !kind.is_file() {
        return Some(dot_git.join("info").join("exclude"));
    }
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

const GIT_FOLDER: &str = ".git";

// Follows no link (the builder's default): files below a link to a folder are never searched.
// `is_ignored` builds the same rules by hand, so a change here must be mirrored there.
fn walker(folder: &Path, threads: usize) -> ignore::WalkParallel {
    WalkBuilder::new(folder)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .ignore(false)
        .hidden(false)
        // The rules of the folders above are not this folder's: it may be a worktree inside
        // another project, or an export in a cache.
        .parents(false)
        // An export of an old side is no repository, yet its `.gitignore` files count.
        .require_git(false)
        .threads(threads)
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != GIT_FOLDER)
        .build_parallel()
}

/// `<` and `>` stay unescaped: `\<` and `\>` are word boundaries.
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

/// False when the receiver is gone. Matches before a NUL byte are held back, so a binary file gives
/// nothing.
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
        for (cores, threads) in [(1, 1), (2, 1), (3, 1), (12, 6)] {
            assert_eq!(half_of(cores), threads, "half of {cores} cores");
        }
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert_eq!(
            TextSearch::half_the_cores(),
            half_of(cores),
            "the machine's own count goes through the same rule"
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
            for other in ["xy", "xxy", "xzy", "x", "y"] {
                assert!(
                    !matcher.is_match(other.as_bytes()).expect("a match"),
                    "{word:?} matches {other:?}"
                );
            }
        }
    }
}
