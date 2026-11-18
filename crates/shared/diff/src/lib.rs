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
//! threads or of drawing. The line diff is the histogram diff with the step that moves a
//! hunk to where a reader expects it, as git has both: on the recorded changes of real
//! files that the tests hold it to, the hunks are those of `git diff --histogram` in 564
//! of 569, and in the other five the same change is placed a few lines otherwise.
//!
//! Lines are counted from 0 here. Whoever shows a line number adds one.

mod words;

use std::ops::Range;

/// The one algorithm of the line diff.
const ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Histogram;

/// The algorithm that finds, among the changed lines of one hunk, the lines that were
/// edited. The one of the lines, for what it does first: it looks for the words that are
/// rare on both sides, and a line is told from its neighbours by its rare words.
const PAIRING_ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Histogram;

/// The algorithm that compares the words of an edited line with those of the line it
/// became. Not the one of the lines: the words of a line are few and many of them come
/// again and again, which is where the histogram diff gives a longer change than need
/// be, and the shortest change is what a reader expects to see marked.
const WORD_ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Myers;

/// The most words, of both sides together, that a hunk may hold and still have its words
/// compared. Comparing words costs up to the square of their number, and a hunk has no
/// largest size: one of twenty thousand rewritten lines took many seconds. With this many
/// words the hardest hunk that was tried took 13 ms where it was measured, and of the
/// 3,780 hunks of the recorded changes that have lines on both sides, two hold more.
const MOST_WORDS: usize = 4096;

/// The most bytes, of both sides together and the line feeds among them, that a hunk may
/// hold and still have its words compared. The words of a line cannot be counted without
/// reading the line, and a line has no largest length: one of 4 MiB took 20 ms to cut
/// into words each time its marks were asked for, whether its words were then too many or
/// not. With this many bytes the reading took 0.3 ms at most where it was measured. It is
/// 16 bytes for each of the [`MOST_WORDS`]; of the 3,780 hunks of the recorded changes
/// that have lines on both sides none holds more, and the largest that is compared holds
/// 9,016.
const MOST_BYTES: usize = 65_536;

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
        match (self, side) {
            (Row::Skipped { .. }, _) => None,
            (Row::Context { old, .. }, Side::Old) | (Row::Removed { old }, Side::Old) => Some(*old),
            (Row::Context { new, .. }, Side::New) | (Row::Added { new }, Side::New) => Some(*new),
            (Row::Removed { .. }, Side::New) | (Row::Added { .. }, Side::Old) => None,
            (Row::Changed { old, .. }, Side::Old) => *old,
            (Row::Changed { new, .. }, Side::New) => *new,
        }
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

    /// Where the line starts in `bytes`, or where the bytes end for the line after the
    /// last.
    fn start(&self, line: u32) -> usize {
        self.starts
            .get(line as usize)
            .copied()
            .unwrap_or(self.bytes.len())
    }

    /// The line with the line feed that ends it, if it has one.
    fn token(&self, line: u32) -> &[u8] {
        &self.bytes[self.starts[line as usize]..self.start(line + 1)]
    }

    /// How many bytes `lines` hold, their line feeds among them. Looked up, not counted:
    /// it costs the same for lines of any length.
    fn len(&self, lines: &Range<u32>) -> usize {
        self.start(lines.end) - self.start(lines.start)
    }
}

impl FileDiff {
    /// Diffs two versions of a file line by line. A version that does not exist, as the
    /// old side of a new file, is given as no bytes.
    ///
    /// A line ends at a line feed; a last line with none is a line too. The bytes need
    /// not be UTF-8. Panics on a version of more lines than an `i32` counts, which the
    /// diff library does not take.
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
        // `new` took no version of more lines than an `i32` counts.
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
        // Unchanged lines are shown where they follow a change and where they come before
        // one. Before the first change nothing has changed yet, unless all is to be shown.
        let mut after_a_change = if context == u32::MAX { context } else { 0 };
        let mut rows = Vec::new();
        // Where the unchanged lines after the last change start, on each side.
        let (mut old, mut new) = (0, 0);
        for hunk in &self.hunks {
            unchanged(&mut rows, old..hunk.old.start, new, after_a_change, context);
            match layout {
                Layout::Unified => {
                    rows.extend(hunk.old.clone().map(|old| Row::Removed { old }));
                    rows.extend(hunk.new.clone().map(|new| Row::Added { new }));
                }
                Layout::Split => {
                    let (mut removed, mut added) = (hunk.old.clone(), hunk.new.clone());
                    for _ in 0..removed.len().max(added.len()) {
                        rows.push(Row::Changed {
                            old: removed.next(),
                            new: added.next(),
                        });
                    }
                }
            }
            (old, new) = (hunk.old.end, hunk.new.end);
            after_a_change = context;
        }
        let end = self.line_count(Side::Old);
        unchanged(&mut rows, old..end, new, after_a_change, 0);
        rows
    }

    /// The changed words of the hunk at `index` of [`FileDiff::hunks`].
    ///
    /// Computed when asked, hunk by hunk, so that a caller pays only for the hunks it
    /// shows. What one hunk costs is bounded: a hunk of more than 4,096 words or of more
    /// than 65,536 bytes, both sides together, is not compared and has nothing marked. A
    /// hunk of too many bytes is not read at all. Panics when there is no such hunk.
    pub fn emphasis(&self, index: usize) -> Emphasis {
        let hunk = &self.hunks[index];
        let mut emphasis = Emphasis {
            first: (hunk.old.start, hunk.new.start),
            ..Emphasis::default()
        };
        if hunk.old.is_empty() || hunk.new.is_empty() {
            return emphasis;
        }
        // Asked before a byte of the hunk is read: reading is what a hunk of long lines
        // costs, even to learn that its words are too many.
        if self.old.len(&hunk.old) + self.new.len(&hunk.new) > MOST_BYTES {
            return emphasis;
        }

        let removed = hunk.old.clone().map(|line| self.old.token(line));
        let added = hunk.new.clone().map(|line| self.new.token(line));
        let Some(old) = words::of_lines(removed, MOST_WORDS) else {
            return emphasis;
        };
        let Some(new) = words::of_lines(added, MOST_WORDS - old.all.len()) else {
            return emphasis;
        };
        let mut input = imara_diff::InternedInput {
            before: Vec::new(),
            after: Vec::new(),
            interner: imara_diff::Interner::new(old.all.len() + new.all.len()),
        };
        input.update_before(old.all.iter().map(|word| word.bytes));
        input.update_after(new.all.iter().map(|word| word.bytes));
        let distinct = input.interner.num_tokens();

        // First all the removed lines against all the added ones, to learn which line
        // became which. Then each edited line against the line it became, and nothing
        // else: marks taken from the first comparison would run from the end of one
        // line into the next wherever two lines end in the same words.
        let whole = imara_diff::Diff::compute(PAIRING_ALGORITHM, &input);
        let edited = words::edited(
            &old,
            &new,
            |word| whole.is_removed(word),
            |word| whole.is_added(word),
        );
        emphasis.old = vec![Vec::new(); hunk.old.len()];
        emphasis.new = vec![Vec::new(); hunk.new.len()];
        let mut diff = imara_diff::Diff::default();
        for (old_line, new_line) in edited {
            let (before, after) = (&old.of_line[old_line], &new.of_line[new_line]);
            let (was, is) = (&input.before[before.clone()], &input.after[after.clone()]);
            diff.compute_with(WORD_ALGORITHM, was, is, distinct);
            // Where the same words could be marked a little earlier or later, this takes
            // the later place and joins marks that can be joined, as git does for lines:
            // in `round(sum, digits),` after `sum,` the marked words after `sum` are
            // ` digits),` and not `, digits)`.
            diff.postprocess_with(was, is, imara_diff::NoSliderHeuristic);
            let removed = |word| diff.is_removed(word);
            let added = |word| diff.is_added(word);
            emphasis.old[old_line] = words::marks(&old.all[before.clone()], removed);
            emphasis.new[new_line] = words::marks(&new.all[after.clone()], added);
        }
        emphasis
    }
}

/// Puts a run of unchanged lines into `rows`: its first `head` lines and its last `tail`
/// lines as they are, and what lies between them, if anything, as one skipped row. `old`
/// is the run in the old version, `new` where it starts in the new one.
fn unchanged(rows: &mut Vec<Row>, old: Range<u32>, new: u32, head: u32, tail: u32) {
    let len = old.end - old.start;
    let head = head.min(len);
    let tail = tail.min(len - head);
    let context = |line: u32| Row::Context {
        old: old.start + line,
        new: new + line,
    };
    rows.extend((0..head).map(context));
    if head + tail < len {
        rows.push(Row::Skipped {
            old: old.start + head..old.end - tail,
            new: new + head..new + len - tail,
        });
    }
    rows.extend((len - tail..len).map(context));
}

/// The words that changed inside the changed lines of one hunk: what a reader's eye should
/// land on when a line was edited and not replaced.
///
/// A word is a run of letters, digits and underscores, a run of white space, or one other
/// character. The removed lines of the hunk are compared with its added lines word by
/// word, to learn which removed line became which added line: the one it shares the most
/// with, white space aside, if that line shares the most with it too. Such a line was
/// edited. Its words are compared with those of the line it became, and a word the two
/// share is not marked.
///
/// Every other line of the hunk was removed or added as a whole and has nothing marked,
/// and so has every line of a hunk that only removes or only adds: the whole line is the
/// change, and its colour already says so.
///
/// A hunk of more than 4,096 words, both sides together, has nothing marked either, and
/// nor has one of more than 65,536 bytes, the line feeds among them. Its words are not
/// compared, since that could take longer than a reader would wait, and a change of that
/// size is read line by line anyway.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Emphasis {
    /// The first line of the hunk in the old version and in the new one.
    first: (u32, u32),
    /// The ranges of each removed line, from the first.
    old: Vec<Vec<Range<usize>>>,
    /// The ranges of each added line, from the first.
    new: Vec<Vec<Range<usize>>>,
}

impl Emphasis {
    /// The ranges of bytes to mark in one line of the hunk, in order and not touching.
    /// They count from the start of the line, as [`FileDiff::line`] returns it, and where
    /// the line is UTF-8 none starts or ends inside a character. Empty for a line outside
    /// the hunk.
    pub fn of(&self, side: Side, line: u32) -> &[Range<usize>] {
        let (first, lines) = match side {
            Side::Old => (self.first.0, &self.old),
            Side::New => (self.first.1, &self.new),
        };
        line.checked_sub(first)
            .and_then(|line| lines.get(line as usize))
            .map_or(&[], Vec::as_slice)
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
/// - `NOTICE.md` and, under `licences`, the licence texts of the projects the changes
///   are copied from;
/// - `record.sh`, which writes `hunks.tsv` again from the archive.
#[cfg(test)]
mod corpus {
    use std::io::Read;

    use ruzstd::decoding::StreamingDecoder;

    use crate::Hunk;

    const ARCHIVE: &[u8] = include_bytes!("../corpus/pairs.tar.zst");
    const GIT_HUNKS: &str = include_str!("../corpus/hunks.tsv");
    const PINS: &str = include_str!("../corpus/pins.tsv");

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
    ///
    /// The three files have to tell of the same changes in the same order: a change with
    /// no recorded hunks, or with no line that says where it comes from, stops the test
    /// that asked.
    pub(crate) fn pairs() -> Vec<Pair> {
        let mut archive = Vec::new();
        StreamingDecoder::new(ARCHIVE)
            .expect("the archive is one zstd frame")
            .read_to_end(&mut archive)
            .expect("the archive unpacks");
        let files = files(&archive);

        // The first line of each list is not a change: the git that was run, and the
        // names of the columns.
        let recorded: Vec<(&str, &str)> = GIT_HUNKS
            .lines()
            .skip(1)
            .map(|line| line.split_once('\t').expect("a name, a tab, the hunks"))
            .collect();
        let pinned: Vec<&str> = PINS
            .lines()
            .skip(1)
            .map(|line| line.split('\t').next().expect("a name"))
            .collect();

        let pairs: Vec<Pair> = files
            .chunks(2)
            .zip(&recorded)
            .map(|(versions, (id, git_hunks))| {
                let [(old_name, old), (new_name, new)] = versions else {
                    panic!("the archive ends on half a change");
                };
                assert!(
                    old_name.starts_with(&format!("{id}/a.")),
                    "{old_name} is not the old version of {id}"
                );
                assert!(
                    new_name.starts_with(&format!("{id}/b.")),
                    "{new_name} is not the new version of {id}"
                );
                Pair {
                    id: (*id).to_owned(),
                    old: old.to_vec(),
                    new: new.to_vec(),
                    git_hunks: hunks(git_hunks),
                }
            })
            .collect();
        assert_eq!(files.len(), 2 * recorded.len(), "files and recorded hunks");
        assert!(
            pairs.iter().map(|pair| pair.id.as_str()).eq(pinned),
            "pins.tsv does not name the changes of the archive, in their order"
        );
        pairs
    }

    /// The files of a tar archive, by name. Of each header of 512 bytes only two fields
    /// are read: the name, at the start, and the size, as octal digits at byte 124. The
    /// content follows its header and is filled up to the next 512 bytes; a header with
    /// no name ends the archive.
    fn files(archive: &[u8]) -> Vec<(&str, &[u8])> {
        let mut files = Vec::new();
        let mut at = 0;
        while archive[at] != 0 {
            let header = &archive[at..at + 512];
            let name = field(&header[..100]);
            let size = usize::from_str_radix(field(&header[124..136]), 8).expect("a size");
            files.push((name, &archive[at + 512..at + 512 + size]));
            at += 512 + size.div_ceil(512) * 512;
        }
        files
    }

    /// A field of a tar header, up to the zero byte or the space that ends it.
    fn field(bytes: &[u8]) -> &str {
        let end = bytes
            .iter()
            .position(|byte| matches!(byte, 0 | b' '))
            .unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..end]).expect("a field in ASCII")
    }

    /// Reads hunks written as `hunks.tsv` writes them: `-a,n+b,m` with a space between
    /// two, in the numbers of git's own `@@ -a,n +b,m @@` line. git counts lines from 1,
    /// and where a side has no lines its number is the line the change comes after: in
    /// both cases the hunk starts, counted from 0, at `a - 1` or at `a`.
    pub(crate) fn hunks(written: &str) -> Vec<Hunk> {
        let side = |text: &str| {
            let (start, count) = text.split_once(',').expect("a start, a comma, a count");
            let start: u32 = start.parse().expect("a line number");
            let count: u32 = count.parse().expect("a count of lines");
            let start = if count == 0 { start } else { start - 1 };
            start..start + count
        };
        written
            .split(' ')
            .map(|hunk| {
                let (old, new) = hunk
                    .strip_prefix('-')
                    .and_then(|sides| sides.split_once('+'))
                    .expect("a hunk as -a,n+b,m");
                Hunk {
                    old: side(old),
                    new: side(new),
                }
            })
            .collect()
    }

    /// Writes hunks as `hunks.tsv` does, for the message of a test that fails.
    pub(crate) fn written(hunks: &[Hunk]) -> String {
        let side = |lines: &std::ops::Range<u32>| {
            let start = if lines.is_empty() {
                lines.start
            } else {
                lines.start + 1
            };
            format!("{start},{}", lines.len())
        };
        let hunks: Vec<String> = hunks
            .iter()
            .map(|hunk| format!("-{}+{}", side(&hunk.old), side(&hunk.new)))
            .collect();
        hunks.join(" ")
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

    /// The five changes of the corpus where the diff library places a change otherwise
    /// than git does, each with the hunks this crate gives for it, written as `hunks.tsv`
    /// writes git's. In all five both answers turn the old file into the new one: they
    /// differ in which of several equal lines they count as the changed ones.
    const PLACED_OTHERWISE: [(&str, &str); 5] = [
        (
            "p0111",
            "-1,10+1,20 -12,5+21,0 -49,1+54,1 -121,1+126,1 -177,0+183,2 -195,0+203,1 \
             -234,0+243,1 -253,7+261,0 -262,5+264,1 -268,2+266,2 -287,1+284,0 -334,2+331,2 \
             -456,1+453,1 -572,0+570,17 -679,3+693,3 -684,1+698,1 -702,2+716,2 -854,1+868,1 \
             -860,0+875,14 -1012,0+1041,1 -1019,1+1047,0 -1021,0+1050,2",
        ),
        (
            "p0117",
            "-11,0+12,2 -46,1+48,4 -69,1+74,1 -76,1+81,1 -81,1+86,1 -170,1+175,1 -273,0+279,3 \
             -278,1+286,1 -285,10+293,1 -314,10+313,1 -328,1+318,1 -331,0+322,8 -334,1+332,1 \
             -467,3+465,13 -471,9+479,3 -481,1+483,31",
        ),
        (
            "p0122",
            "-1,4+1,5 -8,1+9,1 -13,3+14,3 -18,1+18,0 -41,1+41,1 -43,56+43,18 -100,7+61,0 \
             -107,0+63,2 -109,21+65,0 -133,3+69,94 -139,2+166,2 -142,5+169,4 -149,2+175,7 \
             -155,8+186,2 -165,2+190,2 -169,1+194,1 -171,1+196,1 -174,1+199,1 -179,1+204,1 \
             -187,1+212,1 -191,1+216,1 -193,1+218,1 -200,4+225,1 -207,1+228,0 -209,3+230,3 \
             -213,1+234,1 -216,7+237,11 -224,19+249,10 -245,9+261,3 -255,12+264,0 \
             -267,0+266,1 -270,3+269,5 -276,5+277,8 -282,3+286,2 -286,1+289,1 -288,3+291,7 \
             -294,2+301,2 -300,2+307,2 -304,2+311,2 -308,1+315,1 -320,1+327,1 -324,1+331,1 \
             -327,2+334,2 -331,1+338,1 -346,1+353,1 -349,5+356,7 -355,7+364,7 -363,2+372,4 \
             -366,6+377,14 -373,1+392,97",
        ),
        (
            "p0281",
            "-8,1+8,1 -11,2+10,0 -14,1+12,2 -16,1+15,2 -18,2+17,0 -21,5+19,2 -28,4+23,2 \
             -33,1+26,3 -36,1+31,2 -39,7+35,3 -48,4+40,3 -54,56+45,4 -113,1+52,20 \
             -118,2+76,15 -129,1+100,1 -152,1+123,1 -174,1+145,1 -202,2+173,7 -205,5+180,0 \
             -211,5+182,1 -217,7+184,4 -225,1+189,40 -231,1+234,1 -233,2+236,2 -251,1+254,1 \
             -259,7+262,5 -267,1+268,1 -269,1+270,1 -271,4+272,4 -277,1+278,1 -292,0+294,3 \
             -340,3+344,2 -345,2+348,6 -349,7+356,4 -363,3+367,9",
        ),
        (
            "p0369",
            "-373,0+374,7 -375,0+383,12 -378,10+397,1 -392,8+401,0",
        ),
    ];

    #[test]
    fn on_the_569_recorded_changes_the_hunks_give_the_new_file_and_are_gits_on_all_but_five() {
        let pairs = corpus::pairs();
        assert_eq!(pairs.len(), 569);

        let (mut as_git, mut otherwise) = (0, 0);
        for pair in &pairs {
            let id = &pair.id;
            let diff = FileDiff::new(&pair.old, &pair.new);
            let hunks = diff.hunks();
            assert!(
                applied(&pair.old, &pair.new, hunks) == pair.new,
                "{id}: the hunks do not turn the old file into the new one"
            );

            match PLACED_OTHERWISE.iter().find(|(named, _)| named == id) {
                None => {
                    assert!(
                        hunks == pair.git_hunks,
                        "{id}: the hunks are not git's, but {}",
                        corpus::written(hunks)
                    );
                    as_git += 1;
                }
                Some((_, ours)) => {
                    let ours = corpus::hunks(ours);
                    assert!(
                        hunks == ours,
                        "{id}: the hunks are no longer the ones written here, but {}",
                        corpus::written(hunks)
                    );
                    // The other side of the same promise: once git and the crate agree
                    // on one of the five, it has to leave the list.
                    assert!(
                        ours != pair.git_hunks,
                        "{id}: git's recorded hunks are now the crate's"
                    );
                    assert!(
                        applied(&pair.old, &pair.new, &pair.git_hunks) == pair.new,
                        "{id}: git's recorded hunks do not give the new file"
                    );
                    otherwise += 1;
                }
            }
        }
        assert_eq!((as_git, otherwise), (564, 5));
    }

    /// A small file before and after five changes: a line added at the top, so that from
    /// there on a line has another number in the new version than in the old; then a line
    /// edited in one word, three lines rewritten as two (the first gains a call, the
    /// second goes, the third loses its end), a line added and a line removed.
    const BEFORE: &str = r#"fn price(order: &Order) -> u32 {
    let count = order.count;
    let unit = order.unit;
    count * unit
}

fn label(order: &Order) -> String {
    let name = order.name.trim();
    let code = order.code;
    format!("{name} ({code})")
}

fn main() {
    let order = load();
    println!("{}", label(&order));
    println!("{}", price(&order));
}
"#;
    const AFTER: &str = r#"// Prices are in cents.
fn price(order: &Order) -> u32 {
    let count = order.quantity;
    let unit = order.unit;
    count * unit
}

fn label(order: &Order) -> String {
    let name = order.name.trim().to_uppercase();
    format!("{name}")
}

fn main() {
    let order = load();
    check(&order);
    println!("{}", label(&order));
}
"#;

    fn hunk(old: Range<u32>, new: Range<u32>) -> Hunk {
        Hunk { old, new }
    }

    /// What a list of rows shows, whatever its layout: the unchanged and the skipped rows
    /// as they are, and each run of changed rows as the lines it holds of each version.
    #[derive(Debug, PartialEq)]
    enum Shown {
        Unchanged(Row),
        Change { old: Vec<u32>, new: Vec<u32> },
    }

    fn shown(rows: &[Row]) -> Vec<Shown> {
        let mut shown = Vec::new();
        for row in rows {
            if matches!(row, Row::Skipped { .. } | Row::Context { .. }) {
                shown.push(Shown::Unchanged(row.clone()));
                continue;
            }
            if !matches!(shown.last(), Some(Shown::Change { .. })) {
                shown.push(Shown::Change {
                    old: Vec::new(),
                    new: Vec::new(),
                });
            }
            if let Some(Shown::Change { old, new }) = shown.last_mut() {
                old.extend(row.line(Side::Old));
                new.extend(row.line(Side::New));
            }
        }
        shown
    }

    /// The rows of the changes alone, in order.
    fn changed(rows: &[Row]) -> Vec<Row> {
        let unchanged = |row: &&Row| matches!(row, Row::Skipped { .. } | Row::Context { .. });
        rows.iter().filter(|row| !unchanged(row)).cloned().collect()
    }

    /// The marks of a line that has one.
    fn one(range: Range<usize>) -> Vec<Range<usize>> {
        vec![range]
    }

    fn text(diff: &FileDiff, side: Side, line: u32) -> &str {
        std::str::from_utf8(diff.line(side, line)).expect("a line of the test in UTF-8")
    }

    /// The marked parts of one line, as text.
    fn marked<'a>(diff: &'a FileDiff, emphasis: &Emphasis, side: Side, line: u32) -> Vec<&'a str> {
        let text = text(diff, side, line);
        emphasis
            .of(side, line)
            .iter()
            .map(|range| &text[range.clone()])
            .collect()
    }

    /// What is left of some lines of one side when the marked parts are taken out, with
    /// nothing between the lines.
    fn unmarked(diff: &FileDiff, emphasis: &Emphasis, side: Side, lines: Range<u32>) -> String {
        let mut left = String::new();
        for line in lines {
            let text = text(diff, side, line);
            let mut from = 0;
            for range in emphasis.of(side, line) {
                left.push_str(&text[from..range.start]);
                from = range.end;
            }
            left.push_str(&text[from..]);
        }
        left
    }

    #[test]
    fn unified_and_split_lay_out_the_same_hunks_and_emphasis_marks_only_the_changed_words() {
        let diff = FileDiff::new(BEFORE.as_bytes(), AFTER.as_bytes());
        let hunks = [
            hunk(0..0, 0..1),
            hunk(1..2, 2..3),
            hunk(7..10, 8..10),
            hunk(14..14, 14..15),
            hunk(15..16, 16..16),
        ];
        assert_eq!(diff.hunks(), hunks);

        for context in [0, 1, CONTEXT_LINES, u32::MAX] {
            let unified = diff.rows(Layout::Unified, context);
            let split = diff.rows(Layout::Split, context);

            // The same rows around the changes, in the same order, and between them the
            // same lines of each change: those of the hunks, each hunk once.
            assert_eq!(shown(&unified), shown(&split), "context {context}");
            let changes: Vec<Shown> = shown(&unified)
                .into_iter()
                .filter(|shown| matches!(shown, Shown::Change { .. }))
                .collect();
            let of_the_hunks: Vec<Shown> = hunks
                .iter()
                .map(|hunk| Shown::Change {
                    old: hunk.old.clone().collect(),
                    new: hunk.new.clone().collect(),
                })
                .collect();
            assert_eq!(changes, of_the_hunks, "context {context}");

            // And they are two layouts: neither holds a changed row of the other's kind.
            assert!(
                !unified.iter().any(|row| matches!(row, Row::Changed { .. })),
                "context {context}"
            );
            assert!(
                !split
                    .iter()
                    .any(|row| matches!(row, Row::Removed { .. } | Row::Added { .. })),
                "context {context}"
            );
        }

        // How each layout puts the lines of a change in rows: one under the other, the
        // removed lines first; or side by side, in order, with a gap where one side of
        // the change is longer.
        assert_eq!(
            changed(&diff.rows(Layout::Unified, CONTEXT_LINES)),
            [
                Row::Added { new: 0 },
                Row::Removed { old: 1 },
                Row::Added { new: 2 },
                Row::Removed { old: 7 },
                Row::Removed { old: 8 },
                Row::Removed { old: 9 },
                Row::Added { new: 8 },
                Row::Added { new: 9 },
                Row::Added { new: 14 },
                Row::Removed { old: 15 },
            ]
        );
        let beside = |old, new| Row::Changed { old, new };
        assert_eq!(
            changed(&diff.rows(Layout::Split, CONTEXT_LINES)),
            [
                beside(None, Some(0)),
                beside(Some(1), Some(2)),
                beside(Some(7), Some(8)),
                beside(Some(8), Some(9)),
                beside(Some(9), None),
                beside(None, Some(14)),
                beside(Some(15), None),
            ]
        );

        // The line edited in one word: that word is marked on each side, where it
        // stands, and the `count` earlier in the line, which did not change, is not.
        // The line is line 1 of the old version and line 2 of the new one, and each side
        // answers to its own number only.
        let edited = diff.emphasis(1);
        let at = "    let count = order.".len();
        assert_eq!(edited.of(Side::Old, 1), one(at..at + "count".len()));
        assert_eq!(edited.of(Side::New, 2), one(at..at + "quantity".len()));
        assert_eq!(marked(&diff, &edited, Side::Old, 1), ["count"]);
        assert_eq!(marked(&diff, &edited, Side::New, 2), ["quantity"]);
        assert_eq!(edited.of(Side::Old, 2), []);
        assert_eq!(edited.of(Side::New, 1), []);
        // Its marks are for its own lines only.
        assert_eq!(edited.of(Side::Old, 7), []);
        assert_eq!(edited.of(Side::New, 8), []);

        // Three lines rewritten as two. The first gained a call: what it gained is marked
        // in the new line, and in the old line, which lost nothing, nothing is. The third
        // lost its end, and only that is marked. The second became no line of the new
        // version: it is removed as a whole, and has nothing marked, though some of its
        // words stand in the lines around it.
        let rewritten = diff.emphasis(2);
        assert_eq!(rewritten.of(Side::Old, 7), []);
        assert_eq!(marked(&diff, &rewritten, Side::New, 8), [".to_uppercase()"]);
        assert_eq!(rewritten.of(Side::Old, 8), []);
        assert_eq!(marked(&diff, &rewritten, Side::Old, 9), [" ({code})"]);
        assert_eq!(rewritten.of(Side::New, 9), []);
        // Here too each side answers to its own numbers: line 9 is marked in the old
        // version and not in the new one, and line 8 the other way.
        assert_eq!(rewritten.of(Side::Old, 10), []);
        assert_eq!(rewritten.of(Side::New, 7), []);

        // A line that is only added, or only removed, is the change as a whole: nothing
        // in it is marked.
        assert_eq!(diff.emphasis(0).of(Side::New, 0), []);
        assert_eq!(diff.emphasis(3).of(Side::New, 14), []);
        assert_eq!(diff.emphasis(4).of(Side::Old, 15), []);

        // The same holds inside a hunk that also edits a line. Here a line is edited in
        // one word and a line is added under it. The added line ends in the words the
        // edited line ends in, and still the one changed word is all that is marked.
        let diff = FileDiff::new(
            b"    return compute(x);\n",
            b"    return compute(y);\n    log(y);\n",
        );
        assert_eq!(diff.hunks(), [hunk(0..1, 0..2)]);
        let emphasis = diff.emphasis(0);
        assert_eq!(marked(&diff, &emphasis, Side::Old, 0), ["x"]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 0), ["y"]);
        assert_eq!(emphasis.of(Side::New, 1), []);

        // And here a line is removed between two edited lines. Each edited line is
        // marked against the line it became, not against the line that stands beside it
        // in the split layout, and the removed line has nothing marked.
        let diff = FileDiff::new(
            b"let a = 1;\nlet b = 2;\nlet c = 3;\n",
            b"let a = 10;\nlet c = 30;\n",
        );
        assert_eq!(diff.hunks(), [hunk(0..3, 0..2)]);
        let emphasis = diff.emphasis(0);
        assert_eq!(marked(&diff, &emphasis, Side::Old, 0), ["1"]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 0), ["10"]);
        assert_eq!(emphasis.of(Side::Old, 1), []);
        assert_eq!(marked(&diff, &emphasis, Side::Old, 2), ["3"]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 1), ["30"]);
    }

    #[test]
    fn a_last_line_with_no_line_feed_is_a_line_and_gaining_the_feed_is_a_change() {
        let diff = FileDiff::new(b"one\ntwo", b"one\ntwo\n");
        assert_eq!(diff.line_count(Side::Old), 2);
        assert_eq!(diff.line_count(Side::New), 2);
        assert_eq!(diff.line(Side::Old, 1), b"two");
        assert_eq!(diff.line(Side::New, 1), b"two");
        assert_eq!(diff.hunks(), [hunk(1..2, 1..2)]);

        // Nothing of what is shown of the line changed, so nothing in it is marked.
        let emphasis = diff.emphasis(0);
        assert_eq!(emphasis.of(Side::Old, 1), []);
        assert_eq!(emphasis.of(Side::New, 1), []);
    }

    #[test]
    fn a_new_file_is_one_hunk_of_added_lines_with_nothing_beside_them() {
        let diff = FileDiff::new(b"", b"one\ntwo\n");
        assert_eq!(diff.line_count(Side::Old), 0);
        assert_eq!(diff.hunks(), [hunk(0..0, 0..2)]);
        assert_eq!(
            diff.rows(Layout::Unified, CONTEXT_LINES),
            [Row::Added { new: 0 }, Row::Added { new: 1 }]
        );
        let alone = |new| Row::Changed {
            old: None,
            new: Some(new),
        };
        assert_eq!(
            diff.rows(Layout::Split, CONTEXT_LINES),
            [alone(0), alone(1)]
        );
    }

    /// `count` lines, each with its number.
    fn numbered(count: u32) -> String {
        (0..count)
            .map(|number| format!("line {number}\n"))
            .collect()
    }

    /// Unchanged lines in rows, for the old lines `old`; `shift` is how much higher
    /// their numbers are in the new version.
    fn context(old: Range<u32>, shift: u32) -> impl Iterator<Item = Row> {
        old.map(move |old| Row::Context {
            old,
            new: old + shift,
        })
    }

    #[test]
    fn unchanged_lines_farther_from_any_change_than_the_context_are_one_skipped_row() {
        // Twenty lines. Line 5 becomes two lines and line 15 is rewritten, so nine
        // unchanged lines lie between the changes, and from there on the new numbers are
        // one higher than the old.
        let old = numbered(20);
        let new = old
            .replace("line 5\n", "line five\nand a half\n")
            .replace("line 15\n", "line fifteen\n");
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        assert_eq!(diff.hunks(), [hunk(5..6, 5..7), hunk(15..16, 16..17)]);
        let first = [
            Row::Removed { old: 5 },
            Row::Added { new: 5 },
            Row::Added { new: 6 },
        ];
        let second = [Row::Removed { old: 15 }, Row::Added { new: 16 }];
        let skipped = |old: Range<u32>, shift| Row::Skipped {
            new: old.start + shift..old.end + shift,
            old,
        };

        // Three lines each way: the start of the file, the middle of the nine and the
        // end of the file are left out.
        let mut expected = vec![skipped(0..2, 0)];
        expected.extend(context(2..5, 0));
        expected.extend(first.clone());
        expected.extend(context(6..9, 1));
        expected.push(skipped(9..12, 1));
        expected.extend(context(12..15, 1));
        expected.extend(second.clone());
        expected.extend(context(16..19, 1));
        expected.push(skipped(19..20, 1));
        assert_eq!(diff.rows(Layout::Unified, 3), expected);

        // Four lines each way: eight of the nine are shown, and the one left is still a
        // skipped row. The four lines after the last change are the end of the file.
        let mut expected = vec![skipped(0..1, 0)];
        expected.extend(context(1..5, 0));
        expected.extend(first.clone());
        expected.extend(context(6..10, 1));
        expected.push(skipped(10..11, 1));
        expected.extend(context(11..15, 1));
        expected.extend(second.clone());
        expected.extend(context(16..20, 1));
        assert_eq!(diff.rows(Layout::Unified, 4), expected);

        // Five lines each way would be ten of the nine: where the lines after one change
        // meet the lines before the next, each line is shown once.
        let mut expected: Vec<Row> = context(0..5, 0).collect();
        expected.extend(first);
        expected.extend(context(6..15, 1));
        expected.extend(second);
        expected.extend(context(16..20, 1));
        assert_eq!(diff.rows(Layout::Unified, 5), expected);
    }

    #[test]
    fn the_largest_context_shows_the_whole_file() {
        // Changes on the first and on the last line: the context reaches past both ends.
        let old = numbered(40);
        let new = old
            .replace("line 0\n", "line zero\n")
            .replace("line 39\n", "line thirty-nine\n");
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        let mut expected = vec![Row::Removed { old: 0 }, Row::Added { new: 0 }];
        expected.extend(context(1..39, 0));
        expected.extend([Row::Removed { old: 39 }, Row::Added { new: 39 }]);
        assert_eq!(diff.rows(Layout::Unified, u32::MAX), expected);

        // A file with no change has no line near a change, and is still shown whole.
        let same = FileDiff::new(old.as_bytes(), old.as_bytes());
        assert_eq!(same.hunks(), []);
        let all: Vec<Row> = context(0..40, 0).collect();
        assert_eq!(same.rows(Layout::Split, u32::MAX), all);
        assert_eq!(
            same.rows(Layout::Split, CONTEXT_LINES),
            [Row::Skipped {
                old: 0..40,
                new: 0..40
            }]
        );

        let empty = FileDiff::new(b"", b"");
        assert_eq!(empty.rows(Layout::Unified, u32::MAX), []);
        assert_eq!(empty.rows(Layout::Unified, CONTEXT_LINES), []);
    }

    /// Fails unless `rows` hold every line of both versions once and in order, each
    /// unchanged or skipped row holds a line that is the same in both, and no two
    /// skipped rows follow one another.
    fn assert_every_line_once(diff: &FileDiff, rows: &[Row], what: &str) {
        let (mut old, mut new) = (0, 0);
        let mut after_skipped = false;
        for row in rows {
            let same = |old, new| diff.line(Side::Old, old) == diff.line(Side::New, new);
            if let Row::Skipped {
                old: skipped_old,
                new: skipped_new,
            } = row
            {
                assert!(!after_skipped, "{what}: two skipped rows in a row");
                assert!(!skipped_old.is_empty(), "{what}: {row:?} skips nothing");
                assert_eq!(
                    (skipped_old.start, skipped_new.start),
                    (old, new),
                    "{what}: {row:?}"
                );
                assert_eq!(skipped_old.len(), skipped_new.len(), "{what}: {row:?}");
                assert_eq!((row.line(Side::Old), row.line(Side::New)), (None, None));
                assert!(
                    skipped_old
                        .clone()
                        .zip(skipped_new.clone())
                        .all(|(old, new)| same(old, new)),
                    "{what}: {row:?} skips a changed line"
                );
                (old, new) = (skipped_old.end, skipped_new.end);
                after_skipped = true;
                continue;
            }
            after_skipped = false;
            let lines = (row.line(Side::Old), row.line(Side::New));
            assert_ne!(lines, (None, None), "{what}: {row:?} shows no line");
            if let Some(line) = lines.0 {
                assert_eq!(line, old, "{what}: {row:?}");
                old += 1;
            }
            if let Some(line) = lines.1 {
                assert_eq!(line, new, "{what}: {row:?}");
                new += 1;
            }
            if let Row::Context { old, new } = row {
                assert!(same(*old, *new), "{what}: {row:?} is a changed line");
            }
        }
        let end = (diff.line_count(Side::Old), diff.line_count(Side::New));
        assert_eq!((old, new), end, "{what}: lines after the last row");
    }

    #[test]
    fn on_the_recorded_changes_the_rows_of_both_layouts_hold_every_line_once() {
        for pair in corpus::pairs() {
            let diff = FileDiff::new(&pair.old, &pair.new);
            for touching in diff.hunks().windows(2) {
                assert!(
                    touching[0].old.end < touching[1].old.start
                        && touching[0].new.end < touching[1].new.start,
                    "{}: {touching:?} touch",
                    pair.id
                );
            }
            for layout in [Layout::Unified, Layout::Split] {
                for context in [0, CONTEXT_LINES, u32::MAX] {
                    let rows = diff.rows(layout, context);
                    let what = format!("{} as {layout:?} with {context} lines around", pair.id);
                    assert_every_line_once(&diff, &rows, &what);
                }
            }
        }
    }

    #[test]
    fn each_changed_word_is_marked_in_its_own_line_and_neighbours_as_one_range() {
        let diff = FileDiff::new(
            b"let width = 10;\nlet height = area(first, second);\n",
            b"let width = 12;\nlet depth = area(other);\n",
        );
        assert_eq!(diff.hunks(), [hunk(0..2, 0..2)]);
        let emphasis = diff.emphasis(0);
        assert_eq!(marked(&diff, &emphasis, Side::Old, 0), ["10"]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 0), ["12"]);
        // `first`, the comma, the space and `second` all went: one range, not four.
        assert_eq!(
            marked(&diff, &emphasis, Side::Old, 1),
            ["height", "first, second"]
        );
        assert_eq!(emphasis.of(Side::Old, 1), [4..10, 18..31]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 1), ["depth", "other"]);
    }

    #[test]
    fn a_mark_that_could_sit_in_two_places_sits_in_the_later_one() {
        // After `sum` the new line holds a comma, a space, `digits`, a bracket and a
        // comma, and the old line a comma. Either comma can be the old one.
        let diff = FileDiff::new(b"total: sum,\n", b"total: round(sum, digits),\n");
        let emphasis = diff.emphasis(0);
        assert_eq!(emphasis.of(Side::Old, 0), []);
        assert_eq!(
            marked(&diff, &emphasis, Side::New, 0),
            ["round(", " digits),"]
        );
    }

    #[test]
    fn marks_end_between_characters_where_the_line_is_utf8() {
        // `marked` slices the text of the line, which panics inside a character.
        let diff = FileDiff::new(
            "total = café → thé\n".as_bytes(),
            "total = caffè ⇒ thé\n".as_bytes(),
        );
        let emphasis = diff.emphasis(0);
        assert_eq!(marked(&diff, &emphasis, Side::Old, 0), ["café", "→"]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 0), ["caffè", "⇒"]);
    }

    #[test]
    fn bytes_that_are_no_utf8_are_compared_like_any_other() {
        let diff = FileDiff::new(b"key = \xff\xfe;\n", b"key = \xff;\n");
        assert_eq!(diff.line(Side::Old, 0), b"key = \xff\xfe;");
        let emphasis = diff.emphasis(0);
        assert_eq!(emphasis.of(Side::Old, 0), one(7..8));
        assert_eq!(emphasis.of(Side::New, 0), []);
    }

    #[test]
    fn a_hunk_of_more_than_4096_words_has_nothing_marked() {
        // One line: `key`, then a space and a letter `count` times, then `end` and the
        // line feed.
        let line = |letter: &str, count: usize, end: &str| {
            format!("key{}{end}\n", format!(" {letter}").repeat(count))
        };
        // With 1,022 letters on the old side and 1,024 on the new one that is 2,046 and
        // 2,050 words, the line feeds among them, 4,096 together: the most that are
        // compared.
        let (old, new) = (line("a", 1022, ""), line("b", 1024, ""));
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        assert_eq!(diff.hunks(), [hunk(0..1, 0..1)]);
        let emphasis = diff.emphasis(0);
        assert!(!emphasis.of(Side::Old, 0).is_empty());
        assert!(!emphasis.of(Side::New, 0).is_empty());
        assert_eq!(
            unmarked(&diff, &emphasis, Side::Old, 0..1),
            unmarked(&diff, &emphasis, Side::New, 0..1)
        );

        // One word more, on either side, and the hunk is not compared.
        for (old, new) in [
            (line("a", 1022, ";"), new.clone()),
            (old, line("b", 1024, ";")),
        ] {
            let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
            assert_eq!(diff.hunks(), [hunk(0..1, 0..1)]);
            let emphasis = diff.emphasis(0);
            assert_eq!(emphasis.of(Side::Old, 0), []);
            assert_eq!(emphasis.of(Side::New, 0), []);
        }
    }

    #[test]
    fn a_hunk_of_more_than_65536_bytes_has_nothing_marked() {
        // One line: `key = `, a letter `count` times, `end` and the line feed.
        let line = |letter: &str, count: usize, end: &str| {
            format!("key = {}{end}\n", letter.repeat(count))
        };
        // With 30,000 letters on the old side and 35,522 on the new one that is 30,007
        // and 35,529 bytes, the line feeds among them, 65,536 together: the most that
        // are compared. The words are twelve, so it is not their number that decides.
        let (old, new) = (line("a", 30_000, ""), line("b", 35_522, ""));
        assert_eq!(old.len() + new.len(), 65_536);
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        assert_eq!(diff.hunks(), [hunk(0..1, 0..1)]);
        let emphasis = diff.emphasis(0);
        assert_eq!(emphasis.of(Side::Old, 0), one(6..30_006));
        assert_eq!(emphasis.of(Side::New, 0), one(6..35_528));

        // One byte more, on either side, and the hunk is not compared.
        for (old, new) in [
            (line("a", 30_000, ";"), new.clone()),
            (old, line("b", 35_522, ";")),
        ] {
            let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
            assert_eq!(diff.hunks(), [hunk(0..1, 0..1)]);
            let emphasis = diff.emphasis(0);
            assert_eq!(emphasis.of(Side::Old, 0), []);
            assert_eq!(emphasis.of(Side::New, 0), []);
        }
    }

    /// `count` lines on each side, every old line the same and every new line the same:
    /// 11 and 16 words, the line feeds among them, of which the two sides share few, and
    /// those many times over. Comparing such words is what costs the most.
    fn rewritten_lines(count: usize) -> (String, String) {
        (
            "x = x + 1;\n".repeat(count),
            "y = (y, 2) - y;\n".repeat(count),
        )
    }

    /// One line of `count` one-letter words with a space after each, the letters taken
    /// from eight in an order that `step` sets.
    fn long_line(count: usize, step: usize) -> String {
        let mut line: String = (0..count)
            .flat_map(|word| [b"abcdefgh"[(word * step + word / 7) % 8] as char, ' '])
            .collect();
        line.push('\n');
        line
    }

    #[test]
    fn the_marks_of_one_hunk_take_under_50_ms_however_many_words_it_holds() {
        let (few_old, few_new) = rewritten_lines(150);
        let (many_old, many_new) = rewritten_lines(20_000);
        let hunks = [
            // 4,050 words in 300 lines, and 4,002 words in two: under the limit.
            (few_old, few_new, true),
            (long_line(1000, 3), long_line(1000, 5), true),
            // 65,402 bytes, most of them one line of 64,001 words: few enough bytes to
            // be read, and about as many words as are ever cut before a hunk is left
            // alone.
            (long_line(32_000, 3), long_line(700, 5), false),
            // A whole file rewritten, 540,000 words; and two lines of 200,001 each.
            (many_old, many_new, false),
            (long_line(100_000, 3), long_line(100_000, 5), false),
        ];
        for (old, new, compared) in hunks {
            let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
            let lines = diff.line_count(Side::Old);
            assert_eq!(diff.hunks(), [hunk(0..lines, 0..lines)]);

            let mut best = Duration::MAX;
            for _ in 0..5 {
                let start = Instant::now();
                let emphasis = diff.emphasis(0);
                best = best.min(start.elapsed());
                let marked = (0..lines).any(|line| !emphasis.of(Side::Old, line).is_empty());
                assert_eq!(marked, compared, "a hunk of {lines} lines on each side");
            }
            assert!(
                best < Duration::from_millis(50),
                "the best of 5 runs took {best:?} for a hunk of {lines} lines on each side"
            );
        }
    }

    #[test]
    fn a_hunk_of_one_line_of_4_mib_is_left_unmarked_in_under_1_ms() {
        // 4 MiB is the largest version of a file the app shows, so the longest line a
        // hunk can hold there.
        let size = 4 << 20;
        let statement = "var a1=fn(b2,c3);";
        let minified = |name: &str| {
            format!(
                "var {name}=fn(b2,c3);{}\n",
                statement.repeat(size / statement.len())
            )
        };
        let hunks = [
            // A line as a minified script has it: 2.7 million words, one of them changed.
            (minified("a0"), minified("a9")),
            // A line as data has it: one word from end to end, its last character changed.
            // Within the limit of words, and no cheaper to read for that.
            (
                format!("key = {}0;\n", "a".repeat(size)),
                format!("key = {}1;\n", "a".repeat(size)),
            ),
        ];
        for (old, new) in hunks {
            let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
            assert_eq!(diff.hunks(), [hunk(0..1, 0..1)]);

            let mut best = Duration::MAX;
            for _ in 0..5 {
                let start = Instant::now();
                let emphasis = diff.emphasis(0);
                best = best.min(start.elapsed());
                assert_eq!(emphasis.of(Side::Old, 0), []);
                assert_eq!(emphasis.of(Side::New, 0), []);
            }
            // Reading the two lines once, to cut them into words, took about 20 ms where
            // it was measured, and whoever shows the hunk asks again whenever it comes
            // into view. So the limit is far below what reading costs: such a hunk has
            // to be left alone before a byte of it is looked at.
            assert!(
                best < Duration::from_millis(1),
                "the best of 5 runs took {best:?} for a line starting {:?}",
                &old[..24]
            );
        }
    }

    #[test]
    fn a_diff_does_not_pay_for_the_marks_of_its_hunks() {
        // Ten hunks of 140 lines on each side, each just small enough to have its words
        // compared, with one unchanged line between two of them.
        let (hunk_old, hunk_new) = rewritten_lines(140);
        let (mut old, mut new) = (String::new(), String::new());
        for part in 0..10 {
            let between = format!("// part {part}\n");
            old.extend([hunk_old.as_str(), between.as_str()]);
            new.extend([hunk_new.as_str(), between.as_str()]);
        }

        // The two costs are measured on the same machine in the same run, so the test
        // needs no limit in milliseconds: a diff that marked its hunks before anyone
        // asked would take at least as long as the marking does.
        let (mut diffing, mut marking) = (Duration::MAX, Duration::MAX);
        for _ in 0..5 {
            let start = Instant::now();
            let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
            diffing = diffing.min(start.elapsed());
            assert_eq!(diff.hunks().len(), 10);

            let start = Instant::now();
            for (index, hunk) in diff.hunks().iter().enumerate() {
                let emphasis = diff.emphasis(index);
                assert!(!emphasis.of(Side::Old, hunk.old.start).is_empty());
            }
            marking = marking.min(start.elapsed());
        }
        assert!(
            diffing * 4 < marking,
            "the diff took {diffing:?} and marking its hunks {marking:?}, at best of 5 runs"
        );
    }

    #[test]
    fn on_the_recorded_changes_the_marks_are_in_order_inside_their_lines_and_a_marked_line_is_a_line_of_the_other_side_but_for_the_marks()
     {
        let (mut marked_lines, mut marked_bytes) = (0, 0);
        for pair in corpus::pairs() {
            let diff = FileDiff::new(&pair.old, &pair.new);
            for (index, hunk) in diff.hunks().iter().enumerate() {
                let what = format!("{} at {hunk:?}", pair.id);
                let emphasis = diff.emphasis(index);
                let mut marks = 0;
                for (side, lines) in [(Side::Old, &hunk.old), (Side::New, &hunk.new)] {
                    for line in lines.clone() {
                        let text = text(&diff, side, line);
                        let ranges = emphasis.of(side, line);
                        marks += ranges.len();
                        marked_lines += usize::from(!ranges.is_empty());
                        let mut from = 0;
                        for range in ranges {
                            // Not touching the one before: a range that starts where
                            // the last ended would be the same mark cut in two.
                            assert!(
                                (from == 0 || range.start > from) && range.end > range.start,
                                "{what}: {ranges:?} in line {line}"
                            );
                            assert!(
                                text.is_char_boundary(range.start)
                                    && text.is_char_boundary(range.end)
                                    && range.end <= text.len(),
                                "{what}: {range:?} in {text:?}"
                            );
                            from = range.end;
                            marked_bytes += range.len();
                        }
                    }
                    assert_eq!(emphasis.of(side, lines.end), [], "{what}: after the hunk");
                }
                if hunk.old.is_empty() || hunk.new.is_empty() {
                    assert_eq!(marks, 0, "{what}: only added or only removed");
                }

                // A marked line was edited: it became a line of the other side, and with
                // the marks of both taken out the two read the same, so every word that
                // changed between them is marked. The lines that marked lines became
                // come in the order of the marked lines.
                for (side, lines, other, others) in [
                    (Side::Old, &hunk.old, Side::New, &hunk.new),
                    (Side::New, &hunk.new, Side::Old, &hunk.old),
                ] {
                    let mut from = others.start;
                    for line in lines.clone() {
                        if emphasis.of(side, line).is_empty() {
                            continue;
                        }
                        let left = unmarked(&diff, &emphasis, side, line..line + 1);
                        let became = (from..others.end).find(|became| {
                            unmarked(&diff, &emphasis, other, *became..*became + 1) == left
                        });
                        let Some(became) = became else {
                            panic!(
                                "{what}: line {line} of the {side:?} side is marked, and \
                                 with the marks taken out it is no line of the other side"
                            );
                        };
                        from = became + 1;
                    }
                }
            }
        }

        // The invariants above hold as well when a line that was edited has nothing
        // marked. So the sum of what is marked on the whole corpus is written down: a
        // change to which words are marked shows here and has to be looked at. Two hunks
        // of the corpus, both of `p0135`, hold too many words to be compared.
        assert_eq!((marked_lines, marked_bytes), (10_018, 163_656));
    }
}
