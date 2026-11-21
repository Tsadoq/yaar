//! Component test: code intelligence in the composed Window on GPUI's test platform, with
//! the pane, the real Navigator and the real Keeper joined, and the stand-in language
//! server beneath. Keys, clicks and the wheel go in; what the pane shows is read back through
//! its model. Nothing of the answers is told by the test: the Navigator asks the stand-in, and
//! asks the Keeper, through the Window, to write the old side.
//!
//! A name's place on screen: the text of a line is drawn by an element whose selector is
//! `review-text-<row>-<side>:<text>`, and the font of the pane is of fixed width, so the
//! middle of the character at offset `k` is `left + (k + 0.5) * cell`, the cell being the
//! element's width over the text's characters.

mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, point};
use yaar_frame::LinkStatus;
use yaar_link::Request;
use yaar_navigator::{FileKind, Location, STOP_DEADLINE, State, Tree};
use yaar_review_pane::HoverShown;
use yaar_system_tests::{Repo, World, is_alive};

use common::{
    Composed, Found, drawn, give_an_identity, navigator_cache, navigator_for,
    open_composed_with_a_navigator, standin_row, standin_row_started_with, wait_until,
};

/// The file the change leaves in place, which defines the names the change uses.
const LIB: &str = "def total\ndef old_name\n";
/// `main.sil` as the base has it: its second line uses `old_name`.
const MAIN_BEFORE: &str = "def main\nuse old_name\n";
/// `main.sil` as the worktree has it: that line is gone and one using `total` is there.
const MAIN_AFTER: &str = "def main\nuse total\n";
const REMOVED: &str = "use old_name";
const ADDED: &str = "use total";

/// The repository of the test: `lib.sil` and `main.sil` committed on `main`, then `main.sil`
/// changed in the worktree on a branch of its own. Gives it and the commit of the base.
fn repository_with_a_change(world: &World) -> (Repo, String) {
    let repository = world.repository("code");
    give_an_identity(&repository);
    repository.write("lib.sil", LIB);
    repository.write("main.sil", MAIN_BEFORE);
    let base = repository.commit("Add the code");
    repository.git(&["checkout", "--quiet", "-b", "work"]);
    repository.write("main.sil", MAIN_AFTER);
    (repository, base)
}

/// The index of the row of the pane whose text of `side` is `text`, once it is drawn.
fn row_of(window: &Composed, side: &str, text: &str, cx: &mut VisualTestContext) -> usize {
    let mut found = None;
    wait_until(cx, &format!("the row of {text} on its {side} side"), |cx| {
        let rows = window.review.read_with(cx, |pane, _| pane.rows().len());
        found =
            (0..rows).find(|row| drawn(&format!("review-text-{row}-{side}:{text}"), cx).is_some());
        found.is_some()
    });
    found.expect("the row is drawn")
}

/// Clicks on the first byte of `name` in the text of a row, by the rule in the header.
fn click_name(row: usize, side: &str, text: &str, name: &str, cx: &mut VisualTestContext) {
    let bounds = drawn(&format!("review-text-{row}-{side}:{text}"), cx).expect("the text is drawn");
    let cell = bounds.size.width / text.chars().count() as f32;
    let offset = text[..text.find(name).expect("the line holds the name")]
        .chars()
        .count();
    cx.simulate_click(
        point(
            bounds.left() + cell * (offset as f32 + 0.5),
            bounds.center().y,
        ),
        Modifiers::none(),
    );
}

/// What the pane shows as the hover, once the Navigator answered it.
fn answered_hover(window: &Composed, cx: &mut VisualTestContext) -> HoverShown {
    let mut found = None;
    wait_until(cx, "the answer to the hover", |cx| {
        found = window
            .review
            .read_with(cx, |pane, _| pane.hover().cloned())
            .filter(|hover| !hover.waiting);
        found.is_some()
    });
    found.expect("the hover is answered")
}

/// The pane shows the state of the server of the code on screen as `state`.
fn the_line_says(state: &str, cx: &mut VisualTestContext) -> bool {
    drawn(&format!("review-servers-state:{state}"), cx).is_some()
}

/// The file the stand-in of a side logs to: `{root}/../standin.log`, beside the folder it
/// serves, so that each side has a log of its own (the worktree's parent for the new side,
/// the folder the export lies in for the old). The Navigator expands `{root}` as the folder
/// of the side, and the stand-in makes no folder, so the log lies where one already is.
const LOG_BESIDE_ROOT: &str = "{root}/../standin.log";

/// The log of the stand-in that serves `root`, as the row writes it.
fn log_beside(root: &Path) -> PathBuf {
    root.parent()
        .expect("the folder a server serves has a parent")
        .join("standin.log")
}

/// The lines of a stand-in's log, one JSON object each; none while it has not been made.
fn log_of(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// How many requests of `method` the stand-in logged. The window's tests read the log as
/// text, which is what its lines are: `"event":"request"` and `"method":"<method>"`.
fn requests(log: &Path, method: &str) -> usize {
    log_of(log)
        .iter()
        .filter(|line| {
            line.contains("\"event\":\"request\"")
                && line.contains(&format!("\"method\":\"{method}\""))
        })
        .count()
}

/// The folders the stand-in of `log` was started in, from the `cwd` of each `start` line.
fn started_in(log: &Path) -> Vec<PathBuf> {
    log_of(log)
        .iter()
        .filter(|line| line.contains("\"event\":\"start\""))
        .map(|line| {
            let from = line.find("\"cwd\":\"").expect("a start names its folder") + 7;
            let length = line[from..].find('"').expect("the folder ends");
            PathBuf::from(&line[from..from + length])
                .canonicalize()
                .expect("the folder of a start exists")
        })
        .collect()
}

// WIN-10
#[gpui_kit::test]
fn the_composed_window_answers_hover_and_definition_from_the_server_and_a_removed_line_from_the_old_side_the_keeper_exported(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, base) = repository_with_a_change(&world);
    let folder = repository.path().to_owned();
    // Read before the Window opens the folder: from then on a server may run in it.
    let status = repository.git(&["status", "--porcelain"]);
    let navigator = navigator_for(
        &world,
        Found::OnTheMachine,
        &standin_row_started_with(
            Found::OnTheMachine,
            "main.sil",
            &[],
            &["--log", LOG_BESIDE_ROOT],
        ),
    );
    let (mut window, mut cx) = open_composed_with_a_navigator(world, cx, Vec::new(), navigator);
    window.open_this_folder(&folder, &mut cx);
    window.show_the_changes_view(&mut cx);

    // The reviewer comes to a name of an added line: the pane names the server of that file,
    // which the Navigator found, started, and says is ready.
    let added = row_of(&window, "new", ADDED, &mut cx);
    window.click_into_the_review_pane(&mut cx);
    click_name(added, "new", ADDED, "total", &mut cx);
    wait_until(
        &mut cx,
        "the stand-in ready on the line of the pane",
        |cx| the_line_says("ready", cx),
    );
    assert_eq!(
        window
            .review
            .read_with(&cx, |pane, _| pane.name_at())
            .as_deref(),
        Some("total")
    );
    let before = window
        .review
        .read_with(&cx, |pane, _| (pane.position().cloned(), pane.name_at()));
    cx.simulate_keystrokes("shift-k");
    let hover = answered_hover(&window, &mut cx);
    assert_eq!(hover.text.as_deref(), Some("standin: total"));
    assert!(!hover.guessed, "the stand-in was ready, so no guess");
    assert_eq!(hover.place.tree, Tree::Worktree);
    assert_eq!(hover.place.path, PathBuf::from("main.sil"));
    assert_eq!((hover.place.line, hover.place.name.as_str()), (2, "total"));

    // Go to definition opens the file view of the file that defines it, at the line.
    cx.simulate_keystrokes("g d");
    let mut shown = None;
    wait_until(&mut cx, "the file view of the definition", |cx| {
        shown = window
            .review
            .read_with(cx, |pane, _| pane.file_view().cloned());
        shown.is_some()
    });
    let shown = shown.expect("the file view is open");
    assert_eq!(shown.file.path, folder.join("lib.sil"));
    assert!(
        matches!(&shown.file.kind, FileKind::Worktree { relative, .. } if relative == Path::new("lib.sil")),
        "{:?} is not lib.sil of the worktree",
        shown.file.kind
    );
    assert_eq!(shown.file.text, LIB);
    assert_eq!(shown.line, 1, "the line of `def total`");
    assert!(drawn("review-file-view", &mut cx).is_some());
    assert!(drawn("review-file-at", &mut cx).is_some());
    assert!(drawn(&format!("review-row-{added}"), &mut cx).is_none());

    // The return control comes back to the place left in the diff.
    cx.simulate_keystrokes("escape");
    assert_eq!(
        window
            .review
            .read_with(&cx, |pane, _| pane.file_view().cloned()),
        None
    );
    assert_eq!(
        window
            .review
            .read_with(&cx, |pane, _| (pane.position().cloned(), pane.name_at())),
        before
    );
    assert!(drawn(&format!("review-row-{added}"), &mut cx).is_some());

    // Hover on a removed line asks for the old side, which the Keeper writes into the cache
    // folder the Navigator named. The first answer is found by text match while it is
    // written (the old side's server is not there yet), so the test waits for the old
    // side's server to be ready, and asks again.
    let removed = row_of(&window, "old", REMOVED, &mut cx);
    click_name(removed, "old", REMOVED, "old_name", &mut cx);
    assert_eq!(
        window
            .review
            .read_with(&cx, |pane, _| pane.name_at())
            .as_deref(),
        Some("old_name")
    );
    cx.simulate_keystrokes("shift-k");
    let first = answered_hover(&window, &mut cx);
    assert_eq!(first.place.tree, Tree::Version(base.clone()));
    assert_eq!(first.place.path, PathBuf::from("main.sil"));
    assert_eq!(first.place.line, 2);
    let exported_by_the_keeper = |window: &Composed| {
        window
            .tap
            .requests()
            .into_iter()
            .find_map(|request| match request {
                Request::ExportVersion { version, into, .. } if version == base => Some(into),
                _ => None,
            })
    };
    let old_side_ready = |window: &Composed, cx: &mut VisualTestContext| {
        wait_until(cx, "the server of the old side ready", |cx| {
            window.models.navigation.read_with(cx, |navigation, _| {
                navigation.servers().iter().any(|server| {
                    server.old_side.as_deref() == Some(base.as_str())
                        && server.state == State::Ready
                        && server.pid.is_some()
                })
            })
        })
    };
    old_side_ready(&window, &mut cx);
    cx.simulate_keystrokes("shift-k");
    let hover = answered_hover(&window, &mut cx);
    assert_eq!(hover.text.as_deref(), Some("standin: old_name"));
    assert!(!hover.guessed, "the old side's server answered");
    assert_eq!(hover.place.tree, Tree::Version(base.clone()));

    // The old side is the export the Keeper wrote in the app's cache folder of this world, and
    // the worktree no longer holds the name the answer is about.
    let into = exported_by_the_keeper(&window).expect("the Keeper was asked to export the base");
    assert!(
        into.starts_with(navigator_cache(&window.world)),
        "{into:?} lies outside the cache folder of the world"
    );
    assert_eq!(
        std::fs::read_to_string(into.join("main.sil")).expect("the export holds main.sil"),
        MAIN_BEFORE
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("main.sil")).expect("the worktree's file"),
        MAIN_AFTER
    );
    // The repository holds nothing of it.
    assert_eq!(repository.git(&["status", "--porcelain"]), status);

    // Which server answered: each side's stand-in logs beside its own root. The answer was
    // read before this, and a server logs a request before it answers, so what an answer
    // came from is in a log by now. The new side's server was started in the worktree and
    // was asked only about the added line; the old side's was started in the export and
    // answered the removed line, once: the first hover on it is answered by text match while
    // the export is written, so only the second reaches the server.
    let (new_log, old_log) = (log_beside(&folder), log_beside(&into));
    assert_ne!(new_log, old_log, "the two sides share a log");
    assert_eq!(
        started_in(&new_log),
        [folder.canonicalize().expect("the worktree")]
    );
    assert_eq!(
        started_in(&old_log),
        [into.canonicalize().expect("the export")]
    );
    assert_eq!(
        requests(&new_log, "textDocument/hover"),
        1,
        "the new side's server was asked about the removed line"
    );
    assert_eq!(
        requests(&old_log, "textDocument/hover"),
        1,
        "the old side's server was not asked about the removed line exactly once"
    );

    window.end_the_navigator(&mut cx);
    // The end of the Navigator removed what it exported, and the call waited for that.
    assert!(
        !Path::new(&into).exists(),
        "the export outlived the Navigator"
    );
}

/// A composed Window with a Navigator, on the repository of the test with the stand-in
/// running for the file on screen: gives it, its context and the process of the server.
fn with_a_server_running(cx: &mut TestAppContext) -> (Composed, VisualTestContext, u32) {
    let (window, inner, pid, _) = with_a_server_running_over(cx);
    (window, inner, pid)
}

/// [`with_a_server_running`], and the commit of the base the change is counted from.
fn with_a_server_running_over(
    cx: &mut TestAppContext,
) -> (Composed, VisualTestContext, u32, String) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, base) = repository_with_a_change(&world);
    let folder = repository.path().to_owned();
    let navigator = navigator_for(
        &world,
        Found::OnTheMachine,
        &standin_row(Found::OnTheMachine, "main.sil", &[]),
    );
    let (mut window, mut inner) = open_composed_with_a_navigator(world, cx, Vec::new(), navigator);
    window.open_this_folder(&folder, &mut inner);
    window.show_the_changes_view(&mut inner);
    let added = row_of(&window, "new", ADDED, &mut inner);
    window.click_into_the_review_pane(&mut inner);
    click_name(added, "new", ADDED, "total", &mut inner);
    let mut pid = None;
    wait_until(&mut inner, "the stand-in running", |cx| {
        pid = window.models.navigation.read_with(cx, |navigation, _| {
            navigation
                .servers()
                .iter()
                .find(|server| server.state == State::Ready)
                .and_then(|server| server.pid)
        });
        pid.is_some()
    });
    let pid = pid.expect("a server runs");
    assert!(is_alive(pid));
    (window, inner, pid, base)
}

// Of its own: the app's quit ends the Navigator and the servers it started, as a user's
// quit does, and the Window waits for it.
#[gpui_kit::test]
fn the_quit_of_the_app_stops_every_server_before_it_goes_on(cx: &mut TestAppContext) {
    let (_window, _inner, pid) = with_a_server_running(cx);

    cx.update(|app| app.shutdown());
    // The quit waited for it: no wait here.
    assert!(
        !is_alive(pid),
        "the server {pid} outlived the quit of the app"
    );
}

/// Asks the Navigator of the window whose models these are for the hover on `total` in the
/// worktree of `workspace`, and waits for its answer: the server of the stand-in is started
/// by that ask. Then waits until the model shows the stand-in ready, and gives its process.
fn start_the_standin_in(
    models: &yaar_frame::Models,
    workspace: &yaar_link::WorkspaceId,
    cx: &mut VisualTestContext,
    app: &mut TestAppContext,
) -> u32 {
    let place = yaar_navigator::Place {
        workspace: workspace.clone(),
        tree: Tree::Worktree,
        path: PathBuf::from("main.sil"),
        line: 2,
        column: 4,
        name: "total".to_owned(),
        shown: Vec::new(),
    };
    let asking = models
        .navigation
        .update(cx, |navigation, cx| navigation.hover(place, cx));
    let answered = Rc::new(RefCell::new(false));
    let _waiting = app.foreground_executor().spawn({
        let answered = answered.clone();
        async move {
            let _ = asking.await;
            *answered.borrow_mut() = true;
        }
    });
    wait_until(cx, "the answer to the hover", |_| *answered.borrow());
    let mut pid = None;
    wait_until(cx, "the stand-in of the window ready", |cx| {
        pid = models.navigation.read_with(cx, |navigation, _| {
            navigation
                .servers()
                .iter()
                .find(|server| server.state == State::Ready)
                .and_then(|server| server.pid)
        });
        pid.is_some()
    });
    pid.expect("a server runs")
}

/// Lets a server that a test stopped go on again when the test ends, so that none is left
/// stopped by a test that failed before the Navigator killed it.
struct Resumed(u32);

impl Drop for Resumed {
    fn drop(&mut self) {
        // SAFETY: signal CONT to a process the test stopped; one that is gone takes none.
        unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGCONT) };
    }
}

// Of its own: a quit that finds a hover waiting at a server that does not answer and an export
// the Keeper has not answered ends the Navigator in time, and leaves no server. Both hold a
// way to the Navigator's asks, which the quit has to close.
#[gpui_kit::test]
fn a_quit_with_a_hover_waiting_at_a_slow_server_and_an_export_the_keeper_has_not_answered_ends_in_time_and_leaves_no_server(
    cx: &mut TestAppContext,
) {
    let (mut window, mut inner, pid, base) = with_a_server_running_over(cx);
    let workspace = window
        .models
        .workspaces
        .read_with(&inner, |list, _| list.selected().cloned())
        .expect("a workspace is on screen");
    let place = |tree, name: &str| yaar_navigator::Place {
        workspace: workspace.clone(),
        tree,
        path: PathBuf::from("main.sil"),
        line: 2,
        column: 4,
        name: name.to_owned(),
        shown: Vec::new(),
    };

    // The server reads nothing: the hover waits at it.
    let _resumed_server = Resumed(pid);
    // SAFETY: signal STOP to the stand-in this test started.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) }, 0);
    let hover = window
        .models
        .navigation
        .update(&mut inner, |navigation, cx| {
            navigation.hover(place(Tree::Worktree, "total"), cx)
        });
    // The Keeper reads nothing either: the export of the old side that the first ask about
    // it wants is on its way and stays so.
    let keeper = window.world.keeper_pid().expect("the Keeper runs");
    let _resumed_keeper = Resumed(keeper);
    // SAFETY: signal STOP to the Keeper this test's world started.
    assert_eq!(
        unsafe { libc::kill(keeper as libc::pid_t, libc::SIGSTOP) },
        0
    );
    let old_side = window
        .models
        .navigation
        .update(&mut inner, |navigation, cx| {
            navigation.hover(place(Tree::Version(base.clone()), "old_name"), cx)
        });
    // Neither ask is answered and neither is dropped, as a pane's ask is not while it waits.
    let _waiting = [
        cx.foreground_executor().spawn(async move {
            let _ = hover.await;
        }),
        cx.foreground_executor().spawn(async move {
            let _ = old_side.await;
        }),
    ];
    wait_until(&mut inner, "the export on its way to the Keeper", |_| {
        window.tap.requests().iter().any(
            |request| matches!(request, Request::ExportVersion { version, .. } if *version == base),
        )
    });

    let began = Instant::now();
    cx.update(|app| app.shutdown());
    let took = began.elapsed();
    // The quit waited for the Navigator: no wait here.
    assert!(
        !is_alive(pid),
        "the server {pid} outlived the quit of the app"
    );
    assert!(
        took < STOP_DEADLINE + Duration::from_secs(1),
        "the quit took {took:?}"
    );
}

// Of its own: the quit of the app ends the Navigator of every window, not the first's alone.
#[gpui_kit::test]
fn the_quit_of_the_app_stops_the_servers_of_every_window(cx: &mut TestAppContext) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, _) = repository_with_a_change(&world);
    let folder = repository.path().to_owned();
    cx.executor().allow_parking();
    cx.update(yaar_window::init);
    let (first, mut first_cx) = open_with(&world, cx);
    let (second, mut second_cx) = open_with(&world, cx);
    wait_until(
        &mut first_cx,
        "the Keeper to answer the first window",
        |cx| {
            first.link.read_with(cx, |link, _| {
                matches!(link.status(), LinkStatus::Connected(_))
            })
        },
    );
    first.workspaces.update(&mut first_cx, |list, cx| {
        list.open_folder(folder.clone(), cx)
    });
    let mut workspace = None;
    wait_until(&mut first_cx, "the workspace on screen", |cx| {
        workspace = first
            .workspaces
            .read_with(cx, |list, _| list.selected().cloned());
        workspace.is_some()
    });
    let workspace = workspace.expect("a workspace is on screen");
    // The Keeper tells every window of its workspaces, so the second Navigator has it too.
    wait_until(&mut second_cx, "the workspace in the second window", |cx| {
        second
            .workspaces
            .read_with(cx, |list, _| list.workspace(&workspace).is_some())
    });

    let first_pid = start_the_standin_in(&first, &workspace, &mut first_cx, cx);
    let second_pid = start_the_standin_in(&second, &workspace, &mut second_cx, cx);
    assert_ne!(
        first_pid, second_pid,
        "each Navigator runs a server of its own"
    );
    assert!(is_alive(first_pid) && is_alive(second_pid));

    // The second server is stopped, so that it reads no `shutdown` and cannot end of itself:
    // its Navigator kills it after its deadline, and only a quit that waits for the
    // Navigator of every window is still waiting then.
    let _resumed = Resumed(second_pid);
    // SAFETY: signal STOP to the stand-in this test started.
    assert_eq!(
        unsafe { libc::kill(second_pid as libc::pid_t, libc::SIGSTOP) },
        0
    );

    cx.update(|app| app.shutdown());
    // The quit waited for both: no wait here.
    assert!(
        !is_alive(first_pid),
        "the server {first_pid} of the first window outlived the quit of the app"
    );
    assert!(
        !is_alive(second_pid),
        "the server {second_pid} of the second window outlived the quit of the app"
    );
}

// Of its own: ending a Navigator waits for its servers. One that is stopped, so that it
// reads no `shutdown` and cannot end of itself, is killed after the Navigator's deadline,
// and the call returns only once it is reaped.
#[gpui_kit::test]
fn ending_a_navigator_returns_only_once_a_server_that_does_not_end_is_gone(
    cx: &mut TestAppContext,
) {
    let (mut window, mut inner, pid) = with_a_server_running(cx);
    let _resumed = Resumed(pid);
    // SAFETY: signal STOP to the stand-in this test started.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) }, 0);

    // The server is not gone by itself, so a call that did not wait for it leaves it, and
    // `end_the_navigator` looks for it as soon as the call returns.
    window.end_the_navigator(&mut inner);
    assert!(!is_alive(pid));
}

/// Opens a window as `open_with` does, with a Navigator whose registry has the stand-in's
/// row, against the Keeper of `world`.
fn open_with(world: &World, cx: &mut TestAppContext) -> (yaar_frame::Models, VisualTestContext) {
    let navigator = navigator_for(
        world,
        Found::OnTheMachine,
        &standin_row(Found::OnTheMachine, "main.sil", &[]),
    );
    let socket = world.socket();
    let yaar_frame::Opened { window, models, .. } = cx
        .update(|cx| yaar_window::open_with(socket, navigator, cx))
        .expect("open the window");
    (models.clone(), VisualTestContext::from_window(window, cx))
}

// Of its own: `open_with`, which the program's `open` is, hands the Navigator to the window.
#[gpui_kit::test]
fn a_window_opened_with_a_configuration_has_a_navigator_that_tells_its_languages(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    cx.executor().allow_parking();
    cx.update(yaar_window::init);
    let (models, mut inner) = open_with(&world, cx);
    wait_until(&mut inner, "the Navigator's languages", |cx| {
        models.navigation.read_with(cx, |navigation, _| {
            navigation.is_connected()
                && navigation
                    .languages()
                    .iter()
                    .any(|language| language.name == "standin")
        })
    });
    cx.update(|app| app.shutdown());
    // Ended by the quit: nothing is connected to ask, and the model says so once it heard.
    wait_until(&mut inner, "the model to see the Navigator end", |cx| {
        models
            .navigation
            .read_with(cx, |navigation, _| navigation.languages().is_empty())
    });
}

// Of its own: each window has a Navigator of its own, and ending one leaves the other.
#[gpui_kit::test]
fn each_window_has_a_navigator_of_its_own(cx: &mut TestAppContext) {
    let mut world = World::new();
    world.start_keeper();
    cx.executor().allow_parking();
    cx.update(yaar_window::init);
    let (first, mut first_cx) = open_with(&world, cx);
    let (second, mut second_cx) = open_with(&world, cx);
    for (models, window_cx) in [(&first, &mut first_cx), (&second, &mut second_cx)] {
        wait_until(window_cx, "the languages of the window's Navigator", |cx| {
            models
                .navigation
                .read_with(cx, |navigation, _| !navigation.languages().is_empty())
        });
    }

    first
        .navigation
        .update(&mut first_cx, |navigation, cx| navigation.disconnect(cx));
    assert!(
        !first
            .navigation
            .read_with(&first_cx, |navigation, _| navigation.is_connected())
    );
    // The second Navigator was not the first's: it still listens and answers. The model's
    // flags only change when the end of a Navigator reaches it, which may be a turn later
    // than this look, so the test asks the second Navigator for a file and waits for what
    // it tells: an ended Navigator answers nothing but a refusal.
    let file = world.home().join("barrier.sil");
    std::fs::write(&file, "def barrier\n").expect("write the file");
    let asking = second.navigation.update(&mut second_cx, |navigation, cx| {
        navigation.open(
            Location {
                path: file.clone(),
                line: 1,
                column: 0,
                text: String::new(),
            },
            cx,
        )
    });
    let answer = Rc::new(RefCell::new(None));
    let _waiting = cx.foreground_executor().spawn({
        let answer = answer.clone();
        async move {
            let told = asking.await;
            *answer.borrow_mut() = Some(told);
        }
    });
    wait_until(&mut second_cx, "the second Navigator to answer", |_| {
        answer.borrow().is_some()
    });
    let opened = answer
        .take()
        .expect("an answer")
        .expect("the second Navigator is still connected")
        .expect("the second Navigator reads the file");
    assert_eq!(opened.text, "def barrier\n");
    assert!(
        second
            .navigation
            .read_with(&second_cx, |navigation, _| navigation.is_connected())
    );
    // Letting go of the first again is a call that finds nothing to let go of.
    first
        .navigation
        .update(&mut first_cx, |navigation, cx| navigation.disconnect(cx));
    assert!(second.navigation.read_with(&second_cx, |navigation, _| {
        !navigation.languages().is_empty()
    }));
    // The quit of the app ends the second, which the first's end left running.
    cx.update(|app| app.shutdown());
    wait_until(&mut second_cx, "the model to see the Navigator end", |cx| {
        second
            .navigation
            .read_with(cx, |navigation, _| !navigation.is_connected())
    });
}
