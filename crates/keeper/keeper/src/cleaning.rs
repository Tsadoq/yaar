//! Clean-up of the recorded states: the sizes a Window asks for, the clean-up the user asks for,
//! and the sweep by the set time.

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

pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// How often the sweep's thread looks at the clock and the set time: real time, unlike the
/// engine's clock.
pub(crate) const TICK: Duration = Duration::from_secs(1);

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Held for one workspace's whole clean: a sweep that finds the emptied store of a gone worktree
/// would remove its folder under a clean-up's pack. Every caller of `Reviews::expire` or `clean_up`
/// goes through [`clean`].
static CLEANING: Mutex<()> = Mutex::new(());

pub(crate) fn sizes(kept: &Kept) -> Result<Reply, Refusal> {
    let room = kept.reviews.state_sizes().map_err(failed)?;
    let workspaces = kept.store.workspaces().map_err(failed)?;
    // A store of no workspace the Store holds has no line and counts in the total only.
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

pub(crate) fn clean_up_now(
    kept: &Kept,
    agents: &Agents,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let workspaces = kept.store.workspaces().map_err(failed)?;
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

pub(crate) struct Sweeps {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Sweeps {
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

struct Last {
    at: SystemTime,
    keep_for: Duration,
}

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

fn with_kept<T>(shared: &Shared, then: impl FnOnce(&Parts, &Kept) -> T) -> Option<T> {
    let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
    let parts = parts.as_ref()?;
    Some(then(parts, parts.kept()?))
}

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
    // Files go before the Windows are told `StatesDeleted`. A file is as old as it was last
    // written: one set time before now.
    if let Some(before) = now.checked_sub(keep_for) {
        with_kept(shared, |parts, kept| {
            parts.agents().remove_files_of_ended(kept, Some(before))
        });
    }
    shared.tell(&facts);
}
