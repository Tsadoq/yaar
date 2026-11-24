pub(crate) struct Scanner {
    marks: Vec<Vec<u8>>,
    /// The last `longest mark - 1` bytes: a mark split across reads lies in the tail plus the next
    /// read.
    tail: Vec<u8>,
    window: Vec<u8>,
}

impl Scanner {
    /// A scanner for `marks`. An empty one is never found: it would match everywhere.
    pub(crate) fn new(marks: Vec<Vec<u8>>) -> Scanner {
        Scanner {
            marks,
            tail: Vec::new(),
            window: Vec::new(),
        }
    }

    /// Indexes of the marks `read` completes, ordered by last byte, then index. A mark wholly in
    /// earlier reads is not reported again.
    pub(crate) fn scan(&mut self, read: &[u8]) -> Vec<usize> {
        let longest = self.marks.iter().map(Vec::len).max().unwrap_or(0);
        if longest == 0 {
            return Vec::new();
        }
        self.window.clear();
        self.window.extend_from_slice(&self.tail);
        self.window.extend_from_slice(read);
        let old = self.tail.len();

        let mut found = Vec::new();
        for (index, mark) in self.marks.iter().enumerate() {
            if mark.is_empty() {
                continue;
            }
            for (start, candidate) in self.window.windows(mark.len()).enumerate() {
                if start + mark.len() > old && candidate == mark.as_slice() {
                    found.push((start + mark.len(), index));
                }
            }
        }
        found.sort_unstable();

        let keep = (longest - 1).min(self.window.len());
        self.tail.clear();
        self.tail
            .extend_from_slice(&self.window[self.window.len() - keep..]);
        found.into_iter().map(|(_, index)| index).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanner(marks: &[&[u8]]) -> Scanner {
        Scanner::new(marks.iter().map(|mark| mark.to_vec()).collect())
    }

    #[test]
    fn a_mark_in_one_read_is_found_once() {
        let mut scanner = scanner(&[b"\x1b]9;4;0;\x07"]);
        assert_eq!(scanner.scan(b"before \x1b]9;4;0;\x07 after"), [0]);
        assert_eq!(scanner.scan(b"nothing here"), Vec::<usize>::new());
    }

    #[test]
    fn a_mark_cut_between_reads_is_found_when_its_last_byte_comes_and_only_then() {
        let mut scanner = scanner(&[b"abcd"]);
        assert_eq!(scanner.scan(b"xa"), Vec::<usize>::new());
        assert_eq!(scanner.scan(b"b"), Vec::<usize>::new());
        assert_eq!(scanner.scan(b"c"), Vec::<usize>::new());
        assert_eq!(scanner.scan(b"d"), [0]);
        assert_eq!(scanner.scan(b"e"), Vec::<usize>::new());
        assert_eq!(scanner.scan(b"fg"), Vec::<usize>::new());
    }

    #[test]
    fn a_mark_wholly_in_the_tail_is_not_told_again_by_the_next_read() {
        let mut scanner = scanner(&[b"abc", b"wxyz"]);
        assert_eq!(scanner.scan(b"abc"), [0]);
        assert_eq!(scanner.scan(b"d"), Vec::<usize>::new());
    }

    #[test]
    fn each_time_a_mark_is_found_it_is_told_in_the_order_of_its_end_then_its_index() {
        let mut scanner = scanner(&[b"bc", b"c", b"a"]);
        // `a` ends at 1, `bc` and `c` both end at 3, `a` again at 4.
        assert_eq!(scanner.scan(b"abca"), [2, 0, 1, 2]);
    }

    #[test]
    fn a_mark_that_overlaps_itself_is_told_for_each_place_it_ends() {
        let mut scanner = scanner(&[b"aa"]);
        assert_eq!(scanner.scan(b"aaa"), [0, 0]);
        // The third `a` is in the tail: the fourth completes one more, not two.
        assert_eq!(scanner.scan(b"a"), [0]);
    }

    #[test]
    fn a_mark_with_no_bytes_is_never_found_and_no_marks_find_nothing() {
        assert_eq!(scanner(&[b""]).scan(b"anything"), Vec::<usize>::new());
        assert_eq!(scanner(&[]).scan(b"anything"), Vec::<usize>::new());
        let mut mixed = scanner(&[b"", b"x"]);
        assert_eq!(mixed.scan(b"axb"), [1]);
    }

    #[test]
    fn a_read_shorter_than_the_tail_keeps_what_came_before_it() {
        let mut scanner = scanner(&[b"abcdef"]);
        assert_eq!(scanner.scan(b"abc"), Vec::<usize>::new());
        assert_eq!(scanner.scan(b"d"), Vec::<usize>::new());
        assert_eq!(scanner.scan(b"ef"), [0]);
    }
}
