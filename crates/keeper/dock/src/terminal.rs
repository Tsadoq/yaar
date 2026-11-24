use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::Child;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use libc::pid_t;

use crate::pty;
use crate::queue::Input;
use crate::ring::Ring;
use crate::{Ending, Event, Events, Inner, MAX_WAITING_INPUT, TerminalId};

/// How long after the program is collected the Dock waits for the terminal to close: a leftover
/// process can hold it open for ever.
const LEFTOVER_WAIT: Duration = Duration::from_secs(1);

const READ_SIZE: usize = 32 * 1024;

/// A terminal takes only what its line discipline holds; the writer re-checks between pieces.
const WRITE_SIZE: usize = 16 * 1024;

// The most `drain` reads: a leftover process may write for ever, and the terminal must still be let
// go.
const DRAIN_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Copy)]
struct Collected {
    ending: Ending,
    at: Instant,
}

pub(crate) struct Terminal {
    master: OwnedFd,
    /// Also the id of its session.
    pid: pid_t,
    input: Mutex<Input>,
    input_ready: Condvar,
    /// Held while an event of the output is told and while `attach` hands the ring over.
    ring: Mutex<Ring>,
    collected: Mutex<Option<Collected>>,
    collected_ready: Condvar,
    /// Written once the program is collected and never read: threads waiting for the master poll it
    /// too.
    wake_read: OwnedFd,
    wake_write: OwnedFd,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Terminal {
    pub(crate) fn new(
        master: OwnedFd,
        pid: pid_t,
        wake: (OwnedFd, OwnedFd),
        marks: Vec<Vec<u8>>,
    ) -> Terminal {
        Terminal {
            master,
            pid,
            input: Mutex::new(Input::new(MAX_WAITING_INPUT)),
            input_ready: Condvar::new(),
            ring: Mutex::new(Ring::new(marks)),
            collected: Mutex::new(None),
            collected_ready: Condvar::new(),
            wake_read: wake.0,
            wake_write: wake.1,
        }
    }

    pub(crate) fn pid(&self) -> pid_t {
        self.pid
    }

    pub(crate) fn master(&self) -> std::os::fd::RawFd {
        self.master.as_raw_fd()
    }

    /// `false` when the program has ended and nothing is taken.
    pub(crate) fn type_in(&self, bytes: &[u8]) -> bool {
        let mut input = locked(&self.input);
        if input.is_closed() {
            return false;
        }
        input.push(bytes);
        drop(input);
        self.input_ready.notify_one();
        true
    }

    pub(crate) fn stop_typing(&self) {
        locked(&self.input).close();
        self.input_ready.notify_all();
    }

    pub(crate) fn wait_for_program(&self, mut child: Child) {
        let ending = match child.wait() {
            Ok(status) => match (status.code(), status.signal()) {
                (Some(code), _) => Ending::Exited(code),
                (None, Some(signal)) => Ending::Signalled(signal),
                (None, None) => Ending::Exited(-1),
            },
            Err(_) => Ending::Exited(-1),
        };
        *locked(&self.collected) = Some(Collected {
            ending,
            at: Instant::now(),
        });
        self.collected_ready.notify_all();
        locked(&self.input).close();
        self.input_ready.notify_all();
        let _ = pty::write(self.wake_write.as_raw_fd(), &[1]);
    }

    pub(crate) fn write_input(&self) {
        let master = self.master();
        'chunks: loop {
            let chunk = {
                let mut input = locked(&self.input);
                loop {
                    if input.is_closed() {
                        break 'chunks;
                    }
                    if let Some(chunk) = input.pop() {
                        break chunk;
                    }
                    input = self
                        .input_ready
                        .wait(input)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            let mut offset = 0;
            while offset < chunk.len() {
                // Looked at between every two writes: a stop must not wait for a long chunk to be
                // fed into a terminal that is going away.
                if locked(&self.input).is_closed() {
                    break 'chunks;
                }
                let end = chunk.len().min(offset + WRITE_SIZE);
                match pty::write(master, &chunk[offset..end]) {
                    Ok(Some(count)) => {
                        offset += count;
                        locked(&self.input).written(count);
                    }
                    Ok(None) => {
                        match pty::wait(
                            master,
                            libc::POLLOUT,
                            Some(self.wake_read.as_raw_fd()),
                            None,
                        ) {
                            Ok((_, false)) => {}
                            _ => break 'chunks,
                        }
                    }
                    Err(_) => break 'chunks,
                }
            }
        }
        locked(&self.input).close();
    }

    pub(crate) fn tell_events(&self, inner: &Arc<Inner>, id: &TerminalId, mut events: Events) {
        // Counts the end as told whatever happens below, also when `events` panics.
        let retired = Retired { inner, id };
        self.read_to_the_end(&mut events);
        let ending = self.wait_until_collected();
        // A leftover process stays the Dock's to stop. Noted before the terminal is forgotten, so a
        // stop finds the session in one list or the other.
        inner.leave_behind(self.pid);
        inner.forget(id);
        events(Event::Ended(ending));
        drop(retired);
    }

    fn read_to_the_end(&self, events: &mut Events) {
        let master = self.master();
        let mut buffer = vec![0u8; READ_SIZE];
        let mut collected_at: Option<Instant> = None;
        loop {
            let timeout = match collected_at {
                None => None,
                Some(at) => {
                    let left = LEFTOVER_WAIT.saturating_sub(at.elapsed());
                    if left.is_zero() {
                        self.drain(events, &mut buffer);
                        return;
                    }
                    Some(left.as_millis() as u64 + 1)
                }
            };
            let wake = collected_at.is_none().then(|| self.wake_read.as_raw_fd());
            let Ok((ready, woken)) = pty::wait(master, libc::POLLIN, wake, timeout) else {
                return;
            };
            if woken {
                collected_at = locked(&self.collected).map(|collected| collected.at);
            }
            if !ready {
                continue;
            }
            match pty::read(master, &mut buffer) {
                // The end of the terminal, as macOS tells it.
                Ok(Some(0)) => return,
                Ok(Some(count)) => self.tell_output(events, &buffer[..count]),
                Ok(None) => {}
                // The end of the terminal, as Linux tells it.
                Err(_) => return,
            }
        }
    }

    /// Keeps and tells the read, then its marks, under the lock `attach` takes: an attach runs
    /// before or after this read, never between.
    fn tell_output(&self, events: &mut Events, bytes: &[u8]) {
        let mut ring = locked(&self.ring);
        let marks = ring.push(bytes);
        events(Event::Output(bytes.to_vec()));
        for mark in marks {
            events(Event::Mark(mark));
        }
    }

    /// Events told before are in the ring, those after are not, so a caller that adds itself to the
    /// targets inside `attach` gets each byte once.
    pub(crate) fn attach(&self, attach: impl FnOnce(&[u8], bool)) {
        let mut ring = locked(&self.ring);
        let (kept, dropped) = ring.kept();
        attach(kept, dropped);
    }

    fn drain(&self, events: &mut Events, buffer: &mut [u8]) {
        let master = self.master();
        let mut read = 0;
        while read < DRAIN_LIMIT {
            let Ok((ready, _)) = pty::wait(master, libc::POLLIN, None, Some(0)) else {
                return;
            };
            if !ready {
                return;
            }
            match pty::read(master, buffer) {
                Ok(Some(count)) if count > 0 => {
                    read += count;
                    self.tell_output(events, &buffer[..count]);
                }
                _ => return,
            }
        }
    }

    fn wait_until_collected(&self) -> Ending {
        let mut collected = locked(&self.collected);
        loop {
            if let Some(done) = *collected {
                return done.ending;
            }
            collected = self
                .collected_ready
                .wait(collected)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

struct Retired<'a> {
    inner: &'a Arc<Inner>,
    id: &'a TerminalId,
}

impl Drop for Retired<'_> {
    fn drop(&mut self) {
        self.inner.forget(self.id);
        self.inner.told();
    }
}
