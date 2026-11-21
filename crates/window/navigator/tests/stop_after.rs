//! A stop time told to a Navigator that runs is in force at once (NAV-17), shown with the
//! stand-in language server and a clock of the test's own.
//!
//! Every wait is for a state the Navigator tells, with a deadline; no test sleeps. A clause
//! that says a server is still there is read after a barrier: the Navigator handles one event
//! at a time, in order, and tells one list of its servers for each `OnScreen` ask that changes
//! nothing, so the list told for a later ask comes after everything an earlier one made it do,
//! and the stop time told before it is the one the clock is read against.

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::*;
use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{Ask, Clock, STOP_AFTER, ServerState, State, Told, WorkspaceRoot};

/// A time that moves only when the test moves it.
struct TestClock {
    base: Instant,
    offset: Mutex<Duration>,
}

impl TestClock {
    fn new() -> Arc<TestClock> {
        Arc::new(TestClock {
            base: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
        })
    }

    fn advance(&self, by: Duration) {
        *self.offset.lock().unwrap() += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }
}

/// What a workspace of a repository is, for the Navigator.
fn root_of(workspace: &WorkspaceId, repository: &RepositoryId, folder: &Path) -> WorkspaceRoot {
    WorkspaceRoot {
        id: workspace.clone(),
        repository: repository.clone(),
        folder: Some(folder.to_path_buf()),
    }
}

/// Sends `OnScreen(Some(workspace))` again, which changes nothing and is told as the list of
/// the Navigator's servers, and gives that list.
fn barrier(running: &mut Running, workspace: &WorkspaceId) -> Vec<ServerState> {
    running.ask(Ask::OnScreen(Some(workspace.clone())));
    match running.wait_told("the list of servers told for the ask", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    }) {
        Told::Servers(states) => states,
        _ => unreachable!("only a list of servers satisfies the wait"),
    }
}

/// The pid of the stand-in's server of `workspace` in a list.
fn pid_in(list: &[ServerState], workspace: &WorkspaceId) -> Option<u32> {
    list.iter()
        .find(|state| state.workspace == *workspace && state.language == "standin")
        .and_then(|state| state.pid)
}

/// A row whose one server is the stand-in found on the machine's search path.
fn row(log: &Path) -> String {
    format!(
        "[standin]\n\
         extensions = {{ sil = \"standin\" }}\n\
         markers = [\"main.sil\"]\n\
         \n\
         [[standin.server]]\n\
         name = \"standin\"\n\
         machine = [\"standin-lsp\"]\n\
         args = [\"--log\", {:?}]\n\
         trial = [\"--version\"]\n\
         ready = {{ kind = \"answer\" }}\n",
        log.display().to_string()
    )
}

/// A Navigator on a clock of the test's own with `ws1`, a workspace with a server, and `ws2`,
/// one with none, both listed; `ws1` is on screen and its server ready. Gives the
/// Navigator, the clock, the two workspaces and the pid of `ws1`'s server.
fn started(
    scratch: &Scratch,
    log: &str,
) -> (Running, Arc<TestClock>, WorkspaceId, WorkspaceId, u32) {
    let shown = scratch.repository("shown", &[("main.sil", "def alpha\n")]);
    let other = scratch.repository("other", &[("readme.txt", "nothing\n")]);
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let (ws1, ws2, repository) = (workspace_id('1'), workspace_id('2'), repository_id('1'));
    let clock = TestClock::new();
    let mut config = scratch.config(&row(&scratch.root.join(log)));
    config.clock = clock.clone();
    assert_eq!(config.stop_after, STOP_AFTER);
    let mut running = Running::start(config);
    running.ask(Ask::Workspaces(vec![
        root_of(&ws1, &repository, &shown.root),
        root_of(&ws2, &repository, &other.root),
    ]));
    running.ask(Ask::OnScreen(Some(ws1.clone())));
    let first = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let pid = first.pid.unwrap();
    (running, clock, ws1, ws2, pid)
}

#[test]
fn a_shorter_stop_time_told_to_a_running_navigator_stops_an_idle_server_that_the_old_one_would_keep()
 {
    let scratch = Scratch::new();
    let (mut running, clock, ws1, ws2, pid) = started(&scratch, "shorter.log");

    // Off screen. Two minutes is well short of the ten the Navigator started with: the server
    // stays, so the stop that follows is the new time's and no other.
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid));
    clock.advance(Duration::from_secs(120));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid), "stopped by the old time");
    assert!(is_alive(pid));

    // A minute told: the two minutes already passed are more than it, and the very next look
    // at the clock stops the server, before the list that says so.
    running.ask(Ask::StopAfter(Duration::from_secs(60)));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), None, "the idle server is still listed");
    assert!(list.iter().all(|state| state.workspace != ws1), "{list:#?}");
    assert!(!is_alive(pid), "the idle server still runs");
    running.end();
}

#[test]
fn a_longer_stop_time_told_to_a_running_navigator_keeps_an_idle_server_past_the_old_one() {
    let scratch = Scratch::new();
    let (mut running, clock, ws1, ws2, pid) = started(&scratch, "longer.log");

    // Twenty minutes told while the server is on screen, then it goes off screen.
    running.ask(Ask::StopAfter(Duration::from_secs(20 * 60)));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid));

    // Fifteen minutes: past the ten the Navigator started with, short of the twenty told.
    clock.advance(Duration::from_secs(15 * 60));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid), "stopped by the old time");
    assert!(is_alive(pid));

    // The twenty told, reached: the control half, so that the server was kept by the time and
    // not by something else.
    clock.advance(Duration::from_secs(5 * 60));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), None);
    assert!(!is_alive(pid));
    running.end();
}
