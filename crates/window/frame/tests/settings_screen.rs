mod common;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::component::theme::{ActiveTheme, ThemeMode};
use gpui_kit::{
    Action, AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    Modifiers, Pixels, Render, ScrollDelta, ScrollEvent, Size, Styled, TestAppContext,
    VisualTestContext, Window, div, point, px,
};
use yaar_frame::settings_screen::{
    BELOW, BY_LAYOUT, CHARACTERS, CLEAN_UP, CLEANING, DEFAULT_FOLDER, DONT_INSTALL, INSTALL,
    KEEP_SELECTION, KEEP_SERVERS_OUT, KEEP_STATES_DAYS, LEFT, META, NO_BASE, NO_WORKSPACE,
    OTHER_STATES, RIGHT, RUN_SERVERS, RowShown, SETTINGS_CONTEXT, STOP_AFTER_MINUTES, SYSTEM,
    SettingRow, SettingsScreen, TITLES, TOTAL_STATES, WHAT_CHANGED, controls, keep_states_label,
    stop_after_label,
};
use yaar_frame::{
    Asks, BuildPane, CleanUpStates, CloseSettings, NextSetting, NextValue, OpenSetting,
    OpenSettings, Pane, Place, PreviousSetting, PreviousValue, Sends, bind,
};
use yaar_link::{
    Base, BaseSource, Changes, Comparison, Consent, ConsentAnswer, Fact, Hello, NoComparison,
    OptionKey, PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId, Request, RequestKind,
    Setting, StateSize, StateSizes, TerminalPlace, ThemeChoice, TurnEndSelects,
};
use yaar_navigator::{Ask, RepositoryConsent, Told, port};
use yaar_ui::themes::{Appearance, DARK, LIGHT, Themes};

use common::{
    DEADLINE, Shown, drawn, id, open_frame_staged, pressed, repository, wait_until, workspace,
};

const WINDOW: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn hello() -> Hello {
    Hello {
        protocol: PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    }
}

fn repo(letter: char) -> RepositoryId {
    RepositoryId::new(letter.to_string().repeat(32))
}

fn folder_of(letter: char, folder: Option<&str>) -> Setting {
    Setting::WorktreesFolder {
        repository: repo(letter),
        folder: folder.map(PathBuf::from),
    }
}

fn servers(letter: char, allowed: bool) -> Setting {
    Setting::ServersInRepository {
        repository: repo(letter),
        allowed,
    }
}

fn consent(agent: &str, answer: ConsentAnswer) -> Consent {
    Consent {
        agent: agent.to_owned(),
        question: format!("May the app add its extension to the settings of {agent}?"),
        answer,
        problem: None,
    }
}

fn one_repository() -> Vec<Repository> {
    vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )]
}

fn two_repositories() -> Vec<Repository> {
    vec![
        repository('a', vec![workspace('1', "main")]),
        repository('b', vec![workspace('2', "trunk")]),
    ]
}

fn changes_since(workspace: char, base: &str) -> Changes {
    Changes {
        workspace: id(workspace),
        comparison: Comparison::SinceBase {
            base: Base {
                name: base.to_owned(),
                source: BaseSource::Default,
            },
        },
        counted_from: Some(COMMIT.to_owned()),
        commits: Vec::new(),
        files: Vec::new(),
    }
}

fn sizes(listed: &[(char, u64)], total: u64) -> StateSizes {
    StateSizes {
        workspaces: listed
            .iter()
            .map(|(workspace, bytes)| StateSize {
                workspace: id(*workspace),
                bytes: *bytes,
            })
            .collect(),
        total,
    }
}

struct Plain {
    focus: FocusHandle,
}

impl Render for Plain {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

fn plain() -> BuildPane {
    Box::new(move |_, _, cx| {
        let focus = cx.focus_handle();
        let view = cx.new(|_| Plain {
            focus: focus.clone(),
        });
        Pane {
            name: "plain",
            view: view.into(),
            focus,
        }
    })
}

struct Navigator {
    asks: Receiver<Ask>,
    // Dropping the way back would tell the model the Navigator ended.
    _told: async_channel::Sender<Told>,
}

impl Navigator {
    fn connect(models: &yaar_frame::Models, cx: &mut VisualTestContext) -> Navigator {
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
        Navigator {
            asks,
            _told: end.told,
        }
    }

    fn until(
        &mut self,
        cx: &mut VisualTestContext,
        what: &str,
        wanted: impl Fn(&Ask) -> bool,
    ) -> Vec<Ask> {
        let mut read = Vec::new();
        loop {
            let deadline = Instant::now() + DEADLINE;
            let ask = loop {
                cx.run_until_parked();
                match self.asks.recv_timeout(Duration::from_millis(1)) {
                    Ok(ask) => break ask,
                    Err(RecvTimeoutError::Disconnected) => {
                        panic!("the model let the port go, and {what} was expected")
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
                assert!(Instant::now() < deadline, "the model made no ask: {what}");
            };
            let done = wanted(&ask);
            read.push(ask);
            if done {
                return read;
            }
        }
    }
}

struct Window9 {
    shown: Shown,
    cx: VisualTestContext,
    navigator: Navigator,
}

fn opened(
    size: Size<Pixels>,
    settings: Vec<Setting>,
    listed: &[Repository],
    cx: &mut TestAppContext,
) -> Window9 {
    let (mut shown, mut cx) = open_frame_staged(hello(), size, vec![plain()], cx, move |keeper| {
        keeper.set_settings(settings);
    });
    // The screen's own table, which the Window program binds after the Frame's.
    cx.update(|_, app| bind(&controls(), app));
    shown.keeper.list(listed, &mut cx);
    wait_until(&mut cx, "the list of workspaces", |cx| {
        shown
            .models
            .workspaces
            .read_with(cx, |list, _| list.repositories() == listed)
    });
    if let Some(first) = listed.iter().flat_map(|r| &r.workspaces).next() {
        let first = first.id.clone();
        let (asked, request) = shown.keeper.request(&mut cx);
        assert_eq!(
            request,
            Request::GetChanges {
                workspace: first.clone()
            }
        );
        let letter = first.as_str().chars().next().expect("an id");
        shown
            .keeper
            .reply(asked, Ok(Reply::Changes(changes_since(letter, "main"))));
        wait_until(&mut cx, "the changes", |cx| {
            shown
                .models
                .changes
                .read_with(cx, |changes, _| changes.changes().is_some())
        });
    }
    let mut navigator = Navigator::connect(&shown.models, &mut cx);
    navigator.until(&mut cx, "the answers on repositories", |ask| {
        matches!(ask, Ask::Consents(_))
    });
    shown
        .models
        .looks
        .update(&mut cx, |looks, cx| looks.system_is(Appearance::Light, cx));
    Window9 {
        shown,
        cx,
        navigator,
    }
}

impl Window9 {
    fn open_screen(&mut self, answer: StateSizes) -> Entity<SettingsScreen> {
        self.cx.simulate_keystrokes(&pressed::<OpenSettings>());
        let asked = self
            .shown
            .keeper
            .expect(Request::GetStateSizes, &mut self.cx);
        let listing = self
            .shown
            .keeper
            .expect(Request::ListConsents, &mut self.cx);
        let told = self
            .shown
            .models
            .consents
            .read_with(&self.cx, |consents, _| consents.all().to_vec());
        self.shown.keeper.reply(listing, Ok(Reply::Consents(told)));
        self.shown
            .keeper
            .reply(asked, Ok(Reply::StateSizes(answer.clone())));
        wait_until(&mut self.cx, "the sizes", |cx| {
            self.shown
                .models
                .states
                .read_with(cx, |states, _| states.sizes() == Some(&answer))
        });
        assert!(self.is_drawn("settings-screen"));
        self.screen()
    }

    fn open_screen_again(&mut self) {
        self.cx.simulate_keystrokes(&pressed::<OpenSettings>());
        let asked = self
            .shown
            .keeper
            .expect(Request::GetStateSizes, &mut self.cx);
        let listing = self
            .shown
            .keeper
            .expect(Request::ListConsents, &mut self.cx);
        let told = self
            .shown
            .models
            .consents
            .read_with(&self.cx, |consents, _| consents.all().to_vec());
        self.shown.keeper.reply(listing, Ok(Reply::Consents(told)));
        self.shown
            .keeper
            .reply(asked, Ok(Reply::StateSizes(sizes(&[], 0))));
        self.wait("the screen", |_, cx| drawn("settings-screen", cx).is_some());
    }

    fn screen(&mut self) -> Entity<SettingsScreen> {
        self.shown.frame.read_with(&self.cx, |frame, _| {
            frame
                .settings_screen()
                .cloned()
                .expect("the screen is open")
        })
    }

    fn screen_is_open(&mut self) -> bool {
        self.shown
            .frame
            .read_with(&self.cx, |frame, _| frame.settings_screen().is_some())
    }

    fn rows(&mut self) -> Vec<RowShown> {
        rows_now(&self.shown, &self.cx)
    }

    fn cursor(&mut self) -> usize {
        let screen = self.screen();
        screen.read_with(&self.cx, |screen, _| screen.cursor())
    }

    fn index_of(&mut self, row: &SettingRow) -> usize {
        self.rows()
            .iter()
            .position(|shown| shown.row == *row)
            .unwrap_or_else(|| panic!("the screen has no row {row:?}"))
    }

    fn row(&mut self, row: &SettingRow) -> RowShown {
        let at = self.index_of(row);
        self.rows().swap_remove(at)
    }

    fn is_drawn(&mut self, selector: &str) -> bool {
        drawn(selector, &mut self.cx).is_some()
    }

    fn bounds(&mut self, selector: &str) -> Bounds<Pixels> {
        drawn(selector, &mut self.cx).unwrap_or_else(|| panic!("{selector} is not drawn"))
    }

    fn has_focus(&mut self, handle: &FocusHandle) -> bool {
        self.cx.update(|window, _| handle.is_focused(window))
    }

    fn screen_focus(&mut self) -> FocusHandle {
        let screen = self.screen();
        screen.read_with(&self.cx, |screen, _| screen.focus().clone())
    }

    fn click(&mut self, selector: &str) {
        let at = self.bounds(selector).center();
        self.cx.simulate_click(at, Modifiers::none());
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
    }

    fn cursor_to(&mut self, to: usize) {
        // Exactly as many presses as rows between: a cursor that does not move fails instead of
        // looping.
        let from = self.cursor();
        for _ in from..to {
            self.press(&key::<NextSetting>());
        }
        for _ in to..from {
            self.press(&key::<PreviousSetting>());
        }
        assert_eq!(self.cursor(), to, "the cursor moved from row {from}");
    }

    fn value(row: usize, value: usize, label: &str) -> String {
        format!("settings-value-{row}-{value}:{label}")
    }

    /// The row's `settings-chosen-<row>` mark is the chosen value's control (no mark when nothing
    /// is chosen), and that control is painted in the theme's `primary`, every other in
    /// `secondary`.
    fn assert_chosen_drawn(&mut self, row: &SettingRow) {
        let at = self.index_of(row);
        let shown = self.row(row);
        let mark = format!("settings-chosen-{at}");
        let Some(chosen) = shown.chosen else {
            assert!(
                !self.is_drawn(&mark),
                "{row:?}: no value is drawn as chosen"
            );
            return;
        };
        assert_eq!(
            self.bounds(&mark),
            self.bounds(&Window9::value(at, chosen, &shown.values[chosen])),
            "{row:?}: the chosen mark is the control of the value {chosen}, not words inside it"
        );
        let mark = self.bounds(&mark).center();
        for (value, label) in shown.values.iter().enumerate() {
            let control = self.bounds(&Window9::value(at, value, label));
            assert_eq!(
                control.contains(&mark),
                value == chosen,
                "{row:?}: the chosen mark is drawn on the value {value} ({label}) and the model has \
                 {chosen} chosen"
            );
        }
        let (primary, secondary) = self.cx.update(|_, app| {
            let theme = app.theme();
            (theme.primary, theme.secondary)
        });
        for (value, label) in shown.values.iter().enumerate() {
            let (role, wanted) = if value == chosen {
                ("primary", primary)
            } else {
                ("secondary", secondary)
            };
            let ground = self.painted_ground(&Window9::value(at, value, label));
            assert_eq!(
                ground, wanted,
                "{row:?}: the control of the value {value} ({label}) is painted in {ground:?}, \
                 and the theme's {role} is {wanted:?} (the model has {chosen} chosen)"
            );
        }
    }

    /// The solid ground of the one quad painted at the element's bounds in the last frame.
    /// The scene counts in device pixels, so the bounds are scaled by the window's factor first.
    fn painted_ground(&mut self, selector: &str) -> gpui_kit::Hsla {
        let bounds = self.bounds(selector);
        let (scale, quads) = self
            .cx
            .update(|window, _| (window.scale_factor(), window.painted_quads()));
        let at = bounds.scale(scale);
        let grounds: Vec<_> = quads
            .iter()
            .filter(|quad| quad.bounds == at)
            .map(|quad| quad.background)
            .collect();
        assert_eq!(
            grounds.len(),
            1,
            "{selector}: one quad was painted at its bounds {at:?} (scale {scale}), not {} of \
             the {} painted: {:?}",
            grounds.len(),
            quads.len(),
            quads.iter().map(|quad| quad.bounds).collect::<Vec<_>>()
        );
        grounds[0]
            .as_solid()
            .unwrap_or_else(|| panic!("{selector}: the ground painted is not one solid colour"))
    }

    fn assert_every_chosen_drawn(&mut self) {
        for row in self.rows() {
            self.assert_chosen_drawn(&row.row);
        }
    }

    fn settings_set(&self) -> Vec<Setting> {
        self.shown.keeper.settings_set()
    }

    fn heard_set(&mut self, wanted: Setting) {
        self.wait("the Keeper to hear the setting", |shown, _| {
            shown.keeper.settings_set().last() == Some(&wanted)
        });
    }

    fn wait(&mut self, what: &str, mut holds: impl FnMut(&Shown, &mut VisualTestContext) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.cx.run_until_parked();
            if holds(&self.shown, &mut self.cx) {
                return;
            }
            assert!(Instant::now() < deadline, "waited in vain for {what}");
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }

    fn no_dialog(&mut self) {
        assert!(!self.cx.has_pending_prompt(), "no dialog is asked");
        assert_eq!(self.cx.windows().len(), 1, "no second window");
    }

    /// Sets a marker value and waits for the test-Keeper to hear it: a set the screen made earlier
    /// arrives before the marker, so the returned list holds every set made so far.
    fn barrier(&mut self) -> Vec<Setting> {
        let marker = Setting::TurnEnd(TurnEndSelects::KeepSelection);
        let turn_end = self.index_of(&SettingRow::TurnEnd);
        self.cursor_to(turn_end);
        self.press(&key::<NextValue>());
        self.heard_set(marker);
        self.settings_set()
    }

    /// Barrier for the requests the test-Keeper hands to the test: a request the screen sent
    /// earlier comes before `ListWorkspaces` and fails the expect. `barrier` covers the sets.
    fn handed_barrier(&mut self) {
        self.shown
            .models
            .workspaces
            .update(&mut self.cx, |list, cx| list.refresh(cx));
        self.shown
            .keeper
            .expect(Request::ListWorkspaces, &mut self.cx);
    }
}

fn rows_now(shown: &Shown, cx: &VisualTestContext) -> Vec<RowShown> {
    let screen = shown
        .frame
        .read_with(cx, |frame, _| frame.settings_screen().cloned())
        .expect("the screen is open");
    screen.read_with(cx, |screen, cx| screen.rows(cx))
}

fn key<A: Action + Default>() -> String {
    let name = A::default().name();
    controls()
        .into_iter()
        .find(|control| (control.action)().name() == name)
        .expect("the action has a row in the screen's table")
        .key()
}

#[test]
fn the_screens_table_binds_each_key_of_the_screen_in_its_own_context_and_sends_what_the_plan_says()
{
    let rows: Vec<_> = controls()
        .into_iter()
        .map(|control| {
            (
                control.title,
                control.keystroke,
                control.place,
                control.sends,
                control.asks,
            )
        })
        .collect();
    let screen = Place::In(SETTINGS_CONTEXT);
    let set = Some(Sends::Request(RequestKind::SetSetting));
    assert_eq!(
        rows,
        [
            ("Close Settings", "escape", screen, None, Asks::Nothing),
            ("Next Setting", "j", screen, None, Asks::Nothing),
            ("Previous Setting", "k", screen, None, Asks::Nothing),
            ("Next Value", "l", screen, set, Asks::Nothing),
            ("Previous Value", "h", screen, set, Asks::Nothing),
            ("Open Setting", "enter", screen, None, Asks::Nothing),
            (
                "Clean Up Recorded States",
                "shift-c",
                screen,
                Some(Sends::Request(RequestKind::CleanUpStates)),
                Asks::Nothing
            ),
        ]
    );
    let names: Vec<&str> = controls()
        .into_iter()
        .map(|control| (control.action)().name())
        .collect();
    assert_eq!(
        names,
        [
            CloseSettings.name(),
            NextSetting.name(),
            PreviousSetting.name(),
            NextValue.name(),
            PreviousValue.name(),
            OpenSetting.name(),
            CleanUpStates.name(),
        ]
    );
}

#[gpui_kit::test]
fn the_screen_opens_with_the_rows_of_the_app_the_folder_of_the_repository_and_the_states(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));

    let rows = window.rows();
    let kinds: Vec<&SettingRow> = rows.iter().map(|shown| &shown.row).collect();
    assert_eq!(
        kinds,
        [
            &SettingRow::TurnEnd,
            &SettingRow::StopAfter,
            &SettingRow::Theme,
            &SettingRow::Base,
            &SettingRow::KeepStates,
            &SettingRow::TerminalPlace,
            &SettingRow::OptionKey,
            &SettingRow::WorktreesFolder(repo('a')),
            &SettingRow::States,
        ]
    );
    let minutes: Vec<String> = STOP_AFTER_MINUTES
        .iter()
        .map(|minutes| stop_after_label(*minutes))
        .collect();
    let days: Vec<String> = KEEP_STATES_DAYS
        .iter()
        .map(|days| keep_states_label(*days))
        .collect();
    let expected = [
        (
            TITLES[0].to_owned(),
            vec![WHAT_CHANGED, KEEP_SELECTION],
            Some(0),
        ),
        (
            TITLES[1].to_owned(),
            minutes.iter().map(String::as_str).collect(),
            Some(3),
        ),
        (TITLES[2].to_owned(), vec![SYSTEM, LIGHT, DARK], Some(0)),
        (TITLES[3].to_owned(), vec!["main"], Some(0)),
        (
            TITLES[4].to_owned(),
            days.iter().map(String::as_str).collect(),
            Some(3),
        ),
        (TITLES[5].to_owned(), vec![RIGHT, BELOW, LEFT], Some(0)),
        (
            TITLES[6].to_owned(),
            vec![BY_LAYOUT, META, CHARACTERS],
            Some(0),
        ),
        (
            format!("{}: repository-a", TITLES[10]),
            vec![DEFAULT_FOLDER],
            Some(0),
        ),
        (TITLES[9].to_owned(), vec![CLEAN_UP], None),
    ];
    for (shown, (title, values, chosen)) in rows.iter().zip(expected) {
        assert_eq!(shown.title, title);
        assert_eq!(shown.values, values, "{title}");
        assert_eq!(shown.chosen, chosen, "{title}");
    }
    window.assert_every_chosen_drawn();
    assert_eq!(window.cursor(), 0);
    let row = window.bounds("settings-row-0");
    let cursor = window.bounds("settings-cursor");
    assert!(row.contains(&cursor.origin) && row.contains(&cursor.bottom_right()));
    window.no_dialog();
}

#[gpui_kit::test]
fn l_on_the_turn_end_row_chooses_keep_selection_at_once_and_the_keeper_keeps_it(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    assert_eq!(window.cursor(), 0);

    window.press(&key::<NextValue>());

    window
        .shown
        .models
        .settings
        .read_with(&window.cx, |settings, _| {
            assert_eq!(settings.turn_end(), TurnEndSelects::KeepSelection);
        });
    assert_eq!(window.row(&SettingRow::TurnEnd).chosen, Some(1));
    window.heard_set(Setting::TurnEnd(TurnEndSelects::KeepSelection));
    let turn_end = window.index_of(&SettingRow::TurnEnd);
    assert!(window.is_drawn(&format!("settings-chosen-{turn_end}")));
    assert!(window.is_drawn(&Window9::value(turn_end, 1, KEEP_SELECTION)));
    window.assert_chosen_drawn(&SettingRow::TurnEnd);
    window.no_dialog();
}

#[gpui_kit::test]
fn j_and_k_move_the_cursor_and_stop_at_the_first_and_the_last_row_and_l_and_h_stop_at_the_ends_of_a_row(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let last = window.rows().len() - 1;

    window.press(&key::<PreviousSetting>());
    assert_eq!(window.cursor(), 0, "no row before the first");
    window.press(&key::<NextSetting>());
    assert_eq!(window.cursor(), 1);
    let row = window.bounds("settings-row-1");
    let cursor = window.bounds("settings-cursor");
    assert!(
        row.contains(&cursor.origin),
        "the cursor is drawn on its row"
    );
    window.cursor_to(last);
    window.press(&key::<NextSetting>());
    assert_eq!(window.cursor(), last, "no row after the last");

    let turn_end = window.index_of(&SettingRow::TurnEnd);
    window.cursor_to(turn_end);
    window.press(&key::<NextValue>());
    window.press(&key::<NextValue>());
    window.heard_set(Setting::TurnEnd(TurnEndSelects::KeepSelection));
    window.press(&key::<PreviousValue>());
    window.press(&key::<PreviousValue>());
    window.heard_set(Setting::TurnEnd(TurnEndSelects::WhatChanged));
    assert_eq!(
        window.settings_set(),
        [
            Setting::TurnEnd(TurnEndSelects::KeepSelection),
            Setting::TurnEnd(TurnEndSelects::WhatChanged),
        ],
        "a value that is chosen already is not set again"
    );
}

#[gpui_kit::test]
fn a_click_on_a_value_chooses_it_and_puts_the_cursor_on_its_row(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let place = window.index_of(&SettingRow::TerminalPlace);
    assert_eq!(window.cursor(), 0);

    window.click(&Window9::value(place, 1, BELOW));

    assert_eq!(window.cursor(), place);
    assert_eq!(
        window.row(&SettingRow::TerminalPlace).chosen,
        Some(1),
        "the model has it at once"
    );
    window.heard_set(Setting::TerminalPlace(TerminalPlace::Below));
    window.assert_chosen_drawn(&SettingRow::TerminalPlace);
    window.no_dialog();
}

#[gpui_kit::test]
fn a_click_on_a_stop_time_sets_it_and_tells_the_navigator_at_once(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let stop = window.index_of(&SettingRow::StopAfter);
    window.press(&key::<NextSetting>());
    assert_eq!(window.cursor(), stop);
    assert_eq!(window.row(&SettingRow::StopAfter).chosen, Some(3));

    window.click(&Window9::value(stop, 0, &stop_after_label(1)));

    window.heard_set(Setting::StopAfter { seconds: 60 });
    assert_eq!(window.row(&SettingRow::StopAfter).chosen, Some(0));
    window.assert_chosen_drawn(&SettingRow::StopAfter);
    window
        .shown
        .models
        .settings
        .read_with(&window.cx, |settings, _| {
            assert_eq!(settings.stop_after(), Duration::from_secs(60));
        });
    let asks = window
        .navigator
        .until(&mut window.cx, "the stop time", |ask| {
            matches!(ask, Ask::StopAfter(_))
        });
    assert_eq!(asks.last(), Some(&Ask::StopAfter(Duration::from_secs(60))));
    window.no_dialog();
}

#[gpui_kit::test]
fn the_theme_row_applies_dark_at_once_and_a_click_on_light_brings_light_back(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let row = window.index_of(&SettingRow::Theme);
    window.cursor_to(row);
    assert_eq!(
        window.row(&SettingRow::Theme).chosen,
        Some(0),
        "the system's"
    );
    let mode = |window: &mut Window9| window.cx.update(|_, app| app.theme().mode);
    assert_eq!(mode(&mut window), ThemeMode::Light);

    window.press(&key::<NextValue>());
    window.heard_set(Setting::Theme(ThemeChoice::Named(LIGHT.to_owned())));
    window.press(&key::<NextValue>());
    assert_eq!(
        window.row(&SettingRow::Theme).chosen,
        Some(2),
        "Dark is the third value"
    );
    window.heard_set(Setting::Theme(ThemeChoice::Named(DARK.to_owned())));
    window.cx.run_until_parked();
    assert_eq!(mode(&mut window), ThemeMode::Dark);
    let background = window.cx.update(|_, app| app.theme().background);
    assert_eq!(
        background,
        Themes::built_in()
            .named(DARK)
            .expect("built in")
            .palette
            .pane
    );
    window.assert_chosen_drawn(&SettingRow::Theme);

    window.click(&Window9::value(row, 1, LIGHT));
    window.heard_set(Setting::Theme(ThemeChoice::Named(LIGHT.to_owned())));
    window.cx.run_until_parked();
    assert_eq!(mode(&mut window), ThemeMode::Light);
    let background = window.cx.update(|_, app| app.theme().background);
    assert_eq!(
        background,
        Themes::built_in()
            .named(LIGHT)
            .expect("built in")
            .palette
            .pane
    );
    window.assert_chosen_drawn(&SettingRow::Theme);

    window.click(&Window9::value(row, 0, SYSTEM));
    window.heard_set(Setting::Theme(ThemeChoice::System));
    window.no_dialog();
}

#[gpui_kit::test]
fn the_base_row_opens_the_frames_field_and_what_the_keeper_answers_shows_in_the_row(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let base = window.index_of(&SettingRow::Base);
    window.cursor_to(base);
    assert_eq!(window.row(&SettingRow::Base).values, ["main"]);
    assert!(!window.is_drawn("frame-base-field"));

    window.press(&key::<OpenSetting>());
    assert!(window.is_drawn("frame-base-field"));
    window.cx.simulate_input("develop");
    window.press("enter");

    let (asked, request) = window.shown.keeper.request(&mut window.cx);
    assert_eq!(
        request,
        Request::SetBase {
            workspace: id('1'),
            base: "develop".to_owned()
        }
    );
    window.shown.keeper.reply(asked, Ok(Reply::Done));
    assert!(!window.is_drawn("frame-base-field"), "the field is closed");
    assert_eq!(window.cursor(), base, "the cursor did not move");
    window
        .shown
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let asked = window
        .shown
        .keeper
        .expect(Request::GetChanges { workspace: id('1') }, &mut window.cx);
    window
        .shown
        .keeper
        .reply(asked, Ok(Reply::Changes(changes_since('1', "develop"))));
    window.wait("the base to show", |shown, cx| {
        shown
            .models
            .changes
            .read_with(cx, |changes, _| match changes.changes() {
                Some(Changes {
                    comparison: Comparison::SinceBase { base },
                    ..
                }) => base.name == "develop",
                _ => false,
            })
    });
    assert_eq!(window.row(&SettingRow::Base).values, ["develop"]);
    window.press(&key::<NextSetting>());
    assert_eq!(window.cursor(), base + 1);
    window.no_dialog();
}

#[gpui_kit::test]
fn a_click_on_the_base_opens_the_same_field(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let base = window.index_of(&SettingRow::Base);
    assert!(!window.is_drawn("frame-base-field"));

    window.click(&Window9::value(base, 0, "main"));

    assert!(window.is_drawn("frame-base-field"));
    assert_eq!(window.cursor(), base);
    window.no_dialog();
}

#[gpui_kit::test]
fn with_no_workspace_on_screen_the_base_row_says_so_and_nothing_opens(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, Vec::new(), &[], cx);
    window.open_screen(sizes(&[], 0));
    let row = window.row(&SettingRow::Base);
    assert_eq!(row.values, [NO_WORKSPACE]);
    assert_eq!(row.chosen, None);
    let base = window.index_of(&SettingRow::Base);

    window.cursor_to(base);
    window.press(&key::<OpenSetting>());
    window.click(&Window9::value(base, 0, NO_WORKSPACE));

    assert!(!window.is_drawn("frame-base-field"), "no field opens");
    assert_eq!(
        window.barrier(),
        [Setting::TurnEnd(TurnEndSelects::KeepSelection)]
    );
    window.handed_barrier();
}

#[gpui_kit::test]
fn the_wheel_scrolls_the_screen_until_the_row_of_the_states_is_drawn(cx: &mut TestAppContext) {
    let small = Size {
        width: px(1280.),
        height: px(360.),
    };
    let mut window = opened(small, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[('1', 4096)], 4096));
    let states = window.index_of(&SettingRow::States);
    let selector = format!("settings-row-{states}");
    let screen = window.bounds("settings-screen");
    let before = window.bounds(&selector);
    assert!(
        before.bottom() > screen.bottom(),
        "the row of the states is below what the room shows"
    );

    window.cx.simulate_event(ScrollEvent {
        position: screen.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-10_000.))),
        ..ScrollEvent::default()
    });
    window.cx.run_until_parked();

    let after = window.bounds(&selector);
    let screen = window.bounds("settings-screen");
    assert!(
        after.top() >= screen.top() && after.bottom() <= screen.bottom(),
        "the row of the states is in the room now: {after:?} in {screen:?}"
    );
    window.cx.simulate_event(ScrollEvent {
        position: screen.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(10_000.))),
        ..ScrollEvent::default()
    });
    window.cx.run_until_parked();
    let first = window.bounds("settings-row-0");
    assert!(first.top() >= window.bounds("settings-screen").top());
    window.no_dialog();
}

#[gpui_kit::test]
fn a_cursor_that_moves_past_the_room_scrolls_its_row_into_it(cx: &mut TestAppContext) {
    let small = Size {
        width: px(1280.),
        height: px(360.),
    };
    let mut window = opened(small, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let last = window.rows().len() - 1;

    window.cursor_to(last);
    window.cx.run_until_parked();

    let screen = window.bounds("settings-screen");
    let row = window.bounds(&format!("settings-row-{last}"));
    assert!(
        row.top() >= screen.top() && row.bottom() <= screen.bottom(),
        "the row under the cursor is in the room: {row:?} in {screen:?}"
    );
}

#[gpui_kit::test]
fn a_stop_time_that_is_none_of_the_values_shows_none_chosen_and_l_and_h_move_from_it(
    cx: &mut TestAppContext,
) {
    let mut window = opened(
        WINDOW,
        vec![Setting::StopAfter { seconds: 420 }],
        &one_repository(),
        cx,
    );
    window.open_screen(sizes(&[], 0));
    let stop = window.row(&SettingRow::StopAfter);
    assert_eq!(stop.chosen, None);
    assert_eq!(stop.values.len(), STOP_AFTER_MINUTES.len());
    let at = window.index_of(&SettingRow::StopAfter);
    assert!(window.is_drawn(&format!("settings-row-{at}")));
    assert!(
        !window.is_drawn(&format!("settings-chosen-{at}")),
        "no value is drawn as chosen"
    );
    window.assert_chosen_drawn(&SettingRow::StopAfter);

    window.cursor_to(at);
    window.press(&key::<NextValue>());
    window.heard_set(Setting::StopAfter { seconds: 600 });
    assert_eq!(window.row(&SettingRow::StopAfter).chosen, Some(3), "10 min");

    window
        .shown
        .keeper
        .tell(Fact::SettingChanged(Setting::StopAfter { seconds: 420 }));
    window.wait("the 7 minutes", |shown, cx| {
        shown.models.settings.read_with(cx, |settings, _| {
            settings.stop_after() == Duration::from_secs(420)
        })
    });
    window.press(&key::<PreviousValue>());
    window.heard_set(Setting::StopAfter { seconds: 300 });
    assert_eq!(window.row(&SettingRow::StopAfter).chosen, Some(2), "5 min");
}

#[gpui_kit::test]
fn the_rows_of_the_other_settings_choose_each_of_their_values(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));

    let keep = window.index_of(&SettingRow::KeepStates);
    window.click(&Window9::value(keep, 1, &keep_states_label(7)));
    window.heard_set(Setting::KeepStates { days: 7 });
    assert_eq!(window.row(&SettingRow::KeepStates).chosen, Some(1));
    window.assert_chosen_drawn(&SettingRow::KeepStates);
    window.click(&Window9::value(keep, 0, &keep_states_label(1)));
    window.heard_set(Setting::KeepStates { days: 1 });

    let option = window.index_of(&SettingRow::OptionKey);
    window.click(&Window9::value(option, 1, META));
    window.heard_set(Setting::OptionKey(OptionKey::Meta));
    window.click(&Window9::value(option, 2, CHARACTERS));
    window.heard_set(Setting::OptionKey(OptionKey::Characters));
    window.assert_chosen_drawn(&SettingRow::OptionKey);
    window.click(&Window9::value(option, 0, BY_LAYOUT));
    window.heard_set(Setting::OptionKey(OptionKey::ByLayout));

    let place = window.index_of(&SettingRow::TerminalPlace);
    window.click(&Window9::value(place, 2, LEFT));
    window.heard_set(Setting::TerminalPlace(TerminalPlace::Left));
    window.click(&Window9::value(place, 0, RIGHT));
    window.heard_set(Setting::TerminalPlace(TerminalPlace::Right));
    window.no_dialog();
}

#[gpui_kit::test]
fn the_screen_shows_the_size_of_each_workspace_the_states_of_none_and_the_total(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[('1', 2_621_440), ('2', 4_096)], 2_633_728));

    assert!(window.is_drawn(&format!("settings-size-{}", id('1'))));
    assert!(window.is_drawn(&format!("settings-size-{}:main: 2.5 MB", id('1'))));
    assert!(window.is_drawn(&format!("settings-size-{}", id('2'))));
    assert!(window.is_drawn(&format!("settings-size-{}:feature: 4 KB", id('2'))));
    assert!(window.is_drawn("settings-size-other"));
    assert!(window.is_drawn(&format!("settings-size-other:{OTHER_STATES}: 8 KB")));
    assert!(window.is_drawn("settings-size-total"));
    assert!(window.is_drawn(&format!("settings-size-total:{TOTAL_STATES}: 2.5 MB")));
    let states = window.index_of(&SettingRow::States);
    assert!(window.is_drawn(&Window9::value(states, 0, CLEAN_UP)));
    window.no_dialog();
}

#[gpui_kit::test]
fn a_workspace_the_window_does_not_list_is_in_the_other_states_and_no_line_of_its_own(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    // Workspace `9` is not in the list; its 4 KB and the 2 KB of a store with no workspace make up
    // the rest of the total.
    window.open_screen(sizes(&[('1', 1_024), ('9', 4_096)], 1_024 + 4_096 + 2_048));

    assert!(window.is_drawn(&format!("settings-size-{}:main: 1 KB", id('1'))));
    assert!(!window.is_drawn(&format!("settings-size-{}", id('9'))));
    assert!(window.is_drawn(&format!("settings-size-other:{OTHER_STATES}: 6 KB")));
    assert!(window.is_drawn(&format!("settings-size-total:{TOTAL_STATES}: 7 KB")));
}

#[gpui_kit::test]
fn with_no_state_beyond_the_listed_workspaces_there_is_no_line_of_other_states(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[('1', 1_024), ('2', 2_048)], 3_072));
    assert!(window.is_drawn(&format!("settings-size-total:{TOTAL_STATES}: 3 KB")));
    assert!(!window.is_drawn("settings-size-other"));
}

#[gpui_kit::test]
fn shift_c_cleans_up_at_once_the_control_says_so_until_the_sizes_come_and_a_click_cleans_up_again(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[('1', 2_621_440), ('2', 4_096)], 2_633_728));
    let states = window.index_of(&SettingRow::States);
    let control = Window9::value(states, 0, CLEAN_UP);
    let cleaning = Window9::value(states, 0, CLEANING);
    assert!(window.is_drawn(&control));
    assert!(!window.is_drawn(&cleaning));

    window.press(&key::<CleanUpStates>());
    let asked = window
        .shown
        .keeper
        .expect(Request::CleanUpStates, &mut window.cx);
    assert!(
        window.is_drawn(&cleaning),
        "the control reads that it cleans up"
    );
    assert!(!window.is_drawn(&control));
    assert_eq!(window.row(&SettingRow::States).values, [CLEANING]);
    window.press(&key::<CleanUpStates>());

    window
        .shown
        .keeper
        .reply(asked, Ok(Reply::StateSizes(sizes(&[('1', 4_096)], 4_096))));
    window.wait("the new sizes", |shown, cx| {
        shown
            .models
            .states
            .read_with(cx, |states, _| !states.cleaning())
    });
    assert!(window.is_drawn(&control));
    assert!(window.is_drawn(&format!("settings-size-{}:main: 4 KB", id('1'))));
    assert!(window.is_drawn(&format!("settings-size-total:{TOTAL_STATES}: 4 KB")));
    assert!(!window.is_drawn("settings-size-other"));
    assert!(!window.is_drawn(&format!("settings-size-{}", id('2'))));

    window.click(&control);
    let asked = window
        .shown
        .keeper
        .expect(Request::CleanUpStates, &mut window.cx);
    window
        .shown
        .keeper
        .reply(asked, Ok(Reply::StateSizes(sizes(&[('1', 4_096)], 4_096))));
    window.wait("the answer", |shown, cx| {
        shown
            .models
            .states
            .read_with(cx, |states, _| !states.cleaning())
    });
    window
        .shown
        .models
        .workspaces
        .update(&mut window.cx, |list, cx| list.refresh(cx));
    window
        .shown
        .keeper
        .expect(Request::ListWorkspaces, &mut window.cx);
    window.no_dialog();
}

#[gpui_kit::test]
fn a_deletion_the_keeper_tells_while_the_screen_is_open_has_the_sizes_asked_again(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[('1', 8_192)], 8_192));

    window
        .shown
        .keeper
        .tell(Fact::StatesDeleted { workspace: id('1') });
    let asked = window
        .shown
        .keeper
        .expect(Request::GetStateSizes, &mut window.cx);
    window
        .shown
        .keeper
        .reply(asked, Ok(Reply::StateSizes(sizes(&[('1', 4_096)], 4_096))));
    window.wait("the smaller size", |shown, cx| {
        shown.models.states.read_with(cx, |states, _| {
            states.sizes().map(|s| s.total) == Some(4_096)
        })
    });
    assert!(window.is_drawn(&format!("settings-size-total:{TOTAL_STATES}: 4 KB")));
}

#[gpui_kit::test]
fn a_refused_question_for_the_sizes_shows_its_reason_and_the_control_stays(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.cx.simulate_keystrokes(&pressed::<OpenSettings>());
    let asked = window
        .shown
        .keeper
        .expect(Request::GetStateSizes, &mut window.cx);
    let listing = window
        .shown
        .keeper
        .expect(Request::ListConsents, &mut window.cx);
    window
        .shown
        .keeper
        .reply(listing, Ok(Reply::Consents(Vec::new())));
    window.shown.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the states cannot be read".to_owned(),
        }),
    );
    window.wait("the refusal", |shown, cx| {
        shown
            .models
            .states
            .read_with(cx, |states, _| states.problem().is_some())
    });

    assert!(window.is_drawn("settings-size-problem:the states cannot be read"));
    let states = window.index_of(&SettingRow::States);
    assert!(window.is_drawn(&Window9::value(states, 0, CLEAN_UP)));
    assert!(!window.is_drawn("settings-size-total"));
}

#[gpui_kit::test]
fn the_answer_on_a_repositorys_servers_is_listed_and_h_changes_it_and_the_navigator_is_told(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, vec![servers('a', false)], &one_repository(), cx);
    window.shown.keeper.tell(Fact::ConsentsChanged(vec![consent(
        "pi",
        ConsentAnswer::NotAsked,
    )]));
    window.wait("the question for leave", |shown, cx| {
        shown
            .models
            .consents
            .read_with(cx, |consents, _| !consents.all().is_empty())
    });
    window.open_screen(sizes(&[], 0));

    let row = window.row(&SettingRow::Servers(repo('a')));
    assert_eq!(row.title, format!("{}: repository-a", TITLES[8]));
    assert_eq!(row.values, [RUN_SERVERS, KEEP_SERVERS_OUT]);
    assert_eq!(row.chosen, Some(1), "keep them out");
    let extension = window.row(&SettingRow::Extension("pi".to_owned()));
    assert_eq!(extension.title, format!("{}: pi", TITLES[7]));
    assert_eq!(extension.values, [INSTALL, DONT_INSTALL]);
    assert_eq!(extension.chosen, None, "not asked yet");
    let order: Vec<SettingRow> = window.rows().into_iter().map(|shown| shown.row).collect();
    let first = window.index_of(&SettingRow::Extension("pi".to_owned()));
    assert_eq!(
        order[first..],
        [
            SettingRow::Extension("pi".to_owned()),
            SettingRow::Servers(repo('a')),
            SettingRow::WorktreesFolder(repo('a')),
            SettingRow::States,
        ]
    );

    let at = window.index_of(&SettingRow::Servers(repo('a')));
    window.cursor_to(at);
    window.press(&key::<PreviousValue>());
    window.heard_set(servers('a', true));
    assert_eq!(window.row(&SettingRow::Servers(repo('a'))).chosen, Some(0));
    window.assert_chosen_drawn(&SettingRow::Servers(repo('a')));
    let asks = window
        .navigator
        .until(&mut window.cx, "the answer on the repository", |ask| {
            matches!(ask, Ask::Consents(answers)
                if answers.contains(&RepositoryConsent { repository: repo('a'), allowed: true }))
        });
    assert!(matches!(asks.last(), Some(Ask::Consents(_))));
    window.no_dialog();
}

#[gpui_kit::test]
fn an_answer_on_servers_goes_for_the_repository_of_the_row_it_was_chosen_on_and_for_no_other(
    cx: &mut TestAppContext,
) {
    let mut window = opened(
        WINDOW,
        vec![servers('a', false), servers('b', true)],
        &two_repositories(),
        cx,
    );
    window.open_screen(sizes(&[], 0));
    let r = window.index_of(&SettingRow::Servers(repo('a')));
    let s = window.index_of(&SettingRow::Servers(repo('b')));
    assert_ne!(r, s);
    assert_eq!(window.rows()[r].chosen, Some(1), "R: keep them out");
    assert_eq!(window.rows()[s].chosen, Some(0), "S: run them");

    window.cursor_to(s);
    window.press(&key::<NextValue>());
    window.heard_set(servers('b', false));
    assert_eq!(window.settings_set(), [servers('b', false)]);
    assert_eq!(window.rows()[s].chosen, Some(1));
    assert_eq!(window.rows()[r].chosen, Some(1), "R was not touched");
    window.assert_chosen_drawn(&SettingRow::Servers(repo('b')));
    window.assert_chosen_drawn(&SettingRow::Servers(repo('a')));

    window.click(&Window9::value(r, 0, RUN_SERVERS));
    window.heard_set(servers('a', true));
    assert_eq!(
        window.settings_set(),
        [servers('b', false), servers('a', true)]
    );
    assert_eq!(window.rows()[r].chosen, Some(0));
    assert_eq!(window.rows()[s].chosen, Some(1), "S was not touched");

    let asks = window
        .navigator
        .until(&mut window.cx, "the answers on both repositories", |ask| {
            matches!(ask, Ask::Consents(answers)
                if answers.len() == 2
                    && answers.contains(&RepositoryConsent { repository: repo('a'), allowed: true })
                    && answers.contains(&RepositoryConsent { repository: repo('b'), allowed: false }))
        });
    assert!(matches!(asks.last(), Some(Ask::Consents(_))));
    window.no_dialog();
}

#[gpui_kit::test]
fn the_extension_of_an_agent_is_installed_by_a_click_and_the_answer_can_be_changed_later(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.shown.keeper.tell(Fact::ConsentsChanged(vec![consent(
        "pi",
        ConsentAnswer::NotAsked,
    )]));
    window.wait("the question for leave", |shown, cx| {
        shown
            .models
            .consents
            .read_with(cx, |consents, _| !consents.all().is_empty())
    });
    window.open_screen(sizes(&[], 0));
    let at = window.index_of(&SettingRow::Extension("pi".to_owned()));
    assert!(window.is_drawn(&format!(
        "settings-question-{at}:May the app add its extension to the settings of pi?"
    )));

    window.click(&Window9::value(at, 0, INSTALL));
    let asked = window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::Yes,
        },
        &mut window.cx,
    );
    window.shown.keeper.reply(
        asked,
        Ok(Reply::Consents(vec![consent("pi", ConsentAnswer::Yes)])),
    );
    window.wait("the answer to show", |shown, cx| {
        rows_now(shown, cx)[at].chosen == Some(0)
    });
    assert!(
        window.is_drawn(&format!("settings-chosen-{at}")),
        "the answered row still has a chosen value"
    );
    window.assert_chosen_drawn(&SettingRow::Extension("pi".to_owned()));

    window.click(&Window9::value(at, 1, DONT_INSTALL));
    let asked = window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::No,
        },
        &mut window.cx,
    );
    window.shown.keeper.reply(
        asked,
        Ok(Reply::Consents(vec![consent("pi", ConsentAnswer::No)])),
    );
    window.wait("the second answer to show", |shown, cx| {
        rows_now(shown, cx)[at].chosen == Some(1)
    });
    window.assert_chosen_drawn(&SettingRow::Extension("pi".to_owned()));
    window.no_dialog();
}

#[gpui_kit::test]
fn an_extension_answered_before_the_window_started_is_asked_for_at_the_opening_and_listed_with_its_answer(
    cx: &mut TestAppContext,
) {
    // The Keeper tells a Window of an Adapter only while a question waits or something went wrong.
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window
        .shown
        .models
        .consents
        .read_with(&window.cx, |consents, _| assert!(consents.all().is_empty()));

    window.cx.simulate_keystrokes(&pressed::<OpenSettings>());
    let sizes_asked = window
        .shown
        .keeper
        .expect(Request::GetStateSizes, &mut window.cx);
    let listing = window
        .shown
        .keeper
        .expect(Request::ListConsents, &mut window.cx);
    window
        .shown
        .keeper
        .reply(sizes_asked, Ok(Reply::StateSizes(sizes(&[], 0))));
    window.shown.keeper.reply(
        listing,
        Ok(Reply::Consents(vec![consent("pi", ConsentAnswer::Yes)])),
    );
    let pi = SettingRow::Extension("pi".to_owned());
    window.wait("the row of the extension", |shown, cx| {
        rows_now(shown, cx).iter().any(|shown| shown.row == pi)
    });
    let at = window.index_of(&pi);
    assert_eq!(
        window.row(&pi).chosen,
        Some(0),
        "the answer given before is the one chosen"
    );
    assert!(window.is_drawn(&Window9::value(at, 0, INSTALL)));
    window.assert_chosen_drawn(&pi);

    window.click(&Window9::value(at, 1, DONT_INSTALL));
    let asked = window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::No,
        },
        &mut window.cx,
    );
    window.shown.keeper.reply(
        asked,
        Ok(Reply::Consents(vec![consent("pi", ConsentAnswer::No)])),
    );
    window.wait("the answer to show", |shown, cx| {
        rows_now(shown, cx)[at].chosen == Some(1)
    });
    window.no_dialog();
}

#[gpui_kit::test]
fn an_answer_goes_for_the_adapter_of_the_row_it_was_chosen_on_and_for_no_other(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    let listed = |claude, pi| vec![consent("claude", claude), consent("pi", pi)];
    window.shown.keeper.tell(Fact::ConsentsChanged(listed(
        ConsentAnswer::NotAsked,
        ConsentAnswer::NotAsked,
    )));
    window.wait("the two questions for leave", |shown, cx| {
        shown
            .models
            .consents
            .read_with(cx, |consents, _| consents.all().len() == 2)
    });
    window.open_screen(sizes(&[], 0));
    let claude = window.index_of(&SettingRow::Extension("claude".to_owned()));
    let pi = window.index_of(&SettingRow::Extension("pi".to_owned()));
    assert_ne!(claude, pi);

    window.click(&Window9::value(pi, 0, INSTALL));
    let asked = window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::Yes,
        },
        &mut window.cx,
    );
    window.shown.keeper.reply(
        asked,
        Ok(Reply::Consents(listed(
            ConsentAnswer::NotAsked,
            ConsentAnswer::Yes,
        ))),
    );
    window.wait("pi's answer to show", |shown, cx| {
        rows_now(shown, cx)[pi].chosen == Some(0)
    });
    assert_eq!(window.rows()[claude].chosen, None, "claude was not asked");

    window.click(&Window9::value(claude, 1, DONT_INSTALL));
    let asked = window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "claude".to_owned(),
            answer: ConsentAnswer::No,
        },
        &mut window.cx,
    );
    window.shown.keeper.reply(
        asked,
        Ok(Reply::Consents(listed(
            ConsentAnswer::No,
            ConsentAnswer::Yes,
        ))),
    );
    window.wait("claude's answer to show", |shown, cx| {
        rows_now(shown, cx)[claude].chosen == Some(1)
    });
    assert_eq!(window.rows()[pi].chosen, Some(0));

    window.cursor_to(pi);
    window.press(&key::<NextValue>());
    window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::No,
        },
        &mut window.cx,
    );
    window.no_dialog();
}

#[gpui_kit::test]
fn what_went_wrong_with_an_extension_is_drawn_under_its_row(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    let failed = Consent {
        problem: Some("settings.json is not JSON".to_owned()),
        ..consent("pi", ConsentAnswer::Yes)
    };
    window
        .shown
        .keeper
        .tell(Fact::ConsentsChanged(vec![failed]));
    window.wait("the question for leave", |shown, cx| {
        shown
            .models
            .consents
            .read_with(cx, |consents, _| !consents.all().is_empty())
    });
    window.open_screen(sizes(&[], 0));
    let at = window.index_of(&SettingRow::Extension("pi".to_owned()));
    assert!(window.is_drawn(&format!("settings-problem-{at}:settings.json is not JSON")));
}

fn with_folders(cx: &mut TestAppContext, settings: Vec<Setting>) -> (Window9, usize, usize) {
    let mut window = opened(WINDOW, settings, &two_repositories(), cx);
    window.open_screen(sizes(&[], 0));
    let r = window.index_of(&SettingRow::WorktreesFolder(repo('a')));
    let s = window.index_of(&SettingRow::WorktreesFolder(repo('b')));
    (window, r, s)
}

const FOLDER: &str = "/gits/elsewhere";

#[gpui_kit::test]
fn each_listed_repository_has_a_row_for_its_folder_after_the_servers_and_before_the_states(
    cx: &mut TestAppContext,
) {
    let (mut window, r, s) = with_folders(cx, vec![servers('b', true)]);

    let rows = window.rows();
    let servers_at = window.index_of(&SettingRow::Servers(repo('b')));
    assert_eq!(s, r + 1);
    assert_eq!(r, servers_at + 1, "after the row of the servers");
    assert_eq!(
        rows[s + 1].row,
        SettingRow::States,
        "right before the states"
    );
    assert_eq!(rows[r].title, format!("{}: repository-a", TITLES[10]));
    assert_eq!(rows[s].title, format!("{}: repository-b", TITLES[10]));
    for shown in [&rows[r], &rows[s]] {
        assert_eq!(shown.values, [DEFAULT_FOLDER]);
        assert_eq!(shown.chosen, Some(0));
    }

    window
        .shown
        .keeper
        .tell(Fact::SettingChanged(folder_of('a', Some(FOLDER))));
    window.wait("the folder", |shown, cx| {
        shown.models.settings.read_with(cx, |settings, _| {
            settings.worktrees_folder(&repo('a')) == Some(PathBuf::from(FOLDER))
        })
    });
    let rows = window.rows();
    assert_eq!(rows[r].values, [FOLDER]);
    assert_eq!(rows[r].chosen, Some(0));
    assert_eq!(
        rows[s].values,
        [DEFAULT_FOLDER],
        "the other repository's is its own"
    );
    window.no_dialog();
}

#[gpui_kit::test]
fn enter_on_a_folder_row_opens_its_field_holding_the_folder_focused_and_enter_sends_it_and_the_answer_closes_it(
    cx: &mut TestAppContext,
) {
    let (mut window, r, _) = with_folders(cx, vec![folder_of('a', Some(FOLDER))]);
    let screen_focus = window.screen_focus();
    window.cursor_to(r);
    assert!(!window.is_drawn("settings-folder-field"));

    window.press(&key::<OpenSetting>());

    assert!(window.is_drawn("settings-folder-field"));
    assert!(!window.has_focus(&screen_focus), "the field has the focus");
    window.cx.simulate_input("/hjkl");
    window.press("enter");
    assert_eq!(window.cursor(), r);
    window.heard_set(Setting::WorktreesFolder {
        repository: repo('a'),
        folder: Some(PathBuf::from("/gits/elsewhere/hjkl")),
    });
    window.wait("the field to close", |_, cx| {
        drawn("settings-folder-field", cx).is_none()
    });
    assert!(window.has_focus(&screen_focus));
    assert_eq!(
        window.rows()[r].values,
        ["/gits/elsewhere/hjkl"],
        "the row reads what was sent"
    );
    assert!(!window.is_drawn("settings-folder-refused"));
    window.no_dialog();
}

#[gpui_kit::test]
fn the_text_of_the_field_replaced_by_a_new_folder_is_what_enter_sends(cx: &mut TestAppContext) {
    let (mut window, r, _) = with_folders(cx, vec![folder_of('a', Some(FOLDER))]);
    window.cursor_to(r);
    window.press(&key::<OpenSetting>());
    window.shown.keeper.hold_settings();

    window.press(&vec!["backspace"; FOLDER.len()].join(" "));
    window.cx.simulate_input("/x/places");
    window.press("enter");

    assert_eq!(window.rows()[r].values, ["/x/places"]);
    window.heard_set(folder_of('a', Some("/x/places")));
    window.wait("the answer to be held", |shown, _| {
        shown.keeper.held_settings() == 1
    });
    window.shown.keeper.release_settings(1);
    assert_eq!(window.rows()[r].values, ["/x/places"]);
    window.wait("the field to close", |_, cx| {
        drawn("settings-folder-field", cx).is_none()
    });
    assert_eq!(
        window.settings_set().len(),
        1,
        "one request for the one Enter"
    );
}

#[gpui_kit::test]
fn blanks_around_the_folder_are_taken_off(cx: &mut TestAppContext) {
    let (mut window, r, _) = with_folders(cx, Vec::new());
    window.cursor_to(r);
    window.press(&key::<OpenSetting>());

    window.cx.simulate_input("  /x/places ");
    window.press("enter");

    window.heard_set(folder_of('a', Some("/x/places")));
}

#[gpui_kit::test]
fn a_refused_folder_keeps_the_field_open_with_the_reason_and_the_row_reads_what_the_keeper_lists(
    cx: &mut TestAppContext,
) {
    let (mut window, r, _) = with_folders(cx, Vec::new());
    window.cursor_to(r);
    window.press(&key::<OpenSetting>());
    window.shown.keeper.refuse_settings(Some(Refusal::Failed {
        reason: "it lies inside a worktree".to_owned(),
    }));
    let asked_before = window.shown.keeper.settings_asked();

    window.cx.simulate_input("/gits/main/inside");
    window.press("enter");

    window.wait("the reason to be drawn", |_, cx| {
        drawn("settings-folder-refused:it lies inside a worktree", cx).is_some()
    });
    assert!(window.is_drawn("settings-folder-refused"));
    assert!(window.is_drawn("settings-folder-field"), "the field stays");
    window.wait("the list to be asked again", |shown, _| {
        shown.keeper.settings_asked() > asked_before
    });
    window.wait("the row to read the Keeper's list", |shown, cx| {
        rows_now(shown, cx)[r].values == [DEFAULT_FOLDER]
    });
    assert_eq!(window.rows()[r].chosen, Some(0));

    window.shown.keeper.refuse_settings(None);
    window.cx.simulate_input("x");
    window.press("enter");
    window.heard_set(folder_of('a', Some("/gits/main/insidex")));
    window.wait("the field to close", |_, cx| {
        drawn("settings-folder-field", cx).is_none()
    });
    assert!(!window.is_drawn("settings-folder-refused"));
    window.no_dialog();
}

#[gpui_kit::test]
fn the_field_emptied_and_enter_sets_none_and_the_row_reads_the_default(cx: &mut TestAppContext) {
    let (mut window, r, _) = with_folders(cx, vec![folder_of('a', Some(FOLDER))]);
    window.cursor_to(r);
    assert_eq!(window.rows()[r].values, [FOLDER]);
    window.press(&key::<OpenSetting>());

    window.press(&vec!["backspace"; FOLDER.len()].join(" "));
    window.press("enter");

    window.heard_set(folder_of('a', None));
    assert_eq!(window.rows()[r].values, [DEFAULT_FOLDER]);
    assert_eq!(window.rows()[r].chosen, Some(0));
    window.wait("the field to close", |_, cx| {
        drawn("settings-folder-field", cx).is_none()
    });
}

#[gpui_kit::test]
fn escape_in_the_field_closes_it_with_nothing_sent_and_the_screen_stays_open(
    cx: &mut TestAppContext,
) {
    let (mut window, r, _) = with_folders(cx, vec![folder_of('a', Some(FOLDER))]);
    let screen_focus = window.screen_focus();
    window.cursor_to(r);
    window.press(&key::<OpenSetting>());
    window.cx.simulate_input("/unsaved");
    assert!(window.is_drawn("settings-folder-field"));

    window.press("escape");

    assert!(
        !window.is_drawn("settings-folder-field"),
        "the field is closed"
    );
    assert!(window.screen_is_open(), "the screen stays");
    assert!(window.has_focus(&screen_focus), "the focus is the screen's");
    assert_eq!(window.rows()[r].values, [FOLDER], "the row is as it was");
    assert_eq!(
        window.barrier(),
        [Setting::TurnEnd(TurnEndSelects::KeepSelection)]
    );

    window.cursor_to(r);
    window.press(&key::<OpenSetting>());
    window.press("enter");
    window.heard_set(folder_of('a', Some(FOLDER)));
}

#[gpui_kit::test]
fn escape_closes_the_screen_when_no_field_is_open_and_the_key_of_settings_does_too(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));

    window.press("escape");
    assert!(!window.screen_is_open());
    assert!(!window.is_drawn("settings-screen"));

    window.open_screen_again();
    assert!(window.screen_is_open());
    window.press(&pressed::<OpenSettings>());
    assert!(!window.screen_is_open());
}

#[gpui_kit::test]
fn a_click_on_the_value_of_a_row_opens_the_field_of_that_row_and_one_field_is_open_at_a_time(
    cx: &mut TestAppContext,
) {
    let (mut window, r, s) = with_folders(cx, Vec::new());

    window.click(&Window9::value(s, 0, DEFAULT_FOLDER));
    assert!(window.is_drawn("settings-folder-field"));
    assert_eq!(window.cursor(), s, "the cursor is on the row of the click");
    window.cx.simulate_input("/for/b");
    window.press("enter");
    window.heard_set(folder_of('b', Some("/for/b")));
    window.wait("the field to close", |_, cx| {
        drawn("settings-folder-field", cx).is_none()
    });

    window.click(&Window9::value(r, 0, DEFAULT_FOLDER));
    window.cx.simulate_input("/given/up");
    window.click(&Window9::value(s, 0, "/for/b"));
    window.cx.simulate_input("/more");
    window.press("enter");
    window.heard_set(folder_of('b', Some("/for/b/more")));
    assert_eq!(
        window.settings_set(),
        [
            folder_of('b', Some("/for/b")),
            folder_of('b', Some("/for/b/more"))
        ]
    );
    window.no_dialog();
}

#[gpui_kit::test]
fn a_click_on_another_rows_value_while_a_field_is_open_closes_the_field_and_chooses_the_value(
    cx: &mut TestAppContext,
) {
    let (mut window, r, _) = with_folders(cx, Vec::new());
    window.cursor_to(r);
    window.press(&key::<OpenSetting>());
    assert!(window.is_drawn("settings-folder-field"));

    let turn_end = window.index_of(&SettingRow::TurnEnd);
    window.click(&Window9::value(turn_end, 1, KEEP_SELECTION));

    assert!(!window.is_drawn("settings-folder-field"));
    window.heard_set(Setting::TurnEnd(TurnEndSelects::KeepSelection));
    assert_eq!(window.settings_set().len(), 1, "the field sent nothing");
}

#[gpui_kit::test]
fn enter_on_a_row_that_does_not_act_sends_nothing(cx: &mut TestAppContext) {
    let mut window = opened(WINDOW, vec![servers('a', false)], &one_repository(), cx);
    window.shown.keeper.tell(Fact::ConsentsChanged(vec![consent(
        "pi",
        ConsentAnswer::NotAsked,
    )]));
    window.wait("the question for leave", |shown, cx| {
        shown
            .models
            .consents
            .read_with(cx, |consents, _| !consents.all().is_empty())
    });
    window.open_screen(sizes(&[], 0));
    let quiet: Vec<usize> = window
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, shown)| {
            !matches!(
                shown.row,
                SettingRow::Base | SettingRow::WorktreesFolder(_) | SettingRow::States
            )
        })
        .map(|(at, _)| at)
        .collect();
    assert!(
        quiet.len() >= 8,
        "the six choice rows of the app, the servers and the extension: {quiet:?}"
    );
    for row in quiet {
        window.cursor_to(row);
        window.press(&key::<OpenSetting>());
    }
    let states = window.index_of(&SettingRow::States);
    window.cursor_to(states);
    window.press(&key::<OpenSetting>());
    assert!(!window.is_drawn("settings-folder-field"));
    assert!(!window.is_drawn("frame-base-field"));
    assert_eq!(
        window.barrier(),
        [Setting::TurnEnd(TurnEndSelects::KeepSelection)],
        "nothing was set before the barrier"
    );
    window.handed_barrier();
    window.no_dialog();
}

#[gpui_kit::test]
fn a_repository_that_leaves_the_list_takes_its_row_and_an_open_screen_follows_the_models(
    cx: &mut TestAppContext,
) {
    let (mut window, _, _) = with_folders(cx, Vec::new());
    let app = window.index_of(&SettingRow::WorktreesFolder(repo('a')));
    assert_eq!(window.rows().len(), app + 2 + 1);

    window.shown.keeper.tell(Fact::Workspaces(vec![repository(
        'a',
        vec![workspace('1', "main")],
    )]));
    window.wait("the shorter list", |shown, cx| {
        shown
            .models
            .workspaces
            .read_with(cx, |list, _| list.repositories().len() == 1)
    });
    let kinds: Vec<SettingRow> = window.rows().into_iter().map(|shown| shown.row).collect();
    assert_eq!(kinds.len(), app + 1 + 1);
    assert!(!kinds.contains(&SettingRow::WorktreesFolder(repo('b'))));
    assert!(window.is_drawn(&format!("settings-row-{}", kinds.len() - 1)));
    assert!(!window.is_drawn(&format!("settings-row-{}", kinds.len())));
}

#[gpui_kit::test]
fn the_cursor_stays_on_a_row_there_is_when_a_repository_leaves_the_list(cx: &mut TestAppContext) {
    let (mut window, _, _) = with_folders(cx, Vec::new());
    let last = window.rows().len() - 1;
    window.cursor_to(last);

    window.shown.keeper.tell(Fact::Workspaces(vec![repository(
        'a',
        vec![workspace('1', "main")],
    )]));
    window.wait("the shorter list", |shown, cx| {
        shown
            .models
            .workspaces
            .read_with(cx, |list, _| list.repositories().len() == 1)
    });

    let rows = window.rows();
    assert_eq!(window.cursor(), rows.len() - 1, "on the row of the states");
    assert_eq!(rows[window.cursor()].row, SettingRow::States);
    let row = window.bounds(&format!("settings-row-{}", rows.len() - 1));
    let cursor = window.bounds("settings-cursor");
    assert!(row.contains(&cursor.origin));
}

#[gpui_kit::test]
fn a_theme_named_in_other_letters_is_the_one_chosen_and_a_name_no_theme_has_is_the_systems(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    let tell = |window: &mut Window9, name: &str| {
        window
            .shown
            .keeper
            .tell(Fact::SettingChanged(Setting::Theme(ThemeChoice::Named(
                name.to_owned(),
            ))));
        window.wait("the theme", |shown, cx| {
            shown.models.settings.read_with(cx, |settings, _| {
                settings.theme() == ThemeChoice::Named(name.to_owned())
            })
        });
    };

    tell(&mut window, "dark");
    assert_eq!(window.row(&SettingRow::Theme).chosen, Some(2));
    tell(&mut window, "Nowhere");
    assert_eq!(
        window.row(&SettingRow::Theme).chosen,
        Some(0),
        "no theme has the name: the system's is what is in force"
    );
}

#[gpui_kit::test]
fn a_workspace_on_screen_whose_changes_name_no_base_says_so_and_the_row_still_opens_the_field(
    cx: &mut TestAppContext,
) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.open_screen(sizes(&[], 0));
    window
        .shown
        .keeper
        .tell(Fact::ChangesChanged { workspace: id('1') });
    let asked = window
        .shown
        .keeper
        .expect(Request::GetChanges { workspace: id('1') }, &mut window.cx);
    window.shown.keeper.reply(
        asked,
        Ok(Reply::Changes(Changes {
            comparison: Comparison::NothingToCompare {
                base: None,
                why: NoComparison::NoBase,
            },
            ..changes_since('1', "main")
        })),
    );
    window.wait("no base", |shown, cx| {
        rows_now(shown, cx)
            .iter()
            .any(|shown| shown.row == SettingRow::Base && shown.values == [NO_BASE])
    });
    let row = window.row(&SettingRow::Base);
    assert_eq!(row.chosen, None);

    let base = window.index_of(&SettingRow::Base);
    window.cursor_to(base);
    window.press(&key::<OpenSetting>());
    assert!(window.is_drawn("frame-base-field"));
}

fn with_a_question(cx: &mut TestAppContext) -> (Window9, usize) {
    let mut window = opened(WINDOW, Vec::new(), &one_repository(), cx);
    window.shown.keeper.tell(Fact::ConsentsChanged(vec![consent(
        "pi",
        ConsentAnswer::NotAsked,
    )]));
    window.wait("the question for leave", |shown, cx| {
        shown
            .models
            .consents
            .read_with(cx, |consents, _| !consents.all().is_empty())
    });
    window.open_screen(sizes(&[], 0));
    let at = window.index_of(&SettingRow::Extension("pi".to_owned()));
    (window, at)
}

#[gpui_kit::test]
fn l_and_h_on_an_unanswered_extension_choose_the_first_and_the_last_value(cx: &mut TestAppContext) {
    let (mut window, at) = with_a_question(cx);
    window.cursor_to(at);

    window.press(&key::<NextValue>());
    window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::Yes,
        },
        &mut window.cx,
    );
    window.press(&key::<PreviousValue>());
    window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::No,
        },
        &mut window.cx,
    );
}

#[gpui_kit::test]
fn a_refused_answer_to_a_question_for_leave_is_drawn_above_the_rows_until_the_next_answer(
    cx: &mut TestAppContext,
) {
    let (mut window, at) = with_a_question(cx);
    assert!(!window.is_drawn("settings-notice"));

    window.click(&Window9::value(at, 0, INSTALL));
    let asked = window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::Yes,
        },
        &mut window.cx,
    );
    window.shown.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "pi is not installed".to_owned(),
        }),
    );
    window.wait("the reason", |_, cx| {
        drawn("settings-notice:pi is not installed", cx).is_some()
    });
    assert_eq!(
        window.row(&SettingRow::Extension("pi".to_owned())).chosen,
        None
    );

    window.click(&Window9::value(at, 1, DONT_INSTALL));
    window.shown.keeper.expect(
        Request::AnswerConsent {
            agent: "pi".to_owned(),
            answer: ConsentAnswer::No,
        },
        &mut window.cx,
    );
    assert!(!window.is_drawn("settings-notice"));
}
