//! The Navigator's end of the port, played by the test. Asks the model makes by itself
//! (those `is_standing` matches) are kept aside and read with [`NavigatorEnd::standing`].

use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::TestAppContext;
use yaar_frame::Models;
use yaar_navigator::{Ask, Told, port};

use super::DEADLINE;

const PAUSE: Duration = Duration::from_millis(1);

pub struct NavigatorEnd {
    asks: Receiver<Ask>,
    told: async_channel::Sender<Told>,
    standing: Vec<Ask>,
}

impl NavigatorEnd {
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

    fn is_standing(ask: &Ask) -> bool {
        matches!(
            ask,
            Ask::Workspaces(_)
                | Ask::OnScreen(_)
                | Ask::Consents(_)
                | Ask::FilesChanged { .. }
                | Ask::OldSide { .. }
                | Ask::StopAfter(_)
        )
    }

    /// The next ask the model does not make by itself. Fails when none comes within [`DEADLINE`].
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

    /// Asks handed over by now, those the model makes by itself kept aside. Read only after a
    /// barrier the test caused.
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

    pub fn tell(&self, told: Told, cx: &mut TestAppContext) {
        self.told
            .try_send(told)
            .expect("the model reads what the Navigator tells");
        cx.run_until_parked();
    }

    pub fn standing(&mut self, cx: &mut TestAppContext) -> Vec<Ask> {
        let _ = self.asked_so_far(cx);
        self.standing.clone()
    }
}
