//! How a thread follows its code from one tree to the next, over `yaar-diff`'s diff of one file. No
//! hunk touches its lines: it shifts with them. A hunk touches them and the same lines occur
//! exactly once among the added lines, with at least [`MOVED_BLOCK`] letters and digits: it follows
//! that block. Otherwise it sits on the lines that replaced them ([`Notice::Changed`]) or on the
//! one line at the point of deletion ([`Notice::Deleted`]). A notice stays in later trees and
//! `Deleted` outlasts `Changed`. A rename git pairs is the same file; an unpaired deleted file
//! keeps the thread in place with `Deleted`.

#[cfg(test)]
mod corpus;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yaar_diff::{FileDiff, Side as YaarSide};
use yaar_store::Side;

/// The fewest letters and digits a moved block must hold to be followed with no notice, as git's
/// blame counts a moved line.
pub const MOVED_BLOCK: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    pub side: Side,
    pub first: u32,
    pub last: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    Changed,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub at: Place,
    pub notice: Option<Notice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStep<'a> {
    Unchanged,
    Changed {
        before: &'a [u8],
        after: &'a [u8],
    },
    Renamed {
        to: &'a Path,
        before: &'a [u8],
        after: &'a [u8],
    },
    Deleted,
}

pub fn carry(placed: &Placed, changed: Side, step: FileStep<'_>) -> Placed {
    carry_file(std::slice::from_ref(placed), changed, step)
        .pop()
        .expect("one thread carried gives one thread")
}

pub fn carry_file(placed: &[Placed], changed: Side, step: FileStep<'_>) -> Vec<Placed> {
    let affected = |p: &Placed| p.at.side == changed;
    match step {
        FileStep::Unchanged => placed.to_vec(),
        FileStep::Deleted => placed
            .iter()
            .map(|p| {
                if affected(p) {
                    Placed {
                        at: p.at.clone(),
                        notice: Some(stronger(p.notice, Notice::Deleted)),
                    }
                } else {
                    p.clone()
                }
            })
            .collect(),
        FileStep::Changed { before, after } => follow_content(placed, changed, None, before, after),
        FileStep::Renamed { to, before, after } => {
            follow_content(placed, changed, Some(to), before, after)
        }
    }
}

/// A deletion is not taken back by a later change of the line it sits on.
fn stronger(had: Option<Notice>, given: Notice) -> Notice {
    match (had, given) {
        (Some(Notice::Deleted), _) | (_, Notice::Deleted) => Notice::Deleted,
        _ => Notice::Changed,
    }
}

fn follow_content(
    placed: &[Placed],
    changed: Side,
    to: Option<&Path>,
    before: &[u8],
    after: &[u8],
) -> Vec<Placed> {
    if placed.iter().all(|p| p.at.side != changed) {
        return placed.to_vec();
    }
    let diff = FileDiff::new(before, after);
    let moves = Moves::new(&diff);
    placed
        .iter()
        .map(|p| {
            if p.at.side != changed {
                return p.clone();
            }
            let (first, last, notice) = place(&diff, &moves, p.at.first, p.at.last);
            Placed {
                at: Place {
                    path: to.map_or_else(|| p.at.path.clone(), Path::to_path_buf),
                    side: p.at.side,
                    first,
                    last,
                },
                notice: match notice {
                    Some(given) => Some(stronger(p.notice, given)),
                    None => p.notice,
                },
            }
        })
        .collect()
}

/// The lines the diff adds, by content, worked out when a thread first needs it.
struct Moves<'a> {
    diff: &'a FileDiff,
    index: OnceCell<AddedLines<'a>>,
}

struct AddedLines<'a> {
    added: Vec<bool>,
    by_text: HashMap<&'a [u8], Vec<u32>>,
}

impl<'a> Moves<'a> {
    fn new(diff: &'a FileDiff) -> Self {
        Self {
            diff,
            index: OnceCell::new(),
        }
    }

    fn added_lines(&self) -> &AddedLines<'a> {
        self.index.get_or_init(|| {
            let diff = self.diff;
            let mut added = vec![false; diff.line_count(YaarSide::New) as usize];
            let mut by_text: HashMap<&[u8], Vec<u32>> = HashMap::new();
            for hunk in diff.hunks() {
                for line in hunk.new.clone() {
                    added[line as usize] = true;
                    by_text
                        .entry(diff.line(YaarSide::New, line))
                        .or_default()
                        .push(line);
                }
            }
            AddedLines { added, by_text }
        })
    }

    /// Where the old lines `first..=last` (counted from 0) are in the new version, if they occur
    /// there exactly once, as consecutive added lines.
    fn once_added(&self, first: u32, last: u32) -> Option<u32> {
        let wanted = |at: u32| self.diff.line(YaarSide::Old, first + at);
        let count = last - first + 1;
        let lines = self.added_lines();
        let mut found = None;
        for &start in lines.by_text.get(wanted(0))? {
            let fits = start + count <= self.diff.line_count(YaarSide::New)
                && (0..count).all(|at| {
                    lines.added[(start + at) as usize]
                        && self.diff.line(YaarSide::New, start + at) == wanted(at)
                });
            if fits {
                if found.is_some() {
                    return None;
                }
                found = Some(start);
            }
        }
        found
    }
}

fn letters_and_digits(diff: &FileDiff, first: u32, last: u32) -> usize {
    (first..=last)
        .map(|line| {
            String::from_utf8_lossy(diff.line(YaarSide::Old, line))
                .chars()
                .filter(|c| c.is_alphanumeric())
                .count()
        })
        .sum()
}

/// `first..=last` are numbered from 1 as a [`Place`]; the arithmetic is in `yaar-diff`'s numbers:
/// lines from 0, a hunk's range ending after its last line.
fn place(diff: &FileDiff, moves: &Moves<'_>, first: u32, last: u32) -> (u32, u32, Option<Notice>) {
    let old_count = diff.line_count(YaarSide::Old);
    let new_count = diff.line_count(YaarSide::New);
    // A thread past the end of its file is kept on the last line the file has.
    let s = first.saturating_sub(1).min(old_count.saturating_sub(1));
    let e = last.saturating_sub(1).clamp(s, old_count.saturating_sub(1));
    let hunks = diff.hunks();

    let shifted = |line: u32| -> i64 {
        let moved: i64 = hunks
            .iter()
            .filter(|h| h.old.end <= line)
            .map(|h| i64::from(h.new.end - h.new.start) - i64::from(h.old.end - h.old.start))
            .sum();
        i64::from(line) + moved
    };
    let removing = |line: u32| {
        hunks
            .iter()
            .find(|h| h.old.start <= line && line < h.old.end)
    };

    let touched = hunks.iter().any(|h| {
        if h.old.is_empty() {
            // An insertion is inside the range only between two of its lines; one just above or
            // below is not.
            s < h.old.start && h.old.start <= e
        } else {
            h.old.start <= e && h.old.end > s
        }
    });
    if !touched {
        return (
            to_line(shifted(s), new_count),
            to_line(shifted(e), new_count),
            None,
        );
    }

    if letters_and_digits(diff, s, e) >= MOVED_BLOCK
        && let Some(start) = moves.once_added(s, e)
    {
        return (start + 1, start + e - s + 1, None);
    }

    let deleted = hunks
        .iter()
        .any(|h| h.new.is_empty() && h.old.start <= s && e < h.old.end);
    if deleted {
        let at = removing(s).expect("a removed line is in a hunk").new.start;
        let at = at.min(new_count.saturating_sub(1));
        return (at + 1, at + 1, Some(Notice::Deleted));
    }
    let lo = match removing(s) {
        Some(h) => i64::from(h.new.start),
        None => shifted(s),
    };
    let hi = match removing(e) {
        Some(h) if !h.new.is_empty() => i64::from(h.new.end) - 1,
        Some(h) => i64::from(h.new.start) - 1,
        None => shifted(e),
    };
    let (lo, hi) = (lo, hi.max(lo));
    (
        to_line(lo, new_count),
        to_line(hi, new_count),
        Some(Notice::Changed),
    )
}

/// The line number, counted from 1, of line `line` counted from 0, kept inside a version of `count`
/// lines (or on line 1 of one that has none).
fn to_line(line: i64, count: u32) -> u32 {
    let last = i64::from(count.saturating_sub(1));
    (line.clamp(0, last) + 1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: &str = "let total_value = compute(first, second);";

    fn bytes(lines: &[&str]) -> Vec<u8> {
        lines
            .iter()
            .flat_map(|l| format!("{l}\n").into_bytes())
            .collect()
    }

    fn numbered(n: u32) -> Vec<String> {
        (1..=n).map(|i| format!("line {i}")).collect()
    }

    fn text(lines: &[String]) -> Vec<u8> {
        bytes(&lines.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn on(path: &str, side: Side, first: u32, last: u32) -> Placed {
        Placed {
            at: Place {
                path: PathBuf::from(path),
                side,
                first,
                last,
            },
            notice: None,
        }
    }

    fn new_side(first: u32, last: u32) -> Placed {
        on("a.rs", Side::New, first, last)
    }

    fn changed<'a>(before: &'a [u8], after: &'a [u8]) -> FileStep<'a> {
        FileStep::Changed { before, after }
    }

    fn carried(placed: &Placed, before: &[u8], after: &[u8]) -> (u32, u32, Option<Notice>) {
        let got = carry(placed, Side::New, changed(before, after));
        assert_eq!(got.at.path, placed.at.path);
        assert_eq!(got.at.side, placed.at.side);
        (got.at.first, got.at.last, got.notice)
    }

    #[test]
    fn unchanged_lines_below_an_insertion_shift_with_no_notice() {
        let old = text(&numbered(10));
        let mut lines = vec![
            "new 1".to_string(),
            "new 2".to_string(),
            "new 3".to_string(),
        ];
        lines.extend(numbered(10));
        let new = text(&lines);

        assert_eq!(carried(&new_side(6, 7), &old, &new), (9, 10, None));
    }

    #[test]
    fn a_block_of_twenty_letters_and_digits_moved_within_the_file_is_followed_with_no_notice() {
        let mut old = numbered(20);
        old[2] = LONG.to_string();
        let mut new = numbered(20);
        new.remove(2);
        new.insert(14, LONG.to_string());
        assert_eq!(new[13], "line 15");
        assert_eq!(new[14], LONG);

        assert_eq!(
            carried(&new_side(3, 3), &text(&old), &text(&new)),
            (15, 15, None)
        );
    }

    #[test]
    fn a_moved_line_of_fewer_than_twenty_letters_and_digits_gets_a_notice() {
        let mut old = numbered(20);
        old[2] = "});".to_string();
        let mut new = numbered(20);
        new.remove(2);
        new.insert(14, "});".to_string());

        let (first, last, notice) = carried(&new_side(3, 3), &text(&old), &text(&new));
        assert!(notice.is_some(), "placed at {first}..{last} with no notice");
    }

    #[test]
    fn nineteen_letters_and_digits_are_too_few_and_twenty_are_enough() {
        let moves = |line: &str| {
            let mut old = numbered(20);
            old[2] = line.to_string();
            let mut new = numbered(20);
            new.remove(2);
            new.insert(14, line.to_string());
            carried(&new_side(3, 3), &text(&old), &text(&new))
        };

        // The x counts: 19 letters are too few, 20 enough. Spaces and punctuation do not count.
        assert_eq!(moves("x = (aaaaaaaaaaaaaaaaaa) ;").2, Some(Notice::Deleted));
        assert_eq!(moves("x = (aaaaaaaaaaaaaaaaaaa) ;"), (15, 15, None));
    }

    #[test]
    fn letters_count_as_the_unicode_rule_counts_them_not_as_ascii() {
        let line = "// αβγδεζηθικλμνξοπρστυ";
        let mut old = numbered(20);
        old[2] = line.to_string();
        let mut new = numbered(20);
        new.remove(2);
        new.insert(14, line.to_string());

        assert_eq!(
            carried(&new_side(3, 3), &text(&old), &text(&new)),
            (15, 15, None)
        );
    }

    #[test]
    fn a_moved_range_of_several_short_lines_counts_their_letters_together() {
        let mut old = numbered(20);
        old[2] = "first_value = 1;".to_string();
        old[3] = "second_value = 2;".to_string();
        let mut new = numbered(20);
        new.drain(2..4);
        new.insert(13, "first_value = 1;".to_string());
        new.insert(14, "second_value = 2;".to_string());

        assert_eq!(
            carried(&new_side(3, 4), &text(&old), &text(&new)),
            (14, 15, None)
        );
    }

    #[test]
    fn a_block_added_twice_is_not_followed() {
        let mut old = numbered(20);
        old[2] = LONG.to_string();
        let mut new = numbered(20);
        new.remove(2);
        new.insert(10, LONG.to_string());
        new.insert(16, LONG.to_string());

        let (_, _, notice) = carried(&new_side(3, 3), &text(&old), &text(&new));
        assert_eq!(notice, Some(Notice::Deleted));
    }

    #[test]
    fn a_line_that_is_still_where_it_was_stays_though_a_copy_of_it_was_added() {
        let mut old = numbered(20);
        old[2] = LONG.to_string();
        let mut new = old.clone();
        new.insert(14, LONG.to_string());

        assert_eq!(
            carried(&new_side(3, 3), &text(&old), &text(&new)),
            (3, 3, None)
        );
    }

    #[test]
    fn a_rewritten_line_sits_on_the_lines_that_replaced_it_and_says_the_code_changed() {
        let old = text(&numbered(10));
        let mut lines = numbered(10);
        lines.splice(
            4..5,
            ["first replacement", "second replacement"].map(str::to_string),
        );
        let new = text(&lines);

        assert_eq!(
            carried(&new_side(5, 5), &old, &new),
            (5, 6, Some(Notice::Changed))
        );
    }

    #[test]
    fn a_range_rewritten_in_part_covers_what_is_left_and_what_replaced_the_rest() {
        let old = text(&numbered(10));
        let mut lines = numbered(10);
        lines[4] = "replaced".to_string();
        let new = text(&lines);

        assert_eq!(
            carried(&new_side(3, 5), &old, &new),
            (3, 5, Some(Notice::Changed))
        );
    }

    #[test]
    fn deleted_lines_leave_the_thread_on_the_line_at_the_point_of_deletion() {
        let old = text(&numbered(8));
        let mut lines = numbered(8);
        lines.drain(3..5);
        let new = text(&lines);
        assert_eq!(lines[3], "line 6");

        assert_eq!(
            carried(&new_side(4, 5), &old, &new),
            (4, 4, Some(Notice::Deleted))
        );
    }

    #[test]
    fn deleted_lines_at_the_end_of_the_file_leave_the_thread_on_its_last_line() {
        let old = text(&numbered(5));
        let new = text(&numbered(3));

        assert_eq!(
            carried(&new_side(4, 5), &old, &new),
            (3, 3, Some(Notice::Deleted))
        );
    }

    #[test]
    fn a_range_deleted_in_part_stays_on_what_is_left_with_the_notice_that_it_changed() {
        let old = text(&numbered(8));
        let mut lines = numbered(8);
        lines.drain(4..6);
        let new = text(&lines);

        assert_eq!(
            carried(&new_side(4, 6), &old, &new),
            (4, 4, Some(Notice::Changed))
        );
    }

    #[test]
    fn a_line_inserted_inside_a_commented_range_keeps_the_range_whole_with_a_notice() {
        let old = text(&numbered(8));
        let mut lines = numbered(8);
        lines.insert(4, "inserted".to_string());
        let new = text(&lines);

        assert_eq!(
            carried(&new_side(3, 6), &old, &new),
            (3, 7, Some(Notice::Changed))
        );
    }

    #[test]
    fn a_range_grows_by_each_line_put_inside_it() {
        let old = text(&numbered(10));
        let mut lines = numbered(10);
        lines.insert(3, "inserted one".to_string());
        lines.insert(6, "inserted two".to_string());
        lines.insert(7, "inserted three".to_string());
        let new = text(&lines);

        assert_eq!(
            carried(&new_side(3, 7), &old, &new),
            (3, 10, Some(Notice::Changed))
        );
    }

    #[test]
    fn a_line_put_between_the_first_two_or_the_last_two_lines_of_a_range_is_inside_it() {
        let old = text(&numbered(8));
        for after in [3, 5] {
            let mut lines = numbered(8);
            lines.insert(after, "inserted".to_string());
            let new = text(&lines);

            assert_eq!(
                carried(&new_side(3, 6), &old, &new),
                (3, 7, Some(Notice::Changed)),
                "inserted after line {after}"
            );
        }
    }

    #[test]
    fn lines_put_just_above_or_just_below_a_range_are_not_inside_it() {
        let old = text(&numbered(8));
        let mut lines = numbered(8);
        lines.insert(6, "below".to_string());
        lines.insert(2, "above".to_string());
        let new = text(&lines);

        assert_eq!(carried(&new_side(3, 6), &old, &new), (4, 7, None));
    }

    #[test]
    fn a_file_renamed_with_small_edits_takes_its_threads_to_the_new_path() {
        let old = text(&numbered(12));
        let mut lines = numbered(12);
        lines[10] = "edited far below".to_string();
        lines.insert(2, "inserted above".to_string());
        let new = text(&lines);
        let placed = on("src/old name.rs", Side::New, 5, 6);

        let got = carry(
            &placed,
            Side::New,
            FileStep::Renamed {
                to: Path::new("src/new name.rs"),
                before: &old,
                after: &new,
            },
        );

        assert_eq!(
            got,
            Placed {
                at: Place {
                    path: PathBuf::from("src/new name.rs"),
                    side: Side::New,
                    first: 6,
                    last: 7,
                },
                notice: None,
            }
        );
    }

    #[test]
    fn a_file_renamed_with_no_edit_changes_only_the_path() {
        let old = text(&numbered(6));
        let got = carry(
            &new_side(2, 3),
            Side::New,
            FileStep::Renamed {
                to: Path::new("b.rs"),
                before: &old,
                after: &old,
            },
        );

        assert_eq!(got, on("b.rs", Side::New, 2, 3));
    }

    #[test]
    fn a_thread_on_the_rewritten_lines_of_a_renamed_file_says_the_code_changed() {
        let old = text(&numbered(6));
        let mut lines = numbered(6);
        lines[2] = "rewritten".to_string();
        let new = text(&lines);
        let got = carry(
            &new_side(3, 3),
            Side::New,
            FileStep::Renamed {
                to: Path::new("b.rs"),
                before: &old,
                after: &new,
            },
        );

        assert_eq!(
            got,
            Placed {
                notice: Some(Notice::Changed),
                ..on("b.rs", Side::New, 3, 3)
            }
        );
    }

    #[test]
    fn a_file_deleted_with_no_pair_keeps_the_thread_where_it_was_with_the_notice() {
        let got = carry(&new_side(4, 6), Side::New, FileStep::Deleted);

        assert_eq!(
            got,
            Placed {
                notice: Some(Notice::Deleted),
                ..new_side(4, 6)
            }
        );
    }

    #[test]
    fn a_file_that_is_the_same_leaves_its_threads_alone() {
        let placed = Placed {
            notice: Some(Notice::Changed),
            ..new_side(4, 6)
        };

        assert_eq!(carry(&placed, Side::New, FileStep::Unchanged), placed);
    }

    #[test]
    fn a_thread_on_the_other_side_stays_as_it_is_whatever_happens_to_the_file() {
        let old = text(&numbered(10));
        let mut lines = numbered(10);
        lines.splice(0..8, ["only".to_string()]);
        let new = text(&lines);
        let placed = on("a.rs", Side::Old, 4, 5);

        assert_eq!(carry(&placed, Side::New, changed(&old, &new)), placed);
        assert_eq!(carry(&placed, Side::New, FileStep::Deleted), placed);
        assert_eq!(
            carry(
                &placed,
                Side::New,
                FileStep::Renamed {
                    to: Path::new("b.rs"),
                    before: &old,
                    after: &new
                }
            ),
            placed
        );
    }

    #[test]
    fn the_rule_is_the_same_on_the_old_side_when_the_old_side_is_what_changed() {
        let old = text(&numbered(10));
        let mut lines = vec!["new 1".to_string()];
        lines.extend(numbered(10));
        let new = text(&lines);

        let got = carry(&on("a.rs", Side::Old, 6, 7), Side::Old, changed(&old, &new));

        assert_eq!(got, on("a.rs", Side::Old, 7, 8));
    }

    #[test]
    fn a_notice_once_given_stays_through_a_step_that_leaves_the_lines_alone() {
        let old = text(&numbered(6));
        let mut lines = vec!["new 1".to_string()];
        lines.extend(numbered(6));
        let new = text(&lines);
        let placed = Placed {
            notice: Some(Notice::Changed),
            ..new_side(2, 3)
        };

        let got = carry(&placed, Side::New, changed(&old, &new));

        assert_eq!(got.notice, Some(Notice::Changed));
        assert_eq!((got.at.first, got.at.last), (3, 4));
    }

    #[test]
    fn a_notice_of_deletion_is_not_taken_back_by_a_later_change_of_the_line_it_sits_on() {
        let old = text(&numbered(6));
        let mut lines = numbered(6);
        lines[2] = "rewritten".to_string();
        let new = text(&lines);
        let placed = Placed {
            notice: Some(Notice::Deleted),
            ..new_side(3, 3)
        };

        let got = carry(&placed, Side::New, changed(&old, &new));

        assert_eq!(got.notice, Some(Notice::Deleted));
    }

    #[test]
    fn a_change_after_an_earlier_one_replaces_the_notice_changed_by_the_notice_deleted() {
        let old = text(&numbered(6));
        let mut lines = numbered(6);
        lines.remove(2);
        let new = text(&lines);
        let placed = Placed {
            notice: Some(Notice::Changed),
            ..new_side(3, 3)
        };

        let got = carry(&placed, Side::New, changed(&old, &new));

        assert_eq!(got.notice, Some(Notice::Deleted));
    }

    #[test]
    fn a_file_that_became_empty_leaves_the_thread_on_line_one_with_the_notice() {
        let old = text(&numbered(4));

        assert_eq!(
            carried(&new_side(2, 3), &old, b""),
            (1, 1, Some(Notice::Deleted))
        );
    }

    #[test]
    fn a_file_that_began_empty_has_no_thread_to_move_but_does_not_fail() {
        let new = text(&numbered(3));

        let (first, last, _) = carried(&new_side(1, 1), b"", &new);

        assert!(first >= 1 && last >= first && last <= 3, "{first}..{last}");
    }

    #[test]
    fn a_last_line_that_gained_its_line_feed_is_a_changed_line() {
        let old = b"one\ntwo\nthree".to_vec();
        let new = b"one\ntwo\nthree\n".to_vec();

        assert_eq!(
            carried(&new_side(3, 3), &old, &new),
            (3, 3, Some(Notice::Changed))
        );
    }

    #[test]
    fn carrying_the_threads_of_a_file_together_gives_what_carrying_each_gives() {
        let mut old = numbered(30);
        old[2] = LONG.to_string();
        let mut new = numbered(30);
        new.remove(2);
        new.insert(20, LONG.to_string());
        new[7] = "rewritten".to_string();
        new.insert(25, "inserted".to_string());
        new.insert(0, "top".to_string());
        let (old, new) = (text(&old), text(&new));
        let threads = [
            new_side(3, 3),
            new_side(8, 8),
            new_side(5, 6),
            new_side(24, 27),
            on("a.rs", Side::Old, 9, 9),
            Placed {
                notice: Some(Notice::Changed),
                ..new_side(12, 13)
            },
        ];

        let together = carry_file(&threads, Side::New, changed(&old, &new));

        let alone: Vec<Placed> = threads
            .iter()
            .map(|t| carry(t, Side::New, changed(&old, &new)))
            .collect();
        assert_eq!(together, alone);
        assert!(together.iter().any(|p| p.notice.is_some()));
        assert!(together.iter().any(|p| p.notice.is_none()));
    }

    #[test]
    fn carrying_no_thread_gives_none() {
        assert!(carry_file(&[], Side::New, FileStep::Deleted).is_empty());
    }
}
