//! Cutting changed lines into words, and marking the words that changed.

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

/// The words of `lines`, each line given with the line feed that ends it, if it has one.
///
/// The line feed is a word of its own. It can never be marked, since it is not part of
/// the line as it is shown, but it lets the end of a line on one side meet the end of a
/// line on the other.
pub(crate) fn of_lines<'a>(lines: impl Iterator<Item = &'a [u8]>) -> Vec<Word<'a>> {
    let mut words = Vec::new();
    for (line, with_feed) in lines.enumerate() {
        let shown = with_feed.strip_suffix(b"\n").unwrap_or(with_feed);
        let in_line = cut(shown)
            .into_iter()
            .chain((shown.len() < with_feed.len()).then_some(shown.len()..with_feed.len()));
        words.extend(in_line.map(|range| Word {
            line,
            bytes: &with_feed[range.clone()],
            range,
        }));
    }
    words
}

/// The ranges to mark in each of `lines` lines: those of the words that `changed` says
/// changed, by their place in `words`. Words that follow one another are marked as one
/// range.
pub(crate) fn marks(
    words: &[Word<'_>],
    lines: usize,
    changed: impl Fn(u32) -> bool,
) -> Vec<Vec<Range<usize>>> {
    let mut marks: Vec<Vec<Range<usize>>> = vec![Vec::new(); lines];
    for (index, word) in words.iter().enumerate() {
        if word.bytes == b"\n" || !changed(index as u32) {
            continue;
        }
        match marks[word.line].last_mut() {
            Some(last) if last.end == word.range.start => last.end = word.range.end,
            _ => marks[word.line].push(word.range.clone()),
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
