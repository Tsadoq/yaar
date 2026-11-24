//! Code intelligence in the composed Window, with the real Navigator and a stand-in server.
//! A line's text is the element `review-text-<row>-<side>:<text>`, fixed-width: the middle of the
//! character at offset `k` is `left + (k + 0.5) * cell`.

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

const LIB: &str = "def total\ndef old_name\n";
const MAIN_BEFORE: &str = "def main\nuse old_name\n";
const MAIN_AFTER: &str = "def main\nuse total\n";
const REMOVED: &str = "use old_name";
const ADDED: &str = "use total";

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

fn the_line_says(state: &str, cx: &mut VisualTestContext) -> bool {
    drawn(&format!("review-servers-state:{state}"), cx).is_some()
}

/// Each side's stand-in logs beside the folder it serves; the Navigator expands `{root}`.
const LOG_BESIDE_ROOT: &str = "{root}/../standin.log";

fn log_beside(root: &Path) -> PathBuf {
    root.parent()
        .expect("the folder a server serves has a parent")
        .join("standin.log")
}

fn log_of(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn requests(log: &Path, method: &str) -> usize {
    log_of(log)
        .iter()
        .filter(|line| {
            line.contains("\"event\":\"request\"")
                && line.contains(&format!("\"method\":\"{method}\""))
        })
        .count()
}

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

#[gpui_kit::test]
fn the_composed_window_answers_hover_and_definition_from_the_server_and_a_removed_line_from_the_old_side_the_keeper_exported(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, base) = repository_with_a_change(&world);
    let folder = repository.path().to_owned();
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

    // The first hover on the old side is answered by text match while the Keeper writes the
    // export, so the test waits for the old side's server to be ready and asks again.
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
    assert_eq!(repository.git(&["status", "--porcelain"]), status);

    // A server logs a request before it answers. The new side's server had the added line's
    // hover; the old side's only the second hover of the removed line, the first a text match.
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
    assert!(
        !Path::new(&into).exists(),
        "the export outlived the Navigator"
    );
}

fn with_a_server_running(cx: &mut TestAppContext) -> (Composed, VisualTestContext, u32) {
    let (window, inner, pid, _) = with_a_server_running_over(cx);
    (window, inner, pid)
}

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

#[gpui_kit::test]
fn the_quit_of_the_app_stops_every_server_before_it_goes_on(cx: &mut TestAppContext) {
    let (_window, _inner, pid) = with_a_server_running(cx);

    cx.update(|app| app.shutdown());
    assert!(
        !is_alive(pid),
        "the server {pid} outlived the quit of the app"
    );
}

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

fn open_the_folder_in(
    models: &yaar_frame::Models,
    folder: &Path,
    cx: &mut VisualTestContext,
) -> yaar_link::WorkspaceId {
    wait_until(cx, "the Keeper to answer the window", |cx| {
        models.link.read_with(cx, |link, _| {
            matches!(link.status(), LinkStatus::Connected(_))
        })
    });
    models
        .workspaces
        .update(cx, |list, cx| list.open_folder(folder.to_owned(), cx));
    let mut workspace = None;
    wait_until(cx, "the workspace on screen", |cx| {
        workspace = models
            .workspaces
            .read_with(cx, |list, _| list.selected().cloned());
        workspace.is_some()
    });
    workspace.expect("a workspace is on screen")
}

/// Lets a stopped server go on when the test ends, even if it fails first.
struct Resumed(u32);

impl Drop for Resumed {
    fn drop(&mut self) {
        // SAFETY: signal CONT to a process the test stopped; one that is gone takes none.
        unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGCONT) };
    }
}

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

    let _resumed_server = Resumed(pid);
    // SAFETY: signal STOP to the stand-in this test started.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) }, 0);
    let hover = window
        .models
        .navigation
        .update(&mut inner, |navigation, cx| {
            navigation.hover(place(Tree::Worktree, "total"), cx)
        });
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
    assert!(
        !is_alive(pid),
        "the server {pid} outlived the quit of the app"
    );
    assert!(
        took < STOP_DEADLINE + Duration::from_secs(1),
        "the quit took {took:?}"
    );
}

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

    // Stopped, the second server cannot end of itself: its Navigator kills it after the deadline,
    // and only a quit that waits for every window's Navigator is still waiting then.
    let _resumed = Resumed(second_pid);
    // SAFETY: signal STOP to the stand-in this test started.
    assert_eq!(
        unsafe { libc::kill(second_pid as libc::pid_t, libc::SIGSTOP) },
        0
    );

    cx.update(|app| app.shutdown());
    assert!(
        !is_alive(first_pid),
        "the server {first_pid} of the first window outlived the quit of the app"
    );
    assert!(
        !is_alive(second_pid),
        "the server {second_pid} of the second window outlived the quit of the app"
    );
}

#[gpui_kit::test]
fn ending_a_navigator_returns_only_once_a_server_that_does_not_end_is_gone(
    cx: &mut TestAppContext,
) {
    let (mut window, mut inner, pid) = with_a_server_running(cx);
    let _resumed = Resumed(pid);
    // SAFETY: signal STOP to the stand-in this test started.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) }, 0);

    window.end_the_navigator(&mut inner);
    assert!(!is_alive(pid));
}

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

#[gpui_kit::test]
fn a_window_opened_with_a_configuration_has_a_navigator_that_tells_its_languages(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, _) = repository_with_a_change(&world);
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
    let workspace = open_the_folder_in(&models, repository.path(), &mut inner);
    let pid = start_the_standin_in(&models, &workspace, &mut inner, cx);
    assert!(is_alive(pid));

    cx.update(|app| app.shutdown());
    assert!(
        !is_alive(pid),
        "the server {pid} outlived the quit of the app"
    );
    wait_until(&mut inner, "the model to see the Navigator end", |cx| {
        models
            .navigation
            .read_with(cx, |navigation, _| navigation.languages().is_empty())
    });
}

#[gpui_kit::test]
fn each_window_has_a_navigator_of_its_own(cx: &mut TestAppContext) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, _) = repository_with_a_change(&world);
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
    let workspace = open_the_folder_in(&first, repository.path(), &mut first_cx);
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

    first
        .navigation
        .update(&mut first_cx, |navigation, cx| navigation.disconnect(cx));
    assert!(
        !first
            .navigation
            .read_with(&first_cx, |navigation, _| navigation.is_connected())
    );
    wait_until(
        &mut first_cx,
        "the first window's server to be gone",
        |_| !is_alive(first_pid),
    );
    // The model's flags change only when a Navigator's end reaches it, maybe a turn later, so
    // the test asks the second Navigator for a file: an ended one would answer only a refusal.
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
    first
        .navigation
        .update(&mut first_cx, |navigation, cx| navigation.disconnect(cx));
    assert!(second.navigation.read_with(&second_cx, |navigation, _| {
        !navigation.languages().is_empty()
    }));
    assert!(
        is_alive(second_pid),
        "the first window's end took the second window's server with it"
    );
    cx.update(|app| app.shutdown());
    assert!(
        !is_alive(second_pid),
        "the server {second_pid} of the second window outlived the quit of the app"
    );
    wait_until(&mut second_cx, "the model to see the Navigator end", |cx| {
        second
            .navigation
            .read_with(cx, |navigation, _| !navigation.is_connected())
    });
}
