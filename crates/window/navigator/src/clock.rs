//! The time the Navigator stops idle servers by.
//!
//! A server of a workspace off screen for the set time is stopped (FR-065). The Navigator
//! reads the time from the [`Clock`] it was started with and looks at it again at least
//! every [`crate::LOOK_EVERY`] of real time and after every ask; the Window hands it
//! [`SystemClock`]. A test that shows the rule hands it a clock of its own that it moves
//! forward, so that it neither waits the ten minutes nor leans on a short sleep.

use std::time::Instant;

/// Where the Navigator reads the time.
pub trait Clock: Send + Sync {
    /// The time now. Never earlier than an earlier answer of the same clock.
    fn now(&self) -> Instant;
}

/// The machine's monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
