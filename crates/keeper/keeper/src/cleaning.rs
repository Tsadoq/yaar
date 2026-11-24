//! The clean-up of the recorded states, between the Link, the Review engine and the
//! Adapters: the sizes a Window asks for, the clean-up the user asks for, and the one the
//! set time makes, the sweep.
//!
//! | Request | Answered by | Facts |
//! | --- | --- | --- |
//! | `GetStateSizes` | `Reviews::state_sizes`, each store mapped to a workspace the Store holds, in the order the workspaces were first kept; a store of no workspace the Store holds counts in the total only | none |
//! | `CleanUpStates` | for every workspace the Store holds, `Reviews::clean_up`, `Presence::Gone` when `Workspaces::worktree` finds none, else `Presence::Present`; then `Agents::remove_files_of_ended` with no time (every file of a session that does not run); then the sizes, as `GetStateSizes` | `StatesDeleted` of each workspace that lost a state, after the reply |
//!
//! Both are answered apart, on a thread of their own: a clean-up packs every store, and a
//! size walks every folder of states.
//!
//! One workspace is cleaned by one caller at a time, the sweep's or the user's: a sweep that
//! comes due while a clean-up packs waits for it, for it would find the emptied store of a
//! gone worktree and remove its folder under the pack.
//!
//! A workspace that cannot be cleaned up (the Store or git fails for it) does not hold up the
//! others: each failure is written to the standard error in one line, starting `yaar-keeper:`,
//! the `StatesDeleted` of the workspaces that did lose states are told all the same, and the
//! first failure is what the reply says, `Refusal::Failed`, in place of the sizes.
//!
//! # The sweep by the set time
//!
//! [`Sweeps::start`] starts one thread at the Keeper's start. It wakes every [`TICK`] of
//! real time, waiting on a channel with that timeout (never in a plain sleep), so that
//! dropping [`Sweeps`] wakes it and ends it at once; `Drop` joins it. At each wake it reads
//! the engine's clock (the clock the Keeper was started with, `run_with_clock`) and the set
//! time, and sweeps when all of these hold:
//!
//! - the Keeper has queued a reply to a Window since it started (`Shared::has_answered`): the
//!   first sweep does not slow the first Window's start;
//! - and either no sweep ran yet, or the clock reads at least [`SWEEP_EVERY`] past the
//!   clock's time of the last sweep, or the set time differs from the one the last sweep
//!   used (the user changed it: the new time is in force at once).
//!
//! The set time is `Setting::KeepStates` of the Store's settings, in days, or
//! `yaar_review_engine::KEEP_STATES` when the user set none. A sweep:
//!
//! 1. for every workspace the Store holds, calls `Reviews::expire` with that time,
//!    `Presence::Gone` or `Present` as `Workspaces::worktree` finds it, under the Keeper's
//!    parts read for that one workspace (`shared.parts.read()`, let go before the next: a
//!    request that waits for the parts to be written, as the Keeper's end, waits for one
//!    workspace's sweep at the most; the parts may be gone, and the sweep then ends);
//! 2. has the Adapters remove the files of sessions that do not run and whose files were
//!    last written before the clock's now less the set time
//!    (`Agents::remove_files_of_ended` with that time);
//! 3. tells every Window `StatesDeleted` of each workspace that lost a state.
//!
//! The files are removed before the Windows are told. No surface holds a sweep between the
//! two steps, so the order is stated and no test holds it.
//!
//! [`SWEEP_EVERY`] and the age of a state are the clock's; only [`TICK`], how often the
//! thread looks, is real time. A test moves its clock past the set time, or changes the set
//! time, and waits for the sweep's `StatesDeleted` or for `GetHistory` to show the turn
//! expired. A failure of one workspace's sweep is written to the standard error in one
//! line, starting `yaar-keeper:`, and the others go on.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use yaar_link::{Fact, Refusal, Reply, StateSize, StateSizes, WorkspaceId};
use yaar_recorder::Clock;
use yaar_review_engine::{KEEP_STATES, Presence};

use crate::Shared;
use crate::agents::Agents;
use crate::wiring::{Kept, Parts};

/// How long after a sweep the next one runs, by the engine's clock.
pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// How often, in real time, the thread of the sweep looks at the clock and the set time.
pub(crate) const TICK: Duration = Duration::from_secs(1);

/// One day, the unit the set time is kept in.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Held for the whole of one workspace's clean, by the sweep and by the clean-up at once
/// alike, so that two of them never work on a store together. A clean-up that lets go of a
/// gone worktree's last state and packs, with a sweep that finds the store empty and removes
/// its folder under the pack, ends in a failure the user is told of though the clean-up
/// worked, or in a folder that is no repository any more. Held for one workspace at a time
/// and never across a whole sweep or clean-up, so a wait for it is for one workspace's work.
///
/// The lock is the process's and not a Keeper's: the app has one Keeper in a process, and
/// any new caller of `Reviews::expire` or `Reviews::clean_up` has to go through [`clean`],
/// since the engine itself lets two callers on one workspace interleave.
static CLEANING: Mutex<()> = Mutex::new(());

/// `Request::GetStateSizes`, as the module doc says.
pub(crate) fn sizes(kept: &Kept) -> Result<Reply, Refusal> {
    let room = kept.reviews.state_sizes().map_err(failed)?;
    let workspaces = kept.store.workspaces().map_err(failed)?;
    // In the order the workspaces were first kept, each with the store of its own; a store
    // no workspace of the Store names has no line and is in the total.
    let lines = workspaces
        .iter()
        .filter_map(|record| {
            room.workspaces
                .iter()
                .find(|(id, _)| *id == record.id)
                .map(|(_, bytes)| StateSize {
                    workspace: WorkspaceId::new(record.id.as_str()),
                    bytes: *bytes,
                })
        })
        .collect();
    Ok(Reply::StateSizes(StateSizes {
        workspaces: lines,
        total: room.total,
    }))
}

/// `Request::CleanUpStates`, as the module doc says: puts `StatesDeleted` of each workspace
/// that lost a state into `facts`.
pub(crate) fn clean_up_now(
    kept: &Kept,
    agents: &Agents,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let workspaces = kept.store.workspaces().map_err(failed)?;
    // A workspace that cannot be cleaned does not hold up the others: what was deleted is
    // told, and the first failure is what the user is told at the end.
    let mut failure = None;
    for record in workspaces {
        match clean(kept, &record.id, None) {
            Ok(true) => facts.push(Fact::StatesDeleted {
                workspace: WorkspaceId::new(record.id.as_str()),
            }),
            Ok(false) => {}
            Err(reason) => {
                eprintln!(
                    "yaar-keeper: the recorded states of {} could not be cleaned up: {reason}",
                    record.id
                );
                failure.get_or_insert(reason);
            }
        }
    }
    agents.remove_files_of_ended(kept, None);
    match failure {
        Some(reason) => Err(Refusal::Failed { reason }),
        None => sizes(kept),
    }
}

/// The states of `workspace` that nothing refers to, deleted by the set time `keep_for` as
/// the clock reads `now`, or at once when there is none; whether any state went. The sweep
/// hands the time it reads once, so that its states and its files go by the same one.
fn clean(
    kept: &Kept,
    workspace: &yaar_store::WorkspaceId,
    by_time: Option<(Duration, SystemTime)>,
) -> Result<bool, String> {
    let _alone = CLEANING.lock().unwrap_or_else(PoisonError::into_inner);
    let presence = match kept.workspaces.worktree(workspace) {
        Ok(Some(_)) => Presence::Present,
        Ok(None) => Presence::Gone,
        Err(error) => return Err(error.to_string()),
    };
    let expired = match by_time {
        Some((keep_for, now)) => kept.reviews.expire_at(workspace, keep_for, now, presence),
        None => kept.reviews.clean_up(workspace, presence),
    }
    .map_err(|error| error.to_string())?;
    Ok(!expired.states.is_empty())
}

fn failed(error: impl std::fmt::Display) -> Refusal {
    Refusal::Failed {
        reason: error.to_string(),
    }
}

/// The thread of the sweep by the set time, while the Keeper runs. Dropping it ends the
/// thread and waits for it.
pub(crate) struct Sweeps {
    // Dropped to wake the thread and end it.
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Sweeps {
    /// Starts the sweeps of the Keeper whose shared state is `shared`, reading the time from
    /// `clock`, the one the Keeper's engine reads, as the module doc says.
    pub(crate) fn start(shared: Weak<Shared>, clock: Arc<dyn Clock>) -> Sweeps {
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("sweeps the states".to_owned())
            .spawn(move || watch(&shared, clock.as_ref(), &stopped))
            .map_err(|error| {
                eprintln!("yaar-keeper: the sweep of the recorded states could not start: {error}");
            })
            .ok();
        Sweeps {
            stop: Some(stop),
            thread,
        }
    }
}

impl Drop for Sweeps {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What the last sweep went by.
struct Last {
    /// The clock's time when it began.
    at: SystemTime,
    /// The set time it used.
    keep_for: Duration,
}

/// The loop of the thread: looks every [`TICK`], and sweeps when the module doc says.
fn watch(shared: &Weak<Shared>, clock: &dyn Clock, stopped: &Receiver<()>) {
    let mut last: Option<Last> = None;
    // The reason the set time could not be read, written once for as long as it lasts.
    let mut noted = None;
    loop {
        match stopped.recv_timeout(TICK) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(shared) = shared.upgrade() else {
            return;
        };
        if !shared.has_answered() {
            continue;
        }
        let keep_for = match set_time(&shared) {
            Some(Ok(keep_for)) => {
                noted = None;
                keep_for
            }
            // The Keeper has no database, or it is leaving.
            None => continue,
            // Not the default instead: a time the user set could be longer.
            Some(Err(reason)) => {
                if noted.as_ref() != Some(&reason) {
                    eprintln!(
                        "yaar-keeper: the time the states are kept could not be read: {reason}"
                    );
                    noted = Some(reason);
                }
                continue;
            }
        };
        let now = clock.now();
        let due = last.as_ref().is_none_or(|last| {
            last.keep_for != keep_for
                || now
                    .duration_since(last.at)
                    .is_ok_and(|passed| passed >= SWEEP_EVERY)
        });
        if !due {
            continue;
        }
        last = Some(Last { at: now, keep_for });
        sweep(&shared, keep_for, now, stopped);
    }
}

/// The set time: what the user set, or the default. None when there are no parts that keep
/// anything.
fn set_time(shared: &Shared) -> Option<Result<Duration, String>> {
    with_kept(shared, |_, kept| {
        kept.store
            .settings()
            .map(|settings| {
                settings
                    .into_iter()
                    .find_map(|setting| match setting {
                        yaar_store::Setting::KeepStates { days } => Some(DAY * days),
                        _ => None,
                    })
                    .unwrap_or(KEEP_STATES)
            })
            .map_err(|error| error.to_string())
    })
}

/// Runs `then` with the Keeper's parts and what they keep, the parts read for as long as it
/// takes and no longer. None when the parts are gone or keep nothing.
fn with_kept<T>(shared: &Shared, then: impl FnOnce(&Parts, &Kept) -> T) -> Option<T> {
    let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
    let parts = parts.as_ref()?;
    Some(then(parts, parts.kept()?))
}

/// One sweep, as the module doc says, by the set time `keep_for` as the clock reads `now`.
/// Ends early, between two workspaces, when the Keeper is ending.
fn sweep(shared: &Shared, keep_for: Duration, now: SystemTime, stopped: &Receiver<()>) {
    let Some(listed) = with_kept(shared, |_, kept| kept.store.workspaces()) else {
        return;
    };
    let workspaces = match listed {
        Ok(workspaces) => workspaces,
        Err(error) => {
            eprintln!("yaar-keeper: the workspaces could not be listed for the sweep: {error}");
            return;
        }
    };
    let mut facts = Vec::new();
    for record in workspaces {
        // The parts are read for one workspace at a time, so that a request that waits to
        // write them, the Keeper's end, waits for no more than one workspace's sweep.
        if !matches!(stopped.try_recv(), Err(TryRecvError::Empty)) {
            return;
        }
        match with_kept(shared, |_, kept| {
            clean(kept, &record.id, Some((keep_for, now)))
        }) {
            None => return,
            Some(Ok(true)) => facts.push(Fact::StatesDeleted {
                workspace: WorkspaceId::new(record.id.as_str()),
            }),
            Some(Ok(false)) => {}
            Some(Err(reason)) => eprintln!(
                "yaar-keeper: the recorded states of {} could not be swept: {reason}",
                record.id
            ),
        }
    }
    // A file is as old as it was last written: one set time before now.
    if let Some(before) = now.checked_sub(keep_for) {
        with_kept(shared, |parts, kept| {
            parts.agents().remove_files_of_ended(kept, Some(before))
        });
    }
    shared.tell(&facts);
}
