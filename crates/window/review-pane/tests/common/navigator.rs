//! The Navigator's end of the Window's way to code intelligence, played by the test: a
//! port joined to the Frame's model `Navigation`, as the Keeper's end of the Window's
//! socket is played by `KeeperEnd`. The test reads the asks the pane really makes, and
//! answers them, and tells the states of the servers, with the data the Navigator would
//! send; it computes nothing a Navigator computes.
//!
//! What the model tells by itself at times the test does not choose, the workspaces, the
//! one on screen, the answers on the servers inside a repository, the files changed and the
//! old side shown, is kept here and never handed to the test as its next ask; the test
//! reads it with [`NavigatorEnd::standing`].

use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::TestAppContext;
use yaar_frame::Models;
use yaar_navigator::{Ask, Told, port};

use super::DEADLINE;

/// How long a wait on the asks rests between two runs of the window.
const PAUSE: Duration = Duration::from_millis(1);

/// The Navigator, played by the test.
pub struct NavigatorEnd {
    /// The asks, moved by a thread of this end's own from the port onto a channel the
    /// test waits on.
    asks: Receiver<Ask>,
    told: async_channel::Sender<Told>,
    standing: Vec<Ask>,
}

impl NavigatorEnd {
    /// Connects a new port to the model `models.navigation`, and keeps the Navigator's end.
    pub fn connect(models: &Models, cx: &mut TestAppContext) -> NavigatorEnd {
        // The asks are read by a real thread, which the test executor cannot see.
        cx.executor().allow_parking();
        let (port, end) = port();
        let (moved, asks) = channel();
        let from = end.asks;
        std::thread::spawn(move || {
            while let Ok(ask) = from.recv_blocking() {
                if moved.send(ask).is_err() {
                    return;
                }
            }
        });
        models
            .navigation
            .update(cx, |navigation, cx| navigation.connect(port, cx));
        cx.run_until_parked();
        NavigatorEnd {
            asks,
            told: end.told,
            standing: Vec::new(),
        }
    }

    /// Whether `ask` is one the model makes by itself.
    fn is_standing(ask: &Ask) -> bool {
        matches!(
            ask,
            Ask::Workspaces(_)
                | Ask::OnScreen(_)
                | Ask::Consents(_)
                | Ask::FilesChanged { .. }
                | Ask::OldSide { .. }
        )
    }

    /// The next ask of the pane that the model does not make by itself. Fails when none
    /// comes within [`DEADLINE`], or the model let the port go.
    pub fn ask(&mut self, cx: &mut TestAppContext) -> Ask {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.asks.recv_timeout(PAUSE) {
                Ok(ask) if Self::is_standing(&ask) => self.standing.push(ask),
                Ok(ask) => return ask,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the model let the Navigator's port go, and an ask was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(
                Instant::now() < deadline,
                "the pane asked the Navigator nothing"
            );
        }
    }

    /// What the pane asked after everything before: the asks the model has handed over by
    /// now, those it makes by itself kept aside. Read only after a barrier the test caused.
    pub fn asked_so_far(&mut self, cx: &mut TestAppContext) -> Vec<Ask> {
        cx.run_until_parked();
        let mut asked = Vec::new();
        while let Ok(ask) = self.asks.try_recv() {
            if Self::is_standing(&ask) {
                self.standing.push(ask);
            } else {
                asked.push(ask);
            }
        }
        asked
    }

    /// Tells the model `told`, as the Navigator would, and lets the window take it.
    pub fn tell(&self, told: Told, cx: &mut TestAppContext) {
        self.told
            .try_send(told)
            .expect("the model reads what the Navigator tells");
        cx.run_until_parked();
    }

    /// What the model told the Navigator by itself so far, in order.
    pub fn standing(&mut self, cx: &mut TestAppContext) -> Vec<Ask> {
        let _ = self.asked_so_far(cx);
        self.standing.clone()
    }
}
