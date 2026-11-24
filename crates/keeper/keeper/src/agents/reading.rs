use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};
use std::sync::Arc;

use yaar_link::{
    ChangedFile, Commit, Content, CreditedFile, FileSides, FileStatus, History, HistoryEntry,
    Refusal, Reply, ReviewId, SessionId, Step, Turn, TurnEnd, TurnId, WorkspaceId,
};
use yaar_recorder::{Change, HistoryEntry as Recorded, RecordError, StateStore, Worktree};
use yaar_review_engine::{Credited, EngineError, uncommitted_of};
use yaar_store::{StepRecord, TurnEndRecord, TurnRecord};

use super::Agents;
use crate::wiring::{Kept, failed, refusal};

/// How many of its first bytes say whether a file is text: git's own number, as for
/// `Request::ReadFile`.
const TEXT_PROBE: usize = 8000;

impl Agents {
    pub(crate) fn turn(&self, kept: &Kept, turn: &TurnId) -> Result<Reply, Refusal> {
        let (record, steps) = kept
            .store
            .turn(&yaar_store::TurnId::new(turn.as_str()))
            .map_err(failed)?
            .ok_or_else(|| Refusal::UnknownTurn { turn: turn.clone() })?;
        Ok(Reply::Turn(Turn {
            id: TurnId::new(record.id.as_str()),
            session: SessionId::new(record.session.as_str()),
            number: record.number,
            ended: record.ended.map(turn_end),
            start: record.start,
            state: record.state,
            steps: steps.into_iter().map(step).collect(),
        }))
    }

    /// The turns are read before the states: a state a turn names was recorded before the turn was
    /// kept, so the other order could show a live turn as expired.
    pub(crate) fn history(&self, kept: &Kept, workspace: &WorkspaceId) -> Result<Reply, Refusal> {
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        let worktree = worktree_needed(kept, &id, workspace)?;
        // The history lists the commits after `counted_from`; the unpushed commits it also gives
        // lie before that one and are Changes' to show.
        let (counted_from, _) = kept.workspaces.counted_from(&id).map_err(refusal)?;
        let turns = kept.store.turns_of(&id).map_err(failed)?;
        let states = states_of(kept, &id)?;
        let recorded =
            yaar_recorder::history(&worktree, &states, counted_from.as_ref()).map_err(failed)?;
        let held: HashSet<String> = states
            .states()
            .map_err(failed)?
            .into_iter()
            .map(|state| state.id.as_str().to_owned())
            .collect();
        Ok(Reply::History(History {
            workspace: workspace.clone(),
            start: counted_from.map(|commit| commit.to_string()),
            entries: with_turns(recorded, &held, turns),
        }))
    }

    pub(crate) fn compare(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        from: &str,
        to: Option<&str>,
    ) -> Result<Reply, Refusal> {
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        let worktree = kept.workspaces.worktree(&id).map_err(refusal)?;
        if to.is_none() && worktree.is_none() {
            return Err(worktree_is_gone());
        }
        let states = states_of(kept, &id)?;
        states
            .compare(worktree.as_deref(), from, to)
            .map(|changed| {
                Reply::Compared(
                    changed
                        .into_iter()
                        .map(|changed| ChangedFile {
                            path: changed.path,
                            status: status(changed.change),
                        })
                        .collect(),
                )
            })
            .map_err(|error| {
                versions_refusal(error, &states, worktree.as_deref(), [Some(from), to])
            })
    }

    pub(crate) fn compare_for_review(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        review: &ReviewId,
        from: &str,
        to: &str,
    ) -> Result<Reply, Refusal> {
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        let worktree = kept.workspaces.worktree(&id).map_err(refusal)?;
        let found = review_of_workspace(kept, &id, review)?;
        let states = states_of(kept, &id)?;
        // The engine reads both versions from the state store alone: a commit is copied into it
        // first, and a version nothing holds is refused.
        states
            .compare(worktree.as_deref(), from, Some(to))
            .map_err(|error| {
                versions_refusal(error, &states, worktree.as_deref(), [Some(from), Some(to)])
            })?;
        let changes = kept
            .reviews
            .credited_change(found.id(), from, to)
            .map_err(failed)?;
        Ok(Reply::ComparedForReview(
            changes
                .into_iter()
                .map(|change| CreditedFile {
                    file: ChangedFile {
                        path: change.path,
                        status: status(change.change),
                    },
                    by: change.by.iter().map(SessionId::new).collect(),
                })
                .collect(),
        ))
    }

    pub(crate) fn read_versions(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        old: Option<(&str, &Path)>,
        new: (Option<&str>, &Path),
    ) -> Result<Reply, Refusal> {
        for asked in old.map(|(_, path)| path).into_iter().chain([new.1]) {
            if leads_out(asked) {
                return Err(Refusal::OutsideWorkspace {
                    path: asked.to_owned(),
                });
            }
        }
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        let worktree = kept.workspaces.worktree(&id).map_err(refusal)?;
        let states = states_of(kept, &id)?;
        let versions = [old.map(|(version, _)| version), new.0];
        let blame = |error| versions_refusal(error, &states, worktree.as_deref(), versions);
        let old = match old {
            Some((version, path)) => states
                .content_in(worktree.as_deref(), version, path)
                .map_err(blame)?,
            None => None,
        };
        let new = match (new.0, &worktree) {
            (Some(version), _) => states
                .content_in(worktree.as_deref(), version, new.1)
                .map_err(blame)?,
            (None, Some(worktree)) => worktree.content_on_disk(new.1).map_err(failed)?,
            (None, None) => return Err(worktree_is_gone()),
        };
        Ok(Reply::File(FileSides {
            old: content(old),
            new: content(new),
        }))
    }

    pub(crate) fn uncommitted(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        review: Option<&ReviewId>,
    ) -> Result<Reply, Refusal> {
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        worktree_needed(kept, &id, workspace)?;
        let review = review
            .map(|review| review_of_workspace(kept, &id, review))
            .transpose()?;
        let credits = kept.workspaces.credits(&id).map_err(refusal)?;
        let changes: Vec<Credited> = credits
            .iter()
            .map(|credit| Credited {
                path: credit.path.clone(),
                by: credit.by.clone(),
            })
            .collect();
        let listed: Vec<&Credited> = match &review {
            Some(review) => uncommitted_of(review, &changes),
            None => changes.iter().collect(),
        };
        let by_path: HashMap<&Path, &yaar_workspaces::CreditedPath> = credits
            .iter()
            .map(|credit| (credit.path.as_path(), credit))
            .collect();
        Ok(Reply::Uncommitted(
            listed
                .into_iter()
                .map(|listed| {
                    let credit = by_path[listed.path.as_path()];
                    CreditedFile {
                        file: ChangedFile {
                            path: credit.path.clone(),
                            status: status(credit.change.clone()),
                        },
                        by: listed.by.iter().map(SessionId::new).collect(),
                    }
                })
                .collect(),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
enum Place {
    After(usize),
    /// Before the history: its state is kept on a commit the history does not reach. The turn is
    /// left out.
    Before,
    Start,
    End,
}

/// Commits stay, a state that is a turn's becomes that turn, any other state is left out.
/// A turn with no place goes after the turn that started before it (at the start if that one
/// is left out), else before the next (left out with it), else at the end.
fn with_turns(
    recorded: Vec<Recorded>,
    held: &HashSet<String>,
    turns: Vec<TurnRecord>,
) -> Vec<HistoryEntry> {
    let at: HashMap<&str, usize> = recorded
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match entry {
            Recorded::State(state) => Some((state.id.as_str(), index)),
            Recorded::Commit(_) => None,
        })
        .collect();
    let turns: Vec<TurnRecord> = turns
        .into_iter()
        .filter(|turn| turn.ended.is_some())
        .collect();
    let known: Vec<Option<Place>> = turns
        .iter()
        .map(|turn| {
            let state = turn.state.as_deref()?;
            match at.get(state) {
                Some(index) => Some(Place::After(*index)),
                None if held.contains(state) => Some(Place::Before),
                None => None,
            }
        })
        .collect();
    let borrowed = |place: Place| match place {
        Place::Before => Place::Start,
        other => other,
    };
    let mut placed = Vec::with_capacity(turns.len());
    let mut before_it = None;
    for (number, place) in known.iter().enumerate() {
        let place = match place.or(before_it.map(borrowed)) {
            Some(place) => {
                before_it = Some(place);
                place
            }
            // No turn before it has a place: it ended before the first turn that has one, so it
            // takes that turn's place and is left out where that one is.
            None => known[number..]
                .iter()
                .flatten()
                .next()
                .copied()
                .unwrap_or(Place::End),
        };
        placed.push(place);
    }

    let mut after: HashMap<usize, Vec<HistoryEntry>> = HashMap::new();
    let mut at_start = Vec::new();
    let mut at_end = Vec::new();
    for (turn, place) in turns.into_iter().zip(placed) {
        let (id, session) = (
            TurnId::new(turn.id.as_str()),
            SessionId::new(turn.session.as_str()),
        );
        let entry = match turn.state {
            Some(state) if held.contains(&state) => HistoryEntry::Turn {
                turn: id,
                session,
                state: Some(state),
            },
            // A state the Store names and the store no longer holds was deleted by a
            // clean-up: the turn stays, marked so. No state named is a recording that failed.
            Some(_) => HistoryEntry::ExpiredTurn { turn: id, session },
            None => HistoryEntry::Turn {
                turn: id,
                session,
                state: None,
            },
        };
        match place {
            Place::After(index) => after.entry(index).or_default().push(entry),
            Place::Before => {}
            Place::Start => at_start.push(entry),
            Place::End => at_end.push(entry),
        }
    }

    let mut entries = at_start;
    for (index, entry) in recorded.into_iter().enumerate() {
        match entry {
            Recorded::Commit(commit) => entries.push(HistoryEntry::Commit(Commit {
                id: commit.id.to_string(),
                summary: commit.summary,
            })),
            Recorded::State(_) => entries.extend(after.remove(&index).into_iter().flatten()),
        }
    }
    entries.extend(at_end);
    entries
}

fn worktree_needed(
    kept: &Kept,
    id: &yaar_workspaces::WorkspaceId,
    workspace: &WorkspaceId,
) -> Result<Arc<Worktree>, Refusal> {
    kept.workspaces
        .worktree(id)
        .map_err(refusal)?
        .ok_or_else(|| Refusal::UnknownWorkspace {
            workspace: workspace.clone(),
        })
}

/// Refused with `UnknownReview` for a review the Keeper does not hold or one of another
/// workspace.
fn review_of_workspace(
    kept: &Kept,
    workspace: &yaar_workspaces::WorkspaceId,
    review: &ReviewId,
) -> Result<yaar_review_engine::Review, Refusal> {
    let unknown = || Refusal::UnknownReview {
        review: review.clone(),
    };
    let found = kept
        .reviews
        .review(&yaar_store::ReviewId::new(review.as_str()))
        .map_err(|error| match error {
            EngineError::UnknownReview(_) => unknown(),
            other => failed(other),
        })?;
    if found.workspace() == workspace {
        Ok(found)
    } else {
        Err(unknown())
    }
}

pub(super) fn states_of(
    kept: &Kept,
    id: &yaar_workspaces::WorkspaceId,
) -> Result<Arc<StateStore>, Refusal> {
    kept.reviews.states_of(id).map_err(failed)
}

/// `UnknownRevision` naming the first version that neither the state store nor the repository
/// holds, else the failure as it was.
pub(super) fn versions_refusal(
    error: RecordError,
    states: &StateStore,
    worktree: Option<&Worktree>,
    versions: [Option<&str>; 2],
) -> Refusal {
    let known = |version: &str| {
        states.paths(version).is_ok()
            || worktree.is_some_and(|worktree| {
                worktree
                    .resolve(version)
                    .ok()
                    .flatten()
                    .is_some_and(|commit| commit.as_str() == version)
            })
    };
    match versions
        .into_iter()
        .flatten()
        .find(|version| !known(version))
    {
        Some(version) => Refusal::UnknownRevision {
            revision: version.to_owned(),
        },
        None => failed(error),
    }
}

fn worktree_is_gone() -> Refusal {
    failed(EngineError::NoWorktree)
}

fn leads_out(path: &Path) -> bool {
    path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
}

fn status(change: Change) -> FileStatus {
    match change {
        Change::Added => FileStatus::Added,
        Change::Modified => FileStatus::Modified,
        Change::Deleted => FileStatus::Deleted,
        Change::Renamed { from } => FileStatus::Renamed { from },
    }
}

fn turn_end(ended: TurnEndRecord) -> TurnEnd {
    match ended {
        TurnEndRecord::Finished => TurnEnd::Finished,
        TurnEndRecord::Interrupted => TurnEnd::Interrupted,
    }
}

fn step(step: StepRecord) -> Step {
    match step {
        StepRecord::Read(path) => Step::Read { path },
        StepRecord::Edit(path, lines) => Step::Edit { path, lines },
        StepRecord::Write(path) => Step::Write { path },
        StepRecord::Command(command) => Step::Command { command },
    }
}

fn content(version: Option<Vec<u8>>) -> Content {
    let Some(bytes) = version else {
        return Content::Absent;
    };
    let size = bytes.len() as u64;
    if bytes.iter().take(TEXT_PROBE).any(|byte| *byte == 0) {
        Content::Binary { size }
    } else if size > yaar_link::MAX_CONTENT {
        Content::TooLarge { size }
    } else {
        Content::Text(bytes)
    }
}
