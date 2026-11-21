//! Component test: the Frame's models of slice 9, through what the panes and the settings
//! screen use of them: the settings of each kind with its default, the folder for the
//! worktrees of a repository and the Keeper's answer to a set, the stop time the model tells
//! the Navigator, the one chosen agent CLI, the sizes of the recorded states, and a
//! workspace put on screen before the list holds it.
//!
//! The test plays the Keeper on a socket pair and the Navigator at the model's port, with
//! data: it answers what the models ask, tells facts, reads the asks the model really makes,
//! and computes nothing either computes. A negative clause ("asks nothing", "tells nothing")
//! is read after a barrier: a question or an ask the test causes next, which arrives after
//! everything the model sent before it, through the one channel each of them leaves by.

mod common;

use std::cell::Cell;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::{AppContext, TestAppContext};
use yaar_frame::keeper::Attached;
use yaar_frame::{AskError, KeeperLink, Models, NOT_KEPT};
use yaar_link::{
    AgentCli, Fact, Hello, Message, OptionKey, PROTOCOL_VERSION, Refusal, Reply, Repository,
    RepositoryId, Request, Setting, StateSize, StateSizes, TerminalPlace, ThemeChoice,
    TurnEndSelects, WorkspaceId, read_message, write_message,
};
use yaar_navigator::{Ask, RepositoryConsent, Told, port};

use common::{DEADLINE, KeeperEnd, answer, id, repository, wait_until, workspace};

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

fn seconds(seconds: u32) -> Setting {
    Setting::StopAfter { seconds }
}

fn folder_of(letter: char, folder: Option<&str>) -> Setting {
    Setting::WorktreesFolder {
        repository: repo(letter),
        folder: folder.map(PathBuf::from),
    }
}

fn cli(name: &str) -> AgentCli {
    AgentCli {
        agent: name.to_owned(),
        title: name.to_uppercase(),
    }
}

/// Two worktrees of one repository.
fn listed() -> Vec<Repository> {
    vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )]
}

/// The models of a window whose Keeper the test plays: `settings` and `clis` are what the
/// Keeper holds when the Window reaches it. Every model has taken its answer, and the
/// question for the changes of the workspace on screen, which the Frame asks by itself, is
/// read and left unanswered.
fn window(
    settings: Vec<Setting>,
    clis: Vec<AgentCli>,
    cx: &mut TestAppContext,
) -> (Models, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let (held, offered) = (settings.clone(), clis.clone());
    let mut keeper = KeeperEnd::attach_staged(hello(), &link, cx, move |keeper| {
        keeper.set_settings(settings);
        keeper.set_clis(clis);
    });
    keeper.list(&listed(), cx);
    wait_until(cx, "the list, the settings and the CLIs", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.repositories() == listed())
            && models.settings.read_with(cx, |list, _| list.all() == held)
            && models
                .agents
                .read_with(cx, |list, _| list.clis() == offered)
    });
    keeper.expect(Request::GetChanges { workspace: id('1') }, cx);
    (models, keeper)
}

/// A barrier: asks the Keeper for the list of workspaces again and gives every request
/// the Window made up to that question, which is answered. What a model asked before it is
/// among them, and nothing it asked after it is.
fn barrier(models: &Models, keeper: &mut KeeperEnd, cx: &mut TestAppContext) -> Vec<Request> {
    models.workspaces.update(cx, |list, cx| list.refresh(cx));
    let mut before = Vec::new();
    loop {
        let (asked, request) = keeper.request(cx);
        if request == Request::ListWorkspaces {
            keeper.reply(asked, Ok(Reply::Workspaces(listed())));
            return before;
        }
        before.push(request);
    }
}

fn sizes(workspaces: &[(char, u64)], total: u64) -> StateSizes {
    StateSizes {
        workspaces: workspaces
            .iter()
            .map(|(digit, bytes)| StateSize {
                workspace: id(*digit),
                bytes: *bytes,
            })
            .collect(),
        total,
    }
}

// The settings of each kind.

#[gpui_kit::test]
fn each_reader_gives_its_default_while_nothing_is_set(cx: &mut TestAppContext) {
    let (models, keeper) = window(Vec::new(), Vec::new(), cx);
    wait_until(cx, "the question for the settings", |_| {
        keeper.settings_asked() >= 1
    });

    models.settings.read_with(cx, |list, _| {
        assert_eq!(list.turn_end(), TurnEndSelects::WhatChanged);
        assert_eq!(list.stop_after(), Duration::from_secs(600));
        assert_eq!(list.stop_after_set(), None);
        assert_eq!(list.theme(), ThemeChoice::System);
        assert_eq!(list.terminal_place(), TerminalPlace::Right);
        assert_eq!(list.keep_states(), 30);
        assert_eq!(list.option_key(), OptionKey::ByLayout);
        assert_eq!(list.servers_answers(), Vec::new());
        assert_eq!(list.worktrees_folder(&repo('a')), None);
    });
}

#[gpui_kit::test]
fn each_reader_gives_what_the_keeper_listed_then_what_it_told(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(
        vec![
            Setting::TurnEnd(TurnEndSelects::KeepSelection),
            seconds(90),
            Setting::Theme(ThemeChoice::Named("Light".to_owned())),
            Setting::TerminalPlace(TerminalPlace::Below),
            Setting::KeepStates { days: 7 },
            Setting::OptionKey(OptionKey::Meta),
            Setting::ServersInRepository {
                repository: repo('b'),
                allowed: false,
            },
            Setting::ServersInRepository {
                repository: repo('a'),
                allowed: true,
            },
            folder_of('a', Some("/work/trees")),
        ],
        Vec::new(),
        cx,
    );
    models.settings.read_with(cx, |list, _| {
        assert_eq!(list.turn_end(), TurnEndSelects::KeepSelection);
        assert_eq!(list.stop_after(), Duration::from_secs(90));
        assert_eq!(list.stop_after_set(), Some(Duration::from_secs(90)));
        assert_eq!(list.theme(), ThemeChoice::Named("Light".to_owned()));
        assert_eq!(list.terminal_place(), TerminalPlace::Below);
        assert_eq!(list.keep_states(), 7);
        assert_eq!(list.option_key(), OptionKey::Meta);
        assert_eq!(
            list.servers_answers(),
            [(repo('b'), false), (repo('a'), true)],
            "in the order the Keeper listed them"
        );
        assert_eq!(
            list.worktrees_folder(&repo('a')),
            Some(PathBuf::from("/work/trees"))
        );
    });

    for told in [
        Setting::TurnEnd(TurnEndSelects::WhatChanged),
        seconds(5),
        Setting::Theme(ThemeChoice::System),
        Setting::TerminalPlace(TerminalPlace::Left),
        Setting::OptionKey(OptionKey::Characters),
        Setting::ServersInRepository {
            repository: repo('b'),
            allowed: true,
        },
        Setting::KeepStates { days: 90 },
    ] {
        keeper.tell(Fact::SettingChanged(told));
    }
    wait_until(cx, "the last setting told", |cx| {
        models
            .settings
            .read_with(cx, |list, _| list.keep_states() == 90)
    });
    models.settings.read_with(cx, |list, _| {
        assert_eq!(list.turn_end(), TurnEndSelects::WhatChanged);
        assert_eq!(list.stop_after_set(), Some(Duration::from_secs(5)));
        assert_eq!(list.stop_after(), Duration::from_secs(5));
        assert_eq!(list.theme(), ThemeChoice::System);
        assert_eq!(list.terminal_place(), TerminalPlace::Left);
        assert_eq!(list.option_key(), OptionKey::Characters);
        assert_eq!(
            list.servers_answers(),
            [(repo('b'), true), (repo('a'), true)],
            "an answer told again keeps its place"
        );
    });
}

#[gpui_kit::test]
fn the_folder_for_worktrees_is_the_one_told_for_its_repository_and_none_after_none(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let folder = |letter: char, cx: &mut TestAppContext| {
        models
            .settings
            .read_with(cx, |list, _| list.worktrees_folder(&repo(letter)))
    };
    assert_eq!(folder('a', cx), None, "none while nothing is set");

    keeper.tell(Fact::SettingChanged(folder_of('a', Some("/work/trees"))));
    wait_until(cx, "the folder", |cx| folder('a', cx).is_some());
    assert_eq!(folder('a', cx), Some(PathBuf::from("/work/trees")));
    assert_eq!(folder('b', cx), None, "a second repository has none");

    keeper.tell(Fact::SettingChanged(folder_of('b', Some("/other"))));
    wait_until(cx, "the second repository's folder", |cx| {
        folder('b', cx).is_some()
    });
    assert_eq!(
        folder('a', cx),
        Some(PathBuf::from("/work/trees")),
        "one repository's folder is not another's"
    );

    keeper.tell(Fact::SettingChanged(folder_of('a', None)));
    wait_until(cx, "none for the first repository", |cx| {
        folder('a', cx).is_none()
    });
    assert_eq!(folder('b', cx), Some(PathBuf::from("/other")));
}

#[gpui_kit::test]
fn a_folder_set_and_answered_is_held_at_once_sent_and_given_ok_on_done(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let setting = folder_of('a', Some("/work/trees"));

    // Read before the app runs again: the Keeper's answer cannot have been taken.
    let task = models
        .settings
        .update(cx, |list, cx| list.set_answered(setting.clone(), cx));
    models.settings.read_with(cx, |list, _| {
        assert_eq!(
            list.worktrees_folder(&repo('a')),
            Some(PathBuf::from("/work/trees")),
            "held before the Keeper answered"
        );
    });

    assert_eq!(answer(task, cx), Ok(()));
    assert_eq!(keeper.settings_set(), [setting]);
    // A kept setting is no reason to ask the list again.
    barrier(&models, &mut keeper, cx);
    assert_eq!(keeper.settings_asked(), 1);
}

#[gpui_kit::test]
fn a_folder_the_keeper_refuses_gives_its_reason_and_has_the_list_asked_again(
    cx: &mut TestAppContext,
) {
    let (models, keeper) = window(vec![folder_of('a', Some("/kept"))], Vec::new(), cx);
    keeper.refuse_settings(Some(Refusal::Failed {
        reason: "that folder lies inside a worktree".to_owned(),
    }));

    let task = models.settings.update(cx, |list, cx| {
        list.set_answered(folder_of('a', Some("/inside")), cx)
    });
    assert_eq!(
        answer(task, cx),
        Err("that folder lies inside a worktree".to_owned())
    );
    assert_eq!(
        keeper.settings_set(),
        [folder_of('a', Some("/inside"))],
        "the Keeper was asked to keep it"
    );
    wait_until(cx, "the list asked again", |_| keeper.settings_asked() == 2);
    wait_until(cx, "the folder the Keeper keeps", |cx| {
        models.settings.read_with(cx, |list, _| {
            list.worktrees_folder(&repo('a')) == Some(PathBuf::from("/kept"))
        })
    });
}

#[gpui_kit::test]
fn a_refusal_that_is_no_failure_gives_its_own_sentence(cx: &mut TestAppContext) {
    let (models, keeper) = window(Vec::new(), Vec::new(), cx);
    let refusal = Refusal::UnknownRevision {
        revision: "nowhere".to_owned(),
    };
    keeper.refuse_settings(Some(refusal.clone()));

    let task = models.settings.update(cx, |list, cx| {
        list.set_answered(folder_of('a', Some("/x")), cx)
    });
    assert_eq!(answer(task, cx), Err(refusal.to_string()));
}

#[gpui_kit::test]
fn a_set_with_no_keeper_to_reach_gives_the_sentence_that_it_was_not_kept(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link, cx));

    let task = models.settings.update(cx, |list, cx| {
        list.set_answered(folder_of('a', Some("/x")), cx)
    });
    assert_eq!(answer(task, cx), Err(NOT_KEPT.to_owned()));
}

#[gpui_kit::test]
fn an_answer_that_is_no_answer_to_a_set_gives_the_sentence_that_it_was_not_kept(
    cx: &mut TestAppContext,
) {
    // A Keeper that answers every request but the question for the settings with a list of
    // workspaces.
    cx.executor().allow_parking();
    let (ours, theirs) = UnixStream::pair().expect("a socket pair");
    let mut reading = ours.try_clone().expect("a second handle");
    let mut writing = ours;
    std::thread::spawn(move || {
        while let Ok(message) = read_message(&mut reading) {
            if let Message::Request { id, request } = message {
                let reply = match request {
                    Request::GetSettings => Reply::Settings(Vec::new()),
                    _ => Reply::Workspaces(Vec::new()),
                };
                let reply = Message::Reply {
                    to: id,
                    reply: Ok(reply),
                };
                if write_message(&mut writing, &reply).is_err() {
                    return;
                }
            }
        }
    });
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    link.update(cx, |link, cx| {
        link.attached(
            Ok(Attached {
                stream: theirs,
                keeper: hello(),
                started: false,
            }),
            cx,
        )
    });

    let task = models.settings.update(cx, |list, cx| {
        list.set_answered(folder_of('a', Some("/x")), cx)
    });
    assert_eq!(answer(task, cx), Err(NOT_KEPT.to_owned()));
}

// The stop time told to the Navigator.

/// The Navigator, played by the test at the other end of the model's port.
struct Navigator {
    /// The asks, moved by a thread of this end's own from the port onto a channel the test
    /// waits on while the app runs.
    asks: Receiver<Ask>,
    _told: async_channel::Sender<Told>,
}

impl Navigator {
    /// Connects a new port to the model and keeps the Navigator's end.
    fn connect(models: &Models, cx: &mut TestAppContext) -> Navigator {
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
        Navigator {
            asks,
            _told: end.told,
        }
    }

    /// The next ask of the model, waited for with the app running.
    fn ask(&mut self, cx: &mut TestAppContext) -> Ask {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.asks.recv_timeout(Duration::from_millis(1)) {
                Ok(ask) => return ask,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the model let the port go, and an ask was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the model asked nothing");
        }
    }

    /// The next ask, which has to be this one.
    fn expect(&mut self, wanted: Ask, cx: &mut TestAppContext) {
        assert_eq!(self.ask(cx), wanted);
    }

    /// The first telling of a model: the workspaces, the one on screen and the answers on
    /// repositories, which are given.
    fn first_telling(
        &mut self,
        on_screen: char,
        cx: &mut TestAppContext,
    ) -> Vec<RepositoryConsent> {
        let Ask::Workspaces(roots) = self.ask(cx) else {
            panic!("the first thing told is the workspaces")
        };
        assert_eq!(roots.len(), 2);
        self.expect(Ask::OnScreen(Some(id(on_screen))), cx);
        let Ask::Consents(consents) = self.ask(cx) else {
            panic!("the answers on repositories come after the workspace on screen")
        };
        consents
    }
}

/// Puts the other workspace on screen: a barrier for the Navigator's asks, since the model
/// tells it as its next ask and after everything it told before.
fn move_on_screen(models: &Models, navigator: &mut Navigator, cx: &mut TestAppContext) {
    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));
    navigator.expect(Ask::OnScreen(Some(id('2'))), cx);
}

fn stop_after(seconds: u64) -> Ask {
    Ask::StopAfter(Duration::from_secs(seconds))
}

#[gpui_kit::test]
fn a_stop_time_told_while_connected_goes_to_the_navigator_at_each_change(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling('1', cx);

    keeper.tell(Fact::SettingChanged(seconds(90)));
    navigator.expect(stop_after(90), cx);
    keeper.tell(Fact::SettingChanged(seconds(120)));
    navigator.expect(stop_after(120), cx);
}

#[gpui_kit::test]
fn a_value_the_list_holds_already_tells_the_navigator_nothing(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling('1', cx);

    // The Window's own set takes at once, and the Keeper's echo of it comes after.
    models
        .settings
        .update(cx, |list, cx| list.set(seconds(30), cx));
    navigator.expect(stop_after(30), cx);
    keeper.tell(Fact::SettingChanged(seconds(30)));
    // Another kind of setting, then the same stop time again.
    keeper.tell(Fact::SettingChanged(Setting::Theme(ThemeChoice::System)));
    keeper.tell(Fact::SettingChanged(seconds(30)));
    keeper.tell(Fact::SettingChanged(Setting::KeepStates { days: 3 }));
    wait_until(cx, "the last fact told", |cx| {
        models
            .settings
            .read_with(cx, |list, _| list.keep_states() == 3)
    });

    // Everything above reached the model before the screen is moved, and its ask comes
    // after every ask the model made before it.
    move_on_screen(&models, &mut navigator, cx);
}

#[gpui_kit::test]
fn a_navigator_connected_with_no_stop_time_set_is_told_none(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), Vec::new(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    assert_eq!(navigator.first_telling('1', cx), Vec::new());

    move_on_screen(&models, &mut navigator, cx);
}

#[gpui_kit::test]
fn a_navigator_connected_with_a_stop_time_set_is_told_it_after_the_consents_each_time(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(
        vec![
            Setting::ServersInRepository {
                repository: repo('a'),
                allowed: true,
            },
            seconds(45),
            Setting::KeepStates { days: 3 },
        ],
        Vec::new(),
        cx,
    );
    let mut navigator = Navigator::connect(&models, cx);
    assert_eq!(
        navigator.first_telling('1', cx),
        [RepositoryConsent {
            repository: repo('a'),
            allowed: true
        }]
    );
    navigator.expect(stop_after(45), cx);
    move_on_screen(&models, &mut navigator, cx);

    // A Navigator that replaces it is owed the time as well, which it was not told.
    let mut next = Navigator::connect(&models, cx);
    next.first_telling('2', cx);
    next.expect(stop_after(45), cx);
}

#[gpui_kit::test]
fn a_stop_time_the_keeper_refuses_is_followed_by_the_time_it_keeps(cx: &mut TestAppContext) {
    let (models, keeper) = window(Vec::new(), Vec::new(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling('1', cx);
    keeper.refuse_settings(Some(Refusal::Failed {
        reason: "not kept".to_owned(),
    }));

    models
        .settings
        .update(cx, |list, cx| list.set(seconds(90), cx));
    navigator.expect(stop_after(90), cx);
    // The list is asked again, shows what the Keeper keeps, which is no stop time, and the
    // Navigator runs on the default again.
    navigator.expect(stop_after(600), cx);
}

// The chosen agent CLI.

#[gpui_kit::test]
fn the_chosen_cli_is_the_first_until_moved_then_the_next_and_wraps(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), vec![cli("claude"), cli("pi")], cx);
    let chosen = |cx: &mut TestAppContext| {
        models
            .agents
            .read_with(cx, |list, _| list.chosen_cli().cloned())
    };
    let next = |cx: &mut TestAppContext| {
        models
            .agents
            .update(cx, |list, cx| list.choose_next_cli(cx))
    };

    assert_eq!(chosen(cx), Some(cli("claude")));
    next(cx);
    assert_eq!(chosen(cx), Some(cli("pi")));
    next(cx);
    assert_eq!(chosen(cx), Some(cli("claude")), "after the last the first");
}

#[gpui_kit::test]
fn choosing_a_cli_tells_the_observers(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), vec![cli("claude"), cli("pi")], cx);
    let told = Rc::new(Cell::new(0));
    let _watching = cx.update(|cx| {
        let told = told.clone();
        cx.observe(&models.agents, move |_, _| told.set(told.get() + 1))
    });

    models
        .agents
        .update(cx, |list, cx| list.choose_next_cli(cx));
    cx.run_until_parked();
    assert_eq!(told.get(), 1);
}

#[gpui_kit::test]
fn no_cli_is_chosen_while_the_keeper_names_none_and_choosing_does_nothing(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), Vec::new(), cx);
    models
        .agents
        .update(cx, |list, cx| list.choose_next_cli(cx));
    models.agents.read_with(cx, |list, _| {
        assert_eq!(list.chosen_cli(), None);
    });
}

#[gpui_kit::test]
fn a_single_cli_stays_chosen(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), vec![cli("claude")], cx);
    models
        .agents
        .update(cx, |list, cx| list.choose_next_cli(cx));
    models.agents.read_with(cx, |list, _| {
        assert_eq!(list.chosen_cli(), Some(&cli("claude")));
    });
}

#[gpui_kit::test]
fn the_choice_is_kept_by_name_across_a_new_list_and_falls_back_to_the_first_without_it(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), vec![cli("claude"), cli("pi")], cx);
    models
        .agents
        .update(cx, |list, cx| list.choose_next_cli(cx));

    // The Keeper reached again names the CLIs in another order: the choice follows the name,
    // which is no longer the first.
    let link = models.link.clone();
    let mut again = KeeperEnd::attach_staged(hello(), &link, cx, |keeper| {
        keeper.set_clis(vec![cli("other"), cli("pi"), cli("claude")]);
    });
    again.list(&listed(), cx);
    wait_until(cx, "the CLIs in their new order", |cx| {
        models
            .agents
            .read_with(cx, |list, _| list.clis().first() == Some(&cli("other")))
    });
    models.agents.read_with(cx, |list, _| {
        assert_eq!(list.chosen_cli(), Some(&cli("pi")), "kept by name");
    });

    // Then one without it.
    let mut last = KeeperEnd::attach_staged(hello(), &link, cx, |keeper| {
        keeper.set_clis(vec![cli("claude"), cli("other")]);
    });
    last.list(&listed(), cx);
    wait_until(cx, "the CLIs without pi", |cx| {
        models
            .agents
            .read_with(cx, |list, _| list.clis().get(1) == Some(&cli("other")))
    });
    models.agents.read_with(cx, |list, _| {
        assert_eq!(list.chosen_cli(), Some(&cli("claude")), "the first");
    });
}

// The recorded states.

/// How many of these requests ask for the sizes or for a clean-up.
fn asked_for_sizes(requests: &[Request]) -> usize {
    requests
        .iter()
        .filter(|request| matches!(request, Request::GetStateSizes | Request::CleanUpStates))
        .count()
}

fn states_sizes(models: &Models, cx: &mut TestAppContext) -> Option<StateSizes> {
    models
        .states
        .read_with(cx, |states, _| states.sizes().cloned())
}

fn problem(models: &Models, cx: &mut TestAppContext) -> Option<String> {
    models
        .states
        .read_with(cx, |states, _| states.problem().map(str::to_owned))
}

fn cleaning(models: &Models, cx: &mut TestAppContext) -> bool {
    models.states.read_with(cx, |states, _| states.cleaning())
}

#[gpui_kit::test]
fn the_states_ask_nothing_by_themselves_and_a_deletion_before_the_first_question_asks_nothing(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    keeper.tell(Fact::StatesDeleted { workspace: id('1') });

    // A fact told after it, which the Window takes after the deletion, since a Keeper's facts
    // come in the order they were told: once it is taken, the deletion was.
    keeper.tell(Fact::SettingChanged(Setting::KeepStates { days: 3 }));
    wait_until(cx, "the fact told after the deletion", |cx| {
        models
            .settings
            .read_with(cx, |list, _| list.keep_states() == 3)
    });
    let before = barrier(&models, &mut keeper, cx);
    assert_eq!(asked_for_sizes(&before), 0);
    assert_eq!(states_sizes(&models, cx), None);
    assert!(!cleaning(&models, cx));
    assert_eq!(problem(&models, cx), None);
}

#[gpui_kit::test]
fn a_refresh_asks_for_the_sizes_and_takes_the_answer(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let held = sizes(&[('1', 4096), ('2', 8192)], 20480);

    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    assert_eq!(states_sizes(&models, cx), None, "nothing before the answer");
    keeper.reply(asked, Ok(Reply::StateSizes(held.clone())));
    wait_until(cx, "the sizes", |cx| states_sizes(&models, cx).is_some());
    assert_eq!(states_sizes(&models, cx), Some(held));
}

#[gpui_kit::test]
fn of_two_questions_for_the_sizes_only_the_answer_to_the_last_counts(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let (old, new) = (sizes(&[('1', 4096)], 4096), sizes(&[('1', 0)], 0));

    models.states.update(cx, |states, cx| states.refresh(cx));
    let first = keeper.expect(Request::GetStateSizes, cx);
    models.states.update(cx, |states, cx| states.refresh(cx));
    let second = keeper.expect(Request::GetStateSizes, cx);

    keeper.reply(second, Ok(Reply::StateSizes(new.clone())));
    wait_until(cx, "the later answer", |cx| {
        states_sizes(&models, cx).is_some()
    });
    // A problem the older answer would leave is read after a barrier: the Keeper's answer
    // to it follows the older answer, on the one connection.
    keeper.reply(first, Ok(Reply::StateSizes(old)));
    barrier(&models, &mut keeper, cx);
    assert_eq!(states_sizes(&models, cx), Some(new));
}

#[gpui_kit::test]
fn a_deletion_after_a_question_asks_for_the_sizes_again(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    keeper.reply(asked, Ok(Reply::StateSizes(sizes(&[('1', 4096)], 4096))));
    wait_until(cx, "the first sizes", |cx| {
        states_sizes(&models, cx).is_some()
    });

    keeper.tell(Fact::StatesDeleted { workspace: id('1') });
    let again = keeper.expect(Request::GetStateSizes, cx);
    let smaller = sizes(&[('1', 0)], 0);
    keeper.reply(again, Ok(Reply::StateSizes(smaller.clone())));
    wait_until(cx, "the smaller sizes", |cx| {
        states_sizes(&models, cx) == Some(smaller.clone())
    });
}

#[gpui_kit::test]
fn a_clean_up_is_sent_while_it_waits_it_is_cleaning_and_its_sizes_are_taken(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    assert!(!cleaning(&models, cx));

    models.states.update(cx, |states, cx| states.clean_up(cx));
    assert!(
        cleaning(&models, cx),
        "cleaning from the moment it is asked"
    );
    let asked = keeper.expect(Request::CleanUpStates, cx);
    let after = sizes(&[('1', 0)], 0);
    keeper.reply(asked, Ok(Reply::StateSizes(after.clone())));
    wait_until(cx, "the clean-up answered", |cx| !cleaning(&models, cx));
    assert_eq!(states_sizes(&models, cx), Some(after));
    assert_eq!(problem(&models, cx), None);

    // The sizes it left are kept in step with what is deleted next.
    keeper.tell(Fact::StatesDeleted { workspace: id('1') });
    keeper.expect(Request::GetStateSizes, cx);
}

#[gpui_kit::test]
fn a_second_clean_up_while_one_waits_sends_nothing(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    models.states.update(cx, |states, cx| states.clean_up(cx));
    models.states.update(cx, |states, cx| states.clean_up(cx));

    let before = barrier(&models, &mut keeper, cx);
    assert_eq!(
        before
            .iter()
            .filter(|request| **request == Request::CleanUpStates)
            .count(),
        1
    );
}

#[gpui_kit::test]
fn a_refusal_keeps_the_sizes_and_gives_its_sentence_until_the_next_answer(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    let held = sizes(&[('1', 4096)], 4096);
    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    keeper.reply(asked, Ok(Reply::StateSizes(held.clone())));
    wait_until(cx, "the sizes", |cx| states_sizes(&models, cx).is_some());

    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the states cannot be read".to_owned(),
        }),
    );
    wait_until(cx, "the sentence", |cx| problem(&models, cx).is_some());
    assert_eq!(
        problem(&models, cx).as_deref(),
        Some("the states cannot be read")
    );
    assert_eq!(states_sizes(&models, cx), Some(held), "the sizes kept");

    // A clean-up that is refused ends its wait and says why.
    let refusal = Refusal::UnknownRevision {
        revision: "x".to_owned(),
    };
    models.states.update(cx, |states, cx| states.clean_up(cx));
    let asked = keeper.expect(Request::CleanUpStates, cx);
    keeper.reply(asked, Err(refusal.clone()));
    wait_until(cx, "the clean-up's end", |cx| !cleaning(&models, cx));
    assert_eq!(problem(&models, cx), Some(refusal.to_string()));

    // The next answer takes the sentence away.
    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    keeper.reply(asked, Ok(Reply::StateSizes(sizes(&[], 0))));
    wait_until(cx, "no sentence", |cx| problem(&models, cx).is_none());
}

#[gpui_kit::test]
fn an_answer_of_another_kind_is_a_problem_and_keeps_the_sizes(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    keeper.reply(asked, Ok(Reply::Done));

    wait_until(cx, "the sentence", |cx| problem(&models, cx).is_some());
    assert_eq!(problem(&models, cx), Some(AskError::Unexpected.to_string()));
    assert_eq!(states_sizes(&models, cx), None);
}

#[gpui_kit::test]
fn a_deletion_told_while_a_clean_up_waits_has_the_later_question_count(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    models.states.update(cx, |states, cx| states.refresh(cx));
    let asked = keeper.expect(Request::GetStateSizes, cx);
    keeper.reply(asked, Ok(Reply::StateSizes(sizes(&[('1', 4096)], 4096))));
    wait_until(cx, "the sizes", |cx| states_sizes(&models, cx).is_some());

    models.states.update(cx, |states, cx| states.clean_up(cx));
    let cleaned = keeper.expect(Request::CleanUpStates, cx);
    keeper.tell(Fact::StatesDeleted { workspace: id('1') });
    let asked_again = keeper.expect(Request::GetStateSizes, cx);

    // The clean-up's answer comes after the question asked later: it ends the wait, and
    // its sizes are not the last question's.
    let stale = sizes(&[('1', 1)], 1);
    keeper.reply(cleaned, Ok(Reply::StateSizes(stale.clone())));
    wait_until(cx, "the clean-up's end", |cx| !cleaning(&models, cx));
    assert_ne!(states_sizes(&models, cx), Some(stale));
    let last = sizes(&[('1', 0)], 0);
    keeper.reply(asked_again, Ok(Reply::StateSizes(last.clone())));
    wait_until(cx, "the last answer", |cx| {
        states_sizes(&models, cx) == Some(last.clone())
    });
}

// A workspace put on screen.

fn on_screen(models: &Models, cx: &mut TestAppContext) -> Option<WorkspaceId> {
    models
        .workspaces
        .read_with(cx, |list, _| list.selected().cloned())
}

#[gpui_kit::test]
fn a_workspace_the_list_holds_is_put_on_screen_at_once(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), Vec::new(), cx);
    assert_eq!(on_screen(&models, cx), Some(id('1')));

    models
        .workspaces
        .update(cx, |list, cx| list.want_on_screen(id('2'), cx));
    assert_eq!(on_screen(&models, cx), Some(id('2')));
}

#[gpui_kit::test]
fn a_workspace_the_list_does_not_hold_yet_comes_on_screen_with_the_list_that_holds_it(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    models
        .workspaces
        .update(cx, |list, cx| list.want_on_screen(id('3'), cx));
    assert_eq!(
        on_screen(&models, cx),
        Some(id('1')),
        "not a workspace the list lacks"
    );

    let after = vec![repository(
        'a',
        vec![
            workspace('1', "main"),
            workspace('2', "feature"),
            workspace('3', "made"),
        ],
    )];
    keeper.tell(Fact::Workspaces(after.clone()));
    wait_until(cx, "the new workspace on screen", |cx| {
        on_screen(&models, cx) == Some(id('3'))
    });
    models.workspaces.read_with(cx, |list, _| {
        assert_eq!(list.repositories(), after);
    });
}

#[gpui_kit::test]
fn what_the_user_selects_after_outranks_a_workspace_wanted_on_screen(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), Vec::new(), cx);
    models
        .workspaces
        .update(cx, |list, cx| list.want_on_screen(id('3'), cx));
    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));

    keeper.tell(Fact::Workspaces(vec![repository(
        'a',
        vec![
            workspace('1', "main"),
            workspace('2', "feature"),
            workspace('3', "made"),
        ],
    )]));
    wait_until(cx, "the new list", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.workspace(&id('3')).is_some())
    });
    assert_eq!(on_screen(&models, cx), Some(id('2')));
}
