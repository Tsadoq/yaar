//! How a thread follows the code it was written on from one tree to the next: pure
//! functions over the two versions of one file, which never ask what kind of change a
//! tree is. Built by the engine's package "the thread follower, its rules" of slice 6.
//!
//! The rule, one tree at a time, for the lines a thread sits on in the first tree:
//! - no hunk of the diff of the file touches them: the thread shifts with them, with no
//!   notice;
//! - a hunk touches them, and the same lines, byte for byte, occur exactly once among the
//!   lines the diff adds to the file, and hold at least [`MOVED_BLOCK`] letters and
//!   digits: the thread follows that block, with no notice;
//! - otherwise the thread sits on the lines that replaced them in the hunk, with
//!   [`Notice::Changed`], or, where the hunk added nothing in their place, on the one line
//!   at the point of deletion, with [`Notice::Deleted`]. A line put inside the lines it
//!   sits on is a change too: the thread then keeps the whole range, with
//!   [`Notice::Changed`].
//!
//! The diff is `yaar-diff`'s, the one the review pane draws. A file renamed into the next
//! tree is the same file under its new path, as git pairs a rename; a file deleted and not
//! paired keeps the thread where it was, with [`Notice::Deleted`]. A notice, once given,
//! stays given in later trees. A thread on the side that did not change between the two
//! trees stays where it is.

#[cfg(test)]
mod corpus;

use std::path::{Path, PathBuf};

use yaar_store::Side;

/// The fewest letters and digits a moved block must hold for a thread to follow it with
/// no notice, as git's blame counts a moved line.
pub const MOVED_BLOCK: usize = 20;

/// The lines of one file a thread sits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// The path of the file, relative to the folder of the worktree.
    pub path: PathBuf,
    /// The side of the diff.
    pub side: Side,
    /// The first line, counted from 1.
    pub first: u32,
    /// The last line, at least `first`.
    pub last: u32,
}

/// What became of the lines a thread was written on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    /// They were rewritten, or a line was put inside them.
    Changed,
    /// They were deleted, or their file was.
    Deleted,
}

/// Where a thread sits, and whether the code it was written on changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// Where.
    pub at: Place,
    /// What became of the code; none while it only moved or did not change.
    pub notice: Option<Notice>,
}

/// What one file became from one tree to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStep<'a> {
    /// It is the same.
    Unchanged,
    /// Its content changed: from `before` to `after`.
    Changed {
        /// Its content in the first tree.
        before: &'a [u8],
        /// Its content in the next.
        after: &'a [u8],
    },
    /// It was renamed to `to`, its content changing from `before` to `after`, which may be
    /// the same.
    Renamed {
        /// Its path in the next tree.
        to: &'a Path,
        /// Its content in the first tree.
        before: &'a [u8],
        /// Its content in the next.
        after: &'a [u8],
    },
    /// It is gone, and no file of the next tree is paired with it.
    Deleted,
}

/// Carries a thread from one tree to the next, in which the files of side `changed`
/// changed, its file as `step` says: see the module's doc. A thread on the other side
/// stays as it is.
pub fn carry(placed: &Placed, changed: Side, step: FileStep<'_>) -> Placed {
    let _ = (placed, changed, step);
    todo!()
}

/// Carries every thread of one file as [`carry`] does, in their order, with the file's
/// diff worked out once for all of them.
pub fn carry_file(placed: &[Placed], changed: Side, step: FileStep<'_>) -> Vec<Placed> {
    let _ = (placed, changed, step);
    todo!()
}
