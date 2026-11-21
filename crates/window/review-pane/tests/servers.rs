//! Component test: the line of the language server of the code on screen, on GPUI's test
//! platform. The test plays the Keeper on a socket pair and the Navigator at its port: it
//! reads the asks the pane really makes and tells the states of the servers as the
//! Navigator would, as data. Keys, clicks and the wheel go in; the line is read back
//! through what it draws and through the model the pane shows.

mod common;

use std::path::PathBuf;

use gpui_kit::{Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px};
use yaar_diff::Row;
use yaar_link::{Fact, FileStatus, RepositoryId, Setting};
use yaar_navigator::{Ask, Language, Offer, ServerState, State, Told};
use yaar_review_pane::servers::{ALLOW, DOWNLOAD, REFUSE, UPDATE};
use yaar_review_pane::{PaneRow, Position};

use common::navigator::NavigatorEnd;
use common::{
    COMMIT, Stage, bounds_of_row, drawn, file, id, open, since_main, texts, wait_for_files,
    wait_until, workspace,
};

/// The repository every test workspace lies in, as `common::repository` makes it.
fn repository() -> RepositoryId {
    RepositoryId::new("a".repeat(32))
}

/// Another repository, one the Keeper listed no workspace of.
fn other_repository() -> RepositoryId {
    RepositoryId::new("b".repeat(32))
}

fn offer() -> Offer {
    Offer {
        server: "rust-analyzer".to_owned(),
        version: "2026-11-09".to_owned(),
        size: 20_000_000,
    }
}

fn newer() -> Offer {
    Offer {
        version: "2026-12-01".to_owned(),
        ..offer()
    }
}

fn languages() -> Vec<Language> {
    let language = |name: &str, title: &str, extension: &str| Language {
        name: name.to_owned(),
        title: title.to_owned(),
        extensions: vec![extension.to_owned()],
    };
    vec![
        language("rust", "Rust", "rs"),
        language("python", "Python", "py"),
    ]
}

/// The state of a server of the files on disk of workspace `1`, as the Navigator tells it.
fn server(language: &str, title: &str, state: State) -> ServerState {
    ServerState {
        workspace: id('1'),
        language: language.to_owned(),
        title: title.to_owned(),
        old_side: None,
        registered: false,
        state,
        server: None,
        program: None,
        pid: None,
        update: None,
    }
}

fn rust(state: State) -> ServerState {
    server("rust", "Rust", state)
}

fn python(state: State) -> ServerState {
    server("python", "Python", state)
}

/// The server of the old side of the change, read in `COMMIT`.
fn old_side(of: ServerState) -> ServerState {
    ServerState {
        old_side: Some(COMMIT.to_owned()),
        ..of
    }
}

fn missing_with_offer() -> State {
    State::Missing {
        offer: Some(offer()),
    }
}

fn asking(repository: RepositoryId) -> State {
    State::Asking {
        repository,
        program: PathBuf::from("/gits/feature/tools/rust-analyzer"),
    }
}

/// Which line of a changed file the reviewer goes to.
#[derive(Clone, Copy)]
enum Line {
    Removed,
    Added,
}

/// A window with the pane showing a change of `app/main.py` and `src/lib.rs` (each file
/// removes the line `gone` and adds the line `added`, with an unchanged line between them),
/// `README.md`, and the Navigator connected and telling its languages. The reviewer has
/// not clicked yet.
fn open_on_code(cx: &mut TestAppContext) -> (Stage, NavigatorEnd, VisualTestContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        Vec::new(),
        vec![
            file("README.md", FileStatus::Modified),
            file("app/main.py", FileStatus::Modified),
            file("src/lib.rs", FileStatus::Modified),
        ],
    );
    stage.keeper.show(
        &changes,
        |_| texts("start\ngone\nmiddle\n", "start\nmiddle\nadded\n"),
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    let navigator = NavigatorEnd::connect(&stage.models, &mut cx);
    navigator.tell(Told::Languages(languages()), &mut cx);
    (stage, navigator, cx)
}

/// Clicks the line of `path` that `which` names, which puts the reviewer on it and gives
/// the pane the focus.
fn go_to(stage: &Stage, path: &str, which: Line, cx: &mut VisualTestContext) {
    let index = stage
        .pane
        .read_with(cx, |pane, _| {
            pane.rows().iter().position(|row| match row {
                PaneRow::Diff { path: of, row } if of == &PathBuf::from(path) => match which {
                    Line::Removed => matches!(row, Row::Removed { .. }),
                    Line::Added => matches!(row, Row::Added { .. }),
                },
                _ => false,
            })
        })
        .unwrap_or_else(|| panic!("no such line of {path} in the rows"));
    let at = bounds_of_row(index, cx).center();
    cx.simulate_click(at, Modifiers::none());
    let position = stage
        .pane
        .read_with(cx, |pane, _| pane.position().cloned())
        .expect("the click put the reviewer on a line");
    assert_eq!(position.path, PathBuf::from(path));
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
}

/// The state the line draws, if it draws one: of the five the module doc names.
fn drawn_state(cx: &mut VisualTestContext) -> Option<&'static str> {
    ["ready", "starting", "missing", "asking", "downloading"]
        .into_iter()
        .find(|state| drawn(&format!("review-servers-state:{state}"), cx).is_some())
}

/// Holds the line to what it says: the state, and the words of its text.
#[track_caller]
fn assert_line(state: &str, words: &str, cx: &mut VisualTestContext) {
    assert!(drawn("review-servers", cx).is_some(), "no line is drawn");
    assert_eq!(drawn_state(cx), Some(state), "the state drawn");
    assert!(
        drawn(&format!("review-servers-text:{words}"), cx).is_some(),
        "the line does not say {words:?}"
    );
}

/// No dialog came up, and the app holds the one window it had.
#[track_caller]
fn assert_no_dialog(cx: &mut VisualTestContext) {
    assert!(!cx.has_pending_prompt(), "a dialog is open");
    assert_eq!(cx.windows().len(), 1, "the app holds more than one window");
}

fn has_control(name: &str, cx: &mut VisualTestContext) -> bool {
    drawn(&format!("review-servers-{name}"), cx).is_some()
}

#[gpui_kit::test]
fn a_missing_language_server_is_said_on_a_line_that_stays_in_the_pane_and_the_download_is_offered(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_on_code(cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);
    // Until the Navigator tells a server there is no line.
    assert!(drawn("review-servers", &mut cx).is_none());

    navigator.tell(Told::Servers(vec![rust(missing_with_offer())]), &mut cx);

    assert_line("missing", "Rust: no language server found", &mut cx);
    let words = format!("{DOWNLOAD} rust-analyzer 2026-11-09");
    assert!(has_control("download", &mut cx));
    assert!(drawn(&format!("review-servers-download:{words}"), &mut cx).is_some());
    assert_no_dialog(&mut cx);

    // The control asks for the download of the row, by a click and by its key.
    let at = drawn("review-servers-download", &mut cx).unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(
        navigator.ask(&mut cx),
        Ask::Download {
            language: "rust".to_owned()
        }
    );
    assert_no_dialog(&mut cx);
    // A click is on the line, not on the rows: the reviewer's line stays and the pane
    // keeps its focus for the key.
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    cx.simulate_keystrokes("shift-i");
    assert_eq!(
        navigator.ask(&mut cx),
        Ask::Download {
            language: "rust".to_owned()
        }
    );
    assert_no_dialog(&mut cx);

    // The line stays through everything the reviewer does in the pane.
    let stays = |what: &str, cx: &mut VisualTestContext| {
        assert_line("missing", "Rust: no language server found", cx);
        assert!(has_control("download", cx), "the offer went after {what}");
        assert_no_dialog(cx);
    };
    cx.simulate_keystrokes("j");
    stays("j", &mut cx);
    cx.simulate_keystrokes("k");
    stays("k", &mut cx);
    let over_the_rows = bounds_of_row(2, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    stays("the wheel", &mut cx);
    go_to(&stage, "src/lib.rs", Line::Removed, &mut cx);
    // The removed line is read in the old version: its server was not told.
    assert!(drawn("review-servers", &mut cx).is_none());
    navigator.tell(
        Told::Servers(vec![
            rust(missing_with_offer()),
            old_side(rust(State::Ready)),
        ]),
        &mut cx,
    );
    assert_line("ready", "Rust: ready", &mut cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);
    stays("a click on another row", &mut cx);
    cx.simulate_keystrokes("s");
    stays("s", &mut cx);
    cx.simulate_keystrokes("s");
    stays("s again", &mut cx);
    cx.simulate_keystrokes("f");
    stays("f", &mut cx);
    cx.simulate_keystrokes("f");
    stays("f again", &mut cx);
}

#[gpui_kit::test]
fn the_download_control_goes_when_the_offer_does_and_the_download_is_said_while_it_runs(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_on_code(cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);

    // No offer: a language with no download for this machine, or one the user
    // registered a server of their own for.
    navigator.tell(
        Told::Servers(vec![rust(State::Missing { offer: None })]),
        &mut cx,
    );
    assert_line(
        "missing",
        "Rust: no language server found; names are found by text match",
        &mut cx,
    );
    assert!(!has_control("download", &mut cx));
    // The key has nothing to download: it says so and asks nothing.
    cx.simulate_keystrokes("shift-i");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No language server waits to be downloaded".to_owned())
    );

    navigator.tell(
        Told::Servers(vec![rust(State::Downloading { offer: offer() })]),
        &mut cx,
    );
    assert_line(
        "downloading",
        "Rust: downloading rust-analyzer 2026-11-09",
        &mut cx,
    );
    assert!(!has_control("download", &mut cx));

    // The barrier: the next ask the test causes through the same path is the first the
    // pane made since the key with nothing to download, so that key asked nothing.
    navigator.tell(Told::Servers(vec![rust(missing_with_offer())]), &mut cx);
    cx.simulate_keystrokes("shift-i");
    assert_eq!(
        navigator.ask(&mut cx),
        Ask::Download {
            language: "rust".to_owned()
        }
    );
    assert_eq!(navigator.asked_so_far(&mut cx), Vec::new());
}

#[gpui_kit::test]
fn the_update_is_offered_on_the_line_of_a_ready_server_that_runs_an_older_version(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_on_code(cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);

    navigator.tell(Told::Servers(vec![rust(State::Ready)]), &mut cx);
    assert_line("ready", "Rust: ready", &mut cx);
    assert!(!has_control("update", &mut cx));
    cx.simulate_keystrokes("shift-u");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No language server update waits".to_owned())
    );

    let older = ServerState {
        update: Some(newer()),
        ..rust(State::Ready)
    };
    navigator.tell(Told::Servers(vec![older]), &mut cx);
    assert_line("ready", "Rust: ready", &mut cx);
    assert!(drawn("review-servers-update:Update to 2026-12-01", &mut cx).is_some());
    assert_eq!(UPDATE, "Update to");
    let at = drawn("review-servers-update", &mut cx).unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    // The barrier: the key with nothing to update asked nothing, since this is the first.
    assert_eq!(
        navigator.ask(&mut cx),
        Ask::Update {
            language: "rust".to_owned()
        }
    );
    cx.simulate_keystrokes("shift-u");
    assert_eq!(
        navigator.ask(&mut cx),
        Ask::Update {
            language: "rust".to_owned()
        }
    );
    assert_eq!(navigator.asked_so_far(&mut cx), Vec::new());
}

#[gpui_kit::test]
fn the_pane_asks_before_a_server_inside_a_repository_first_runs_and_asks_once_for_that_repository(
    cx: &mut TestAppContext,
) {
    let (mut stage, navigator, mut cx) = open_on_code(cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);

    navigator.tell(Told::Servers(vec![rust(asking(repository()))]), &mut cx);

    // The pane asks, and the words name the program and the repository.
    assert_line(
        "asking",
        "Rust: /gits/feature/tools/rust-analyzer lies inside the repository repository. \
         Run language servers from this repository?",
        &mut cx,
    );
    assert!(drawn(&format!("review-servers-yes:{ALLOW}"), &mut cx).is_some());
    assert!(drawn(&format!("review-servers-no:{REFUSE}"), &mut cx).is_some());
    assert!(!has_control("download", &mut cx));
    assert_no_dialog(&mut cx);

    // A click on yes sets the answer of the repository.
    let at = drawn("review-servers-yes", &mut cx).unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    let allowed = Setting::ServersInRepository {
        repository: repository(),
        allowed: true,
    };
    wait_until(&mut cx, "the answer reaching the Keeper", |_| {
        stage.keeper.settings_set() == [allowed.clone()]
    });
    assert_no_dialog(&mut cx);

    // The Keeper tells it, and the Navigator, which did not hear it yet, tells the
    // question again as the newest state the pane has: it is not asked again.
    stage.keeper.tell(Fact::SettingChanged(allowed.clone()));
    wait_until(&mut cx, "the answer in the Frame's model", |cx| {
        stage
            .models
            .navigation
            .read_with(cx, |navigation, cx| navigation.consent(&repository(), cx))
            == Some(true)
    });
    navigator.tell(Told::Servers(vec![rust(asking(repository()))]), &mut cx);
    assert!(drawn("review-servers", &mut cx).is_some());
    assert!(!has_control("yes", &mut cx));
    assert!(!has_control("no", &mut cx));
    assert!(
        drawn(
            "review-servers-text:Rust: language servers of the repository repository may run",
            &mut cx
        )
        .is_some()
    );

    // With no question shown the keys say so and send nothing.
    cx.simulate_keystrokes("shift-y");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No question about language servers waits".to_owned())
    );
    cx.simulate_keystrokes("shift-q");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No question about language servers waits".to_owned())
    );

    // Another repository is asked about, once, by its own question.
    navigator.tell(
        Told::Servers(vec![rust(asking(other_repository()))]),
        &mut cx,
    );
    assert!(has_control("yes", &mut cx));
    assert!(has_control("no", &mut cx));
    cx.simulate_keystrokes("shift-q");
    let refused = Setting::ServersInRepository {
        repository: other_repository(),
        allowed: false,
    };
    // The barrier: the Keeper hears the requests in the order they were sent, so when it
    // has heard the answer for the other repository it has heard everything before it:
    // the two keys with no question sent nothing.
    wait_until(&mut cx, "the second answer reaching the Keeper", |_| {
        stage.keeper.settings_set().len() >= 2
    });
    assert_eq!(stage.keeper.settings_set(), [allowed, refused.clone()]);
    // The question for it goes the same way.
    stage.keeper.tell(Fact::SettingChanged(refused));
    navigator.tell(
        Told::Servers(vec![rust(asking(other_repository()))]),
        &mut cx,
    );
    assert!(!has_control("yes", &mut cx));
    assert_no_dialog(&mut cx);
}

#[gpui_kit::test]
fn the_keys_and_the_click_that_refuse_a_repository_send_the_refusal_of_that_repository(
    cx: &mut TestAppContext,
) {
    let (stage, navigator, mut cx) = open_on_code(cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);
    navigator.tell(Told::Servers(vec![rust(asking(repository()))]), &mut cx);

    let at = drawn("review-servers-no", &mut cx).unwrap().center();
    cx.simulate_click(at, Modifiers::none());
    let refused = Setting::ServersInRepository {
        repository: repository(),
        allowed: false,
    };
    wait_until(&mut cx, "the refusal reaching the Keeper", |_| {
        stage.keeper.settings_set() == [refused.clone()]
    });

    // A repository asked about with the keys: yes.
    navigator.tell(
        Told::Servers(vec![rust(asking(other_repository()))]),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-y");
    let allowed = Setting::ServersInRepository {
        repository: other_repository(),
        allowed: true,
    };
    wait_until(&mut cx, "the answer by key reaching the Keeper", |_| {
        stage.keeper.settings_set() == [refused.clone(), allowed.clone()]
    });
}

#[gpui_kit::test]
fn the_line_says_where_the_server_stands_for_a_built_in_row_and_a_registered_one_alike(
    cx: &mut TestAppContext,
) {
    let (stage, navigator, mut cx) = open_on_code(cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);

    for registered in [false, true] {
        let row = |state| ServerState {
            registered,
            ..rust(state)
        };
        for (state, words) in [
            (
                State::Starting,
                "Rust: starting; names are found by text match meanwhile",
            ),
            (State::Ready, "Rust: ready"),
            (
                State::Missing { offer: None },
                "Rust: no language server found; names are found by text match",
            ),
        ] {
            let name = match state {
                State::Starting => "starting",
                State::Ready => "ready",
                _ => "missing",
            };
            navigator.tell(Told::Servers(vec![row(state)]), &mut cx);
            assert_line(name, words, &mut cx);
            // Nothing marks the registered row apart, and a state with no offer has no
            // control for either.
            assert!(!has_control("download", &mut cx), "{registered} {name}");
            assert!(!has_control("update", &mut cx), "{registered} {name}");
            assert!(!has_control("yes", &mut cx), "{registered} {name}");
        }
    }
}

#[gpui_kit::test]
fn the_line_follows_the_file_of_the_line_the_reviewer_is_on_and_a_removed_line_shows_the_old_side(
    cx: &mut TestAppContext,
) {
    let (stage, navigator, mut cx) = open_on_code(cx);
    navigator.tell(
        Told::Servers(vec![
            rust(State::Ready),
            old_side(rust(State::Missing { offer: None })),
            python(State::Starting),
        ]),
        &mut cx,
    );

    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);
    assert_line("ready", "Rust: ready", &mut cx);
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, cx| pane.server_on_screen(cx))
            .map(|server| server.language),
        Some("rust".to_owned())
    );

    go_to(&stage, "app/main.py", Line::Added, &mut cx);
    assert_line(
        "starting",
        "Python: starting; names are found by text match meanwhile",
        &mut cx,
    );

    // A removed line is read in the old version: the server of that version.
    go_to(&stage, "src/lib.rs", Line::Removed, &mut cx);
    assert_line(
        "missing",
        "Rust: no language server found; names are found by text match",
        &mut cx,
    );
    let on_screen = stage
        .pane
        .read_with(&cx, |pane, cx| pane.server_on_screen(cx))
        .expect("a server is on screen");
    assert_eq!(on_screen.old_side.as_deref(), Some(COMMIT));

    // A file no row knows has no server, so no line, though others are told.
    let readme = stage.pane.read_with(&cx, |pane, _| {
        pane.rows().iter().position(
            |row| matches!(row, PaneRow::Diff { path, .. } if path == &PathBuf::from("README.md")),
        )
    });
    let at = bounds_of_row(readme.expect("the readme has rows"), &mut cx).center();
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane
            .position()
            .map(|p: &Position| p.path.clone())),
        Some(PathBuf::from("README.md"))
    );
    assert!(drawn("review-servers", &mut cx).is_none());
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, cx| pane.server_on_screen(cx)),
        None
    );
}

#[gpui_kit::test]
fn there_is_no_line_while_no_navigator_is_connected(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(Vec::new(), vec![file("src/lib.rs", FileStatus::Modified)]);
    stage.keeper.show(
        &changes,
        |_| texts("start\ngone\nmiddle\n", "start\nmiddle\nadded\n"),
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    go_to(&stage, "src/lib.rs", Line::Added, &mut cx);
    assert!(drawn("review-servers", &mut cx).is_none());
    // The key then has nothing to act on, and says so.
    cx.simulate_keystrokes("shift-i");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        Some("No language server waits to be downloaded".to_owned())
    );
}
