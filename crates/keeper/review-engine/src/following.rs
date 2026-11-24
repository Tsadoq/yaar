use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yaar_recorder::{Change, ChangedPath, GitError, StateId, StateStore};
use yaar_store::{NoticeRecord, Side, ThreadId, ThreadPlace, WorkspaceId};

use crate::engine::{EngineError, Reviews, is_outside};
use crate::follow::{FileStep, Notice, Place, Placed, carry, carry_file};

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

struct Candidate {
    thread: ThreadId,
    /// What the Store held when read; compared before keeping, to skip threads changed meanwhile.
    was: Option<ThreadPlace>,
    from: String,
    at: Placed,
}

impl Reviews {
    /// Gives the threads whose place or notice changed. Only forward, by the order of
    /// `StateStore::states`: two recordings can run at once, and their calls may arrive in the
    /// other order. A thread still on its anchor goes only to the newest state: it writes nothing
    /// when it does not move, so a later call for an older state would carry it back. The turn is
    /// not held while trees are read and diffed; it is taken at the end to drop threads changed
    /// meanwhile.
    pub fn follow(
        &self,
        workspace: &WorkspaceId,
        to: &StateId,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let states = self.states_of(workspace)?;
        let order: HashMap<String, usize> = states
            .states()?
            .into_iter()
            .enumerate()
            .map(|(n, state)| (state.id.as_str().to_owned(), n))
            .collect();
        // A state the store does not list has no place in the order: nothing is carried to it.
        let Some(&to_at) = order.get(to.as_str()) else {
            return Ok(Vec::new());
        };

        let mut candidates = Vec::new();
        for review in self.store.reviews_of(workspace)? {
            for thread in review.threads {
                // A file outside the worktree has no version to carry from, and no diff names it.
                if thread.resolved
                    || thread.anchor.side != Side::New
                    || is_outside(&thread.anchor.path)
                {
                    continue;
                }
                let (from, at) = match &thread.now {
                    Some(now) => (
                        now.tree.clone(),
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
                        thread.anchor.tree.clone(),
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
                // A tree the list does not hold is a commit: only a thread still on its anchor sits
                // in one.
                if from == to.as_str() || order.get(&from).is_some_and(|&n| n >= to_at) {
                    continue;
                }
                let to_is_newest = to_at + 1 == order.len();
                if thread.now.is_none() && !to_is_newest {
                    continue;
                }
                candidates.push(Candidate {
                    thread: thread.id,
                    was: thread.now,
                    from,
                    at,
                });
            }
        }

        let mut failure: Option<EngineError> = None;
        let mut carried: Vec<(usize, Placed)> = Vec::new();
        let mut trees: Vec<&str> = candidates.iter().map(|c| c.from.as_str()).collect();
        trees.sort_unstable();
        trees.dedup();
        for from in trees {
            let changes = match states.changed_paths(from, to.as_str()) {
                Ok(changes) => changes,
                Err(e) => {
                    failure.get_or_insert(e.into());
                    continue;
                }
            };
            let mut files: Vec<(&Path, Vec<usize>)> = Vec::new();
            for (n, candidate) in candidates
                .iter()
                .enumerate()
                .filter(|(_, c)| c.from == from)
            {
                let path = candidate.at.at.path.as_path();
                match files.iter_mut().find(|(file, _)| *file == path) {
                    Some((_, on_it)) => on_it.push(n),
                    None => files.push((path, vec![n])),
                }
            }
            for (path, on_it) in files {
                let step = match step_of(&states, from, to.as_str(), &changes, path) {
                    Ok(step) => step,
                    Err(e) => {
                        failure.get_or_insert(e.into());
                        continue;
                    }
                };
                let placed: Vec<Placed> = on_it.iter().map(|&n| candidates[n].at.clone()).collect();
                for (n, now) in
                    on_it
                        .into_iter()
                        .zip(carry_file(&placed, Side::New, step.as_step()))
                {
                    carried.push((n, now));
                }
            }
        }

        if carried.is_empty() {
            return match failure {
                Some(e) => Err(e),
                None => Ok(Vec::new()),
            };
        }

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
        let mut changed = Vec::new();
        for (n, now) in carried {
            let candidate = &candidates[n];
            if current.get(&candidate.thread) != Some(&candidate.was) {
                continue;
            }
            if now == candidate.at {
                if candidate.was.is_none() {
                    continue;
                }
            } else {
                changed.push(candidate.thread.clone());
            }
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
        match failure {
            Some(e) => Err(e),
            None => Ok(changed),
        }
    }

    /// Nothing is kept. An old-side place or one outside the worktree is returned as it is.
    pub fn carry_between(
        &self,
        workspace: &WorkspaceId,
        placed: &Placed,
        from: &str,
        to: &str,
    ) -> Result<Placed, EngineError> {
        if placed.at.side != Side::New || is_outside(&placed.at.path) {
            return Ok(placed.clone());
        }
        let states = self.states_of(workspace)?;
        let changes = states.changed_paths(from, to)?;
        let step = step_of(&states, from, to, &changes, &placed.at.path)?;
        Ok(carry(placed, Side::New, step.as_step()))
    }

    /// A failure is written down, since the state was taken all the same. Called without the turn.
    pub(crate) fn followed(&self, workspace: &WorkspaceId, state: &StateId) {
        if let Err(e) = self.follow(workspace, state) {
            eprintln!(
                "yaar-keeper: the threads of {workspace} could not follow the state {state}: {e}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[String]) -> Vec<u8> {
        lines
            .iter()
            .flat_map(|l| format!("{l}\n").into_bytes())
            .collect()
    }

    fn on(side: Side, line: u32) -> Placed {
        Placed {
            at: Place {
                path: PathBuf::from("a.rs"),
                side,
                first: line,
                last: line,
            },
            notice: None,
        }
    }

    #[test]
    fn a_thread_on_a_removed_line_stays_put_through_three_rounds_of_changes_to_the_new_side() {
        let mut version: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
        let mut removed = on(Side::Old, 4);
        let mut together = vec![on(Side::Old, 4), on(Side::New, 4)];

        let rounds: [fn(&mut Vec<String>); 3] = [
            |v| v[3] = "rewritten in the first round".to_string(),
            |v| {
                v.insert(0, "added in the second round".to_string());
                v.insert(1, "and another".to_string());
            },
            |v| {
                v.drain(2..7);
                v.push("added in the third round".to_string());
            },
        ];
        for (round, change) in rounds.into_iter().enumerate() {
            let before = text(&version);
            change(&mut version);
            let after = text(&version);
            let step = FileStep::Changed {
                before: &before,
                after: &after,
            };
            removed = carry(&removed, Side::New, step);
            together = carry_file(&together, Side::New, step);

            assert_eq!(removed, on(Side::Old, 4), "after round {}", round + 1);
            assert_eq!(
                together[0],
                on(Side::Old, 4),
                "after round {}, carried with a thread of the new side",
                round + 1
            );
        }
        assert!(
            together[1].notice.is_some(),
            "the changes did touch the new side: {:?}",
            together[1]
        );

        let content = text(&version);
        let renamed = FileStep::Renamed {
            to: Path::new("b.rs"),
            before: &content,
            after: &content,
        };
        for step in [renamed, FileStep::Deleted] {
            removed = carry(&removed, Side::New, step);
            together = carry_file(&together, Side::New, step);
            assert_eq!(removed, on(Side::Old, 4), "after {step:?}");
            assert_eq!(
                together[0],
                on(Side::Old, 4),
                "after {step:?}, with another"
            );
        }
        assert_eq!(
            together[1].notice,
            Some(Notice::Deleted),
            "the file is gone on the new side"
        );
    }
}
