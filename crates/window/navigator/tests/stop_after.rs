//! Waits are for told states with a deadline, never sleeps. Events are handled in order, so a
//! server list read after `barrier` reflects every earlier ask.

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::*;
use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{Ask, Clock, STOP_AFTER, ServerState, State, Told, WorkspaceRoot};

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

fn root_of(workspace: &WorkspaceId, repository: &RepositoryId, folder: &Path) -> WorkspaceRoot {
    WorkspaceRoot {
        id: workspace.clone(),
        repository: repository.clone(),
        folder: Some(folder.to_path_buf()),
    }
}

fn barrier(running: &mut Running, workspace: &WorkspaceId) -> Vec<ServerState> {
    running.ask(Ask::OnScreen(Some(workspace.clone())));
    match running.wait_told("the list of servers told for the ask", DEADLINE, |told| {
        matches!(told, Told::Servers(_))
    }) {
        Told::Servers(states) => states,
        _ => unreachable!("only a list of servers satisfies the wait"),
    }
}

fn pid_in(list: &[ServerState], workspace: &WorkspaceId) -> Option<u32> {
    list.iter()
        .find(|state| state.workspace == *workspace && state.language == "standin")
        .and_then(|state| state.pid)
}

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

    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid));
    clock.advance(Duration::from_secs(120));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid), "stopped by the old time");
    assert!(is_alive(pid));

    // The two minutes passed exceed the minute told: the next look at the clock stops the server.
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

    running.ask(Ask::StopAfter(Duration::from_secs(20 * 60)));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid));

    clock.advance(Duration::from_secs(15 * 60));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), Some(pid), "stopped by the old time");
    assert!(is_alive(pid));

    clock.advance(Duration::from_secs(5 * 60));
    let list = barrier(&mut running, &ws2);
    assert_eq!(pid_in(&list, &ws1), None);
    assert!(!is_alive(pid));
    running.end();
}
