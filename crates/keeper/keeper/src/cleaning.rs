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
//! [`SWEEP_EVERY`] and the age of a state are the clock's; only [`TICK`], how often the
//! thread looks, is real time. A test moves its clock past the set time, or changes the set
//! time, and waits for the sweep's `StatesDeleted` or for `GetHistory` to show the turn
//! expired. A failure of one workspace's sweep is written to the standard error in one
//! line, starting `yaar-keeper:`, and the others go on.

use std::sync::{Arc, Weak};
use std::time::Duration;

use yaar_link::{Fact, Refusal, Reply};
use yaar_recorder::Clock;

use crate::Shared;
use crate::agents::Agents;
use crate::wiring::Kept;

/// How long after a sweep the next one runs, by the engine's clock.
pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// How often, in real time, the thread of the sweep looks at the clock and the set time.
pub(crate) const TICK: Duration = Duration::from_secs(1);

/// `Request::GetStateSizes`, as the module doc says.
pub(crate) fn sizes(kept: &Kept) -> Result<Reply, Refusal> {
    let _ = kept;
    todo!("the sizes, as the module doc says")
}

/// `Request::CleanUpStates`, as the module doc says: puts `StatesDeleted` of each workspace
/// that lost a state into `facts`.
pub(crate) fn clean_up_now(
    kept: &Kept,
    agents: &Agents,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let _ = (kept, agents, facts);
    todo!("clean every workspace up, then the Adapters' files, as the module doc says")
}

/// The thread of the sweep by the set time, while the Keeper runs. Dropping it ends the
/// thread and waits for it.
pub(crate) struct Sweeps {
    _shared: Weak<Shared>,
    _clock: Arc<dyn Clock>,
}

impl Sweeps {
    /// Starts the sweeps of the Keeper whose shared state is `shared`, reading the time from
    /// `clock`, the one the Keeper's engine reads, as the module doc says.
    pub(crate) fn start(shared: Weak<Shared>, clock: Arc<dyn Clock>) -> Sweeps {
        // Until the sweep is built the Keeper sweeps nothing.
        Sweeps {
            _shared: shared,
            _clock: clock,
        }
    }
}
