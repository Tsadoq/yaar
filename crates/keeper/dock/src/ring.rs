use std::collections::VecDeque;

use crate::RING_BYTES;
use crate::marks::Scanner;

/// One lock holds the kept bytes, the dropped flag and the mark scan, so what is told, kept and
/// attached always agree.
pub(crate) struct Ring {
    bytes: VecDeque<u8>,
    dropped: bool,
    scanner: Scanner,
}

impl Ring {
    pub(crate) fn new(marks: Vec<Vec<u8>>) -> Ring {
        Ring {
            bytes: VecDeque::new(),
            dropped: false,
            scanner: Scanner::new(marks),
        }
    }

    /// Keeps `read` as the newest bytes, dropping the oldest past [`RING_BYTES`]; returns the marks
    /// it completes.
    pub(crate) fn push(&mut self, read: &[u8]) -> Vec<usize> {
        self.bytes.extend(read);
        if self.bytes.len() > RING_BYTES {
            let over = self.bytes.len() - RING_BYTES;
            self.bytes.drain(..over);
            self.dropped = true;
        }
        self.scanner.scan(read)
    }

    /// The kept bytes, oldest first, and whether older ones were dropped.
    pub(crate) fn kept(&mut self) -> (&[u8], bool) {
        (self.bytes.make_contiguous(), self.dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_read_is_kept_whole_while_it_fits() {
        let mut ring = Ring::new(Vec::new());
        ring.push(b"one ");
        ring.push(b"two");
        assert_eq!(ring.kept(), (&b"one two"[..], false));
    }

    #[test]
    fn a_ring_filled_to_the_byte_has_dropped_nothing_and_one_more_byte_drops_the_oldest() {
        let mut ring = Ring::new(Vec::new());
        ring.push(&vec![b'a'; RING_BYTES - 1]);
        ring.push(b"z");
        let (kept, dropped) = ring.kept();
        assert_eq!((kept.len(), dropped), (RING_BYTES, false));

        ring.push(b"y");
        let (kept, dropped) = ring.kept();
        assert!(dropped);
        assert_eq!(kept.len(), RING_BYTES);
        assert_eq!(&kept[RING_BYTES - 2..], b"zy");
        assert!(kept[..RING_BYTES - 2].iter().all(|byte| *byte == b'a'));
    }

    #[test]
    fn a_read_longer_than_the_ring_leaves_its_newest_bytes_and_says_it_dropped() {
        let mut ring = Ring::new(Vec::new());
        ring.push(b"old");
        let mut big: Vec<u8> = (0..RING_BYTES + 10).map(|n| (n % 251) as u8).collect();
        ring.push(&big);
        let (kept, dropped) = ring.kept();
        assert!(dropped);
        assert_eq!(kept, &big.split_off(10)[..]);
    }

    #[test]
    fn once_something_was_dropped_the_ring_says_so_for_good() {
        let mut ring = Ring::new(Vec::new());
        ring.push(&vec![b'a'; RING_BYTES + 1]);
        assert!(ring.kept().1);
        ring.push(b"b");
        assert!(ring.kept().1);
    }

    #[test]
    fn marks_are_found_in_what_is_pushed() {
        let mut ring = Ring::new(vec![b"end".to_vec()]);
        assert_eq!(ring.push(b"the e"), Vec::<usize>::new());
        assert_eq!(ring.push(b"nd"), [0]);
    }
}
