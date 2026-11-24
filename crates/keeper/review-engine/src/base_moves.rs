use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yaar_recorder::{Change, ChangedPath, CommitId, GitError, StateStore, Worktree};
use yaar_store::{NoticeRecord, Side, ThreadPlace};

use crate::engine::{EngineError, Reviews, is_outside};
use crate::follow::{FileStep, Notice, Place, Placed, carry_file};
use crate::{ThreadId, WorkspaceId};

struct Candidate {
    thread: ThreadId,
    /// What the Store held when read; compared before keeping, to skip threads changed meanwhile.
    was: Option<ThreadPlace>,
    at: Placed,
}

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

/// A file git does not list is unchanged, and so is one whose old bytes the store lacks.
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
    /// Gives the ids of the open old-side threads placed on `from` that now name `to`, also those
    /// whose lines stayed. A file that cannot be read keeps its threads; the first such failure is
    /// returned after the others were kept.
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

    /// Also gives the first read failure; fails as a whole only when nothing was kept.
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
