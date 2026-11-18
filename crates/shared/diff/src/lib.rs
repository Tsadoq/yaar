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

    // DIFF-02
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
}
