//! The time a state is recorded at; tests inject their own clock.

use std::time::SystemTime;

/// Source of the recording time. `Debug` because the state store holding it derives `Debug`.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The time now. git keeps it to the second.
    fn now(&self) -> SystemTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
