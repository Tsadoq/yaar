use std::path::Path;

use yaar_diff::Side;
use yaar_link::{ChangedFile, FileStatus, Placed, Review, Side as LinkSide, Thread, ThreadId};

use crate::{PaneRow, ShownFile};

pub(crate) fn link_side(side: Side) -> LinkSide {
    match side {
        Side::Old => LinkSide::Old,
        Side::New => LinkSide::New,
    }
}

pub(crate) fn pane_side(side: LinkSide) -> Side {
    match side {
        LinkSide::Old => Side::Old,
        LinkSide::New => Side::New,
    }
}

/// The old side of a renamed file is the file by its old path.
pub(crate) fn path_in(file: &ChangedFile, side: Side) -> &Path {
    match (&file.status, side) {
        (FileStatus::Renamed { from }, Side::Old) => from,
        _ => &file.path,
    }
}

pub(crate) enum Place<'a> {
    Anchor(&'a Thread),
    Now(&'a Placed),
}

impl Place<'_> {
    pub(crate) fn at(&self) -> (&Path, LinkSide, u32) {
        match self {
            Place::Anchor(thread) => (
                &thread.anchor.path,
                thread.anchor.side,
                thread.anchor.lines.last,
            ),
            Place::Now(now) => (&now.path, now.side, now.lines.last),
        }
    }
}

/// None when the thread sits in no line of that version. Without a `now` it sits on its anchor;
/// with one, on `now` when the new side is `now.tree` or the files on disk, on its anchor when
/// the new side is the anchor's own version, nowhere otherwise.
pub(crate) fn place_of<'a>(thread: &'a Thread, shown: Option<&str>) -> Option<Place<'a>> {
    let Some(now) = &thread.now else {
        return Some(Place::Anchor(thread));
    };
    match shown {
        None => Some(Place::Now(now)),
        Some(version) if version == now.tree => Some(Place::Now(now)),
        Some(version) if thread.anchor.tree.as_deref() == Some(version) => {
            Some(Place::Anchor(thread))
        }
        Some(_) => None,
    }
}

pub(crate) fn with_threads(
    file: &ChangedFile,
    diff: Vec<PaneRow>,
    review: Option<&Review>,
    shown: Option<&str>,
) -> (Vec<PaneRow>, Vec<ThreadId>) {
    let Some(review) = review.filter(|review| !review.threads.is_empty()) else {
        return (diff, Vec::new());
    };
    let mut rows = Vec::with_capacity(diff.len());
    let mut placed: Vec<ThreadId> = Vec::new();
    for row in diff {
        let here: Vec<&Thread> = match &row {
            PaneRow::Diff { row: shown_row, .. } => review
                .threads
                .iter()
                .filter(|thread| {
                    let Some(place) = place_of(thread, shown) else {
                        return false;
                    };
                    let (path, link_side, last) = place.at();
                    let side = pane_side(link_side);
                    path_in(file, side) == path
                        && shown_row.line(side) == Some(last.wrapping_sub(1))
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

pub(crate) fn thread_rows(thread: &Thread) -> impl Iterator<Item = PaneRow> + '_ {
    std::iter::once(PaneRow::Thread {
        thread: thread.id.clone(),
    })
    .chain(thread.comments.iter().map(|comment| PaneRow::Comment {
        thread: thread.id.clone(),
        comment: comment.id.clone(),
    }))
}

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

impl crate::ReviewPane {
    /// None for the files on disk, as in the comparison, the uncommitted changes and Flow.
    pub(crate) fn new_side_shown(&self) -> Option<String> {
        match &self.listed {
            Some(crate::tree::Source::Versions { new, .. }) => new.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use yaar_link::{Anchor, Lines, Notice, ThreadState};

    use super::*;

    fn lines(last: u32) -> Lines {
        Lines { first: last, last }
    }

    fn thread(carried: bool) -> Thread {
        Thread {
            id: ThreadId::new("7".repeat(32)),
            anchor: Anchor {
                tree: Some("1".repeat(40)),
                path: PathBuf::from("a.rs"),
                side: LinkSide::New,
                lines: lines(3),
                text: "x".to_owned(),
            },
            resolved: false,
            addressed: false,
            comments: Vec::new(),
            state: ThreadState::Sent,
            now: carried.then(|| Placed {
                tree: "2".repeat(40),
                path: PathBuf::from("b.rs"),
                side: LinkSide::New,
                lines: lines(7),
                notice: Some(Notice::Changed),
            }),
        }
    }

    fn at(thread: &Thread, shown: Option<&str>) -> Option<(PathBuf, u32)> {
        let place = place_of(thread, shown)?;
        let (path, _, last) = place.at();
        Some((path.to_owned(), last))
    }

    #[test]
    fn a_thread_the_keeper_never_carried_sits_on_its_anchor_in_any_version() {
        let thread = thread(false);
        for shown in [None, Some("1".repeat(40)), Some("9".repeat(40))] {
            assert_eq!(
                at(&thread, shown.as_deref()),
                Some((PathBuf::from("a.rs"), 3))
            );
        }
    }

    #[test]
    fn a_thread_that_was_carried_sits_on_its_new_lines_on_disk_and_in_the_version_it_was_carried_to()
     {
        let thread = thread(true);
        let there = Some((PathBuf::from("b.rs"), 7));
        assert_eq!(at(&thread, None), there);
        assert_eq!(at(&thread, Some(&"2".repeat(40))), there);
    }

    #[test]
    fn a_thread_that_was_carried_sits_on_its_anchor_in_the_version_it_was_written_in_and_nowhere_in_another()
     {
        let thread = thread(true);
        assert_eq!(
            at(&thread, Some(&"1".repeat(40))),
            Some((PathBuf::from("a.rs"), 3))
        );
        assert_eq!(at(&thread, Some(&"9".repeat(40))), None);
    }
}
