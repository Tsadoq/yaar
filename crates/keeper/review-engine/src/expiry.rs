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
//! The last two are not FR-125's three; they are what a running turn counts its change from
//! and what the next turn will count from (the Keeper takes a turn's start as the newest
//! state when the turn starts). Whether the newest state may outlive the set time is put to
//! Matteo (`DECISIONS.md`, "Blockers"); until he answers it is kept.
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

use std::collections::BTreeMap;
use std::time::Duration;

use yaar_store::TurnId;

use crate::WorkspaceId;
use crate::engine::{EngineError, Reviews};

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
        let _ = (workspace, keep_for, presence);
        todo!("delete what the set time and the references allow, as the module doc says")
    }

    /// Deletes at once every state and kept commit of `workspace` that no one refers to,
    /// whatever its age but for the hour of `yaar_recorder::SAME_STATE_FOR`, then packs the
    /// store, as the module doc says.
    pub fn clean_up(
        &self,
        workspace: &WorkspaceId,
        presence: Presence,
    ) -> Result<Expired, EngineError> {
        let _ = (workspace, presence);
        todo!("delete what no one refers to, as the module doc says")
    }

    /// How each turn of `workspace` stands with its recorded state, by the turn's id: every
    /// turn of the workspace's sessions the Store holds.
    pub fn turn_marks(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<BTreeMap<TurnId, TurnMark>, EngineError> {
        let _ = workspace;
        todo!("held, expired or not recorded, for each turn")
    }

    /// The room the recorded states take on disk, for each workspace and in total, as the
    /// module doc says.
    pub fn state_sizes(&self) -> Result<StoreRoom, EngineError> {
        todo!("the sizes of the folder of the state stores")
    }
}
