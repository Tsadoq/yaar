//! Line diff and word-level emphasis between two versions of a file, as bytes. Lines count from 0.

mod words;

use std::ops::Range;

const ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Histogram;

/// Histogram keys on the words that are rare on both sides, which pairs lines well.
const PAIRING_ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Histogram;

/// Myers, not histogram: words of a line repeat a lot, and histogram then marks more than the
/// shortest change.
const WORD_ALGORITHM: imara_diff::Algorithm = imara_diff::Algorithm::Myers;

/// Most words, both sides together, of a hunk whose words are compared. Comparing costs up to
/// the square of the word count, and a hunk has no size bound.
const MOST_WORDS: usize = 4096;

/// Most bytes, both sides together with line feeds, of a hunk whose words are compared:
/// 16 per word of MOST_WORDS. Checked before any line is cut, since a line has no length bound.
const MOST_BYTES: usize = 65_536;

pub const CONTEXT_LINES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    New,
}

/// Either range may be empty, never both; an empty one sits before the line at its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<u32>,
    pub new: Range<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layout {
    Unified,
    Split,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Skipped {
        old: Range<u32>,
        new: Range<u32>,
    },
    Context {
        old: u32,
        new: u32,
    },
    Removed {
        old: u32,
    },
    Added {
        new: u32,
    },
    /// Split only: a removed line beside an added one; the shorter side of the hunk has None.
    Changed {
        old: Option<u32>,
        new: Option<u32>,
    },
}

impl Row {
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

#[derive(Debug, Clone)]
pub struct FileDiff {
    old: Text,
    new: Text,
    hunks: Vec<Hunk>,
}

#[derive(Debug, Clone)]
struct Text {
    bytes: Vec<u8>,
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

    fn start(&self, line: u32) -> usize {
        self.starts
            .get(line as usize)
            .copied()
            .unwrap_or(self.bytes.len())
    }

    fn token(&self, line: u32) -> &[u8] {
        &self.bytes[self.starts[line as usize]..self.start(line + 1)]
    }

    fn len(&self, lines: &Range<u32>) -> usize {
        self.start(lines.end) - self.start(lines.start)
    }
}

impl FileDiff {
    /// Panics on a version of more lines than an `i32` counts (imara-diff limit).
    pub fn new(old: &[u8], new: &[u8]) -> Self {
        // Lines compare with their line feed, as in git: a last line that gained or lost it is
        // changed.
        let input = imara_diff::InternedInput::new(old, new);
        let mut diff = imara_diff::Diff::compute(ALGORITHM, &input);
        // Places each hunk where git does, not where the algorithm found it.
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

    /// Two hunks never touch: an unchanged line lies between them.
    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    pub fn line_count(&self, side: Side) -> u32 {
        // `new` panics past what an `i32` counts, so the line count fits a `u32`.
        self.text(side).starts.len() as u32
    }

    /// A version of no lines has no last line to lack one, so it does.
    pub fn ends_with_line_feed(&self, side: Side) -> bool {
        self.text(side)
            .bytes
            .last()
            .is_none_or(|byte| *byte == b'\n')
    }

    /// Without the line feed. Panics when the line does not exist.
    pub fn line(&self, side: Side, line: u32) -> &[u8] {
        let token = self.text(side).token(line);
        token.strip_suffix(b"\n").unwrap_or(token)
    }

    /// `context` is the unchanged lines kept around a change; `u32::MAX` shows the whole file.
    pub fn rows(&self, layout: Layout, context: u32) -> Vec<Row> {
        // Before the first change nothing trails a change, unless all is shown.
        let mut after_a_change = if context == u32::MAX { context } else { 0 };
        let mut rows = Vec::new();
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

    /// A hunk of more than 4,096 words or 65,536 bytes, both sides together, is left unmarked.
    /// Panics when there is no such hunk.
    pub fn emphasis(&self, index: usize) -> Emphasis {
        let hunk = &self.hunks[index];
        let mut emphasis = Emphasis {
            first: (hunk.old.start, hunk.new.start),
            ..Emphasis::default()
        };
        if hunk.old.is_empty() || hunk.new.is_empty() {
            return emphasis;
        }
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

        // First all removed lines against all added ones, to learn which became which; then
        // each edited line against its own: marks from the first pass run into the next line
        // where two lines end alike.
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
            // Takes the later of two equal places and joins marks, as git does for lines: in
            // `round(sum, digits),` the marked words after `sum` are ` digits),`, not `, digits)`.
            diff.postprocess_with(was, is, imara_diff::NoSliderHeuristic);
            let removed = |word| diff.is_removed(word);
            let added = |word| diff.is_added(word);
            emphasis.old[old_line] = words::marks(&old.all[before.clone()], removed);
            emphasis.new[new_line] = words::marks(&new.all[after.clone()], added);
        }
        emphasis
    }
}

/// Pushes the first `head` and last `tail` lines of a run, and a Skipped row for what lies
/// between. `new` is where the run starts in the new version.
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

/// Only a line that became a line of the other side is marked, not lines removed or added whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Emphasis {
    first: (u32, u32),
    old: Vec<Vec<Range<usize>>>,
    new: Vec<Vec<Range<usize>>>,
}

impl Emphasis {
    /// Byte ranges to mark in a line, in order and not touching, from the start of
    /// [`FileDiff::line`]; on character boundaries where the line is UTF-8. Empty for a line
    /// outside the hunk.
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

/// Real changes from `corpus/`: both versions in `pairs.tar.zst`, the hunks of
/// `git diff --histogram` in `hunks.tsv`.
#[cfg(test)]
mod corpus {
    use std::io::Read;

    use ruzstd::decoding::StreamingDecoder;

    use crate::Hunk;

    const ARCHIVE: &[u8] = include_bytes!("../corpus/pairs.tar.zst");
    const GIT_HUNKS: &str = include_str!("../corpus/hunks.tsv");
    const PINS: &str = include_str!("../corpus/pins.tsv");

    pub(crate) struct Pair {
        pub(crate) id: String,
        pub(crate) old: Vec<u8>,
        pub(crate) new: Vec<u8>,
        pub(crate) git_hunks: Vec<Hunk>,
    }

    pub(crate) fn pairs() -> Vec<Pair> {
        let mut archive = Vec::new();
        StreamingDecoder::new(ARCHIVE)
            .expect("the archive is one zstd frame")
            .read_to_end(&mut archive)
            .expect("the archive unpacks");
        let files = files(&archive);

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

    /// Reads the name at the start of each 512-byte header and the size in octal at byte 124.
    /// Content is padded to 512 bytes; a header with no name ends the archive.
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

    fn field(bytes: &[u8]) -> &str {
        let end = bytes
            .iter()
            .position(|byte| matches!(byte, 0 | b' '))
            .unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..end]).expect("a field in ASCII")
    }

    /// Reads `-a,n+b,m` as in git's `@@` line. git counts from 1 and gives an empty side the line
    /// it follows, so a side starts, counted from 0, at `a - 1`, or at `a` when it is empty.
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

    /// Cut independently of the crate, so hunks are checked against the bytes.
    fn lines(bytes: &[u8]) -> Vec<&[u8]> {
        bytes.split_inclusive(|byte| *byte == b'\n').collect()
    }

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

        let mut best = Duration::MAX;
        for _ in 0..7 {
            let start = Instant::now();
            let diff = FileDiff::new(&old, &new);
            best = best.min(start.elapsed());

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

    /// Changes where the diff library places a change otherwise than git, with this crate's hunks.
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

    fn changed(rows: &[Row]) -> Vec<Row> {
        let unchanged = |row: &&Row| matches!(row, Row::Skipped { .. } | Row::Context { .. });
        rows.iter().filter(|row| !unchanged(row)).cloned().collect()
    }

    fn one(range: Range<usize>) -> Vec<Range<usize>> {
        vec![range]
    }

    fn text(diff: &FileDiff, side: Side, line: u32) -> &str {
        std::str::from_utf8(diff.line(side, line)).expect("a line of the test in UTF-8")
    }

    fn marked<'a>(diff: &'a FileDiff, emphasis: &Emphasis, side: Side, line: u32) -> Vec<&'a str> {
        let text = text(diff, side, line);
        emphasis
            .of(side, line)
            .iter()
            .map(|range| &text[range.clone()])
            .collect()
    }

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

        let edited = diff.emphasis(1);
        let at = "    let count = order.".len();
        assert_eq!(edited.of(Side::Old, 1), one(at..at + "count".len()));
        assert_eq!(edited.of(Side::New, 2), one(at..at + "quantity".len()));
        assert_eq!(marked(&diff, &edited, Side::Old, 1), ["count"]);
        assert_eq!(marked(&diff, &edited, Side::New, 2), ["quantity"]);
        assert_eq!(edited.of(Side::Old, 2), []);
        assert_eq!(edited.of(Side::New, 1), []);
        assert_eq!(edited.of(Side::Old, 7), []);
        assert_eq!(edited.of(Side::New, 8), []);

        let rewritten = diff.emphasis(2);
        assert_eq!(rewritten.of(Side::Old, 7), []);
        assert_eq!(marked(&diff, &rewritten, Side::New, 8), [".to_uppercase()"]);
        assert_eq!(rewritten.of(Side::Old, 8), []);
        assert_eq!(marked(&diff, &rewritten, Side::Old, 9), [" ({code})"]);
        assert_eq!(rewritten.of(Side::New, 9), []);
        assert_eq!(rewritten.of(Side::Old, 10), []);
        assert_eq!(rewritten.of(Side::New, 7), []);

        assert_eq!(diff.emphasis(0).of(Side::New, 0), []);
        assert_eq!(diff.emphasis(3).of(Side::New, 14), []);
        assert_eq!(diff.emphasis(4).of(Side::Old, 15), []);

        let diff = FileDiff::new(
            b"    return compute(x);\n",
            b"    return compute(y);\n    log(y);\n",
        );
        assert_eq!(diff.hunks(), [hunk(0..1, 0..2)]);
        let emphasis = diff.emphasis(0);
        assert_eq!(marked(&diff, &emphasis, Side::Old, 0), ["x"]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 0), ["y"]);
        assert_eq!(emphasis.of(Side::New, 1), []);

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

        let emphasis = diff.emphasis(0);
        assert_eq!(emphasis.of(Side::Old, 1), []);
        assert_eq!(emphasis.of(Side::New, 1), []);
    }

    #[test]
    fn a_version_says_whether_its_last_line_ends_with_a_line_feed() {
        let diff = FileDiff::new(b"one\ntwo", b"one\ntwo\n");
        assert!(!diff.ends_with_line_feed(Side::Old));
        assert!(diff.ends_with_line_feed(Side::New));
        let diff = FileDiff::new(b"one\n", b"one\r\ntwo");
        assert!(diff.ends_with_line_feed(Side::Old));
        assert!(!diff.ends_with_line_feed(Side::New));
        let diff = FileDiff::new(b"", b"\n");
        assert!(diff.ends_with_line_feed(Side::Old));
        assert!(diff.ends_with_line_feed(Side::New));
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

    fn numbered(count: u32) -> String {
        (0..count)
            .map(|number| format!("line {number}\n"))
            .collect()
    }

    fn context(old: Range<u32>, shift: u32) -> impl Iterator<Item = Row> {
        old.map(move |old| Row::Context {
            old,
            new: old + shift,
        })
    }

    #[test]
    fn unchanged_lines_farther_from_any_change_than_the_context_are_one_skipped_row() {
        // Line 5 becomes two and line 15 is rewritten: nine unchanged lines lie between, and new
        // numbers run one higher after line 5.
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

        let mut expected = vec![skipped(0..1, 0)];
        expected.extend(context(1..5, 0));
        expected.extend(first.clone());
        expected.extend(context(6..10, 1));
        expected.push(skipped(10..11, 1));
        expected.extend(context(11..15, 1));
        expected.extend(second.clone());
        expected.extend(context(16..20, 1));
        assert_eq!(diff.rows(Layout::Unified, 4), expected);

        let mut expected: Vec<Row> = context(0..5, 0).collect();
        expected.extend(first);
        expected.extend(context(6..15, 1));
        expected.extend(second);
        expected.extend(context(16..20, 1));
        assert_eq!(diff.rows(Layout::Unified, 5), expected);
    }

    #[test]
    fn the_largest_context_shows_the_whole_file() {
        let old = numbered(40);
        let new = old
            .replace("line 0\n", "line zero\n")
            .replace("line 39\n", "line thirty-nine\n");
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        let mut expected = vec![Row::Removed { old: 0 }, Row::Added { new: 0 }];
        expected.extend(context(1..39, 0));
        expected.extend([Row::Removed { old: 39 }, Row::Added { new: 39 }]);
        assert_eq!(diff.rows(Layout::Unified, u32::MAX), expected);

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
        assert_eq!(
            marked(&diff, &emphasis, Side::Old, 1),
            ["height", "first, second"]
        );
        assert_eq!(emphasis.of(Side::Old, 1), [4..10, 18..31]);
        assert_eq!(marked(&diff, &emphasis, Side::New, 1), ["depth", "other"]);
    }

    #[test]
    fn a_mark_that_could_sit_in_two_places_sits_in_the_later_one() {
        // Either comma could be the old one.
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
        // `marked` slices the line, which panics inside a character.
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
        let line = |letter: &str, count: usize, end: &str| {
            format!("key{}{end}\n", format!(" {letter}").repeat(count))
        };
        // 2,046 and 2,050 words, line feeds included: 4,096 together, the most compared.
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
        let line = |letter: &str, count: usize, end: &str| {
            format!("key = {}{end}\n", letter.repeat(count))
        };
        // 30,007 and 35,529 bytes: 65,536 together, the most compared. Only twelve words, so bytes
        // decide.
        let (old, new) = (line("a", 30_000, ""), line("b", 35_522, ""));
        assert_eq!(old.len() + new.len(), 65_536);
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        assert_eq!(diff.hunks(), [hunk(0..1, 0..1)]);
        let emphasis = diff.emphasis(0);
        assert_eq!(emphasis.of(Side::Old, 0), one(6..30_006));
        assert_eq!(emphasis.of(Side::New, 0), one(6..35_528));

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

        let lines = |letter: &str, count: usize, end: &str, which: usize| {
            (0..4)
                .map(|line| {
                    let end = if line == which { end } else { "" };
                    format!("key_{line} = {}{end}\n", letter.repeat(count))
                })
                .collect::<String>()
        };
        let (old, new) = (lines("a", 8_000, "", 4), lines("b", 8_366, "", 4));
        assert_eq!(old.len() + new.len(), 65_536);
        let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
        assert_eq!(diff.hunks(), [hunk(0..4, 0..4)]);
        let emphasis = diff.emphasis(0);
        for line in 0..4 {
            assert_eq!(emphasis.of(Side::Old, line), one(8..8_008), "line {line}");
            assert_eq!(emphasis.of(Side::New, line), one(8..8_374), "line {line}");
        }

        for line in [1, 2] {
            for (old, new) in [
                (lines("a", 8_000, ";", line), new.clone()),
                (old.clone(), lines("b", 8_366, ";", line)),
            ] {
                let diff = FileDiff::new(old.as_bytes(), new.as_bytes());
                assert_eq!(diff.hunks(), [hunk(0..4, 0..4)]);
                let emphasis = diff.emphasis(0);
                for mark in 0..4 {
                    assert_eq!(emphasis.of(Side::Old, mark), [], "line {mark}");
                    assert_eq!(emphasis.of(Side::New, mark), [], "line {mark}");
                }
            }
        }
    }

    fn rewritten_lines(count: usize) -> (String, String) {
        (
            "x = x + 1;\n".repeat(count),
            "y = (y, 2) - y;\n".repeat(count),
        )
    }

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
            (few_old, few_new, true),
            (long_line(1000, 3), long_line(1000, 5), true),
            // 65,402 bytes, mostly one line of 64,001 words: few enough bytes to be read, about as
            // many words as are cut before a hunk is left alone.
            (long_line(32_000, 3), long_line(700, 5), false),
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
        // 4 MiB is the largest file version the app shows, so it bounds the longest line of a hunk.
        let size = 4 << 20;
        let statement = "var a1=fn(b2,c3);";
        let minified = |name: &str| {
            format!(
                "var {name}=fn(b2,c3);{}\n",
                statement.repeat(size / statement.len())
            )
        };
        let hunks = [
            (minified("a0"), minified("a9")),
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
            assert!(
                best < Duration::from_millis(1),
                "the best of 5 runs took {best:?} for a line starting {:?}",
                &old[..24]
            );
        }
    }

    #[test]
    fn a_diff_does_not_pay_for_the_marks_of_its_hunks() {
        let (hunk_old, hunk_new) = rewritten_lines(140);
        let (mut old, mut new) = (String::new(), String::new());
        for part in 0..10 {
            let between = format!("// part {part}\n");
            old.extend([hunk_old.as_str(), between.as_str()]);
            new.extend([hunk_new.as_str(), between.as_str()]);
        }

        // Both costs are measured in one run, so no limit in ms: a diff that marked eagerly would
        // take at least as long as the marking.
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

        // The totals pin which words are marked on the corpus, so any change to marking fails here.
        // Two hunks of `p0135` hold too many words to be compared.
        assert_eq!((marked_lines, marked_bytes), (10_018, 163_656));
    }
}
