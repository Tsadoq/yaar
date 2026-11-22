//! The clean-up of recorded states: which states the engine keeps, and when it has the
//! Recorder delete the others (FR-125, FR-126).
//!
//! # What refers to a state
//!
//! A state is referred to, and kept whatever its age, when its full id is one of these, as
//! the Store holds them for the workspace:
//!
//! | Who refers | Where the Store keeps it |
//! | --- | --- |
//! | a thread, where it was written | `Thread::anchor.tree` of every thread of every review of the workspace |
//! | a thread, where it sits now | `Thread::now`'s `ThreadPlace::tree` |
//! | a round | `Round::state` of every round |
//! | a reviewed mark | `ReviewRecord::reviewed` of every review |
//! | a turn that has not ended | `TurnRecord::start` of every turn of the workspace's sessions whose `ended` is none |
//! | the worktree itself | the newest state of the store, while the workspace's worktree is there |
//!
//! The last two are not the three of the requirement; they are what a running turn counts
//! its change from and what the next turn will count from (the Keeper takes a turn's start
//! as the newest state when the turn starts). The owner decided that the newest state of a
//! worktree that is there is kept however old; the rule lives in one function, `newest_kept`,
//! and is shown by one test, so that it can be taken out alone.
//!
//! A state recorded less than `yaar_recorder::SAME_STATE_FOR` ago is kept by both
//! clean-ups too: a call that records a state (a turn's end, a round, a reviewed mark)
//! records it first and names it after, outside the engine's turn, and a state that young
//! may be about to be named.
//!
//! A kept commit (`StateStore::kept_commits`) is referred to when a thread's anchor or
//! placement, a round or a reviewed mark names it.
//!
//! # When a state is deleted
//!
//! - [`crate::Reviews::expire`], by the set time (FR-125), for a workspace whose worktree is
//!   there ([`Presence::Present`]): a state no one refers to is deleted once `keep_for` plus
//!   `yaar_recorder::SAME_STATE_FOR` has passed since it was recorded, as
//!   `StateStore::recorded_times` gives it and the engine's clock reads now
//!   ([`crate::Reviews::with_clock`]); the hour added is the longest a recording gives an
//!   older state back, so no turn's state goes before the set time has passed since the
//!   turn. A state from the future (the clock moved back) is young. A kept commit is never
//!   let go by the set time: a comparison or a reader may be about to read it, and it costs
//!   the room of commits the repository holds anyway.
//! - [`crate::Reviews::clean_up`], at once (FR-126): every state no one refers to, whatever
//!   its age but for the hour above, and every kept commit no one refers to.
//! - A workspace whose worktree is gone ([`Presence::Gone`]): by the set time, its store is
//!   deleted as one, every state referred to or not, once its newest state was recorded at
//!   least `keep_for` plus `yaar_recorder::SAME_STATE_FOR` ago (ADR-011 point 10: the store of
//!   a worktree that is gone is deleted after the same time); until then only what the
//!   present rule deletes is deleted, but for the newest-state referrer, which a gone
//!   worktree has not. A gone worktree's reviews stay: a thread keeps its own text. The
//!   clean-up at once deletes, of a gone worktree, what no one refers to, as for a present
//!   one but with no newest-state referrer.
//!
//! A store's folder is removed, and forgotten by the engine (taken out of the stores it
//! keeps open, so that a later call opens it anew, empty), when it is deleted as one, or
//! when after a clean-up it holds no state and no kept commit and the worktree is gone. A
//! store folder that is a symbolic link (`symlink_metadata`) is never cleaned and never
//! removed: the engine writes one line to the standard error, starting `yaar-keeper:`,
//! naming it, and passes it over. Removal uses `fs::remove_dir_all` only on a real folder.
//!
//! # How, and under which locks
//!
//! In this order:
//! 1. Under the engine's turn of writing (`Reviews::turn`): read what refers (the Store's
//!    reviews, rounds, marks and turns of the workspace), the store's states with their
//!    times (`StateStore::recorded_times`), and for the clean-up at once its kept commits
//!    (`StateStore::kept_commits`); work out the ids to delete; have the Recorder delete
//!    their refs (`StateStore::delete`, a quick run of git). A thread, round or mark kept
//!    meanwhile waits for the turn, so none made while the engine decides is missed; a
//!    state recorded after the read is in no list.
//! 2. With the turn let go: when step 1 deleted something, or for the clean-up at once,
//!    `StateStore::pack`, which removes the objects from disk and may take a while on a
//!    large store. Saves, sends and marks of every workspace go on meanwhile.
//! 3. The room the store takes, before step 1 and after step 2 (`yaar_recorder::sizes` of
//!    its folder), for [`Expired`].
//!
//! A sweep that finds nothing to delete runs no git after step 1's reads.
//!
//! # The room the states take
//!
//! [`crate::Reviews::state_sizes`] gives `yaar_recorder::sizes` of the folder of the state
//! stores, each store's folder mapped back to its workspace by its name, `<workspace>.git`;
//! a folder of another name counts in the total only.

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

/// The unit `st_blocks` counts in.
const BLOCK: u64 = 512;

/// The time a state nothing refers to is kept when the user set none: 30 days (FR-125).
pub const KEEP_STATES: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What one clean-up of a workspace deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expired {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The full ids of the states deleted.
    pub states: Vec<String>,
    /// The room its store took on disk before, and after; after is 0 when the store's
    /// folder was removed.
    pub before: u64,
    /// See `before`.
    pub after: u64,
}

/// How a turn of a workspace's history stands with its recorded state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMark {
    /// Its state is in the store: the turn can be shown.
    Held,
    /// Its state was recorded and deleted since by a clean-up: the turn is expired.
    Expired,
    /// No state was recorded at its end: the recording failed, or it has not ended.
    NotRecorded,
}

/// Whether the worktree of a workspace is there, as the caller finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Its folder is there and is the workspace's.
    Present,
    /// It is gone, moved and not repaired, or cannot be read.
    Gone,
}

/// The room the recorded states take on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRoom {
    /// Each workspace with a state store, by the name of the store's folder, in the order
    /// of their names.
    pub workspaces: Vec<(WorkspaceId, u64)>,
    /// What the folder of every state store takes.
    pub total: u64,
}

/// When a clean-up deletes a state.
#[derive(Clone, Copy)]
enum Rule {
    /// By the set time: once `keep_for` and the hour have passed since it was recorded.
    SetTime(Duration),
    /// At once: once the hour has passed since it was recorded.
    AtOnce,
}

impl Rule {
    /// How long a state must have been recorded to go.
    fn wait(self) -> Duration {
        match self {
            Rule::SetTime(keep_for) => keep_for.saturating_add(SAME_STATE_FOR),
            Rule::AtOnce => SAME_STATE_FOR,
        }
    }
}

/// Whether `recorded` lies at least `wait` before `now`. A state from the future, the clock
/// having moved back, is young.
fn is_old(now: SystemTime, recorded: SystemTime, wait: Duration) -> bool {
    now.duration_since(recorded).is_ok_and(|age| age >= wait)
}

/// The state of a worktree that is there which is kept as the worktree's own, whatever its
/// age: the newest of `recorded`, which is ordered oldest first. A worktree that is gone
/// has none. This is the one place the rule lives.
fn newest_kept(recorded: &[(StateId, SystemTime)], presence: Presence) -> Option<&StateId> {
    match presence {
        Presence::Present => recorded.last().map(|(state, _)| state),
        Presence::Gone => None,
    }
}

/// An error of the file system, as the engine reports it for a folder of the states.
fn on_folder(folder: &Path, e: &io::Error) -> EngineError {
    EngineError::Record(RecordError::Store {
        folder: folder.to_owned(),
        reason: e.to_string(),
    })
}

impl Reviews {
    /// Deletes the states of `workspace` that no one refers to and that were recorded at
    /// least `keep_for` and `yaar_recorder::SAME_STATE_FOR` ago, by the engine's clock, and
    /// the store of a gone worktree as one once it is that old, as the module doc says.
    /// Gives what it deleted; a workspace with no state store deletes nothing and makes
    /// none.
    pub fn expire(
        &self,
        workspace: &WorkspaceId,
        keep_for: Duration,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        self.clean(workspace, Rule::SetTime(keep_for), presence)
    }

    /// Deletes at once every state and kept commit of `workspace` that no one refers to,
    /// whatever its age but for the hour of `yaar_recorder::SAME_STATE_FOR`, then packs the
    /// store, as the module doc says.
    pub fn clean_up(
        &self,
        workspace: &WorkspaceId,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        self.clean(workspace, Rule::AtOnce, presence)
    }

    /// How each turn of `workspace` stands with its recorded state, by the turn's id: every
    /// turn of the workspace's sessions the Store holds.
    pub fn turn_marks(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<BTreeMap<TurnId, TurnMark>, EngineError> {
        // The turns are read before the states: a state a turn names was recorded before the
        // turn was kept, so it is in the list unless a clean-up took it.
        let turns = self.store.turns_of(workspace)?;
        let folder = self.store_folder(workspace);
        let held: BTreeSet<String> = match fs::symlink_metadata(&folder) {
            Ok(_) => self
                .states_of(workspace)?
                .states()?
                .into_iter()
                .map(|state| state.id.as_str().to_owned())
                .collect(),
            // No store: none of the states it held is there.
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

    /// The room the recorded states take on disk, for each workspace and in total, as the
    /// module doc says.
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

    // The folder of the state store of `workspace`.
    fn store_folder(&self, workspace: &WorkspaceId) -> std::path::PathBuf {
        self.states.join(format!("{workspace}.git"))
    }

    // Every full id the Store names for `workspace` as a state or a kept commit that must
    // stay, but for the newest state of the worktree (`newest_kept`): the module doc's table.
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

    // Both clean-ups, as the module doc says.
    fn clean(
        &self,
        workspace: &WorkspaceId,
        rule: Rule,
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

        // Step 1: what refers, what the store holds and how old it is, and the refs deleted,
        // all under the engine's turn.
        let turn = self.turn();
        let referred = self.referred_to(workspace)?;
        let recorded = states.recorded_times()?;
        let kept = if at_once || gone {
            states.kept_commits()?
        } else {
            Vec::new()
        };
        let now = self.clock.now();

        // A gone worktree's store goes as one once its newest state is old enough.
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

        // Step 2: the objects go, with the turn let go.
        let went = !deleted.states.is_empty() || !deleted.commits.is_empty();
        if went || at_once {
            states.pack()?;
        }

        // A store of a gone worktree that holds nothing is removed too.
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

        // Step 3: the room after.
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

    // Removes the folder of a store and forgets it, so that a later call opens one anew,
    // empty. The list of open stores is held meanwhile, so that no call opens the folder
    // while it goes. `store` is let go first: its reader of contents ends with it. Never a
    // link: removed only when it is a folder.
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
