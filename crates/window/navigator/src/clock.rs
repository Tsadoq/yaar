//! The time the Navigator stops idle servers by. A test hands it a clock of its own and moves
//! that clock forward by hand instead of waiting.

use std::time::Instant;

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
