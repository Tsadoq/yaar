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
    starts: Vec<usize>,
}

impl Text {
    fn new(bytes: &[u8]) -> Self {
        let mut starts = Vec::new();
        let mut start = 0;
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            starts.push(start);
            start += line.len();
        }
        Self {
            bytes: bytes.to_vec(),
            starts,
        }
    }

    /// The line with the line feed that ends it, if it has one.
    fn token(&self, line: u32) -> &[u8] {
        let line = line as usize;
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.bytes.len());
        &self.bytes[self.starts[line]..end]
    }
}

impl FileDiff {
    /// Diffs two versions of a file line by line. A version that does not exist, as the
    /// old side of a new file, is given as no bytes.
    ///
    /// A line ends at a line feed; a last line with none is a line too. The bytes need
    /// not be UTF-8.
    pub fn new(old: &[u8], new: &[u8]) -> Self {
        // A line is compared with its line feed, as git compares it: a last line that
        // gained or lost its line feed is a changed line.
        let input = imara_diff::InternedInput::new(old, new);
        let mut diff = imara_diff::Diff::compute(ALGORITHM, &input);
        // Without this step the changes are right but sit where the algorithm happened to
        // find them, not where git puts them.
        diff.postprocess_lines(&input);
        let hunks = diff
            .hunks()
            .map(|hunk| Hunk {
                old: hunk.before,
                new: hunk.after,
            })
            .collect();
        Self {
            old: Text::new(old),
            new: Text::new(new),
            hunks,
        }
    }

    fn text(&self, side: Side) -> &Text {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }

    /// The changes, in the order of the file. Two hunks never touch: at least one
    /// unchanged line lies between them.
    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    /// How many lines the version on `side` has.
    pub fn line_count(&self, side: Side) -> u32 {
        // The diff refuses a version of more lines than a `u32` counts.
        self.text(side).starts.len() as u32
    }

    /// The bytes of one line of the version on `side`, without the line feed that ends
    /// it. Panics when the version has no such line.
    pub fn line(&self, side: Side, line: u32) -> &[u8] {
        let token = self.text(side).token(line);
        token.strip_suffix(b"\n").unwrap_or(token)
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// A version cut into lines the way git counts them: a line is its bytes with the line
    /// feed that ends it. The tests cut their own lines, so that the hunks are held to the
    /// bytes they were given and not to the crate's own account of them.
    fn lines(bytes: &[u8]) -> Vec<&[u8]> {
        bytes.split_inclusive(|byte| *byte == b'\n').collect()
    }

    /// The old version with every hunk applied: its removed lines left out, and the lines
    /// the hunk names in the new version put in their place.
    fn applied(old: &[u8], new: &[u8], hunks: &[Hunk]) -> Vec<u8> {
        let (old, new) = (lines(old), lines(new));
        let mut out: Vec<&[u8]> = Vec::new();
        let mut kept = 0;
        for hunk in hunks {
            assert!(
                !(hunk.old.is_empty() && hunk.new.is_empty()),
                "{hunk:?} changes nothing"
            );
            out.extend(&old[kept..hunk.old.start as usize]);
            assert_eq!(
                out.len(),
                hunk.new.start as usize,
                "{hunk:?} does not sit where its new side says"
            );
            out.extend(&new[hunk.new.start as usize..hunk.new.end as usize]);
            kept = hunk.old.end as usize;
        }
        out.extend(&old[kept..]);
        out.concat()
    }

    /// Source-like text of 21,220 lines: constants and functions at the left edge, `if`
    /// blocks nested up to six deep inside the functions, one line in ten blank. Made from
    /// a fixed seed, so every run diffs the same bytes.
    fn nested_source() -> Vec<u8> {
        let mut state: u64 = 0x5eed_0000_0000_d1ff;
        let mut next = move |below: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % below
        };
        let mut text = String::new();
        let mut depth = 0;
        for number in 0..21_220 {
            let roll = next(100);
            let indent = "    ".repeat(depth);
            let line = if roll < 10 {
                String::new()
            } else if depth == 0 && roll < 30 {
                format!("const LIMIT_{number}: u64 = {};", next(10_000))
            } else if depth == 0 {
                depth = 1;
                format!("fn step_{number}(input: u64) -> u64 {{")
            } else if roll < 28 && depth < 7 {
                depth += 1;
                format!("{indent}if input > {} {{", next(500))
            } else if roll < 48 {
                depth -= 1;
                format!("{}}}", "    ".repeat(depth))
            } else {
                format!("{indent}let value_{} = input + {};", next(40), next(500))
            };
            text.push_str(&line);
            text.push('\n');
        }
        text.into_bytes()
    }

    /// The same text with the leading spaces of every line halved, as a formatter set to
    /// another indent width leaves it.
    fn reindented(source: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(source.len());
        for line in lines(source) {
            let spaces = line.iter().take_while(|byte| **byte == b' ').count();
            out.extend_from_slice(&line[spaces / 2..]);
        }
        out
    }

    // DIFF-01
    #[test]
    fn a_file_of_21220_reindented_lines_diffs_in_under_50_ms() {
        let old = nested_source();
        let new = reindented(&old);
        let indented = lines(&old).iter().filter(|line| line[0] == b' ').count();
        assert_eq!(lines(&old).len(), 21_220);
        assert_eq!(lines(&new).len(), 21_220);
        assert!(
            indented > 14_000,
            "only {indented} lines are indented: too few to stand for a re-indented file"
        );

        // The best of several runs: a busy machine can slow any one of them, and the limit
        // is on what the diff costs, not on what else the machine was doing.
        let mut best = Duration::MAX;
        for _ in 0..7 {
            let start = Instant::now();
            let diff = FileDiff::new(&old, &new);
            best = best.min(start.elapsed());

            // A diff that is fast and wrong would pass the limit: it has to be the change.
            assert_eq!(applied(&old, &new, diff.hunks()), new);
            let removed: usize = diff.hunks().iter().map(|hunk| hunk.old.len()).sum();
            assert!(
                removed >= indented,
                "{removed} lines removed of {indented} changed"
            );
        }
        assert!(
            best < Duration::from_millis(50),
            "the best of 7 runs took {best:?}"
        );
    }
}
