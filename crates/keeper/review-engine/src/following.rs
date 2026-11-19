//! Carrying every thread of a workspace to each new state the engine records, one state
//! at a time, and between any two trees: commits and recorded states alike, which only
//! the Recorder tells apart. Built by the engine's package "following through rounds" of
//! slice 6.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yaar_recorder::{Change, ChangedPath, GitError, StateId, StateStore};
use yaar_store::{NoticeRecord, Side, ThreadId, ThreadPlace, WorkspaceId};

use crate::engine::{EngineError, Reviews};
use crate::follow::{FileStep, Notice, Place, Placed, carry, carry_file};

/// What one file became between two trees, with the bytes it needs: [`FileStep`] owns
/// nothing, and this holds what it points at.
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

/// What became of the file at `path` of tree `from` in tree `to`, as `changes`, the paths
/// that differ between the two with renames paired, says; the bytes are read from the
/// state store. A file git does not list is unchanged, and so is one whose old bytes the
/// store does not hold, which cannot be diffed.
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

/// One thread to be carried: where it sits and in which tree, as read.
struct Candidate {
    thread: ThreadId,
    /// What the Store held of its place when it was read; compared when it is kept.
    was: Option<ThreadPlace>,
    /// The tree it sits in.
    from: String,
    at: Placed,
}

impl Reviews {
    /// Carries every thread of every review of `workspace` that is not resolved and sits on
    /// the new side to the recorded state `to`, from the tree it sits in now (where it was
    /// carried last, else its anchor's), by [`crate::follow::carry_file`] over the files of
    /// the two trees as the workspace's state store reads them and pairs their renames;
    /// keeps where each one sits now, but for a thread that has no place yet and sits where
    /// its anchor says, which has none to keep. Gives the threads whose place or notice
    /// changed.
    ///
    /// A thread is carried only forward: to a `to` newer than the tree it sits in, by the
    /// order of the state store's list (`yaar_recorder::StateStore::states`), whose order
    /// is the order the states were recorded. A thread that sits in `to` already, or in a
    /// state newer than `to`, is left: two recordings of one workspace can run at once (a
    /// turn's end and a new thread's first save, say), and their calls of this may come in
    /// the other order. A thread that sits on its anchor, in a commit, is carried to any
    /// state.
    ///
    /// The engine's turn is not held while the trees are read and diffed, which may take
    /// long with many threads: a report of the agent, which waits for the engine's turn,
    /// must not wait for that. The turn is taken only at the end, to read the threads
    /// again, leave out each one whose place changed meanwhile (another call carried it,
    /// or the reviewer resolved it), and keep the rest with one `Store::set_places`.
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
        // A state the store does not list has no place in the order: nothing is carried
        // to it.
        let Some(&newest) = order.get(to.as_str()) else {
            return Ok(Vec::new());
        };

        // What there is to carry, read without the turn.
        let mut candidates = Vec::new();
        for review in self.store.reviews_of(workspace)? {
            for thread in review.threads {
                if thread.resolved || thread.anchor.side != Side::New {
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
                // Forward only: a tree that is a state is carried from when it is older
                // than `to`; a tree that is not one is a commit, carried from to any state.
                if from == to.as_str() || order.get(&from).is_some_and(|&n| n >= newest) {
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

        // The trees are read and diffed once for each tree the threads sit in, and each file
        // once for all the threads on it. A tree that cannot be read leaves its threads
        // where they are; the first such failure is given when the others are kept.
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

        // The turn: the threads are read again, and one that was resolved, deleted or
        // carried meanwhile is left out.
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
                // A thread that sits where its anchor says, and was not placed anywhere
                // else before, has no place to keep.
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

    /// Where a thread placed at `placed` in tree `from` sits in tree `to`, two versions of
    /// `workspace` read through its state store: recorded states or commits it holds,
    /// whichever each is. Nothing is kept.
    pub fn carry_between(
        &self,
        workspace: &WorkspaceId,
        placed: &Placed,
        from: &str,
        to: &str,
    ) -> Result<Placed, EngineError> {
        // A thread on the old side stays where it is whatever the trees are: nothing is read.
        if placed.at.side != Side::New {
            return Ok(placed.clone());
        }
        let states = self.states_of(workspace)?;
        let changes = states.changed_paths(from, to)?;
        let step = step_of(&states, from, to, &changes, &placed.at.path)?;
        Ok(carry(placed, Side::New, step.as_step()))
    }

    /// The engine took the state `state` of `workspace`: every thread is carried to it, by
    /// [`Reviews::follow`], and a failure is written down, since the state was taken all
    /// the same. Called after the state was recorded, without the engine's turn.
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

    // THREAD-06
    #[test]
    fn a_thread_on_a_removed_line_stays_put_through_three_rounds_of_changes_to_the_new_side() {
        let mut version: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
        let mut removed = on(Side::Old, 4);
        // A thread on the new side, on the line that stands where the removed one stood,
        // as the control: the same changes do move and notice a thread on the new side.
        let mut control = on(Side::New, 4);

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
            control = carry(&control, Side::New, step);

            assert_eq!(removed, on(Side::Old, 4), "after round {}", round + 1);
        }
        assert!(
            control.notice.is_some(),
            "the changes did touch the new side: {control:?}"
        );
    }
}
