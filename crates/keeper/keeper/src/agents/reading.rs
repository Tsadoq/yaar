//! What a Window reads of the agents' side but the agents themselves: one turn, the
//! history of a worktree, its uncommitted changes as a review lists them, and two
//! versions of a change. Each is read from the Store, the state stores, the Review engine
//! and Workspaces, through `Kept`, and from nothing of this side's thread.

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
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
use crate::wiring::{Kept, refusal};

/// How many of its first bytes say whether a file is text: git's own number, as for
/// `Request::ReadFile`.
const TEXT_PROBE: usize = 8000;

impl Agents {
    /// `Request::GetTurn`.
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
            state: record.state,
            steps: steps.into_iter().map(step).collect(),
        }))
    }

    /// `Request::GetHistory`: the commits since the one the changes are counted from and
    /// the turns that ended on them, read from the state store, the commits and the Store.
    /// No `git status` runs.
    pub(crate) fn history(&self, kept: &Kept, workspace: &WorkspaceId) -> Result<Reply, Refusal> {
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        let worktree = worktree_needed(kept, &id, workspace)?;
        // The history lists the commits after the one the changes are counted from. The
        // commits `counted_from` gives besides it, those not on a remote, come before
        // that commit: they are Changes' to show, and no entry of this history.
        let (counted_from, _) = kept.workspaces.counted_from(&id).map_err(refusal)?;
        let states = states_of(kept, &id)?;
        let recorded =
            yaar_recorder::history(&worktree, &states, counted_from.as_ref()).map_err(failed)?;
        let held: HashSet<String> = states
            .states()
            .map_err(failed)?
            .into_iter()
            .map(|state| state.id.as_str().to_owned())
            .collect();
        let turns = kept.store.turns_of(&id).map_err(failed)?;
        Ok(Reply::History(History {
            workspace: workspace.clone(),
            start: counted_from.map(|commit| commit.to_string()),
            entries: with_turns(recorded, &held, turns),
        }))
    }

    /// `Request::CompareVersions`.
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

    /// `Request::ReadVersions`: each side at its own path, in a version of the state store
    /// or on disk.
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

    /// `Request::GetUncommitted`: the review's list, by `yaar_review_engine::uncommitted_of`
    /// over `Workspaces::credits`.
    pub(crate) fn uncommitted(
        &self,
        kept: &Kept,
        workspace: &WorkspaceId,
        review: Option<&ReviewId>,
    ) -> Result<Reply, Refusal> {
        let id = yaar_workspaces::WorkspaceId::new(workspace.as_str());
        // The workspace first, which costs no git: a refusal of the review is no later
        // for it.
        worktree_needed(kept, &id, workspace)?;
        let review = review
            .map(|review| {
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
                // A review lists the changes of its own workspace.
                if found.workspace() == &id {
                    Ok(found)
                } else {
                    Err(unknown())
                }
            })
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

/// Where a turn goes among the entries of the recorder.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// After the state at this index of the recorder's entries.
    After(usize),
    /// Before the start of the history: its state is kept, on a commit the history does
    /// not reach. A turn that has this place is left out.
    Before,
    /// Before the first entry: where a turn with no place of its own goes when the turn it
    /// follows is left out as before the start.
    Start,
    /// After the last entry.
    End,
}

/// The entries of the recorder with the turns that ended put among them: a commit stays, a
/// state that is a turn's becomes that turn (the turns that ended on one state, in the
/// order they started), any other state is left out.
///
/// A turn with no state, or whose state is not held, has no place of its own: it goes right
/// after the turn that started before it, or, when none did, before the turn that started
/// after it, or, with none known, at the end. When the turn it follows is left out as before
/// the start, the place is the start of the history: it ended after that turn. When the turn
/// it precedes is left out, so is it: it ended before that turn. A turn that has not ended
/// is not listed.
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
    // The place of a turn that borrows one from the turn before it: a turn left out is no
    // place to go after, and the turn that follows it ended after the start, as far as is
    // known.
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
            // No turn before it has a place: it started before the first turn that has one,
            // so it ended before it, and where that one is left out so is this one. This
            // place is not one to borrow from.
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
        let entry = HistoryEntry::Turn {
            turn: TurnId::new(turn.id.as_str()),
            session: SessionId::new(turn.session.as_str()),
            state: turn.state.filter(|state| held.contains(state)),
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

/// The worktree of a workspace; refused with `UnknownWorkspace` when the Keeper does not
/// hold the workspace and when its worktree is gone.
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

/// The state store of a workspace.
fn states_of(kept: &Kept, id: &yaar_workspaces::WorkspaceId) -> Result<Arc<StateStore>, Refusal> {
    kept.reviews.states_of(id).map_err(failed)
}

/// The refusal of a comparison or a reading that failed: `UnknownRevision` naming the first
/// version that is neither held by the state store nor a commit of the repository, else
/// the failure as it was.
fn versions_refusal(
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

/// What is refused for the files on disk of a worktree that is gone.
fn worktree_is_gone() -> Refusal {
    failed(EngineError::NoWorktree)
}

/// A failure the user cannot mend from here, as a sentence.
fn failed(error: impl Display) -> Refusal {
    Refusal::Failed {
        reason: error.to_string(),
    }
}

/// Whether a path, meant to be counted from the folder of a worktree, leads out of it by
/// its own words.
fn leads_out(path: &Path) -> bool {
    path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
}

/// What happened to a file, in the words of the Link.
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

/// One version of a file as far as it travels, as `Request::ReadFile` sends it: its bytes
/// when it is text of a size the Link carries, else its size alone. What is not text is
/// said to be so whatever its size.
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
