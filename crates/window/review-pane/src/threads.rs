//! Where the threads of a review sit among the rows of the pane.

use std::path::Path;

use yaar_diff::Side;
use yaar_link::{ChangedFile, FileStatus, Review, Side as LinkSide, Thread, ThreadId};

use crate::{PaneRow, ShownFile};

/// The Link's name of a side of a diff.
pub(crate) fn link_side(side: Side) -> LinkSide {
    match side {
        Side::Old => LinkSide::Old,
        Side::New => LinkSide::New,
    }
}

/// The side of the pane that a thread of the Link is on.
pub(crate) fn pane_side(side: LinkSide) -> Side {
    match side {
        LinkSide::Old => Side::Old,
        LinkSide::New => Side::New,
    }
}

/// The path of a changed file in the version on `side`: where a thread on that side of
/// the file names it. The old side of a renamed file is the file by its old path.
pub(crate) fn path_in(file: &ChangedFile, side: Side) -> &Path {
    match (&file.status, side) {
        (FileStatus::Renamed { from }, Side::Old) => from,
        _ => &file.path,
    }
}

/// The rows of one file's diff with the threads that sit on them: each thread follows the
/// row that shows its last line, as one row for the thread and one for each of its
/// comments. Gives the rows and the threads it placed.
pub(crate) fn with_threads(
    file: &ChangedFile,
    diff: Vec<PaneRow>,
    review: Option<&Review>,
) -> (Vec<PaneRow>, Vec<ThreadId>) {
    let Some(review) = review.filter(|review| !review.threads.is_empty()) else {
        return (diff, Vec::new());
    };
    let mut rows = Vec::with_capacity(diff.len());
    let mut placed: Vec<ThreadId> = Vec::new();
    for row in diff {
        let here: Vec<&Thread> = match &row {
            PaneRow::Diff { row: shown, .. } => review
                .threads
                .iter()
                .filter(|thread| {
                    let side = pane_side(thread.anchor.side);
                    path_in(file, side) == thread.anchor.path
                        && shown.line(side) == Some(thread.anchor.lines.last.wrapping_sub(1))
                        // A line is on one row only, but a thread is placed once.
                        && !placed.contains(&thread.id)
                })
                .collect(),
            _ => Vec::new(),
        };
        rows.push(row);
        for thread in here {
            placed.push(thread.id.clone());
            rows.extend(thread_rows(thread));
        }
    }
    (rows, placed)
}

/// The row of a thread and the row of each of its comments.
pub(crate) fn thread_rows(thread: &Thread) -> impl Iterator<Item = PaneRow> + '_ {
    std::iter::once(PaneRow::Thread {
        thread: thread.id.clone(),
    })
    .chain(thread.comments.iter().map(|comment| PaneRow::Comment {
        thread: thread.id.clone(),
        comment: comment.id.clone(),
    }))
}

/// The heading and the rows of the threads that no row of a file shows, after the last
/// file: a thread is never hidden. Nothing when every thread is placed.
pub(crate) fn elsewhere(files: &[ShownFile], review: Option<&Review>) -> Vec<PaneRow> {
    let Some(review) = review else {
        return Vec::new();
    };
    let lost: Vec<&Thread> = review
        .threads
        .iter()
        .filter(|thread| !files.iter().any(|file| file.placed.contains(&thread.id)))
        .collect();
    if lost.is_empty() {
        return Vec::new();
    }
    std::iter::once(PaneRow::ThreadsElsewhere)
        .chain(lost.into_iter().flat_map(thread_rows))
        .collect()
}
