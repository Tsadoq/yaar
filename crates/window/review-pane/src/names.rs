//! The names of a line of code, as the reviewer moves from one to the next and as the
//! pane asks the Navigator about them. No language is known here: a name is a run of
//! letters, digits, `_` and `$` that does not start with a digit, where a letter or a digit
//! is any of Unicode's (`char::is_alphanumeric`).

use std::ops::Range;

/// The names of `line`, as ranges of its bytes, in the order they stand.
pub(crate) fn names_of(line: &str) -> Vec<Range<usize>> {
    let _ = line;
    todo!()
}

/// The index, in [`names_of`]'s list, of the name that holds the byte `at`, or whose end is
/// `at`; none where no name does.
pub(crate) fn name_at(line: &str, at: usize) -> Option<usize> {
    let _ = (line, at);
    todo!()
}
