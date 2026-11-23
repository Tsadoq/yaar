//! Threads on the old side when the base moves (ADR-013 point 4).
//!
//! A thread on a removed line sits on the version the change is counted from: its anchor's
//! `tree` is that commit, its side old. While the base stands it does not move. When the
//! commit the changes of a workspace are counted from changes (the user set another base,
//! or the base branch, or the fork point, moved), [`crate::Reviews::base_moved`] carries
//! each open thread on the old side placed on the old commit to the new one, by the rule
//! the new side's threads follow ([`crate::carry_file`], one file at a time): with the
//! file's text in `from` and in `to`, read through the state store, which keeps both
//! commits (`StateStore::content_in`, which copies a commit it lacks from the worktree).
//!
//! What moves, and what does not:
//!
//! | Thread | What happens |
//! | --- | --- |
//! | On the old side, open, placed on `from` (its `now` is that commit, or with no `now` its anchor's `tree` is) | carried to `to`: its new placement is kept with `Store::set_places`, on the old side, with the notice the follower gives |
//! | On the old side, resolved | stays where it is |
//! | On the old side, placed on another commit | stays where it is: it is not on the base that moved |
//! | On the new side | not touched: the new side's threads follow the states |
//! | With an absolute path (a file outside the worktree) | not touched: it is never carried |
//!
//! The placements are kept in one call of the Store, under the engine's turn of writing.
//! Every thread kept is given back, also one whose lines stayed where they were: its place
//! now names `to`, which is what the next move of the base looks for. When at least one was
//! kept, the Keeper tells `ReviewsChanged` of the workspace. A `from` equal to `to` moves
//! nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yaar_recorder::{Change, ChangedPath, CommitId, GitError, StateStore, Worktree};
use yaar_store::{NoticeRecord, Side, ThreadPlace};

use crate::engine::{EngineError, Reviews, is_outside};
use crate::follow::{FileStep, Notice, Place, Placed, carry_file};
use crate::{ThreadId, WorkspaceId};

/// One thread to be carried: where it sits on the old commit, as read.
struct Candidate {
    thread: ThreadId,
    /// What the Store held of its place when it was read; compared when it is kept.
    was: Option<ThreadPlace>,
    at: Placed,
}

/// What one file became between the two commits, with the bytes it needs.
enum Step {
    Unchanged,
    Changed {
        before: Vec<u8>,
        after: Vec<u8>,
    },
    Renamed {
        to: PathBuf,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    Deleted,
}

impl Step {
    fn as_step(&self) -> FileStep<'_> {
        match self {
            Step::Unchanged => FileStep::Unchanged,
            Step::Changed { before, after } => FileStep::Changed { before, after },
            Step::Renamed { to, before, after } => FileStep::Renamed { to, before, after },
            Step::Deleted => FileStep::Deleted,
        }
    }
}

/// What became of the file at `path` of `from` in `to`, as `changes`, the paths that differ
/// with renames paired, says. A file git does not list is unchanged, and so is one whose
/// old bytes the store does not hold, which cannot be diffed.
fn step_of(
    states: &StateStore,
    from: &str,
    to: &str,
    changes: &[ChangedPath],
    path: &Path,
) -> Result<Step, GitError> {
    let named = changes.iter().find(|c| c.path == path);
    let moved = changes
        .iter()
        .find(|c| matches!(&c.change, Change::Renamed { from } if from == path));
    let step = match (named, moved) {
        (Some(c), _) if c.change == Change::Deleted => Step::Deleted,
        (Some(c), _) if c.change == Change::Modified => {
            match (states.content(from, path)?, states.content(to, path)?) {
                (Some(before), Some(after)) => Step::Changed { before, after },
                (Some(_), None) => Step::Deleted,
                (None, _) => Step::Unchanged,
            }
        }
        (_, Some(c)) => match (states.content(from, path)?, states.content(to, &c.path)?) {
            (Some(before), Some(after)) => Step::Renamed {
                to: c.path.clone(),
                before,
                after,
            },
            _ => Step::Unchanged,
        },
        _ => Step::Unchanged,
    };
    Ok(step)
}

impl Reviews {
    /// The commit the changes of `workspace` are counted from moved from `from` to `to`:
    /// carries its threads on the old side, as the module doc says, and gives the ids of the
    /// threads whose place now names `to`, whether their lines moved or stayed.
    ///
    /// The commits are read through the workspace's state store, which copies from
    /// `worktree` the one it lacks; when that fails nothing is carried. A file that cannot
    /// be read leaves its threads where they are, the others are carried, and the first
    /// such failure is given after they were kept (so their ids are not: see
    /// [`Reviews::base_moved_in_part`], which gives both). The engine's turn is held only
    /// to read the threads again, leave out any whose place changed meanwhile (another call
    /// carried it, or the reviewer resolved it) and keep the rest with one
    /// `Store::set_places`.
    pub fn base_moved(
        &self,
        workspace: &WorkspaceId,
        worktree: &Worktree,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<ThreadId>, EngineError> {
        match self.base_moved_in_part(workspace, worktree, from, to)? {
            (moved, None) => Ok(moved),
            (_, Some(failure)) => Err(failure),
        }
    }

    /// [`Reviews::base_moved`] for a caller that tells others what was kept: the ids of the
    /// threads whose place now names `to`, and with them the first failure to read a file,
    /// when there was one, whose threads stay where they were. It fails as a whole only
    /// when nothing was kept.
    pub fn base_moved_in_part(
        &self,
        workspace: &WorkspaceId,
        worktree: &Worktree,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<(Vec<ThreadId>, Option<EngineError>), EngineError> {
        if from == to {
            return Ok((Vec::new(), None));
        }
        let mut candidates = Vec::new();
        for review in self.store.reviews_of(workspace)? {
            for thread in review.threads {
                let (tree, at) = match &thread.now {
                    Some(now) => (
                        &now.tree,
                        Placed {
                            at: Place {
                                path: now.path.clone(),
                                side: now.side,
                                first: now.first,
                                last: now.last,
                            },
                            notice: now.notice.map(notice_of),
                        },
                    ),
                    None => (
                        &thread.anchor.tree,
                        Placed {
                            at: Place {
                                path: thread.anchor.path.clone(),
                                side: thread.anchor.side,
                                first: thread.anchor.first,
                                last: thread.anchor.last,
                            },
                            notice: None,
                        },
                    ),
                };
                if thread.resolved
                    || at.at.side != Side::Old
                    || is_outside(&at.at.path)
                    || tree != from.as_str()
                {
                    continue;
                }
                candidates.push(Candidate {
                    thread: thread.id,
                    was: thread.now,
                    at,
                });
            }
        }
        if candidates.is_empty() {
            return Ok((Vec::new(), None));
        }

        let states = self.states_of(workspace)?;
        let changes = states.compare(Some(worktree), from.as_str(), Some(to.as_str()))?;
        let mut failure: Option<EngineError> = None;
        let mut files: HashMap<&Path, Vec<usize>> = HashMap::new();
        for (n, candidate) in candidates.iter().enumerate() {
            files
                .entry(candidate.at.at.path.as_path())
                .or_default()
                .push(n);
        }
        let mut carried: Vec<(usize, Placed)> = Vec::new();
        for (path, on_it) in files {
            let step = match step_of(&states, from.as_str(), to.as_str(), &changes, path) {
                Ok(step) => step,
                Err(e) => {
                    failure.get_or_insert(e.into());
                    continue;
                }
            };
            let placed: Vec<Placed> = on_it.iter().map(|&n| candidates[n].at.clone()).collect();
            carried.extend(
                on_it
                    .into_iter()
                    .zip(carry_file(&placed, Side::Old, step.as_step())),
            );
        }

        let mut moved = Vec::new();
        if !carried.is_empty() {
            let _turn = self.turn();
            let mut current: HashMap<ThreadId, Option<ThreadPlace>> = HashMap::new();
            for review in self.store.reviews_of(workspace)? {
                for thread in review.threads {
                    if !thread.resolved {
                        current.insert(thread.id, thread.now);
                    }
                }
            }
            let mut kept = Vec::new();
            for (n, now) in carried {
                let candidate = &candidates[n];
                if current.get(&candidate.thread) != Some(&candidate.was) {
                    continue;
                }
                moved.push(candidate.thread.clone());
                kept.push((
                    candidate.thread.clone(),
                    Some(ThreadPlace {
                        tree: to.as_str().to_owned(),
                        path: now.at.path,
                        side: now.at.side,
                        first: now.at.first,
                        last: now.at.last,
                        notice: now.notice.map(record_of),
                    }),
                ));
            }
            self.store.set_places(&kept)?;
        }
        Ok((moved, failure))
    }
}

fn notice_of(record: NoticeRecord) -> Notice {
    match record {
        NoticeRecord::Changed => Notice::Changed,
        NoticeRecord::Deleted => Notice::Deleted,
    }
}

fn record_of(notice: Notice) -> NoticeRecord {
    match notice {
        Notice::Changed => NoticeRecord::Changed,
        Notice::Deleted => NoticeRecord::Deleted,
    }
}
