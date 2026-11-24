//! A state is kept whatever its age when the Store names it (a thread's anchor or place, a round's
//! state, a reviewed mark, the start of an unended turn), when it is the newest of a present
//! worktree, or when it is younger than `SAME_STATE_FOR` (an hour): a call records a state and
//! names it after. `expire` deletes the rest older than `keep_for` plus that hour (the longest a
//! recording gives an older state back), and a gone worktree's whole store once its newest state is
//! that old. Otherwise kept commits are never let go by the set time: a comparison or reader may be
//! about to read one, and it costs only room the repository already holds. Order: under the turn,
//! read what refers and the states, work out the ids and have the Recorder delete their refs; with
//! the turn released, pack; then measure the room. Deciding under the turn means a thread, round or
//! mark kept meanwhile waits and is never missed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::time::{Duration, SystemTime};

use yaar_recorder::{Deleted, RecordError, SAME_STATE_FOR, StateId, sizes};
use yaar_store::TurnId;

use crate::WorkspaceId;
use crate::engine::{EngineError, Reviews};

/// `st_blocks` counts in 512-byte units, whatever the file system's block size.
const BLOCK: u64 = 512;

pub const KEEP_STATES: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expired {
    pub workspace: WorkspaceId,
    pub states: Vec<String>,
    /// The room its store took on disk before and after; after is 0 when the folder was removed.
    pub before: u64,
    pub after: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMark {
    Held,
    /// Its state was recorded and deleted since by a clean-up.
    Expired,
    /// No state was recorded at its end: the recording failed, or it has not ended.
    NotRecorded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Present,
    /// The worktree is gone, moved and not repaired, or cannot be read.
    Gone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRoom {
    pub workspaces: Vec<(WorkspaceId, u64)>,
    pub total: u64,
}

#[derive(Clone, Copy)]
enum Rule {
    SetTime(Duration),
    AtOnce,
}

impl Rule {
    fn wait(self) -> Duration {
        match self {
            Rule::SetTime(keep_for) => keep_for.saturating_add(SAME_STATE_FOR),
            Rule::AtOnce => SAME_STATE_FOR,
        }
    }
}

/// A state from the future, the clock having moved back, is young.
fn is_old(now: SystemTime, recorded: SystemTime, wait: Duration) -> bool {
    now.duration_since(recorded).is_ok_and(|age| age >= wait)
}

fn newest_kept(recorded: &[(StateId, SystemTime)], presence: Presence) -> Option<&StateId> {
    match presence {
        Presence::Present => recorded.last().map(|(state, _)| state),
        Presence::Gone => None,
    }
}

fn on_folder(folder: &Path, e: &io::Error) -> EngineError {
    EngineError::Record(RecordError::Store {
        folder: folder.to_owned(),
        reason: e.to_string(),
    })
}

impl Reviews {
    pub fn expire(
        &self,
        workspace: &WorkspaceId,
        keep_for: Duration,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        self.clean(workspace, Rule::SetTime(keep_for), None, presence)
    }

    /// [`Reviews::expire`] counting ages from `now`, for a sweep that judges other things by the
    /// time it began.
    pub fn expire_at(
        &self,
        workspace: &WorkspaceId,
        keep_for: Duration,
        now: SystemTime,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        self.clean(workspace, Rule::SetTime(keep_for), Some(now), presence)
    }

    pub fn clean_up(
        &self,
        workspace: &WorkspaceId,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        self.clean(workspace, Rule::AtOnce, None, presence)
    }

    pub fn turn_marks(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<BTreeMap<TurnId, TurnMark>, EngineError> {
        // Turns are read before states: a state a turn names was recorded before the turn was kept,
        // so it is in the list unless a clean-up took it.
        let turns = self.store.turns_of(workspace)?;
        let folder = self.store_folder(workspace);
        let held: BTreeSet<String> = match fs::symlink_metadata(&folder) {
            Ok(_) => self
                .states_of(workspace)?
                .states()?
                .into_iter()
                .map(|state| state.id.as_str().to_owned())
                .collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(on_folder(&folder, &e)),
        };
        Ok(turns
            .into_iter()
            .map(|turn| {
                let mark = match &turn.state {
                    None => TurnMark::NotRecorded,
                    Some(state) if held.contains(state) => TurnMark::Held,
                    Some(_) => TurnMark::Expired,
                };
                (turn.id, mark)
            })
            .collect())
    }

    pub fn state_sizes(&self) -> Result<StoreRoom, EngineError> {
        let found = sizes(&self.states).map_err(|e| on_folder(&self.states, &e))?;
        let workspaces = found
            .stores
            .into_iter()
            .filter_map(|(name, room)| {
                let id = name.to_str()?.strip_suffix(".git")?;
                (!id.is_empty()).then(|| (WorkspaceId::new(id), room))
            })
            .collect();
        Ok(StoreRoom {
            workspaces,
            total: found.total,
        })
    }

    fn store_folder(&self, workspace: &WorkspaceId) -> std::path::PathBuf {
        self.states.join(format!("{workspace}.git"))
    }

    fn referred_to(&self, workspace: &WorkspaceId) -> Result<BTreeSet<String>, EngineError> {
        let mut named = BTreeSet::new();
        for review in self.store.reviews_of(workspace)? {
            named.extend(review.reviewed);
            named.extend(review.rounds.into_iter().filter_map(|round| round.state));
            for thread in review.threads {
                named.insert(thread.anchor.tree);
                named.extend(thread.now.map(|place| place.tree));
            }
        }
        for turn in self.store.turns_of(workspace)? {
            if turn.ended.is_none() {
                named.extend(turn.start);
            }
        }
        Ok(named)
    }

    fn clean(
        &self,
        workspace: &WorkspaceId,
        rule: Rule,
        at: Option<SystemTime>,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        let folder = self.store_folder(workspace);
        let nothing = |room| Expired {
            workspace: workspace.clone(),
            states: Vec::new(),
            before: room,
            after: room,
        };
        match fs::symlink_metadata(&folder) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(nothing(0)),
            Err(e) => return Err(on_folder(&folder, &e)),
            Ok(meta) if meta.file_type().is_symlink() => {
                eprintln!(
                    "yaar-keeper: the state store {} is a link: it is not cleaned up",
                    folder.display()
                );
                return Ok(nothing(meta.blocks() * BLOCK));
            }
            Ok(_) => {}
        }
        let before = sizes(&folder).map_err(|e| on_folder(&folder, &e))?.total;
        let states = self.states_of(workspace)?;
        let at_once = matches!(rule, Rule::AtOnce);
        let gone = presence == Presence::Gone;

        let turn = self.turn();
        let referred = self.referred_to(workspace)?;
        let recorded = states.recorded_times()?;
        let kept = if at_once || gone {
            states.kept_commits()?
        } else {
            Vec::new()
        };
        let now = at.unwrap_or_else(|| self.clock.now());

        let store_is_old = matches!(rule, Rule::SetTime(_))
            && gone
            && recorded
                .last()
                .is_some_and(|(_, at)| is_old(now, *at, rule.wait()));
        if store_is_old {
            drop(turn);
            let states_in_it = recorded
                .iter()
                .map(|(state, _)| state.as_str().to_owned())
                .collect();
            self.remove_store(workspace, states, &folder)?;
            return Ok(Expired {
                workspace: workspace.clone(),
                states: states_in_it,
                before,
                after: 0,
            });
        }

        let newest = newest_kept(&recorded, presence);
        let mut doomed: BTreeSet<String> = recorded
            .iter()
            .filter(|(state, at)| {
                !referred.contains(state.as_str())
                    && Some(state) != newest
                    && is_old(now, *at, rule.wait())
            })
            .map(|(state, _)| state.as_str().to_owned())
            .collect();
        if at_once {
            doomed.extend(kept.iter().filter(|id| !referred.contains(*id)).cloned());
        }
        let deleted = if doomed.is_empty() {
            Deleted::default()
        } else {
            states.delete(&doomed)?
        };
        drop(turn);

        let went = !deleted.states.is_empty() || !deleted.commits.is_empty();
        if went || at_once {
            states.pack()?;
        }

        let left = recorded
            .iter()
            .any(|(state, _)| !deleted.states.contains(&state.as_str().to_owned()))
            || kept.iter().any(|id| !deleted.commits.contains(id));
        if gone && !left {
            self.remove_store(workspace, states, &folder)?;
            return Ok(Expired {
                workspace: workspace.clone(),
                states: deleted.states,
                before,
                after: 0,
            });
        }
        drop(states);

        let after = if went || at_once {
            sizes(&folder).map_err(|e| on_folder(&folder, &e))?.total
        } else {
            before
        };
        Ok(Expired {
            workspace: workspace.clone(),
            states: deleted.states,
            before,
            after,
        })
    }

    // The list of open stores is held meanwhile, so no call opens the folder while it goes. `store`
    // is dropped first: its content reader ends with it. Removes only a real folder, never a link.
    fn remove_store(
        &self,
        workspace: &WorkspaceId,
        store: std::sync::Arc<yaar_recorder::StateStore>,
        folder: &Path,
    ) -> Result<(), EngineError> {
        drop(store);
        let mut opened = self
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        opened.remove(workspace);
        match fs::symlink_metadata(folder) {
            Ok(meta) if meta.is_dir() => {
                fs::remove_dir_all(folder).map_err(|e| on_folder(folder, &e))
            }
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(on_folder(folder, &e)),
        }
    }
}
