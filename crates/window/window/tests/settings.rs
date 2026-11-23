//! Component tests: the settings in the composed Window on GPUI's test platform, with every
//! pane and the settings screen, the real Keeper beneath it and the real Navigator beside
//! it. The Frame's rows play the Keeper with data; these join the screen and the real Keeper
//! (WIN-11), as the Window's other tests join the Navigator.
//!
//! The keys are pressed as a user presses them. What a setting does is read back through the
//! model that shows it, at once, and from a second Window opened on a Keeper started again.
//! The tests wait for facts with a deadline, and the clock a span rests on is the test's own.

mod common;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::theme::{ActiveTheme, ThemeMode};
use gpui_kit::{TestAppContext, VisualTestContext, WindowAppearance};
use yaar_frame::settings_screen::{
    CLEAN_UP, RowShown, SettingRow, TOTAL_STATES, size_words, stop_after_label,
};
use yaar_frame::theme::{self, Appearance, DARK, LIGHT};
use yaar_frame::{Beside, SETTINGS_CONTEXT};
use yaar_link::{
    Fact, HistoryEntry, Reply, Request, SessionId, Setting, StateSizes, TerminalId, TerminalPlace,
    ThemeChoice, TurnEndSelects, TurnId, WorkspaceId,
};
use yaar_navigator::{Clock, Place, State, Tree};
use yaar_review_pane::{ChangeShown, PaneRow};
use yaar_system_tests::standin::{PROMPT, Script};
use yaar_system_tests::{Client, DEADLINE, Repo, World, is_alive};

use common::{
    Composed, Found, drawn, give_an_identity, key_of, navigator_for, open_another_composed_in,
    open_composed_with_a_navigator, standin_row, wait_until,
};

thread_local! {
    /// The settings the Keeper told the Window of this test, in the order the Window heard
    /// them (`hear_the_settings`).
    static ECHOES: Rc<RefCell<Vec<Setting>>> = Rc::default();
}

/// The Window as the program opens it, with the Navigator and the real Keeper of `world`.
fn open_the_window(world: World, cx: &mut TestAppContext) -> (Composed, VisualTestContext) {
    let navigator = navigator_for(
        &world,
        Found::OnTheMachine,
        &standin_row(Found::OnTheMachine, "main.sil", &[]),
    );
    let (window, mut cx) = open_composed_with_a_navigator(world, cx, Vec::new(), navigator);
    hear_the_settings(&window, &mut cx);
    (window, cx)
}

/// Has what the Keeper tells the Window of the settings kept in order, as the Window hears it
/// (what the Frame's list of settings hears, which is subscribed first, has been taken by the
/// time this is told). The Keeper tells a setting to every Window once it keeps it, the
/// Window that set it included, and that echo takes the setting the Window shows to the value
/// it had when the Keeper kept it: two sets of one setting in quick succession show the first
/// value again for a moment, until the second echo comes. A test that sets one setting twice
/// waits for the echoes before it reads or sets it again ([`wait_for_the_echoes`]).
fn hear_the_settings(window: &Composed, cx: &mut VisualTestContext) {
    let echoes = ECHOES.with(Rc::clone);
    echoes.borrow_mut().clear();
    cx.update(|_, app| {
        app.subscribe(&window.models.link, move |_, fact: &Fact, _| {
            if let Fact::SettingChanged(setting) = fact {
                echoes.borrow_mut().push(setting.clone());
            }
        })
        .detach();
    });
}

/// Waits, with the app running, until the Window has heard the Keeper's word on every set of
/// the kind of `setting` that has passed the tap.
fn wait_for_the_echoes(window: &Composed, setting: &Setting, cx: &mut VisualTestContext) {
    wait_until(
        cx,
        &format!("the Keeper's word on every set of {setting:?}'s kind"),
        |_| {
            let sent = window
                .tap
                .requests()
                .iter()
                .filter(|request| {
                    matches!(request, Request::SetSetting { setting: set }
                        if set.same_thing_as(setting))
                })
                .count();
            let heard = ECHOES.with(|echoes| {
                echoes
                    .borrow()
                    .iter()
                    .filter(|told| told.same_thing_as(setting))
                    .count()
            });
            heard >= sent
        },
    );
}

/// A setting of the kind the screen's `row` sets.
fn kind_of(row: &SettingRow) -> Setting {
    match row {
        SettingRow::TurnEnd => Setting::TurnEnd(TurnEndSelects::WhatChanged),
        SettingRow::StopAfter => Setting::StopAfter { seconds: 0 },
        SettingRow::Theme => Setting::Theme(ThemeChoice::System),
        SettingRow::KeepStates => Setting::KeepStates { days: 0 },
        SettingRow::TerminalPlace => Setting::TerminalPlace(TerminalPlace::Right),
        other => panic!("the test sets no setting of the row {other:?}"),
    }
}

/// The theme's mode the toolkit draws in now.
fn mode(cx: &mut VisualTestContext) -> ThemeMode {
    cx.update(|_, app| app.theme().mode)
}

/// The name of the definition `Looks` applied last.
fn applied(cx: &mut VisualTestContext) -> String {
    cx.update(|_, app| theme::applied(app).name)
}

/// What `Looks` knows of the system's setting.
fn system(window: &Composed, cx: &mut VisualTestContext) -> Option<Appearance> {
    window.models.looks.read_with(cx, |looks, _| looks.system())
}

/// The rows the settings screen shows now.
fn rows(window: &Composed, cx: &mut VisualTestContext) -> Vec<RowShown> {
    let screen = window
        .frame
        .read_with(cx, |frame, _| frame.settings_screen().cloned())
        .expect("the screen is open");
    screen.read_with(cx, |screen, cx| screen.rows(cx))
}

/// The index of the row of the screen that is `wanted`, and the one the cursor is on.
fn row_and_cursor(
    window: &Composed,
    wanted: &SettingRow,
    cx: &mut VisualTestContext,
) -> (usize, usize) {
    let at = rows(window, cx)
        .iter()
        .position(|shown| shown.row == *wanted)
        .unwrap_or_else(|| panic!("the screen has no row {wanted:?}"));
    let screen = window
        .frame
        .read_with(cx, |frame, _| frame.settings_screen().cloned())
        .expect("the screen is open");
    (at, screen.read_with(cx, |screen, _| screen.cursor()))
}

/// Opens the settings screen by its key and waits until it has the focus.
fn open_the_screen(window: &Composed, cx: &mut VisualTestContext) {
    window.leave_the_terminal_off_macos(cx);
    cx.simulate_keystrokes(&key_of("Settings…"));
    wait_until(cx, "the settings screen with the focus", |cx| {
        window
            .frame
            .read_with(cx, |frame, _| frame.settings_screen().is_some())
            && window.focus_is_in(SETTINGS_CONTEXT, cx)
    });
}

/// Closes the screen by its key and waits until it is gone.
fn close_the_screen(window: &Composed, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(&key_of("Close Settings"));
    wait_until(cx, "the settings screen to close", |cx| {
        window
            .frame
            .read_with(cx, |frame, _| frame.settings_screen().is_none())
    });
}

/// Puts the cursor on `row` with `j` and `k`.
fn go_to(window: &Composed, row: &SettingRow, cx: &mut VisualTestContext) {
    let (next, previous) = (key_of("Next Setting"), key_of("Previous Setting"));
    loop {
        let (at, cursor) = row_and_cursor(window, row, cx);
        if cursor == at {
            return;
        }
        cx.simulate_keystrokes(if cursor < at { &next } else { &previous });
    }
}

/// Chooses the value of `row` labelled `label`, by the cursor and `l` or `h`, as a user
/// does; it is the model's, once the last key is pressed.
///
/// The echoes of the earlier sets of the row's setting are heard first, so that what the row
/// shows when the keys are pressed is what was last chosen, and what the model shows after
/// the last key is that choice, which the test reads at once.
fn choose(window: &Composed, row: &SettingRow, label: &str, cx: &mut VisualTestContext) {
    wait_for_the_echoes(window, &kind_of(row), cx);
    go_to(window, row, cx);
    let (next, previous) = (key_of("Next Value"), key_of("Previous Value"));
    let shown = |window: &Composed, cx: &mut VisualTestContext| {
        rows(window, cx)
            .into_iter()
            .find(|shown| shown.row == *row)
            .expect("the row is shown")
    };
    let values = shown(window, cx).values;
    let wanted = values
        .iter()
        .position(|value| value == label)
        .unwrap_or_else(|| panic!("the row {row:?} has no value {label:?}: {values:?}"));
    // At most one key for each value there is.
    for _ in 0..=values.len() {
        match shown(window, cx).chosen {
            Some(chosen) if chosen == wanted => return,
            Some(chosen) if chosen > wanted => cx.simulate_keystrokes(&previous),
            _ => cx.simulate_keystrokes(&next),
        }
    }
    panic!("the keys did not come to {label:?} on {row:?}");
}

/// Waits until the Window has sent a request `wanted` accepts to the Keeper, which the tap
/// reads, while the app runs: what a request waits for in the Window goes on.
fn wait_for_request(
    window: &Composed,
    what: &str,
    wanted: impl Fn(&Request) -> bool,
    cx: &mut VisualTestContext,
) {
    wait_until(cx, what, |_| window.tap.requests().iter().any(&wanted));
}

/// A repository with a change to review and the world's agent, the stand-in, started in a
/// terminal of the Window for it. Gives the terminal once the stand-in shows its prompt.
fn start_the_agent(window: &mut Composed, cx: &mut VisualTestContext) -> TerminalId {
    cx.simulate_keystrokes(&key_of("New Agent Terminal"));
    wait_until(cx, "the agent's terminal to take the focus", |cx| {
        window.focus_is_in("Program", cx)
    });
    let terminal = window
        .terminals
        .read_with(&*cx, |pane, _| pane.terminals())
        .into_iter()
        .next()
        .expect("the pane holds the terminal");
    wait_until(cx, "the stand-in's prompt", |cx| {
        window.shows_on_screen(&terminal, PROMPT.trim_end(), cx)
    });
    terminal
}

/// The turn end the Keeper tells next: its session, its turn and the state it recorded.
struct Ended {
    _session: SessionId,
    _turn: TurnId,
    state: String,
}

/// Types a line to the stand-in as a user does, and waits for the end of the turn it starts,
/// which the Keeper tells the client.
fn a_turn(client: &mut Client, terminal: &TerminalId) -> Ended {
    client.type_line(terminal, "do the next thing");
    match client.wait_for_fact(DEADLINE, |fact| matches!(fact, Fact::TurnEnded { .. })) {
        Fact::TurnEnded {
            session,
            turn,
            state: Some(state),
            ..
        } => Ended {
            _session: session,
            _turn: turn,
            state,
        },
        other => panic!("a turn ended with no state recorded: {other:?}"),
    }
}

/// The paths of the files the pane's Changes lists, in the order of its rows.
fn files_of_changes(window: &Composed, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    window.review.read_with(&*cx, |pane, _| {
        pane.rows()
            .iter()
            .filter_map(|row| match row {
                PaneRow::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    })
}

/// What a read gave, compared byte for byte and shown as text when two differ.
#[derive(PartialEq, Eq)]
struct Bytes(Vec<u8>);

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} bytes: {:?}",
            self.0.len(),
            String::from_utf8_lossy(&self.0)
        )
    }
}

/// What a clean-up must leave as it was in the user's repository: the seven reads of it and
/// every file of its objects with its size.
#[derive(Debug, PartialEq, Eq)]
struct Reads {
    git: Vec<Bytes>,
    objects: Vec<(PathBuf, u64)>,
}

/// Runs git in `folder` as a reader of a repository does: with no optional lock, so that
/// reading never rewrites the index, and with nothing of the user's configuration.
fn read_git(home: &Path, folder: &Path, args: &[&str]) -> Vec<u8> {
    let mut git = Command::new("git");
    git.env_clear();
    if let Some(search) = std::env::var_os("PATH") {
        git.env("PATH", search);
    }
    let output = git
        .current_dir(folder)
        .args(args)
        .env("HOME", home)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z")
        .env("GIT_COMMITTER_NAME", "A Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// Every file below `folder` with its size, the path relative to `below`.
fn files_below(folder: &Path, below: &Path, found: &mut Vec<(PathBuf, u64)>) {
    for entry in fs::read_dir(folder).expect("read the folder") {
        let path = entry.expect("an entry").path();
        let meta = fs::symlink_metadata(&path).expect("the kind");
        if meta.is_dir() {
            files_below(&path, below, found);
        } else {
            found.push((
                path.strip_prefix(below).expect("below").to_owned(),
                meta.len(),
            ));
        }
    }
}

/// The repository's reads: status with the ignored files, the refs, the worktrees, the
/// stashes, the unreachable objects, the bytes of the index and of the configuration, and
/// the sorted list of every file of the objects with its size.
fn reads(home: &Path, repo: &Repo) -> Reads {
    let git_folder = repo.path().join(".git");
    let mut objects = Vec::new();
    files_below(
        &git_folder.join("objects"),
        &git_folder.join("objects"),
        &mut objects,
    );
    objects.sort();
    let git = [
        &[
            "status",
            "--porcelain=v2",
            "--ignored",
            "--untracked-files=all",
        ][..],
        &["for-each-ref"],
        &["worktree", "list", "--porcelain"],
        &["stash", "list"],
        &["fsck", "--unreachable", "--no-reflogs"],
    ]
    .iter()
    .map(|args| read_git(home, repo.path(), args))
    .chain([
        fs::read(git_folder.join("index")).expect("the index"),
        fs::read(git_folder.join("config")).expect("the configuration"),
    ])
    .map(Bytes)
    .collect();
    Reads { git, objects }
}

/// Two objects no ref reaches, planted before the first read: a blob and a dangling commit.
/// Gives their ids, which [`still_there`] finds again.
fn plant(home: &Path, repo: &Repo) -> Vec<String> {
    let file = home.join("planted.txt");
    fs::write(&file, "a blob nothing refers to\n").expect("write the blob's file");
    let blob = repo.git(&["hash-object", "-w", file.to_str().expect("a path of text")]);
    repo.write("README.md", "edited\n");
    let dangling = repo.git(&["stash", "create"]);
    repo.git(&["checkout", "--quiet", "--", "README.md"]);
    assert!(!dangling.is_empty(), "the edit left no commit to plant");
    vec![blob, dangling]
}

/// `git cat-file -e` of each planted object succeeds.
fn still_there(repo: &Repo, planted: &[String]) {
    for id in planted {
        repo.git(&["cat-file", "-e", id]);
    }
}

/// The world of the settings' test: the stand-in as the agent CLI, which does a turn that
/// writes `first.txt` and one that writes `second.txt`, and then ends.
fn world_with_two_turns() -> World {
    let mut world = World::new();
    world.agents_on_path();
    world.agent_script(
        "default",
        &Script::parse("turn\nwrite first.txt one\nend\nturn\nwrite second.txt two\nend\nexit"),
    );
    world.start_keeper();
    world
}

// WIN-11, the opening as the program makes it: the program opens its window from the
// application's start, outside any update of the app, where the toolkit has told the window's
// observers by the time the window is open. The test opens its window inside an update, where
// that notice comes only when the update ends, so the model is read inside the update, before
// it can come: what the model holds then is what the Frame's `open` told it, before the window
// existed. The Window program's own tell at the opening (step 1 of `appearance`) only repeats
// it, and nothing here can tell it from its absence.
#[gpui_kit::test]
fn the_model_of_the_theme_holds_the_systems_appearance_the_moment_the_window_program_has_the_window(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let navigator = navigator_for(
        &world,
        Found::OnTheMachine,
        &standin_row(Found::OnTheMachine, "main.sil", &[]),
    );
    let tap = world.tap();
    cx.executor().allow_parking();
    let (window, told, applied_then) = cx.update(|app| {
        yaar_window::init(app);
        let builders = yaar_window::panes()
            .into_iter()
            .map(|entry| (entry.build)())
            .collect();
        let opened = yaar_window::open_panes(tap.socket(), builders, navigator, app)
            .expect("open the Window on the test platform");
        let told = opened.models.looks.read(app).system();
        (opened.window, told, theme::applied(app).name)
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    let platform = cx.update(|window, _| window.appearance());
    assert_eq!(
        platform,
        WindowAppearance::Light,
        "GPUI's test platform has a light window"
    );
    assert_eq!(
        told,
        Some(Appearance::Light),
        "the model of the theme knew the window's appearance when the opening returned"
    );
    assert_eq!(
        applied_then, LIGHT,
        "the definition applied then is the light one"
    );
    cx.quit();
}

// WIN-11, the opening: once the opening has run, the theme follows the system.
#[gpui_kit::test]
fn the_window_tells_its_theme_the_systems_appearance_at_its_opening(cx: &mut TestAppContext) {
    let mut world = World::new();
    world.start_keeper();
    let (window, mut cx) = open_the_window(world, cx);
    cx.run_until_parked();

    // Nothing is set, so the system's setting is what the Window draws in.
    let platform = cx.update(|window, _| window.appearance());
    assert_eq!(
        platform,
        WindowAppearance::Light,
        "GPUI's test platform has a light window"
    );
    assert_eq!(
        system(&window, &mut cx),
        Some(Appearance::Light),
        "the Window program told the model what the window reports"
    );
    assert_eq!(applied(&mut cx), LIGHT);
    assert_eq!(mode(&mut cx), ThemeMode::Light);
    assert_eq!(
        window
            .models
            .looks
            .read_with(&cx, |looks, cx| looks.chosen(cx)),
        ThemeChoice::System,
        "no theme was chosen"
    );
    cx.quit();
}

// WIN-11: the settings screen sets what a turn end selects, the stop time, the theme, the place
// of the terminal pane and the time states are kept; each is in force at once and still set
// after the Keeper restarts; the screen shows the sizes of the recorded states and its
// clean-up reaches the Keeper.
#[gpui_kit::test]
fn each_setting_of_the_screen_is_in_force_at_once_and_is_still_set_after_the_keeper_restarts_and_the_screen_shows_the_sizes_and_cleans_up(
    app: &mut TestAppContext,
) {
    let world = world_with_two_turns();
    let mut client = world.client();
    let (mut window, mut cx) = open_the_window(world, app);
    let folder = window.open_a_folder(&mut cx);
    let workspace: WorkspaceId = window
        .models
        .workspaces
        .read_with(&cx, |list, _| list.selected().cloned())
        .expect("the opened folder is on screen");
    assert!(folder.exists());
    // The repository is the user's: two objects no ref reaches are planted in it before the
    // first read, so that a clean-up that reaches past the states shows.
    let home = window.world.home();
    let planted = plant(&home, &window.repositories[0]);

    // A turn of the agent is recorded. Nothing is set: the turn's end selects what changed
    // since the review, which is what the pane holds afterwards.
    let terminal = start_the_agent(&mut window, &mut cx);
    let first = a_turn(&mut client, &terminal);
    wait_until(
        &mut cx,
        "the pane to select what the first turn changed",
        |cx| {
            window.review.read_with(cx, |pane, _| {
            matches!(pane.change_shown(), ChangeShown::Since { ref to, .. } if *to == first.state)
        })
        },
    );
    assert!(
        !window
            .review
            .read_with(&cx, |pane, _| pane.keeps_selection())
    );
    let selected = window.review.read_with(&cx, |pane, _| pane.change_shown());
    // What changed since the base: the file the turn wrote and the one the worktree held changed.
    let files = ["first.txt", "notes.txt"].map(PathBuf::from).to_vec();
    wait_until(&mut cx, "the files of the first turn in Changes", |cx| {
        let mut listed = files_of_changes(&window, cx);
        listed.sort();
        listed == files
    });

    // The screen, opened by its key, shows the room the recorded states take: the workspace
    // of the world with more than nothing, and the total.
    open_the_screen(&window, &mut cx);
    go_to(&window, &SettingRow::States, &mut cx);
    let mut before: Option<StateSizes> = None;
    wait_until(&mut cx, "the sizes of the recorded states", |cx| {
        before = window
            .models
            .states
            .read_with(cx, |states, _| states.sizes().cloned());
        before.as_ref().is_some_and(|sizes| {
            sizes
                .workspaces
                .iter()
                .any(|size| size.workspace == workspace && size.bytes > 0)
        })
    });
    let before = before.expect("the Keeper gave the sizes");
    // The workspace's line: its name (the branch checked out) and its bytes in words.
    let name = window
        .models
        .workspaces
        .read_with(&cx, |list, _| {
            list.workspace(&workspace)
                .and_then(|held| held.head.clone())
        })
        .expect("the workspace has a branch checked out")
        .label();
    let bytes = before
        .workspaces
        .iter()
        .find(|size| size.workspace == workspace)
        .expect("the sizes name the workspace")
        .bytes;
    let drawn_workspace = format!(
        "settings-size-{}:{name}: {}",
        workspace.as_str(),
        size_words(bytes)
    );
    let drawn_total = format!(
        "settings-size-total:{TOTAL_STATES}: {}",
        size_words(before.total)
    );
    wait_until(&mut cx, "the screen to draw the sizes", |cx| {
        drawn(&drawn_workspace, cx).is_some() && drawn(&drawn_total, cx).is_some()
    });

    // What a turn end selects: `l` on its row. The model has it before anything is waited for.
    choose(
        &window,
        &SettingRow::TurnEnd,
        yaar_frame::settings_screen::KEEP_SELECTION,
        &mut cx,
    );
    assert_eq!(
        window.models.settings.read_with(&cx, |s, _| s.turn_end()),
        TurnEndSelects::KeepSelection
    );
    assert!(
        window
            .review
            .read_with(&cx, |pane, _| pane.keeps_selection()),
        "the review pane reads it at once"
    );

    // The stop time, one step down from the 10 minutes there is: the model has it at once.
    choose(
        &window,
        &SettingRow::StopAfter,
        &stop_after_label(5),
        &mut cx,
    );
    assert_eq!(
        window.models.settings.read_with(&cx, |s, _| s.stop_after()),
        Duration::from_secs(300)
    );

    // The theme, Light and then Dark: the toolkit draws in dark at once.
    choose(&window, &SettingRow::Theme, LIGHT, &mut cx);
    assert_eq!(applied(&mut cx), LIGHT);
    choose(&window, &SettingRow::Theme, DARK, &mut cx);
    assert_eq!(mode(&mut cx), ThemeMode::Dark, "the theme is dark at once");
    assert_eq!(applied(&mut cx), DARK);
    assert_eq!(
        cx.update(|_, app| app.theme().background),
        theme::dark().colors.background
    );

    // The time states are kept, 60 days for the 30 there are.
    choose(
        &window,
        &SettingRow::KeepStates,
        &yaar_frame::settings_screen::keep_states_label(60),
        &mut cx,
    );
    assert_eq!(
        window
            .models
            .settings
            .read_with(&cx, |s, _| s.keep_states()),
        60
    );

    // The terminal pane below: the Frame lays the panes out so at once.
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.beside()),
        Beside::Right
    );
    choose(
        &window,
        &SettingRow::TerminalPlace,
        yaar_frame::settings_screen::BELOW,
        &mut cx,
    );
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.beside()),
        Beside::Below,
        "the terminal pane lies below at once"
    );

    // Each reached the real Keeper.
    let sent = [
        Setting::TurnEnd(TurnEndSelects::KeepSelection),
        Setting::StopAfter { seconds: 300 },
        Setting::Theme(ThemeChoice::Named(DARK.to_owned())),
        Setting::KeepStates { days: 60 },
        Setting::TerminalPlace(TerminalPlace::Below),
    ];
    for setting in &sent {
        wait_for_request(
            &window,
            "the setting to reach the Keeper",
            |request| matches!(request, Request::SetSetting { setting: set } if set == setting),
            &mut cx,
        );
    }

    // A second turn ends with the selection kept: the pane offers what it changed and leaves
    // the selection, and with it the files, as they were.
    close_the_screen(&window, &mut cx);
    let second = a_turn(&mut client, &terminal);
    assert_ne!(first.state, second.state, "the second turn changed a file");
    wait_until(&mut cx, "the offer of what the second turn changed", |cx| {
        window.review.read_with(cx, |pane, _| {
            matches!(pane.offer(), Some(ChangeShown::Since { to, .. }) if *to == second.state)
        })
    });
    assert_eq!(
        window.review.read_with(&cx, |pane, _| pane.change_shown()),
        selected,
        "the selection is the one the reviewer had"
    );
    assert_eq!(
        files_of_changes(&window, &mut cx),
        files,
        "the files are the ones the selection had"
    );

    // The clean-up: the screen again, with the sizes now, and `shift-c`. It reaches the
    // Keeper, the sizes after are no larger, and the history still holds both turns.
    open_the_screen(&window, &mut cx);
    go_to(&window, &SettingRow::States, &mut cx);
    // The room the states take with the second turn's recorded, as the Keeper says it: the
    // clean-up only deletes, so no size is larger after it.
    let Ok(Reply::StateSizes(now)) = client.request(Request::GetStateSizes) else {
        panic!("the Keeper did not give the sizes of the recorded states");
    };
    let control = format!(
        "settings-value-{}-0:{CLEAN_UP}",
        row_and_cursor(&window, &SettingRow::States, &mut cx).0
    );
    wait_until(&mut cx, "the control of the clean-up to be drawn", |cx| {
        drawn(&control, cx).is_some()
    });
    let repository_before = reads(&home, &window.repositories[0]);
    cx.simulate_keystrokes(&key_of("Clean Up Recorded States"));
    wait_for_request(
        &window,
        "the clean-up to reach the Keeper",
        |request| matches!(request, Request::CleanUpStates),
        &mut cx,
    );
    wait_until(&mut cx, "the clean-up to end", |cx| {
        window
            .models
            .states
            .read_with(cx, |states, _| !states.cleaning())
    });
    // Nothing of the user's was written: every read of the repository is as it was, and the
    // objects nothing refers to are still there.
    assert_eq!(reads(&home, &window.repositories[0]), repository_before);
    still_there(&window.repositories[0], &planted);
    let after = window
        .models
        .states
        .read_with(&cx, |states, _| states.sizes().cloned())
        .expect("the sizes after the clean-up");
    assert!(after.total <= now.total, "{after:?} is larger than {now:?}");
    for size in &after.workspaces {
        let was = now
            .workspaces
            .iter()
            .find(|was| was.workspace == size.workspace)
            .map_or(0, |was| was.bytes);
        assert!(size.bytes <= was, "{size:?} is larger than before ({was})");
    }
    let history = client
        .history(&workspace)
        .expect("the Keeper has the history");
    let turns = history
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry,
                HistoryEntry::Turn { .. } | HistoryEntry::ExpiredTurn { .. }
            )
        })
        .count();
    assert_eq!(turns, 2, "the history holds both turns: {history:?}");
    close_the_screen(&window, &mut cx);

    // The Keeper ends with no goodbye once the agent is gone, and starts again on what it
    // kept; a second Window opened on it shows every setting chosen.
    wait_until(&mut cx, "the agent's terminal to end", |cx| {
        window.models.terminals.read_with(cx, |list, _| {
            list.terminals().iter().all(|held| held.id != terminal)
        })
    });
    let mut world = window.into_world();
    drop(client);
    world.kill_keeper();
    world.start_keeper();
    let (second, mut cx) = open_another_composed_in(world, app, Vec::new());
    wait_until(
        &mut cx,
        "the Keeper's settings in the second Window",
        |cx| {
            second
                .models
                .settings
                .read_with(cx, |settings, _| settings.all().len() >= sent.len())
        },
    );
    second.models.settings.read_with(&cx, |settings, _| {
        assert_eq!(settings.turn_end(), TurnEndSelects::KeepSelection);
        assert_eq!(settings.stop_after(), Duration::from_secs(300));
        assert_eq!(settings.stop_after_set(), Some(Duration::from_secs(300)));
        assert_eq!(settings.theme(), ThemeChoice::Named(DARK.to_owned()));
        assert_eq!(settings.keep_states(), 60);
        assert_eq!(settings.terminal_place(), TerminalPlace::Below);
    });
    // Read through the model of the theme, which is what the screen shows.
    assert_eq!(
        second
            .models
            .looks
            .read_with(&cx, |looks, cx| looks.chosen(cx)),
        ThemeChoice::Named(DARK.to_owned())
    );
    // And the pane of the second Window lays out as the setting says.
    wait_until(
        &mut cx,
        "the second Window to lay the panes out below",
        |cx| {
            second
                .frame
                .read_with(cx, |frame, _| frame.beside() == Beside::Below)
        },
    );
    cx.quit();
}

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

/// A repository whose files the stand-in language server serves, `main.sil` among them.
fn repository_of_code(world: &World, name: &str) -> Repo {
    let repository = world.repository(name);
    give_an_identity(&repository);
    repository.write("main.sil", "def main\nuse total\n");
    repository.commit("Add the code");
    repository.git(&["checkout", "--quiet", "-b", "work"]);
    repository.write("main.sil", "def main\nuse total\nuse more\n");
    repository
}

// WIN-11, the stop time: chosen on the screen, it is the time the real Navigator stops the
// servers of a workspace off screen by, at once.
#[gpui_kit::test]
fn a_stop_time_chosen_on_the_screen_is_the_one_the_navigator_stops_the_servers_off_screen_by(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let coded = repository_of_code(&world, "coded");
    let other = common::repository_to_review(&world, "other");
    let clock = TestClock::new();
    let mut navigator = navigator_for(
        &world,
        Found::OnTheMachine,
        &standin_row(Found::OnTheMachine, "main.sil", &[]),
    );
    navigator.clock = clock.clone();
    let (mut window, mut cx) = open_composed_with_a_navigator(world, cx, Vec::new(), navigator);

    // A server runs for the repository of code: the stand-in answers a hover on its file.
    window.open_this_folder(coded.path(), &mut cx);
    let coded_workspace = window
        .models
        .workspaces
        .read_with(&cx, |list, _| list.selected().cloned())
        .expect("a workspace is on screen");
    let place = Place {
        workspace: coded_workspace,
        tree: Tree::Worktree,
        path: PathBuf::from("main.sil"),
        line: 2,
        column: 4,
        name: "total".to_owned(),
        shown: Vec::new(),
    };
    let asking = window
        .models
        .navigation
        .update(&mut cx, |navigation, cx| navigation.hover(place, cx));
    let answered = std::rc::Rc::new(std::cell::RefCell::new(false));
    let _waiting = cx.foreground_executor().spawn({
        let answered = answered.clone();
        async move {
            let _ = asking.await;
            *answered.borrow_mut() = true;
        }
    });
    wait_until(&mut cx, "the answer to the hover", |_| *answered.borrow());
    let mut pid = None;
    wait_until(&mut cx, "the stand-in ready", |cx| {
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

    // Its workspace goes off screen, and the screen sets the stop time to 5 minutes, below the
    // 10 there are. The clock then moves 6 minutes: past the time chosen, before the one
    // that was.
    window.open_this_folder(other.path(), &mut cx);
    open_the_screen(&window, &mut cx);
    choose(
        &window,
        &SettingRow::StopAfter,
        &stop_after_label(5),
        &mut cx,
    );
    clock.advance(Duration::from_secs(6 * 60));

    wait_until(
        &mut cx,
        "the server of the workspace off screen to stop",
        |_| !is_alive(pid),
    );
    window.end_the_navigator(&mut cx);
}

// Of its own: when a forced appearance is let go, the window's appearance is read again and
// told, however the choice came to be the system's: both when it is chosen and when it is a
// name no theme has. A choice that moves between themes lets none go and tells nothing.
#[gpui_kit::test]
fn a_forced_appearance_let_go_has_the_windows_appearance_read_again_and_told(
    cx: &mut TestAppContext,
) {
    let mut world = World::new();
    world.start_keeper();
    let (window, mut cx) = open_the_window(world, cx);
    assert_eq!(system(&window, &mut cx), Some(Appearance::Light));

    // While a theme is chosen the window's appearance is the forced one, which the Window
    // program notes: the test plays that the window reported Dark then, as a window with a
    // dark theme forced on it does, whatever the system's setting is.
    open_the_screen(&window, &mut cx);
    choose(&window, &SettingRow::Theme, DARK, &mut cx);
    let noted_dark = |window: &Composed, cx: &mut VisualTestContext| {
        window.models.looks.update(cx, |looks, cx| {
            looks.system_is(Appearance::Dark, cx);
        });
    };
    noted_dark(&window, &mut cx);
    assert_eq!(system(&window, &mut cx), Some(Appearance::Dark));

    // From one theme to another nothing is let go: the model is told nothing.
    choose(&window, &SettingRow::Theme, LIGHT, &mut cx);
    assert_eq!(applied(&mut cx), LIGHT);
    assert_eq!(
        system(&window, &mut cx),
        Some(Appearance::Dark),
        "a choice that leaves one theme for another tells the model nothing"
    );

    // Back to the system: the appearance is read again, and it is the platform's.
    choose(
        &window,
        &SettingRow::Theme,
        yaar_frame::settings_screen::SYSTEM,
        &mut cx,
    );
    assert_eq!(
        system(&window, &mut cx),
        Some(Appearance::Light),
        "the appearance the platform reports was told when the forced one was let go"
    );
    assert_eq!(applied(&mut cx), LIGHT);

    // The same for a name no theme has: it is the system's as well.
    choose(&window, &SettingRow::Theme, DARK, &mut cx);
    noted_dark(&window, &mut cx);
    assert_eq!(applied(&mut cx), DARK);
    wait_for_the_echoes(&window, &Setting::Theme(ThemeChoice::System), &mut cx);
    window.models.settings.update(&mut cx, |settings, cx| {
        settings.set(
            Setting::Theme(ThemeChoice::Named("No such theme".to_owned())),
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(
        system(&window, &mut cx),
        Some(Appearance::Light),
        "a name no theme has lets the forced appearance go too"
    );
    assert_eq!(applied(&mut cx), LIGHT);
    cx.quit();
}
