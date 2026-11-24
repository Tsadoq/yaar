//! The names of a line of code: runs of Unicode letters and digits, `_` and `$` that do not
//! start with a digit. A run that starts with a digit (`0xff`, `2nd`) is a number, and no
//! part of it is a name.

use std::ops::Range;

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

pub(crate) fn names_of(line: &str) -> Vec<Range<usize>> {
    let mut names = Vec::new();
    let mut run: Option<usize> = None;
    // A sentinel past the end closes a run that the line ends in.
    for (at, c) in line
        .char_indices()
        .chain(std::iter::once((line.len(), ' ')))
    {
        match (run, is_name_char(c)) {
            (None, true) => run = Some(at),
            (Some(start), false) => {
                run = None;
                let number = line[start..].chars().next().is_some_and(char::is_numeric);
                if !number {
                    names.push(start..at);
                }
            }
            _ => {}
        }
    }
    names
}

/// The index in `names_of`'s list of the name whose characters hold byte `at`. The byte
/// right after a name is not in it: a pointer is on the character it is over.
pub(crate) fn name_at(line: &str, at: usize) -> Option<usize> {
    names_of(line)
        .iter()
        .position(|name| name.start <= at && at < name.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(line: &str) -> Vec<&str> {
        names_of(line).into_iter().map(|name| &line[name]).collect()
    }

    #[test]
    fn a_name_is_a_run_of_letters_digits_underscores_and_dollars() {
        assert_eq!(
            read("let total = perimeter(sides);"),
            ["let", "total", "perimeter", "sides"]
        );
        assert_eq!(read("$el.a_b9 += _x"), ["$el", "a_b9", "_x"]);
        assert_eq!(read("a.b->c::d"), ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_run_that_starts_with_a_digit_is_a_number_and_no_name() {
        assert_eq!(read("x = 0xff + 1e9 + 2nd + y2"), ["x", "y2"]);
        assert_eq!(read("42"), [] as [&str; 0]);
    }

    #[test]
    fn letters_and_digits_are_those_of_any_script() {
        assert_eq!(read("naïve = größe + 日本語"), ["naïve", "größe", "日本語"]);
        assert_eq!(read("x = ٣abc"), ["x"]);
    }

    #[test]
    fn the_ranges_are_bytes_of_the_line() {
        let line = "é = größe";
        let names = names_of(line);
        assert_eq!(names, [0..2, 5..12]);
        assert_eq!(&line[names[1].clone()], "größe");
    }

    #[test]
    fn a_line_with_no_name_has_none() {
        assert!(names_of("").is_empty());
        assert!(names_of("  ;;  ( ) -> {}").is_empty());
    }

    #[test]
    fn a_name_at_the_end_of_the_line_is_found() {
        assert_eq!(read("end"), ["end"]);
        assert_eq!(names_of("a end"), [0..1, 2..5]);
    }

    #[test]
    fn a_byte_holds_the_name_it_is_in_and_the_byte_after_the_name_is_not() {
        let line = "ab  cd";
        assert_eq!(name_at(line, 0), Some(0));
        assert_eq!(name_at(line, 1), Some(0));
        assert_eq!(name_at(line, 2), None);
        assert_eq!(name_at(line, 3), None);
        assert_eq!(name_at(line, 4), Some(1));
        assert_eq!(name_at(line, 5), Some(1));
        assert_eq!(name_at(line, 6), None);
        assert_eq!(name_at(line, 7), None);
        assert_eq!(name_at("", 0), None);
        assert_eq!(name_at("é(x", 0), Some(0));
        assert_eq!(name_at("é(x", 1), Some(0));
        assert_eq!(name_at("é(x", 2), None);
        assert_eq!(name_at("é(x", 3), Some(1));
        assert_eq!(name_at("a.b", 1), None);
    }
}
