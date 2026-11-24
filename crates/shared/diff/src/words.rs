use std::ops::Range;

pub(crate) struct Word<'a> {
    line: usize,
    range: Range<usize>,
    pub(crate) bytes: &'a [u8],
}

impl Word<'_> {
    fn is_space(&self) -> bool {
        std::str::from_utf8(self.bytes).is_ok_and(|text| text.chars().all(char::is_whitespace))
    }
}

pub(crate) struct Words<'a> {
    pub(crate) all: Vec<Word<'a>>,
    pub(crate) of_line: Vec<Range<usize>>,
}

/// None when there are more than `most` words. The line that goes over is still cut whole, so
/// the caller bounds line length. A line feed is a word of its own: never marked, but it lets
/// line ends meet.
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

/// The (old line, new line) pairs where a line became another. `removed` and `added` say which
/// words, by index in `all`, have no counterpart; the rest match in order. A line became the
/// line it shares most non-space bytes with, if that line shares most with it too (ties: first).
pub(crate) fn edited(
    old: &Words<'_>,
    new: &Words<'_>,
    removed: impl Fn(u32) -> bool,
    added: impl Fn(u32) -> bool,
) -> Vec<(usize, usize)> {
    // Shared bytes per (old line, new line). Matched words come in the same order on both sides, so
    // each pair's words form one run.
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    Space,
    Other,
}

/// A word is a run of letters, digits and underscores, a run of white space, or one other
/// character. Words never end inside a UTF-8 character; bytes that are not UTF-8 are words alone.
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

        let all_but_the_space = |word| word != 1;
        assert_eq!(marks(&words.all[0..4], all_but_the_space), [0..1, 2..3]);
        assert_eq!(marks(&words.all[4..7], all_but_the_space), [0..1, 2..3]);
    }

    #[test]
    fn lines_of_more_words_than_allowed_are_not_cut() {
        let lines = [&b"a b\n"[..], &b"c d"[..]];
        assert!(of_lines(lines.into_iter(), 7).is_some());
        assert!(of_lines(lines.into_iter(), 6).is_none());
        assert!(of_lines(lines.into_iter(), 4).is_none());
    }

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
        assert_eq!(
            edited_lines(
                &[b"return compute(x);\n"],
                &[4],
                &[b"return compute(y);\n", b"log(y);\n"],
                &[4, 5, 6, 7, 8, 9, 10],
            ),
            [(0, 0)]
        );

        // Counted in bytes, not words: three one-byte words against one ten-byte word.
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
        assert_eq!(edited_lines(&[b"    x\n"], &[1], &[b"    y\n"], &[1]), []);
    }
}
