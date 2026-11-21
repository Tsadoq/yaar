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

use std::path::{Path, PathBuf};

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, point};
use yaar_link::Request;
use yaar_navigator::{FileKind, State, Tree};
use yaar_review_pane::HoverShown;
use yaar_system_tests::{DEADLINE, Repo, World, is_alive, wait_until as wait_for};

use common::{
    Composed, Found, drawn, give_an_identity, navigator_cache, navigator_for,
    open_composed_with_a_navigator, standin_row, wait_until,
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

// WIN-10
#[gpui_kit::test]
fn the_composed_window_answers_hover_and_definition_from_the_server_and_a_removed_line_from_the_old_side_the_keeper_exported(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, base) = repository_with_a_change(&world);
    let folder = repository.path().to_owned();
    let navigator = navigator_for(
        &world,
        Found::OnTheMachine,
        &standin_row(Found::OnTheMachine, "main.sil", &[]),
    );
    let (mut window, mut cx) = open_composed_with_a_navigator(world, cx, Vec::new(), navigator);
    window.open_this_folder(&folder, &mut cx);
    window.show_the_changes_view(&mut cx);
    let status = repository.git(&["status", "--porcelain"]);

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

    window.end_the_navigator(&mut cx);
    assert!(
        wait_for(DEADLINE, || !Path::new(&into).exists()),
        "the export outlived the Navigator"
    );
}

// Of its own: the app's quit ends the Navigator and the servers it started, as a user's
// quit does, and the Window waits for it.
#[gpui_kit::test]
fn the_quit_of_the_app_stops_every_server_before_it_goes_on(cx: &mut TestAppContext) {
    let mut world = World::new();
    world.start_keeper();
    let (repository, _) = repository_with_a_change(&world);
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

    cx.update(|app| app.shutdown());
    // The quit waited for it: no wait here.
    assert!(
        !is_alive(pid),
        "the server {pid} outlived the quit of the app"
    );
}
