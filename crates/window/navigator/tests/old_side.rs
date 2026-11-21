//! The old side of a diff, shown with the stand-in language server: no process for it before
//! the reviewer's first use of a removed line (OLD-05), the export of an earlier old side gone
//! when another change is selected (OLD-06), the language servers of the workspace on screen
//! running and stopped, the old side's included, when it has been off screen for the set time
//! (NAV-11), and what the export is made of and when it is asked for.
//!
//! The test plays the Keeper's end of the export ([`Running::export_when_wanted`]) as the tests
//! of the panes do: it writes the version with git and hands the answer back. Every wait is for
//! something the Navigator tells, with a deadline. A clause that says something did not
//! happen is read after a barrier: a hover of a workspace the Navigator was never told of is
//! answered at once, and the loop handles one event at a time, in order, so that its answer
//! comes after everything the asks before it made the Navigator do ([`settled`]). The clock of
//! NAV-11 is the test's own and moves only when the test moves it.

mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::*;
use yaar_link::WorkspaceId;
use yaar_navigator::{
    Answered, Ask, AskId, Clock, ExportAsk, FileKind, Location, NavigatorConfig, Place, STOP_AFTER,
    ServerState, State, Told, Tree, WorkspaceRoot,
};

/// Every line of a stand-in's log, as the JSON it is; none when the log is not there.
fn log_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a line of the log is JSON"))
        .collect()
}

/// The folders the stand-in was started in, in order of the starts, from its log.
fn started_in(log: &Path) -> Vec<PathBuf> {
    log_of(log)
        .iter()
        .filter(|line| line["event"] == "start")
        .map(|line| PathBuf::from(line["cwd"].as_str().expect("the folder of a start")))
        .collect()
}

/// The stand-in's row `standin`, ready at once, logging to `log`, and `extra` after its
/// server's keys.
fn row(log: &Path, extra: &str) -> String {
    standin_row(
        "standin",
        &["--log", &log.display().to_string()],
        "answer",
        extra,
    )
}

/// A repository whose base commit holds `main.sil` (`def alpha`, then a line that uses
/// `legacy`) and `legacy.sil`, and whose worktree has taken both away: the change on screen
/// removes the line that used `legacy`. The scratch's `bin` holds the stand-in.
fn removed_line_repository(scratch: &Scratch) -> Repo {
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse legacy\n"),
            ("legacy.sil", "def legacy\n"),
        ],
    );
    std::fs::remove_file(repo.root.join("legacy.sil")).expect("remove the file");
    std::fs::write(repo.root.join("main.sil"), "def alpha\n").expect("write the change");
    repo
}

/// The removed line, at the base: the use of `legacy`.
fn removed(repo: &Repo, workspace: &WorkspaceId) -> Place {
    place(
        repo,
        workspace,
        Tree::Version(repo.base.clone()),
        "main.sil",
        2,
        "legacy",
        0,
    )
}

/// Asks a hover at `place` without waiting for its answer, so that the test can play the
/// Keeper before the answer is read: the Navigator tells the export it wants when it handles
/// the ask.
fn ask_hover(running: &Running, id: u64, place: Place) {
    running.ask(Ask::Hover {
        id: AskId(id),
        place,
    });
}

/// The answer to the hover asked as `id`: what was told already, or else waited for. A test
/// reads the stream in order, and the answer may come before or after a state it waits for,
/// so both look at what was read first.
fn answer_of(running: &mut Running, id: u64) -> Answered<Option<String>> {
    let is_it = |told: &Told| matches!(told, Told::Hover { id: got, .. } if *got == AskId(id));
    let told = match running.told.iter().find(|told| is_it(told)) {
        Some(told) => told.clone(),
        None => running.wait_told("the answer to a hover", DEADLINE, is_it),
    };
    match told {
        Told::Hover { answer, .. } => answer,
        _ => unreachable!("only the answer to the ask satisfies the wait"),
    }
}

/// The state of the old side's server of `version` of `workspace`, from the lists told since
/// the `since`th thing told, once it is ready and its process is not `not`: what was read
/// already, or else waited for.
fn old_side_ready(
    running: &mut Running,
    since: usize,
    workspace: &WorkspaceId,
    version: &str,
    not: Option<u32>,
) -> ServerState {
    let wanted = |state: &ServerState| state.state == State::Ready && state.pid != not;
    let read = running.told[since..].iter().find_map(|told| match told {
        Told::Servers(states) => server_in(states, workspace, Some(version)).filter(wanted),
        _ => None,
    });
    match read {
        Some(state) => state,
        None => running.wait_state(workspace, "standin", Some(version), DEADLINE, wanted),
    }
}

/// The first use of a removed line: a hover at `place` on the old side, answered by text
/// match while the export is written, the export played, and the old side's server of
/// `workspace` waited for until it is ready. Gives what the Navigator asked the Keeper for,
/// and the old side's state.
fn use_the_old_side(
    running: &mut Running,
    repo: &Repo,
    workspace: &WorkspaceId,
    place: Place,
) -> (ExportAsk, ServerState) {
    let Tree::Version(version) = place.tree.clone() else {
        panic!("the first use of the old side is a place of a version");
    };
    let since = running.told.len();
    let id = 1_000 + exports_wanted(running) as u64;
    ask_hover(running, id, place);
    let asked = running.export_when_wanted(repo, DEADLINE);
    assert_eq!(asked.version, version);
    let meanwhile = answer_of(running, id);
    assert!(
        meanwhile.guessed,
        "the old side answered before it was ready: {meanwhile:#?}"
    );
    let ready = old_side_ready(running, since, workspace, &version, None);
    (asked, ready)
}

/// A hover of a workspace the Navigator was never told of, which it answers at once, marked.
/// Its answer comes after everything the asks before it made the Navigator do. Gives the
/// last list of servers told by then.
fn settled(running: &mut Running) -> Vec<ServerState> {
    let nobody = Place {
        workspace: workspace_id('9'),
        tree: Tree::Worktree,
        path: PathBuf::from("nothing.sil"),
        line: 1,
        column: 0,
        name: "nothing".to_owned(),
        shown: Vec::new(),
    };
    let answer = running.hover(nobody, DEADLINE);
    assert!(answer.guessed, "{answer:#?}");
    last_servers(running)
}

/// The newest list of servers the Navigator told.
fn last_servers(running: &Running) -> Vec<ServerState> {
    running
        .told
        .iter()
        .rev()
        .find_map(|told| match told {
            Told::Servers(states) => Some(states.clone()),
            _ => None,
        })
        .expect("a list of servers was told")
}

/// How many exports the Navigator wanted so far.
fn exports_wanted(running: &Running) -> usize {
    running
        .told
        .iter()
        .filter(|told| matches!(told, Told::ExportWanted(_)))
        .count()
}

/// Every server state of an old side told so far, in any list.
fn old_sides_told(running: &Running) -> Vec<ServerState> {
    running
        .told
        .iter()
        .filter_map(|told| match told {
            Told::Servers(states) => Some(states),
            _ => None,
        })
        .flatten()
        .filter(|state| state.old_side.is_some())
        .cloned()
        .collect()
}

/// What a workspace of a repository is, for the Navigator.
fn root_of(workspace: &WorkspaceId, repo: &Repo) -> WorkspaceRoot {
    WorkspaceRoot {
        id: workspace.clone(),
        repository: repository_id('1'),
        folder: Some(repo.root.clone()),
    }
}

// ---------------------------------------------------------------------------------------
// OLD-05: no process for the old side before the first use of a removed line.
// ---------------------------------------------------------------------------------------

#[test]
fn no_process_for_the_old_side_runs_before_the_reviewers_first_use_of_a_removed_line() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    // The change is on screen, and the reviewer reads a line of the new side: answered by
    // the new side's server, which is the barrier for the ask before it.
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: Some(repo.base.clone()),
    });
    let new_side = running.hover(
        place(&repo, &ws, Tree::Worktree, "main.sil", 1, "alpha", 0),
        DEADLINE,
    );
    assert_eq!(
        new_side,
        Answered {
            value: Some("standin: alpha".to_owned()),
            guessed: false
        }
    );
    assert_eq!(
        exports_wanted(&running),
        0,
        "an export was wanted before any use"
    );
    assert_eq!(old_sides_told(&running), vec![]);
    assert_eq!(
        started_in(&log),
        std::slice::from_ref(&repo.root),
        "a second process was started"
    );

    // The first use of a removed line: answered by text match at once, and the export asked
    // for; once it is written, the old side's server starts in it.
    let (asked, old) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    assert_eq!(asked.workspace, ws);
    assert_eq!(started_in(&log), [repo.root.clone(), asked.into.clone()]);
    assert!(old.pid.is_some_and(is_alive));

    // It serves the base: the line that is gone from the worktree.
    let hover = running.hover(removed(&repo, &ws), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: legacy".to_owned()),
            guessed: false
        }
    );
    running.end();
}

// ---------------------------------------------------------------------------------------
// OLD-06: the export of the earlier old side is gone.
// ---------------------------------------------------------------------------------------

/// A second commit on `repo`: a file `second.sil`. Gives its id.
fn commit_a_second(repo: &Repo) -> String {
    std::fs::write(repo.root.join("second.sil"), "def second\n").expect("write the file");
    repo.git(&["add", "second.sil"]);
    repo.git(&["commit", "--quiet", "-m", "second"]);
    repo.git(&["rev-parse", "HEAD"])
}

/// The folders of `cache` named after this process, the own caches of its Navigators.
fn own_caches(cache: &Path) -> Vec<String> {
    let prefix = format!("{}-", std::process::id());
    std::fs::read_dir(cache)
        .expect("read the cache")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix))
        .collect()
}

#[test]
fn after_another_change_is_selected_the_export_of_the_earlier_old_side_is_gone() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let second = commit_a_second(&repo);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    // The old side of the base is used.
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: Some(repo.base.clone()),
    });
    let (earlier, old) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    let old_pid = old.pid.expect("the old side runs");
    assert!(earlier.into.is_dir(), "the export of the base is not there");
    assert!(earlier.into.ends_with("old/1"), "{:?}", earlier.into);

    // Another change is selected. The Navigator handled the ask, and the one after it, before
    // the barrier's answer came: the old server was stopped and reaped, and then the export
    // removed, before the ask was handled that is a step further on.
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: Some(second.clone()),
    });
    running.ask(Ask::OnScreen(Some(ws.clone())));
    let list = settled(&mut running);
    assert!(
        !earlier.into.exists(),
        "the export of the earlier old side is still there"
    );
    assert!(!is_alive(old_pid), "the old side's server still runs");
    assert_eq!(
        list.iter()
            .filter(|state| state.old_side.is_some())
            .collect::<Vec<_>>(),
        Vec::<&ServerState>::new(),
        "the list still names an old side"
    );
    assert!(
        list.iter()
            .any(|state| state.old_side.is_none() && state.state == State::Ready),
        "the new side no longer runs: {list:#?}"
    );
    assert_eq!(started_in(&log).len(), 2, "a server started for no reason");

    // The next old side is another export, counted on from the first.
    let place_in_second = place(
        &repo,
        &ws,
        Tree::Version(second.clone()),
        "second.sil",
        1,
        "second",
        0,
    );
    let (later, _) = use_the_old_side(&mut running, &repo, &ws, place_in_second);
    assert!(later.into.ends_with("old/2"), "{:?}", later.into);
    assert!(later.into.is_dir());
    assert!(
        !earlier.into.exists(),
        "the earlier export came back into being"
    );
    running.end();
}

#[test]
fn the_export_and_the_server_of_a_workspace_that_leaves_the_list_are_gone_and_the_end_removes_the_own_cache()
 {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: Some(repo.base.clone()),
    });
    let (asked, old) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    let old_pid = old.pid.expect("the old side runs");
    assert!(asked.into.is_dir());
    let cache = scratch.root.join("navigator");
    assert_eq!(own_caches(&cache).len(), 1);

    running.ask(Ask::Workspaces(vec![]));
    running.ask(Ask::OnScreen(None));
    let list = settled(&mut running);
    assert_eq!(list, vec![]);
    assert!(
        !asked.into.exists(),
        "the export of a closed workspace is still there"
    );
    assert!(!is_alive(old_pid));

    // The Navigator's end removes its own cache folder, and only its own.
    let other = cache.join("targets");
    std::fs::create_dir_all(&other).expect("make a folder that is no Navigator's");
    running.end();
    assert_eq!(own_caches(&cache), Vec::<String>::new());
    assert!(other.is_dir(), "a folder of no Navigator was removed");
}

#[test]
fn the_end_of_the_navigator_removes_the_export_in_use_after_its_server_is_gone() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    let (asked, _) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    assert!(asked.into.is_dir());
    // `end` proves every server's process gone; the export is removed by then.
    running.end();
    assert!(
        !asked.into.exists(),
        "the export outlived the Navigator that made it"
    );
    assert_eq!(
        own_caches(&scratch.root.join("navigator")),
        Vec::<String>::new()
    );
}

#[test]
fn a_workspace_whose_worktree_is_gone_loses_its_export() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    let (asked, old) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));

    running.ask(Ask::Workspaces(vec![WorkspaceRoot {
        id: ws.clone(),
        repository,
        folder: None,
    }]));
    settled(&mut running);
    assert!(
        !asked.into.exists(),
        "the export of a gone worktree is there"
    );
    assert!(!is_alive(old.pid.expect("the old side runs")));
    running.end();
}

/// What ends the old side's server and drops its export in [`export_outlives_its_server`].
enum Drops {
    /// Another change is selected.
    AnotherChange,
    /// The workspace leaves the list.
    WorkspaceLeaves,
    /// The workspace's worktree is gone.
    WorktreeGone,
    /// The Navigator ends.
    NavigatorEnds,
}

/// `text` in single quotes, for a `/bin/sh` script.
fn sh_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Uses the old side with a server that is a script of the test: it runs the stand-in as its
/// child, and when the child has ended it writes a line `<its working folder> present` or
/// `... gone` into a file, by whether that folder is still there, and ends as its child did.
/// Then `drops` ends the old side. The export may be removed only after the server's process
/// has ended, which only the server's own end can show: a surface of the Navigator does not
/// tell the order of two things done before it answers.
fn export_outlives_its_server(drops: Drops) {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let second = commit_a_second(&repo);
    let log = scratch.root.join("standin.log");
    let ends = scratch.root.join("ends");
    let wrapper = scratch.root.join("bin").join("standin-ends");
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then exec {standin} \"$@\"; fi\n\
         here=$(pwd -P)\n\
         {standin} \"$@\"\n\
         code=$?\n\
         if [ -d \"$here\" ]; then echo \"$here present\" >> {ends}; else echo \"$here gone\" >> {ends}; fi\n\
         exit $code\n",
        standin = sh_quoted(&standin().display().to_string()),
        ends = sh_quoted(&ends.display().to_string()),
    );
    std::fs::write(&wrapper, script).expect("write the script");
    std::fs::set_permissions(
        &wrapper,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make it executable");
    let text = row(&log, "");
    let wrapped = text.replace(
        "machine = [\"standin-lsp\"]",
        "machine = [\"standin-ends\"]",
    );
    assert_ne!(wrapped, text, "the row names the stand-in's own program");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&wrapped));
    running.show(&repo, &ws, &repository);
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: Some(repo.base.clone()),
    });
    let (asked, old) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    let old_pid = old.pid.expect("the old side runs");
    let export = std::fs::canonicalize(&asked.into).expect("the export is there");

    match drops {
        Drops::AnotherChange => {
            running.ask(Ask::OldSide {
                workspace: ws.clone(),
                version: Some(second),
            });
            running.ask(Ask::OnScreen(Some(ws.clone())));
            settled(&mut running);
        }
        Drops::WorkspaceLeaves => {
            running.ask(Ask::Workspaces(vec![]));
            running.ask(Ask::OnScreen(None));
            settled(&mut running);
        }
        Drops::WorktreeGone => {
            running.ask(Ask::Workspaces(vec![WorkspaceRoot {
                id: ws.clone(),
                repository,
                folder: None,
            }]));
            settled(&mut running);
        }
        Drops::NavigatorEnds => {}
    }
    if !matches!(drops, Drops::NavigatorEnds) {
        assert!(!is_alive(old_pid), "the old side's server still runs");
    }
    running.end();

    assert!(!asked.into.exists(), "the export is still there");
    let ends: Vec<String> = std::fs::read_to_string(&ends)
        .expect("the server's script wrote when it ended")
        .lines()
        .map(str::to_owned)
        .collect();
    let of_the_export = |state: &str| format!("{} {state}", export.display());
    assert!(
        ends.contains(&of_the_export("present")),
        "the export was removed before the old side's server ended: {ends:#?}"
    );
    assert!(!ends.contains(&of_the_export("gone")), "{ends:#?}");
}

#[test]
fn the_old_servers_folder_is_there_when_its_process_ends_after_another_change_is_selected() {
    export_outlives_its_server(Drops::AnotherChange);
}

#[test]
fn the_old_servers_folder_is_there_when_its_process_ends_with_its_workspace_leaving_the_list() {
    export_outlives_its_server(Drops::WorkspaceLeaves);
}

#[test]
fn the_old_servers_folder_is_there_when_its_process_ends_with_its_worktree_gone() {
    export_outlives_its_server(Drops::WorktreeGone);
}

#[test]
fn the_old_servers_folder_is_there_when_its_process_ends_with_the_navigator() {
    export_outlives_its_server(Drops::NavigatorEnds);
}

// ---------------------------------------------------------------------------------------
// NAV-11: the servers of the workspace on screen run; off screen for the set time they stop.
// ---------------------------------------------------------------------------------------

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

/// The state of the stand-in's server of `workspace` in a list, of the old side of `old` or
/// of the files on disk.
fn server_in(
    list: &[ServerState],
    workspace: &WorkspaceId,
    old: Option<&str>,
) -> Option<ServerState> {
    list.iter()
        .find(|state| state.workspace == *workspace && state.old_side.as_deref() == old)
        .cloned()
}

#[test]
fn the_servers_of_the_workspace_on_screen_run_and_stop_when_it_was_off_screen_for_the_set_time_the_old_side_included()
 {
    // "10 minutes with nothing set": the constant, and the configuration the Window hands the
    // Navigator, which only computes its folders from the environment.
    assert_eq!(STOP_AFTER, Duration::from_secs(600));
    assert_eq!(NavigatorConfig::for_this_user().stop_after, STOP_AFTER);
    let scratch = Scratch::new();
    let shown = removed_line_repository(&scratch);
    let other = scratch.repository("other", &[("readme.txt", "nothing\n")]);
    let log = scratch.root.join("standin.log");
    let (ws1, ws2) = (workspace_id('1'), workspace_id('2'));
    let clock = TestClock::new();
    let mut config = scratch.config(&row(&log, ""));
    config.clock = clock.clone();
    assert_eq!(config.stop_after, STOP_AFTER);
    let mut running = Running::start(config);
    running.ask(Ask::Workspaces(vec![
        root_of(&ws1, &shown),
        WorkspaceRoot {
            id: ws2.clone(),
            repository: repository_id('2'),
            folder: Some(other.root.clone()),
        },
    ]));
    running.ask(Ask::OnScreen(Some(ws1.clone())));

    // The workspace on screen has its server with no ask about a file; the one with no `.sil`
    // file has none until it is asked.
    let new_side = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });
    let new_pid = new_side.pid.expect("the new side runs");
    let list = settled(&mut running);
    assert_eq!(server_in(&list, &ws2, None), None);
    assert_eq!(started_in(&log), std::slice::from_ref(&shown.root));

    // The old side's first use.
    running.ask(Ask::OldSide {
        workspace: ws1.clone(),
        version: Some(shown.base.clone()),
    });
    let (asked, old) = use_the_old_side(&mut running, &shown, &ws1, removed(&shown, &ws1));
    let old_pid = old.pid.expect("the old side runs");

    // The other workspace comes on screen: the first goes off it. One second short of the
    // set time, everything of it still runs. Each ask is the barrier of the clock: the
    // Navigator reads the clock before it handles an ask, and tells its servers after.
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let list = settled(&mut running);
    assert_eq!(
        (
            server_in(&list, &ws1, None).and_then(|state| state.pid),
            server_in(&list, &ws1, Some(&shown.base)).and_then(|state| state.pid)
        ),
        (Some(new_pid), Some(old_pid))
    );
    clock.advance(STOP_AFTER - Duration::from_secs(1));
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let list = settled(&mut running);
    assert_eq!(
        (
            server_in(&list, &ws1, None).and_then(|state| state.pid),
            server_in(&list, &ws1, Some(&shown.base)).and_then(|state| state.pid)
        ),
        (Some(new_pid), Some(old_pid)),
        "a server was stopped one second early"
    );
    assert!(is_alive(new_pid) && is_alive(old_pid));

    // The set time: both are stopped and reaped, and the list that comes after says so. The
    // export stays until another change is shown or the workspace closes.
    clock.advance(Duration::from_secs(1));
    running.ask(Ask::OnScreen(Some(ws2.clone())));
    let list = settled(&mut running);
    assert_eq!(
        list.iter()
            .filter(|state| state.workspace == ws1)
            .collect::<Vec<_>>(),
        Vec::<&ServerState>::new(),
        "a server of the workspace off screen is still listed"
    );
    assert!(!is_alive(new_pid), "the new side's server still runs");
    assert!(!is_alive(old_pid), "the old side's server still runs");
    assert!(asked.into.is_dir(), "the export did not stay");

    // Back on screen the new side starts again, with no ask; the old side at its next use,
    // in the export that stayed.
    running.ask(Ask::OnScreen(Some(ws1.clone())));
    let again = running.wait_state(&ws1, "standin", None, DEADLINE, |state| {
        state.state == State::Ready && state.pid != Some(new_pid)
    });
    assert!(is_alive(again.pid.expect("the new side runs")));
    let list = settled(&mut running);
    assert_eq!(
        server_in(&list, &ws1, Some(&shown.base)),
        None,
        "the old side started before its use"
    );
    let since = running.told.len();
    ask_hover(&running, 2_000, removed(&shown, &ws1));
    let old_again = old_side_ready(&mut running, since, &ws1, &shown.base, Some(old_pid));
    assert!(is_alive(old_again.pid.expect("the old side runs")));
    assert_eq!(
        exports_wanted(&running),
        1,
        "the export that stayed was written again"
    );
    answer_of(&mut running, 2_000);
    let hover = running.hover(removed(&shown, &ws1), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: legacy".to_owned()),
            guessed: false
        }
    );
    assert_eq!(
        started_in(&log),
        [
            shown.root.clone(),
            asked.into.clone(),
            shown.root.clone(),
            asked.into.clone()
        ]
    );
    running.end();
}

// ---------------------------------------------------------------------------------------
// When the export is asked for.
// ---------------------------------------------------------------------------------------

#[test]
fn an_export_that_is_being_written_is_asked_for_once() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);

    // Two asks about the old side while the Keeper has not answered, then its answer. The
    // asks are handled before the answer, so a second wish for the export would have been
    // told before the server starts.
    ask_hover(&running, 1, removed(&repo, &ws));
    ask_hover(&running, 2, removed(&repo, &ws));
    running.export_when_wanted(&repo, DEADLINE);

    // Each was answered by text match, marked, with no server of the old side yet.
    assert!(answer_of(&mut running, 1).guessed);
    assert!(answer_of(&mut running, 2).guessed);
    old_side_ready(&mut running, 0, &ws, &repo.base, None);
    assert_eq!(exports_wanted(&running), 1);

    // A later ask does not ask again either.
    let hover = running.hover(removed(&repo, &ws), DEADLINE);
    assert!(!hover.guessed, "{hover:#?}");
    assert_eq!(exports_wanted(&running), 1);
    running.end();
}

#[test]
fn an_export_that_failed_leaves_nothing_and_the_next_ask_asks_again() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    // A version the played Keeper cannot write: it is no full commit id.
    let mut unwritable = removed(&repo, &ws);
    unwritable.tree = Tree::Version("not-a-commit".to_owned());

    ask_hover(&running, 1, unwritable.clone());
    let first = running.export_when_wanted(&repo, DEADLINE);
    assert!(answer_of(&mut running, 1).guessed);
    let list = settled(&mut running);
    assert!(!first.into.exists(), "a failed export left its folder");
    assert_eq!(old_sides_told(&running), vec![]);
    assert!(
        server_in(&list, &ws, Some("not-a-commit")).is_none(),
        "a server started on an export that failed"
    );

    // The next ask asks again, into a folder of its own that does not exist yet.
    ask_hover(&running, 2, unwritable);
    let second = running.export_when_wanted(&repo, DEADLINE);
    assert!(answer_of(&mut running, 2).guessed);
    assert_eq!(exports_wanted(&running), 2);
    assert_ne!(second.into, first.into);
    assert!(second.into.ends_with("old/2"), "{:?}", second.into);
    running.end();
}

#[test]
fn an_export_still_being_written_when_the_old_side_is_dropped_is_removed_when_its_answer_comes() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    running.wait_state(&ws, "standin", None, DEADLINE, |state| {
        state.state == State::Ready
    });

    // The ask, then the change is dropped, then the Keeper's answer: the Navigator handles
    // them in that order, so the answer finds the export no longer wanted.
    ask_hover(&running, 1, removed(&repo, &ws));
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: None,
    });
    let asked = running.export_when_wanted(&repo, DEADLINE);
    assert!(answer_of(&mut running, 1).guessed);
    settled(&mut running);
    assert!(
        !asked.into.exists(),
        "an export nobody wants any more was kept"
    );
    assert_eq!(old_sides_told(&running), vec![]);
    assert_eq!(started_in(&log), std::slice::from_ref(&repo.root));
    running.end();
}

#[test]
fn a_second_answer_for_an_export_in_use_leaves_it_and_its_server_answering() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    let (asked, old) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    let (pid, file) = (old.pid, asked.into.join("legacy.sil"));
    assert!(file.is_file(), "the export was not written");

    // The Keeper answers the same ask again, as a Frame that told it twice would. The
    // Navigator handles it before the hover, whose answer is the barrier.
    running.ask(Ask::Exported {
        asked: asked.clone(),
        failed: None,
    });
    let hover = running.hover(removed(&repo, &ws), DEADLINE);
    assert_eq!(
        hover,
        Answered {
            value: Some("standin: legacy".to_owned()),
            guessed: false
        }
    );
    assert!(file.is_file(), "the export in use was removed");
    let list = last_servers(&running);
    let still = server_in(&list, &ws, Some(&repo.base)).expect("the old side's server");
    assert_eq!((still.state, still.pid), (State::Ready, pid));
    assert_eq!(exports_wanted(&running), 1);
    running.end();
}

#[test]
fn an_answer_for_a_folder_that_is_not_under_the_navigators_own_old_folder_removes_nothing() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row(&log, "")));
    running.show(&repo, &ws, &repository);
    // An export in use, so that the Navigator's `old` folder is there.
    let (asked, _) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));
    // A folder of the machine's, which no export of this Navigator made.
    let elsewhere = scratch.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("make the folder");
    std::fs::write(elsewhere.join("keep.txt"), "mine\n").expect("write the file");

    // Named plainly, and named by a path that starts in the export's folder and climbs out
    // of it: the export is `<cache>/<own>/old/<n>`, and the folder is four levels above.
    let climbing = asked.into.join("../../../../elsewhere");
    assert!(
        climbing.join("keep.txt").is_file(),
        "the path leaves the export"
    );
    for into in [elsewhere.clone(), climbing] {
        running.ask(Ask::Exported {
            asked: ExportAsk {
                workspace: ws.clone(),
                version: repo.base.clone(),
                into,
            },
            failed: None,
        });
        settled(&mut running);
        assert!(
            elsewhere.join("keep.txt").is_file(),
            "a folder the Navigator did not make was removed"
        );
    }
    assert!(asked.into.is_dir(), "the export in use was removed");
    running.end();
}

#[test]
fn two_navigators_of_one_process_keep_their_exports_apart() {
    let scratch = Scratch::new();
    let repo = removed_line_repository(&scratch);
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let (log_a, log_b) = (scratch.root.join("a.log"), scratch.root.join("b.log"));
    let mut first = Running::start(scratch.config(&row(&log_a, "")));
    let mut second = Running::start(scratch.config(&row(&log_b, "")));
    for running in [&first, &second] {
        running.show(&repo, &ws, &repository);
    }
    let (a, _) = use_the_old_side(&mut first, &repo, &ws, removed(&repo, &ws));
    let (b, _) = use_the_old_side(&mut second, &repo, &ws, removed(&repo, &ws));
    assert_ne!(a.into.parent(), b.into.parent());
    assert!(a.into.ends_with("old/1") && b.into.ends_with("old/1"));

    // One's end removes nothing of the other's.
    first.end();
    assert!(!a.into.exists());
    assert!(b.into.is_dir(), "the other Navigator's export was removed");
    second.end();
    assert!(!b.into.exists());
}

// ---------------------------------------------------------------------------------------
// What an export is made of.
// ---------------------------------------------------------------------------------------

/// The row of the stand-in whose old side links the folders named `deps`.
fn row_linking_deps(log: &Path) -> String {
    row(log, "\n[standin.old_side]\nlink = [\"deps\"]\n")
}

/// The target of the link at `path`, when it is one.
fn link_target(path: &Path) -> Option<PathBuf> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.file_type().is_symlink())
        .and_then(|_| std::fs::read_link(path).ok())
}

#[test]
fn an_export_links_the_folders_its_row_names_to_the_worktrees_and_removing_it_leaves_them() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    // The base holds `pkg` and `vend/deps` (a folder of that name that git tracks).
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse legacy\n"),
            ("pkg/lib.sil", "def lib\n"),
            ("vend/deps/kept.sil", "def kept\n"),
        ],
    );
    // The worktree holds folders named `deps` that git does not track: at its root, in `pkg`,
    // in `new`, which the base has no folder for, and inside the one at its root.
    for (file, text) in [
        ("deps/x.sil", "def x\n"),
        ("pkg/deps/y.sil", "def y\n"),
        ("new/deps/z.sil", "def z\n"),
        ("deps/inner/deps/w.sil", "def w\n"),
    ] {
        let file = repo.root.join(file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_linking_deps(&log)));
    running.show(&repo, &ws, &repository);
    let (asked, _) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));

    let export = &asked.into;
    assert_eq!(
        link_target(&export.join("deps")),
        Some(repo.root.join("deps")),
        "the folder at the root is not linked"
    );
    assert_eq!(
        link_target(&export.join("pkg/deps")),
        Some(repo.root.join("pkg/deps")),
        "the folder inside a folder the version holds is not linked"
    );
    assert!(
        std::fs::symlink_metadata(export.join("new")).is_err(),
        "a folder the version does not hold was made for a link"
    );
    assert!(
        std::fs::symlink_metadata(export.join("vend/deps"))
            .unwrap()
            .is_dir(),
        "the version's own folder was replaced by a link"
    );
    assert!(export.join("vend/deps/kept.sil").is_file());
    assert!(
        !export.join(".git").exists(),
        "the export holds a git folder"
    );

    // The old side is dropped: the export goes, and the folders the links pointed at stay.
    running.ask(Ask::OldSide {
        workspace: ws.clone(),
        version: None,
    });
    settled(&mut running);
    assert!(!export.exists());
    for (file, text) in [
        ("deps/x.sil", "def x\n"),
        ("pkg/deps/y.sil", "def y\n"),
        ("deps/inner/deps/w.sil", "def w\n"),
    ] {
        assert_eq!(
            std::fs::read_to_string(repo.root.join(file))
                .as_deref()
                .ok(),
            Some(text),
            "{file} of the worktree was removed with the export"
        );
    }
    running.end();
}

#[test]
fn a_folder_the_version_lacks_is_neither_linked_into_nor_made_for_a_link_however_deep_it_lies() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    // The base holds `a/b`, two folders deep.
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse legacy\n"),
            ("a/b/lib.sil", "def lib\n"),
        ],
    );
    // The worktree holds a `deps` inside `a/b`, and an untracked tree, as a build leaves one,
    // with a `deps` far down it.
    for file in [
        "a/b/deps/x.sil",
        "target/debug/build/crate/deps/y.sil",
        "a/c/cache/deps/z.sil",
    ] {
        let file = repo.root.join(file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "def x\n").unwrap();
    }
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_linking_deps(&log)));
    running.show(&repo, &ws, &repository);
    let (asked, _) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));

    let export = &asked.into;
    assert_eq!(
        link_target(&export.join("a/b/deps")),
        Some(repo.root.join("a/b/deps")),
        "the folder two folders down in the version is not linked"
    );
    assert!(
        std::fs::symlink_metadata(export.join("target")).is_err(),
        "a folder the version does not hold was made for a link"
    );
    assert!(
        std::fs::symlink_metadata(export.join("a/c")).is_err(),
        "a folder of a folder the version holds, which it lacks itself, was made for a link"
    );
    running.end();
}

#[test]
fn a_link_is_never_made_through_a_folder_of_the_export_that_is_itself_a_link() {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    // The base holds `pkg` as a link to a folder of the scratch, outside the repository.
    let outside = scratch.root.join("outside");
    std::fs::create_dir(&outside).expect("make the folder the link points at");
    let repo = scratch.repository("project", &[("main.sil", "def alpha\nuse legacy\n")]);
    std::os::unix::fs::symlink(&outside, repo.root.join("pkg")).expect("link pkg");
    repo.git(&["add", "pkg"]);
    repo.git(&["commit", "--quiet", "-m", "a link"]);
    let base = repo.git(&["rev-parse", "HEAD"]);
    // The worktree has `pkg` as a real folder with a `deps` in it.
    std::fs::remove_file(repo.root.join("pkg")).expect("remove the link");
    std::fs::create_dir_all(repo.root.join("pkg/deps")).expect("make the folders");
    std::fs::write(repo.root.join("pkg/deps/y.sil"), "def y\n").expect("write the file");

    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_linking_deps(&log)));
    running.show(&repo, &ws, &repository);
    let at_the_base = place(&repo, &ws, Tree::Version(base), "main.sil", 2, "legacy", 0);
    let (asked, _) = use_the_old_side(&mut running, &repo, &ws, at_the_base);

    assert_eq!(
        link_target(&asked.into.join("pkg")),
        Some(outside.clone()),
        "the export does not hold the base's link"
    );
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        0,
        "a link was written through the export's link into a folder outside it"
    );
    running.end();
}

#[test]
fn a_file_of_an_export_opens_as_a_file_of_its_version_and_one_reached_through_a_link_as_the_worktrees()
 {
    let scratch = Scratch::new();
    scratch.put_standin(&scratch.root.join("bin"), "standin-lsp");
    let repo = scratch.repository(
        "project",
        &[
            ("main.sil", "def alpha\nuse legacy\n"),
            (".gitignore", "deps/\n"),
        ],
    );
    std::fs::create_dir_all(repo.root.join("deps")).unwrap();
    std::fs::write(repo.root.join("deps/x.sil"), "def x\n").unwrap();
    let log = scratch.root.join("standin.log");
    let (ws, repository) = (workspace_id('1'), repository_id('1'));
    let mut running = Running::start(scratch.config(&row_linking_deps(&log)));
    running.show(&repo, &ws, &repository);
    let (asked, _) = use_the_old_side(&mut running, &repo, &ws, removed(&repo, &ws));

    let mut open = |id: u64, path: PathBuf| {
        running.ask(Ask::Open {
            id: AskId(id),
            location: Location {
                path,
                line: 1,
                column: 0,
                text: String::new(),
            },
        });
        match running.wait_told(
            "the file opened",
            DEADLINE,
            |told| matches!(told, Told::Opened { id: got, .. } if *got == AskId(id)),
        ) {
            Told::Opened { file, .. } => file.expect("the file opens"),
            _ => unreachable!("only the answer to the ask satisfies the wait"),
        }
    };
    let in_the_export = open(1, asked.into.join("main.sil"));
    assert_eq!(in_the_export.text, "def alpha\nuse legacy\n");
    assert_eq!(
        in_the_export.kind,
        FileKind::Version {
            workspace: ws.clone(),
            version: repo.base.clone(),
            relative: PathBuf::from("main.sil"),
        }
    );
    // A file under a link is the worktree's own, which git ignores: it is not the version's.
    let through_the_link = open(2, asked.into.join("deps/x.sil"));
    assert_eq!(through_the_link.text, "def x\n");
    assert_eq!(through_the_link.kind, FileKind::Outside);
    running.end();
}
