//! The bytes that wait for one program to read them.

use std::collections::VecDeque;

/// What has been handed to a terminal and not yet written to it, in order, with a limit on
/// how much may wait.
pub(crate) struct Input {
    chunks: VecDeque<Vec<u8>>,
    /// Bytes handed over and not yet written, those of a chunk being written included.
    waiting: usize,
    limit: usize,
    /// Nothing more is taken: the program has ended.
    closed: bool,
}

impl Input {
    pub(crate) fn new(limit: usize) -> Input {
        Input {
            chunks: VecDeque::new(),
            waiting: 0,
            limit,
            closed: false,
        }
    }

    /// Queues what fits under the limit and drops the rest. Gives how many bytes were
    /// kept.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> usize {
        if self.closed {
            return 0;
        }
        let kept = bytes.len().min(self.limit - self.waiting);
        if kept > 0 {
            self.chunks.push_back(bytes[..kept].to_vec());
            self.waiting += kept;
        }
        kept
    }

    /// The oldest chunk that waits, if any.
    pub(crate) fn pop(&mut self) -> Option<Vec<u8>> {
        self.chunks.pop_front()
    }

    /// Tells that `count` bytes of what was popped have been written. After a close
    /// nothing is counted any more.
    pub(crate) fn written(&mut self, count: usize) {
        self.waiting = self.waiting.saturating_sub(count);
    }

    /// Nothing waits, and nothing more is taken.
    pub(crate) fn close(&mut self) {
        self.chunks.clear();
        self.waiting = 0;
        self.closed = true;
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_come_out_in_the_order_they_went_in() {
        let mut input = Input::new(100);
        input.push(b"one ");
        input.push(b"two");
        assert_eq!(input.pop().as_deref(), Some(&b"one "[..]));
        assert_eq!(input.pop().as_deref(), Some(&b"two"[..]));
        assert_eq!(input.pop(), None);
    }

    #[test]
    fn what_does_not_fit_under_the_limit_is_dropped_and_what_is_written_makes_room() {
        let mut input = Input::new(10);
        assert_eq!(input.push(b"0123456"), 7);
        // Only the first three bytes of this one fit.
        assert_eq!(input.push(b"abcdef"), 3);
        assert_eq!(input.push(b"more"), 0);

        let first = input.pop().expect("a chunk");
        assert_eq!(first, b"0123456");
        // A chunk being written still counts as waiting.
        assert_eq!(input.push(b"x"), 0);
        input.written(first.len());
        assert_eq!(input.push(b"xyz"), 3);

        assert_eq!(input.pop().as_deref(), Some(&b"abc"[..]));
        assert_eq!(input.pop().as_deref(), Some(&b"xyz"[..]));
    }

    #[test]
    fn a_closed_queue_holds_nothing_and_takes_nothing() {
        let mut input = Input::new(10);
        input.push(b"abc");
        input.close();
        assert!(input.is_closed());
        assert_eq!(input.push(b"d"), 0);
        assert_eq!(input.pop(), None);
    }
}
