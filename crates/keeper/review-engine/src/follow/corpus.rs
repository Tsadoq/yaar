//! Row THREAD-08: the recorded corpus of real file changes, each followed in one round and
//! in three. The pairs of `crates/shared/diff/corpus/pairs.tar.zst` and the truth of
//! `corpus/anchors.tsv` of this crate are compiled into the test program, so that it opens
//! no file and runs no git.
//!
//! The truth is `git blame --reverse -M` between the first version of a file and the last,
//! repaired and corrected by hand: it knows the two ends and nothing between them. Three
//! rounds therefore cut the change into three steps, and the thread is carried through
//! each in turn with [`carry_file`], where one round carries it through the whole change
//! at once. The `n` hunks of the diff of the two versions are cut at `ceil(n / 3)` and
//! `ceil(2n / 3)`; a change of one or two hunks has empty steps, which change nothing.
//!
//! The follower is a rule, not an oracle, and this test names where it differs from the
//! truth instead of hiding it: [`ONE_ROUND`] and [`THREE_ROUNDS`] hold each anchor that
//! differs, with what the follower gives. The test fails on a difference that is not
//! listed and on a listed one that went away, so a change of the rule shows at once.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;

use ruzstd::decoding::StreamingDecoder;
use yaar_diff::{FileDiff, Hunk, Side as YaarSide};
use yaar_store::Side;

use super::{FileStep, Notice, Place, Placed, carry_file};

const ARCHIVE: &[u8] = include_bytes!("../../../../shared/diff/corpus/pairs.tar.zst");
const ANCHORS: &str = include_str!("../../corpus/anchors.tsv");

/// The most anchors one round may place otherwise than the truth does: the rule's own
/// count, 6 wrong places, 9 notices though the code survived and 2 not noticed though the
/// code was gone.
const MOST_DIFFERING_IN_ONE_ROUND: usize = 17;

/// What became of the lines of an anchor, as the truth says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    /// Every line is there, in one block.
    Intact,
    /// Some of the lines are there.
    Partial,
    /// None is.
    Gone,
}

/// One anchor of the corpus with the truth about it, in the lines of its pair's two
/// versions, counted from 1.
#[derive(Debug, Clone)]
struct Anchor {
    pair: String,
    id: String,
    first: u32,
    last: u32,
    truth: Truth,
    first_after: u32,
    last_after: u32,
}

/// What a follower gave for an anchor it did not place as the truth says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Differs {
    pair: &'static str,
    anchor: &'static str,
    first: u32,
    last: u32,
    notice: Option<Notice>,
}

const fn differs(
    pair: &'static str,
    anchor: &'static str,
    first: u32,
    last: u32,
    notice: Option<Notice>,
) -> Differs {
    Differs {
        pair,
        anchor,
        first,
        last,
        notice,
    }
}

/// The anchors that one round places otherwise than the truth does, with what the follower
/// gives: the lines it puts the thread on and its notice. Seventeen, the rule's own count
/// in the experiment that chose it: six intact blocks placed on other lines with no notice,
/// nine intact blocks given a notice, and two changed or deleted lines given none.
const ONE_ROUND: &[Differs] = &[
    // Intact, placed on other lines.
    differs("p0017", "a5", 122, 122, None),
    differs("p0035", "a6", 206, 206, None),
    differs("p0087", "a9", 621, 621, None),
    differs("p0420", "a6", 29, 29, None),
    differs("p0435", "a4", 103, 103, None),
    differs("p0523", "a4", 565, 566, None),
    // Intact, given a notice.
    differs("p0129", "a3", 413, 413, Some(Notice::Changed)),
    differs("p0257", "a8", 65, 65, Some(Notice::Deleted)),
    differs("p0282", "a0", 16, 17, Some(Notice::Changed)),
    differs("p0296", "a4", 3, 3, Some(Notice::Changed)),
    differs("p0310", "a4", 1223, 1235, Some(Notice::Changed)),
    differs("p0359", "a9", 230, 230, Some(Notice::Changed)),
    differs("p0442", "a8", 475, 475, Some(Notice::Deleted)),
    differs("p0532", "a4", 211, 221, Some(Notice::Changed)),
    differs("p0532", "a9", 211, 221, Some(Notice::Changed)),
    // Changed or deleted, given no notice.
    differs("p0400", "a6", 2693, 2694, None),
    differs("p0530", "a3", 148, 148, None),
];

/// The anchors that three rounds place otherwise than the truth does: the seventeen of
/// [`ONE_ROUND`], placed as one round places them, and eleven more. Each of the eleven is
/// an intact block that moved: one round follows it, and three rounds lose it where a round
/// cut the move in two.
const THREE_ROUNDS: &[Differs] = &[
    // The seventeen of one round.
    differs("p0017", "a5", 122, 122, None),
    differs("p0035", "a6", 206, 206, None),
    differs("p0087", "a9", 621, 621, None),
    differs("p0420", "a6", 29, 29, None),
    differs("p0435", "a4", 103, 103, None),
    differs("p0523", "a4", 565, 566, None),
    differs("p0129", "a3", 413, 413, Some(Notice::Changed)),
    differs("p0257", "a8", 65, 65, Some(Notice::Deleted)),
    differs("p0282", "a0", 16, 17, Some(Notice::Changed)),
    differs("p0296", "a4", 3, 3, Some(Notice::Changed)),
    differs("p0310", "a4", 1223, 1235, Some(Notice::Changed)),
    differs("p0359", "a9", 230, 230, Some(Notice::Changed)),
    differs("p0442", "a8", 475, 475, Some(Notice::Deleted)),
    differs("p0532", "a4", 211, 221, Some(Notice::Changed)),
    differs("p0532", "a9", 211, 221, Some(Notice::Changed)),
    differs("p0400", "a6", 2693, 2694, None),
    differs("p0530", "a3", 148, 148, None),
    // Intact and moved, given a notice by three rounds only.
    differs("p0040", "a7", 59, 59, Some(Notice::Changed)),
    differs("p0089", "a7", 8, 8, Some(Notice::Changed)),
    differs("p0257", "a4", 209, 209, Some(Notice::Changed)),
    differs("p0262", "a5", 684, 684, Some(Notice::Deleted)),
    differs("p0262", "a7", 684, 684, Some(Notice::Deleted)),
    differs("p0262", "a9", 684, 684, Some(Notice::Deleted)),
    differs("p0359", "a5", 230, 231, Some(Notice::Changed)),
    differs("p0379", "a5", 144, 155, Some(Notice::Changed)),
    differs("p0379", "a6", 144, 155, Some(Notice::Changed)),
    differs("p0428", "a6", 71, 72, Some(Notice::Changed)),
    differs("p0431", "a6", 279, 279, Some(Notice::Deleted)),
];

/// The files of a tar archive, by name. Of each header of 512 bytes only two fields are
/// read: the name, at the start, and the size, as octal digits at byte 124. The content
/// follows its header and is filled up to the next 512 bytes; a header with no name ends
/// the archive.
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

/// Every change of the corpus: its name, the file before and the file after, in the
/// order of the archive.
fn pairs(archive: &[u8]) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    files(archive)
        .chunks(2)
        .map(|versions| {
            let [(old_name, old), (new_name, new)] = versions else {
                panic!("the archive ends on half a change");
            };
            let id = old_name.split('/').next().expect("a folder");
            assert!(old_name.starts_with(&format!("{id}/a.")), "{old_name}");
            assert!(new_name.starts_with(&format!("{id}/b.")), "{new_name}");
            (id.to_owned(), old.to_vec(), new.to_vec())
        })
        .collect()
}

/// The anchors of `anchors.tsv` the truth gives a verdict for, by pair: `SPLIT` and
/// `EXCLUDE` are left out.
fn anchors() -> BTreeMap<String, Vec<Anchor>> {
    let mut by_pair: BTreeMap<String, Vec<Anchor>> = BTreeMap::new();
    for row in ANCHORS.lines().skip(1) {
        let cells: Vec<&str> = row.split('\t').collect();
        let [pair, id, first, last, truth, first_after, last_after] = cells[..] else {
            panic!("seven cells in {row}");
        };
        let truth = match truth {
            "INTACT" => Truth::Intact,
            "PARTIAL" => Truth::Partial,
            "GONE" => Truth::Gone,
            "SPLIT" | "EXCLUDE" => continue,
            other => panic!("the verdict {other}"),
        };
        let number = |cell: &str| cell.parse::<u32>().expect("a line number");
        by_pair.entry(pair.to_owned()).or_default().push(Anchor {
            pair: pair.to_owned(),
            id: id.to_owned(),
            first: number(first),
            last: number(last),
            truth,
            first_after: number(first_after),
            last_after: number(last_after),
        });
    }
    by_pair
}

/// The lines of a version as git counts them: each with the line feed that ends it.
fn lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|byte| *byte == b'\n').collect()
}

/// `old` with `hunks`, the first hunks of its diff to `new`, applied: the removed lines
/// left out and the lines the hunk names in `new` put in their place.
fn applied(old: &[u8], new: &[u8], hunks: &[Hunk]) -> Vec<u8> {
    let (old, new) = (lines(old), lines(new));
    let mut out: Vec<&[u8]> = Vec::new();
    let mut kept = 0;
    for hunk in hunks {
        out.extend(&old[kept..hunk.old.start as usize]);
        assert_eq!(
            out.len(),
            hunk.new.start as usize,
            "{hunk:?} is not in place"
        );
        out.extend(&new[hunk.new.start as usize..hunk.new.end as usize]);
        kept = hunk.old.end as usize;
    }
    out.extend(&old[kept..]);
    out.concat()
}

/// The first version, the two between and the last, cut by hunks as the module's doc says.
fn versions(first: &[u8], last: &[u8]) -> [Vec<u8>; 4] {
    let diff = FileDiff::new(first, last);
    let hunks = diff.hunks();
    let n = hunks.len();
    let cuts = [0, n.div_ceil(3), (2 * n).div_ceil(3), n];
    let version = |k: usize| applied(first, last, &hunks[..cuts[k]]);
    let versions = [version(0), version(1), version(2), version(3)];
    assert_eq!(versions[0], first, "no hunk applied is the first version");
    assert!(
        versions[3] == last,
        "all the hunks applied are not the last version"
    );
    versions
}

/// How a file changes from one version to the next, as the engine would tell it: a file
/// that did not change is not listed in a tree's changes.
fn step<'a>(before: &'a [u8], after: &'a [u8]) -> FileStep<'a> {
    if before == after {
        FileStep::Unchanged
    } else {
        FileStep::Changed { before, after }
    }
}

/// The anchors of one pair as threads on the new side of the file in its first version.
fn threads(anchors: &[Anchor]) -> Vec<Placed> {
    anchors
        .iter()
        .map(|anchor| Placed {
            at: Place {
                path: PathBuf::from("file"),
                side: Side::New,
                first: anchor.first,
                last: anchor.last,
            },
            notice: None,
        })
        .collect()
}

/// Whether a follower placed the anchor as the truth says in the last version: a block
/// that is intact on the lines the truth names with no notice, code that changed or was
/// deleted with a notice.
fn agrees(anchor: &Anchor, got: &Placed) -> bool {
    assert_eq!(got.at.path, PathBuf::from("file"));
    assert_eq!(got.at.side, Side::New);
    match anchor.truth {
        Truth::Intact => {
            got.notice.is_none()
                && (got.at.first, got.at.last) == (anchor.first_after, anchor.last_after)
        }
        Truth::Partial | Truth::Gone => got.notice.is_some(),
    }
}

/// What the follower gave for each anchor it did not place as the truth says.
fn disagreements(anchors: &[Anchor], got: &[Placed]) -> Vec<Differs> {
    anchors
        .iter()
        .zip(got)
        .filter(|(anchor, got)| !agrees(anchor, got))
        .map(|(anchor, got)| Differs {
            pair: leak(&anchor.pair),
            anchor: leak(&anchor.id),
            first: got.at.first,
            last: got.at.last,
            notice: got.notice,
        })
        .collect()
}

/// A name that lives as long as the program, for a [`Differs`] built at run time.
fn leak(name: &str) -> &'static str {
    Box::leak(name.to_owned().into_boxed_str())
}

/// The lines the diff of the two versions adds to the new one, counted from 1.
fn added_lines(first: &[u8], last: &[u8]) -> BTreeSet<u32> {
    FileDiff::new(first, last)
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.new.clone().map(|line| line + 1))
        .collect()
}

/// What `carry_file` gives for every anchor of every pair, in one round and in three.
struct Followed {
    one: Vec<Differs>,
    three: Vec<Differs>,
    /// How many anchors were placed.
    placed: usize,
    /// For each anchor that three rounds place otherwise than the truth does and one round
    /// as the truth does, whether its truth lines are all lines the one-step diff adds.
    moved: Vec<(Differs, bool)>,
}

fn follow_the_corpus() -> Followed {
    let mut archive = Vec::new();
    StreamingDecoder::new(ARCHIVE)
        .expect("the archive is one zstd frame")
        .read_to_end(&mut archive)
        .expect("the archive unpacks");
    let by_pair = anchors();
    let mut followed = Followed {
        one: Vec::new(),
        three: Vec::new(),
        placed: 0,
        moved: Vec::new(),
    };
    let mut pairs_with_anchors = 0;
    for (name, first, last) in pairs(&archive) {
        let Some(anchors) = by_pair.get(&name) else {
            continue;
        };
        pairs_with_anchors += 1;
        let start = threads(anchors);
        followed.placed += anchors.len();

        let one = carry_file(&start, Side::New, step(&first, &last));
        let [v0, v1, v2, v3] = versions(&first, &last);
        let mut three = start;
        for (before, after) in [(&v0, &v1), (&v1, &v2), (&v2, &v3)] {
            three = carry_file(&three, Side::New, step(before, after));
        }

        let in_one = disagreements(anchors, &one);
        let in_three = disagreements(anchors, &three);
        let added = added_lines(&first, &last);
        for differing in &in_three {
            if in_one
                .iter()
                .any(|d| (d.pair, d.anchor) == (differing.pair, differing.anchor))
            {
                continue;
            }
            let anchor = anchors
                .iter()
                .find(|a| a.pair == differing.pair && a.id == differing.anchor)
                .expect("an anchor of the pair");
            let truth_lines_are_added = anchor.first_after >= 1
                && (anchor.first_after..=anchor.last_after).all(|line| added.contains(&line));
            followed.moved.push((*differing, truth_lines_are_added));
        }
        followed.one.extend(in_one);
        followed.three.extend(in_three);
    }
    assert_eq!(
        pairs_with_anchors, 569,
        "every change of the corpus has anchors"
    );
    followed
}

/// A list of the constants above in the order the corpus is followed in: by pair, then by
/// anchor, though the constants are grouped by what went wrong.
fn by_name(list: &[Differs]) -> Vec<Differs> {
    let mut sorted = list.to_vec();
    sorted.sort_by_key(|d| (d.pair, d.anchor));
    sorted
}

/// A list of anchors as the test prints it, to be put in the constants above.
fn written(list: &[Differs]) -> String {
    list.iter()
        .map(|d| {
            format!(
                "    differs({:?}, {:?}, {}, {}, {:?}),\n",
                d.pair, d.anchor, d.first, d.last, d.notice
            )
        })
        .collect()
}

// THREAD-08
#[test]
fn over_the_corpus_of_real_changes_each_followed_in_one_round_and_in_three_the_anchors_that_differ_from_the_truth_are_named()
 {
    let followed = follow_the_corpus();
    assert_eq!(followed.placed, 5536, "the anchors with a verdict");
    println!(
        "THREAD-08: of {} anchors one round differs on {}, three rounds on {}",
        followed.placed,
        followed.one.len(),
        followed.three.len()
    );

    // Where the rule differs from the truth is what the lists say, no more and no less.
    assert!(
        followed.one == by_name(ONE_ROUND) && followed.three == by_name(THREE_ROUNDS),
        "one round differs on:\n{}three rounds differ on:\n{}",
        written(&followed.one),
        written(&followed.three)
    );

    // The ceilings: the rule misplaces few anchors in one round, and three rounds go wrong
    // further only on code that moved, which a round may cut in two.
    assert!(
        followed.one.len() <= MOST_DIFFERING_IN_ONE_ROUND,
        "one round differs on {} anchors, past {MOST_DIFFERING_IN_ONE_ROUND}",
        followed.one.len()
    );
    for (differing, truth_lines_are_added) in &followed.moved {
        assert!(
            truth_lines_are_added,
            "{differing:?} is placed otherwise by three rounds only, and its lines are not code the diff adds"
        );
    }
}
