//! Cutting changed lines into words, finding the lines that were edited, and marking the
//! words that changed in them.

use std::ops::Range;

/// One word of some lines, or the line feed that ends one of them.
pub(crate) struct Word<'a> {
    /// Which of the lines it is in, counted from the first of them.
    line: usize,
    /// Where it is in that line.
    range: Range<usize>,
    /// Its bytes, which are what makes two words the same word.
    pub(crate) bytes: &'a [u8],
}

impl Word<'_> {
    /// Whether it is white space, the line feed among it.
    fn is_space(&self) -> bool {
        std::str::from_utf8(self.bytes).is_ok_and(|text| text.chars().all(char::is_whitespace))
    }
}

/// The words of some lines.
pub(crate) struct Words<'a> {
    /// All of them, in the order of the lines.
    pub(crate) all: Vec<Word<'a>>,
    /// Which of them are the words of each line.
    pub(crate) of_line: Vec<Range<usize>>,
}

/// The words of `lines`, each line given with the line feed that ends it, if it has one;
/// or none when they are more than `most` words. The cutting stops at the line that goes
/// over, so lines without end cost no more than the first of them.
///
/// The line feed is a word of its own. It can never be marked, since it is not part of
/// the line as it is shown, but it lets the end of a line on one side meet the end of a
/// line on the other.
pub(crate) fn of_lines<'a>(
    lines: impl Iterator<Item = &'a [u8]>,
    most: usize,
) -> Option<Words<'a>> {
    let mut words = Words {
        all: Vec::new(),
        of_line: Vec::new(),
    };
    for (line, with_feed) in lines.enumerate() {
        let shown = with_feed.strip_suffix(b"\n").unwrap_or(with_feed);
        let in_line = cut(shown)
            .into_iter()
            .chain((shown.len() < with_feed.len()).then_some(shown.len()..with_feed.len()));
        let first = words.all.len();
        words.all.extend(in_line.map(|range| Word {
            line,
            bytes: &with_feed[range.clone()],
            range,
        }));
        words.of_line.push(first..words.all.len());
        if words.all.len() > most {
            return None;
        }
    }
    Some(words)
}

/// The lines that were edited: for each, its place among the `old` lines and the place
/// among the `new` lines of the line it became, in the order of the lines.
///
/// `removed` and `added` are the outcome of comparing all the old words with all the new
/// ones: which old words, by their place in `old.all`, stand for no new word, and which
/// new ones for no old word. The others stand for each other in order. What two lines
/// share is the bytes of the words of one that stand for words of the other. White space
/// is left out of the count: any two lines share some, so it tells nothing of which line
/// became which.
///
/// A line became the line it shares the most with, if that line shares the most with it
/// too; where a line shares as much with two, the first of them counts. A line that
/// shares nothing, and a line whose nearest line is nearer to another, became no line: it
/// was removed, or added, as a whole.
pub(crate) fn edited(
    old: &Words<'_>,
    new: &Words<'_>,
    removed: impl Fn(u32) -> bool,
    added: impl Fn(u32) -> bool,
) -> Vec<(usize, usize)> {
    // What each two lines share, as an old line, a new line and a count of bytes. Words
    // that stand for each other come in the same order on both sides, so all that two
    // lines share comes in one run, and the runs come in the order of the lines.
    let mut shared: Vec<(usize, usize, usize)> = Vec::new();
    let mut kept = new
        .all
        .iter()
        .enumerate()
        .filter(|(index, _)| !added(*index as u32))
        .map(|(_, word)| word);
    for (index, word) in old.all.iter().enumerate() {
        if removed(index as u32) {
            continue;
        }
        let other = kept
            .next()
            .expect("as many new words stand for an old one as old words for a new one");
        if word.is_space() {
            continue;
        }
        match shared.last_mut() {
            Some((old_line, new_line, bytes))
                if (*old_line, *new_line) == (word.line, other.line) =>
            {
                *bytes += word.bytes.len();
            }
            _ => shared.push((word.line, other.line, word.bytes.len())),
        }
    }

    // For each line the most it shares with a line of the other side, and with which.
    let mut most_of_old: Vec<(usize, Option<usize>)> = vec![(0, None); old.of_line.len()];
    let mut most_of_new: Vec<(usize, Option<usize>)> = vec![(0, None); new.of_line.len()];
    for &(old_line, new_line, bytes) in &shared {
        if bytes > most_of_old[old_line].0 {
            most_of_old[old_line] = (bytes, Some(new_line));
        }
        if bytes > most_of_new[new_line].0 {
            most_of_new[new_line] = (bytes, Some(old_line));
        }
    }
    shared
        .iter()
        .map(|&(old_line, new_line, _)| (old_line, new_line))
        .filter(|&(old_line, new_line)| {
            most_of_old[old_line].1 == Some(new_line) && most_of_new[new_line].1 == Some(old_line)
        })
        .collect()
}

/// The ranges to mark in one line: those of its `words` that `changed` says changed, by
/// their place among them. Words that follow one another are marked as one range.
pub(crate) fn marks(words: &[Word<'_>], changed: impl Fn(u32) -> bool) -> Vec<Range<usize>> {
    let mut marks: Vec<Range<usize>> = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if word.bytes == b"\n" || !changed(index as u32) {
            continue;
        }
        match marks.last_mut() {
            Some(last) if last.end == word.range.start => last.end = word.range.end,
            _ => marks.push(word.range.clone()),
        }
    }
    marks
}

/// What a character is, for the cutting of a line into words.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A letter, a digit or an underscore: part of a name or a number.
    Name,
    /// White space.
    Space,
    /// Anything else: a word by itself.
    Other,
}

/// Cuts one line into words: a run of letters, digits and underscores, a run of white
/// space, or one other character.
///
/// Where the line is UTF-8 a word never ends inside a character, so the text of the line
/// can be sliced by the ranges. Bytes that are no UTF-8 are words by themselves.
fn cut(line: &[u8]) -> Vec<Range<usize>> {
    let mut words: Vec<Range<usize>> = Vec::new();
    let mut last = Kind::Other;
    let mut at = 0;
    for chunk in line.utf8_chunks() {
        for character in chunk.valid().chars() {
            let kind = if character.is_alphanumeric() || character == '_' {
                Kind::Name
            } else if character.is_whitespace() {
                Kind::Space
            } else {
                Kind::Other
            };
            let end = at + character.len_utf8();
            match words.last_mut() {
                Some(word) if kind == last && kind != Kind::Other => word.end = end,
                _ => words.push(at..end),
            }
            last = kind;
            at = end;
        }
        if !chunk.invalid().is_empty() {
            let end = at + chunk.invalid().len();
            words.push(at..end);
            last = Kind::Other;
            at = end;
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_cut_into_names_runs_of_white_space_and_single_other_characters() {
        let line = "let x_1 =  f(a2, \"é\")\t ;";
        let words: Vec<&str> = cut(line.as_bytes())
            .into_iter()
            .map(|range| &line[range])
            .collect();
        assert_eq!(
            words,
            [
                "let", " ", "x_1", " ", "=", "  ", "f", "(", "a2", ",", " ", "\"", "é", "\"", ")",
                "\t ", ";"
            ]
        );
    }

    #[test]
    fn the_line_feed_is_a_word_of_its_line_that_is_never_marked() {
        let lines = [&b"a b\n"[..], &b"c d"[..]];
        let words = of_lines(lines.into_iter(), usize::MAX).expect("words without limit");
        let bytes: Vec<&[u8]> = words.all.iter().map(|word| word.bytes).collect();
        assert_eq!(bytes, [&b"a"[..], b" ", b"b", b"\n", b"c", b" ", b"d"]);
        assert_eq!(words.of_line, [0..4, 4..7]);

        // Every word but the space is called changed, the line feed among them: the
        // last mark of the first line still ends where the shown line ends, and the
        // marks of the second line count from its own start.
        let all_but_the_space = |word| word != 1;
        assert_eq!(marks(&words.all[0..4], all_but_the_space), [0..1, 2..3]);
        assert_eq!(marks(&words.all[4..7], all_but_the_space), [0..1, 2..3]);
    }

    #[test]
    fn lines_of_more_words_than_allowed_are_not_cut() {
        // Four words and three, the line feed among them.
        let lines = [&b"a b\n"[..], &b"c d"[..]];
        assert!(of_lines(lines.into_iter(), 7).is_some());
        assert!(of_lines(lines.into_iter(), 6).is_none());
        // The first line alone is within the limit, and the two together are not.
        assert!(of_lines(lines.into_iter(), 4).is_none());
    }

    /// The lines that were edited, when the words at `removed` of the old lines and at
    /// `added` of the new ones stand for no word of the other side.
    fn edited_lines(
        old: &[&[u8]],
        removed: &[u32],
        new: &[&[u8]],
        added: &[u32],
    ) -> Vec<(usize, usize)> {
        edited(
            &of_lines(old.iter().copied(), usize::MAX).expect("words without limit"),
            &of_lines(new.iter().copied(), usize::MAX).expect("words without limit"),
            |word| removed.contains(&word),
            |word| added.contains(&word),
        )
    }

    #[test]
    fn a_line_became_the_line_it_shares_the_most_with_if_that_line_shares_the_most_with_it() {
        // `return compute(` stands in the first new line, `);` and the line feed in the
        // second. Each new line shares the most with the one old line, which can have
        // become only one of them: the one it shares the most with.
        assert_eq!(
            edited_lines(
                &[b"return compute(x);\n"],
                &[4],
                &[b"return compute(y);\n", b"log(y);\n"],
                &[4, 5, 6, 7, 8, 9, 10],
            ),
            [(0, 0)]
        );

        // What is shared is counted in bytes, not in words: the new line holds three
        // words of the first old line, of one byte each, and one word of the second, of
        // ten bytes.
        assert_eq!(
            edited_lines(
                &[b"a.b.c\n", b"identifier\n"],
                &[3, 4, 5],
                &[b"a.b identifier\n"],
                &[3],
            ),
            [(1, 0)]
        );
    }

    #[test]
    fn of_two_lines_that_share_as_much_with_a_line_it_became_the_first() {
        // The line feed that stands is the one of the second new line.
        assert_eq!(
            edited_lines(&[b"ab cd\n"], &[1], &[b"ab\n", b"cd\n"], &[1]),
            [(0, 0)]
        );
        assert_eq!(
            edited_lines(&[b"ab\n", b"cd\n"], &[1], &[b"ab cd\n"], &[1]),
            [(0, 0)]
        );
    }

    #[test]
    fn white_space_is_nothing_shared() {
        // The indentation and the line feed stand on both sides, and the lines are still
        // two lines with no word in common.
        assert_eq!(edited_lines(&[b"    x\n"], &[1], &[b"    y\n"], &[1]), []);
    }
}
