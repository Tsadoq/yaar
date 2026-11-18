//! Review pane (`W-REVIEW`).
//!
//! Owns: Flow and Changes, unified and split diff, threads and comment boxes, file views, follow.
//!
//! Surface:
//! - [`entry`], the pane as the Window program registers it: the one line that program
//!   needs. It holds [`pane`], which builds the pane for the Frame, and [`controls`];
//! - [`ReviewPane`], the view, and the model of what it shows: its [`PaneRow`]s, its
//!   [`Layout`], and the [`Position`] of the line the reviewer is on;
//! - [`controls`], the actions of the pane with their keys: [`LineDown`], [`LineUp`],
//!   [`ToggleLayout`].
//!
//! The pane shows the changes of the workspace on screen: it observes
//! `yaar_frame::WorkspaceChanges`, asks through it for the two versions of each changed
//! file, and diffs them with `yaar-diff` off the thread that draws. It reads and shows;
//! nothing in it changes a file, and it offers no way to.
//!
//! It depends on the Frame and on no other pane.

use std::path::PathBuf;

use gpui_kit::*;
use yaar_diff::{Row, Side};
use yaar_frame::{BuildPane, Control, Models, PaneEntry};
use yaar_link::FileStatus;

pub use yaar_diff::Layout;

actions!(
    review,
    [
        /// Moves the reviewer one line down.
        LineDown,
        /// Moves the reviewer one line up.
        LineUp,
        /// Switches the diff between the unified and the split view.
        ToggleLayout,
    ]
);

/// The key context of the pane: its keys work while it has the focus.
pub const KEY_CONTEXT: &str = "ReviewPane";

/// The layout a diff is shown in until the reviewer switches it.
pub const FIRST_LAYOUT: Layout = Layout::Unified;

/// The name of the pane in the Frame.
pub const NAME: &str = "review";

/// The pane as the Window program registers it: its name, its keys and its builder.
pub fn entry() -> PaneEntry {
    PaneEntry {
        name: NAME,
        controls,
        build: pane,
    }
}

/// Builds the Review pane for a window, under the name [`NAME`].
pub fn pane() -> BuildPane {
    todo!()
}

/// The actions of the pane with their keys, all in [`KEY_CONTEXT`]: `j` and `down` for
/// [`LineDown`], `k` and `up` for [`LineUp`], `s` for [`ToggleLayout`]. None of them
/// sends a message a user starts.
pub fn controls() -> Vec<Control> {
    todo!()
}

/// One line of one version of one changed file: where the reviewer is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Position {
    /// The path of the file, as the changes list it.
    pub path: PathBuf,
    /// The version the line belongs to. A line that did not change is on both sides and
    /// is named by its new one.
    pub side: Side,
    /// The number of the line in that version, counted from 0.
    pub line: u32,
}

/// Why the lines of a changed file are not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unshown {
    /// The Keeper has not sent the file yet, or the diff is still being computed.
    Loading,
    /// The file is not text.
    Binary,
    /// The file is too large to send.
    TooLarge,
    /// The Keeper refused to send it.
    Refused,
}

/// One row of the pane, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneRow {
    /// The line that says what the changes are compared with: the first row, once the
    /// changes are here. When there is nothing to compare with, it says why and offers
    /// to set a base.
    Comparison {
        /// Whether the row offers to set a base: with
        /// `yaar_link::Comparison::NothingToCompare`, and with no other comparison.
        offers_base: bool,
    },
    /// A commit of the comparison. The commits follow the comparison line, newest first,
    /// before the first file.
    Commit {
        /// Its full id.
        id: String,
        /// The first line of its message.
        summary: String,
    },
    /// The heading of a changed file. The rows of its diff follow.
    File {
        /// Its path.
        path: PathBuf,
        /// What happened to it.
        status: FileStatus,
    },
    /// A row of the diff of the file above, in the layout of the pane.
    Diff {
        /// The path of the file.
        path: PathBuf,
        /// The row.
        row: Row,
    },
    /// Stands for the diff of the file above while it cannot be shown.
    Unshown {
        /// The path of the file.
        path: PathBuf,
        /// Why.
        why: Unshown,
    },
}

/// The Review pane: a view, and the model of what it shows.
///
/// The reviewer is always on one line once a diff is shown: at first the first changed
/// line of the first file. [`LineDown`] and [`LineUp`] move to the next and the previous
/// line of the diff, across files, and a click on a line moves there. The pane scrolls so
/// that the line stays in view; the wheel scrolls without moving the reviewer.
///
/// [`ToggleLayout`] changes how the rows are laid out and nothing else: the reviewer
/// stays on the same line of the same version of the same file, and that line stays in
/// view.
///
/// For tests, which cannot read what is drawn, each row of a diff that is laid out
/// carries a selector that `debug_bounds` finds: `review-row-<index>`, with its index in
/// [`ReviewPane::rows`].
pub struct ReviewPane {
    models: Models,
    focus: FocusHandle,
    layout: Layout,
    rows: Vec<PaneRow>,
    position: Option<Position>,
}

impl ReviewPane {
    /// A pane that shows the changes of the workspace on screen of `models`, in
    /// [`FIRST_LAYOUT`].
    pub fn new(models: &Models, window: &mut Window, cx: &mut Context<Self>) -> Self {
        todo!()
    }

    /// The layout the diffs are shown in.
    pub fn layout(&self) -> Layout {
        todo!()
    }

    /// The rows of the pane, top to bottom, in the layout it has now: the comparison
    /// line, its commits, then each changed file with the rows of its diff. These are
    /// the rows the pane draws from, not a second account of them.
    pub fn rows(&self) -> &[PaneRow] {
        todo!()
    }

    /// The text of one line of one version of a changed file, without its line end;
    /// none while the pane does not hold that file. Bytes that are not UTF-8 are shown
    /// as the replacement character.
    pub fn text(&self, position: &Position) -> Option<String> {
        todo!()
    }

    /// The line the reviewer is on; none while no diff is shown.
    pub fn position(&self) -> Option<&Position> {
        todo!()
    }

    /// The index in [`ReviewPane::rows`] of the row that shows the line the reviewer is
    /// on.
    pub fn position_row(&self) -> Option<usize> {
        todo!()
    }

    /// The indices in [`ReviewPane::rows`] of the rows in view, as last laid out.
    pub fn rows_in_view(&self) -> std::ops::Range<usize> {
        todo!()
    }
}
