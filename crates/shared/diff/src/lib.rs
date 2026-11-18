//! Diff module (`W-DIFF`).
//!
//! Owns: the line diff and word-level emphasis, used by the Review pane and the Review engine.
//!
//! Surface:
//! - [`FileDiff`], the difference between two versions of one file: its [`Hunk`]s, the
//!   lines of both versions, and its [`Row`]s in either [`Layout`];
//! - [`Emphasis`], the changed words inside the changed lines of one hunk.
//!
//! It takes two versions of a file as bytes and knows nothing of git, of files on disk, of
//! threads or of drawing. The line diff is git's histogram diff with the step that moves a
//! hunk to where a reader expects it, so the hunks are the ones `git diff --histogram`
//! prints.
//!
//! Lines are counted from 0 here. Whoever shows a line number adds one.

use std::ops::Range;

/// The one algorithm of the line diff.
const ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Histogram;

/// The unchanged lines shown around each change unless the caller asks otherwise; git's
/// own default.
pub const CONTEXT_LINES: u32 = 3;

/// One of the two versions of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The version before the change.
    Old,
    /// The version after it.
    New,
}

/// One change: the lines it removes from the old version and the lines it adds to the new
/// one. Either range may be empty, never both. An empty range still says where the change
/// sits on that side: before the line at its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The removed lines of the old version.
    pub old: Range<u32>,
    /// The added lines of the new version.
    pub new: Range<u32>,
}

/// How the rows of a diff are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layout {
    /// One column: removed lines above the added lines that replace them.
    Unified,
    /// Two columns: the old version on the left, the new one on the right.
    Split,
}

/// One row of a diff as it is shown. Both layouts are made of the same hunks and the same
/// unchanged lines around them; they differ only in how the changed lines are put in rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Lines that are not shown, between two changes or before the first and after the
    /// last. The same row in both layouts.
    Skipped {
        /// The lines of the old version that are left out.
        old: Range<u32>,
        /// The same lines in the new version.
        new: Range<u32>,
    },
    /// An unchanged line, with its number on each side. The same row in both layouts.
    Context {
        /// Its number in the old version.
        old: u32,
        /// Its number in the new version.
        new: u32,
    },
    /// Unified only: a removed line of the old version.
    Removed {
        /// Its number in the old version.
        old: u32,
    },
    /// Unified only: an added line of the new version.
    Added {
        /// Its number in the new version.
        new: u32,
    },
    /// Split only: a removed line on the left beside an added line on the right. The
    /// lines of a hunk are paired in order; where one side of the hunk is longer, the
    /// other side of the row is empty.
    Changed {
        /// The removed line, by its number in the old version.
        old: Option<u32>,
        /// The added line, by its number in the new version.
        new: Option<u32>,
    },
}

impl Row {
    /// The line this row shows on `side`, if it shows one there.
    pub fn line(&self, side: Side) -> Option<u32> {
        todo!()
    }
}

/// The difference between two versions of one file.
#[derive(Debug, Clone)]
pub struct FileDiff {
    old: Text,
    new: Text,
    hunks: Vec<Hunk>,
}

/// One version of a file, cut into lines.
#[derive(Debug, Clone)]
struct Text {
    bytes: Vec<u8>,
    /// Where each line starts in `bytes`.
    starts: Vec<u32>,
}

impl FileDiff {
    /// Diffs two versions of a file line by line. A version that does not exist, as the
    /// old side of a new file, is given as no bytes.
    ///
    /// A line ends at a line feed; a last line with none is a line too. The bytes need
    /// not be UTF-8.
    pub fn new(old: &[u8], new: &[u8]) -> Self {
        todo!()
    }

    /// The changes, in the order of the file. Two hunks never touch: at least one
    /// unchanged line lies between them.
    pub fn hunks(&self) -> &[Hunk] {
        todo!()
    }

    /// How many lines the version on `side` has.
    pub fn line_count(&self, side: Side) -> u32 {
        todo!()
    }

    /// The bytes of one line of the version on `side`, without the line feed that ends
    /// it. Panics when the version has no such line.
    pub fn line(&self, side: Side, line: u32) -> &[u8] {
        todo!()
    }

    /// The rows of the diff in `layout`, with up to `context` unchanged lines shown
    /// before and after each change. Unchanged lines farther from any change are left
    /// out, each run of them as one [`Row::Skipped`]; `u32::MAX` shows the whole file.
    ///
    /// For one `context`, the two layouts hold the same [`Row::Skipped`] and
    /// [`Row::Context`] rows in the same order, and between them the same lines of the
    /// same hunks.
    pub fn rows(&self, layout: Layout, context: u32) -> Vec<Row> {
        todo!()
    }

    /// The changed words of the hunk at `index` of [`FileDiff::hunks`].
    ///
    /// Computed when asked, hunk by hunk, so that a caller pays only for the hunks it
    /// shows.
    pub fn emphasis(&self, index: usize) -> Emphasis {
        todo!()
    }
}

/// The words that changed inside the changed lines of one hunk: what a reader's eye should
/// land on when a line was edited and not replaced.
///
/// The removed lines of the hunk are compared with its added lines word by word, where a
/// word is a run of letters, digits and underscores, a run of white space, or one other
/// character. A word the two sides share is not marked. A hunk that only removes or only
/// adds has nothing marked: the whole line is the change, and its colour already says so.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Emphasis {
    old: Vec<Vec<Range<usize>>>,
    new: Vec<Vec<Range<usize>>>,
}

impl Emphasis {
    /// The ranges of bytes to mark in one line of the hunk, in order and not touching.
    /// They count from the start of the line, as [`FileDiff::line`] returns it. Empty for
    /// a line outside the hunk.
    pub fn of(&self, side: Side, line: u32) -> &[Range<usize>] {
        todo!()
    }
}

/// The recorded changes of real files that the line diff is held to.
///
/// The data lies in the folder `corpus` of this crate and is compiled into the test
/// program, so a test opens no file:
/// - `pairs.tar.zst`, the two versions of every change, `pNNNN/a.<ext>` and
///   `pNNNN/b.<ext>`, as one archive;
/// - `hunks.tsv`, the hunks `git diff --diff-algorithm=histogram` gave for each change,
///   one line for each, under a first line that names the git that was run;
/// - `pins.tsv`, where each change comes from: the project, the two commits, the paths;
/// - `NOTICE.md` and the licence texts of the projects the changes are copied from;
/// - `record.sh`, which writes `hunks.tsv` again from the archive.
#[cfg(test)]
mod corpus {
    use ruzstd::decoding::StreamingDecoder;

    use crate::Hunk;

    /// One recorded change of one file.
    pub(crate) struct Pair {
        /// Its name in the corpus: `p0000`.
        pub(crate) id: String,
        /// The file before the change.
        pub(crate) old: Vec<u8>,
        /// The file after it.
        pub(crate) new: Vec<u8>,
        /// The hunks git gave.
        pub(crate) git_hunks: Vec<Hunk>,
    }

    /// Every change of the corpus, in the order of their names.
    pub(crate) fn pairs() -> Vec<Pair> {
        todo!()
    }
}
