//! The time a state is recorded at, which the clean-up of states counts from.
//!
//! A state commit carries the time it was recorded as its committer date: the Recorder
//! sets it from the [`Clock`] the state store was opened with
//! ([`crate::StateStore::open_with`]), and reads it back with
//! [`crate::StateStore::recorded_times`]. The Keeper hands every state store
//! [`SystemClock`]; a test that shows the set time of the clean-up hands a clock of its own
//! that it moves forward, so that it neither waits thirty days nor sets the machine's clock.
//! A store opened with [`crate::StateStore::open`] has [`SystemClock`].

use std::time::SystemTime;

/// Where the Recorder reads the time a state is recorded at. A test's clock derives `Debug`
/// as the state store that holds it does.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The time now, by the wall clock. git keeps it to the second.
    fn now(&self) -> SystemTime;
}

/// The machine's wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
