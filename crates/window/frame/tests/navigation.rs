//! Component test: the shared model of code intelligence, through what the panes use of
//! it. The test plays the Keeper on a socket pair and the Navigator at the other end of the
//! model's port, with data: it reads the asks the model really makes and tells it what a
//! Navigator would tell, and computes nothing a Navigator computes.
//!
//! A negative clause ("tells nothing", "sends no cancel") is read after a barrier: the ask
//! the test makes next through the same port, which arrives after everything the model
//! sent before it. The model's asks all leave through one channel, in the order they are
//! made.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::{AppContext, Task, TestAppContext};
use yaar_frame::{AskError, Asking, KeeperLink, LinkStatus, Models, NavError};
use yaar_link::{
    Base, BaseSource, ChangedFile, Changes, Comparison, Fact, FileStatus, FlowForm, Hello,
    PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId, Request, RequestId, Setting,
    WorkspaceId,
};
use yaar_navigator::{
    Answered, Ask, AskId, ExportAsk, FileKind, Language, Location, OpenedFile, Place,
    RepositoryConsent, ServerState, State, Told, Tree, WorkspaceRoot, port,
};

use common::{DEADLINE, KeeperEnd, id, repository, wait_until, workspace};

/// The full id of a commit, as the Link carries it.
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

fn allowed(letter: char, allowed: bool) -> Setting {
    Setting::ServersInRepository {
        repository: repo(letter),
        allowed,
    }
}

fn consent(letter: char, allowed: bool) -> RepositoryConsent {
    RepositoryConsent {
        repository: repo(letter),
        allowed,
    }
}

fn file(path: &str) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status: FileStatus::Modified,
    }
}

fn changes(workspace: char, files: &[&str]) -> Changes {
    Changes {
        workspace: id(workspace),
        comparison: Comparison::SinceBase {
            base: Base {
                name: "main".to_owned(),
                source: BaseSource::Default,
            },
        },
        counted_from: Some(COMMIT.to_owned()),
        commits: Vec::new(),
        files: files.iter().map(|path| file(path)).collect(),
    }
}

fn get_changes(workspace: char) -> Request {
    Request::GetChanges {
        workspace: id(workspace),
    }
}

fn place(path: &str, name: &str) -> Place {
    Place {
        workspace: id('1'),
        tree: Tree::Worktree,
        path: PathBuf::from(path),
        line: 3,
        column: 4,
        name: name.to_owned(),
        shown: vec![PathBuf::from("src/a.rs")],
    }
}

fn location(path: &str, line: u32) -> Location {
    Location {
        path: PathBuf::from(path),
        line,
        column: 0,
        text: format!("text of line {line}"),
    }
}

fn language(name: &str, title: &str, extensions: &[&str]) -> Language {
    Language {
        name: name.to_owned(),
        title: title.to_owned(),
        extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
    }
}

fn server(workspace: char, language: &str, old_side: Option<&str>, state: State) -> ServerState {
    ServerState {
        workspace: id(workspace),
        language: language.to_owned(),
        title: language.to_owned(),
        old_side: old_side.map(str::to_owned),
        registered: false,
        state,
        server: None,
        program: None,
        pid: None,
        update: None,
    }
}

fn hovered(text: &str, guessed: bool) -> Answered<Option<String>> {
    Answered {
        value: Some(text.to_owned()),
        guessed,
    }
}

/// The Navigator, played by the test at the other end of the model's port.
struct Navigator {
    /// The asks, moved by a thread of this end's own from the port onto a channel the test
    /// waits on while the app runs.
    asks: Receiver<Ask>,
    told: async_channel::Sender<Told>,
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
            told: end.told,
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

    /// The next ask, which has to be this one. A fact that nothing was told in between is
    /// read this way, with the expected ask as the barrier.
    fn expect(&mut self, wanted: Ask, cx: &mut TestAppContext) {
        assert_eq!(self.ask(cx), wanted);
    }

    /// Nothing reaches this Navigator any more: the model let its port go, and everything
    /// it had sent was read first, so an ask that was still on its way would be read here.
    /// This is the barrier of a negative clause about a Navigator that is replaced.
    fn ends_with_nothing_more(&mut self, cx: &mut TestAppContext) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.asks.recv_timeout(Duration::from_millis(1)) {
                Ok(ask) => panic!("an ask reached a Navigator that was replaced: {ask:?}"),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(
                Instant::now() < deadline,
                "the model still holds the port of the Navigator it replaced"
            );
        }
    }

    /// The first telling: the workspaces, the one on screen and the answers on
    /// repositories, which the test reads whatever they hold.
    fn first_telling(
        &mut self,
        cx: &mut TestAppContext,
    ) -> (
        Vec<WorkspaceRoot>,
        Option<WorkspaceId>,
        Vec<RepositoryConsent>,
    ) {
        let Ask::Workspaces(roots) = self.ask(cx) else {
            panic!("the first thing told is the workspaces")
        };
        let Ask::OnScreen(on_screen) = self.ask(cx) else {
            panic!("the workspace on screen comes after the workspaces")
        };
        let Ask::Consents(consents) = self.ask(cx) else {
            panic!("the answers on repositories come after the workspace on screen")
        };
        (roots, on_screen, consents)
    }

    /// Tells the model `told`, as the Navigator would, and lets the app take it.
    fn tell(&self, told: Told, cx: &mut TestAppContext) {
        self.told
            .try_send(told)
            .expect("the model reads what the Navigator tells");
        cx.run_until_parked();
    }

    /// The Navigator ends: nothing is told any more.
    fn end(self, cx: &mut TestAppContext) {
        drop(self.told);
        cx.run_until_parked();
    }
}

/// What a pane keeps of an ask while it waits: where its answer will be, and the task that
/// holds the ask (dropping the task is what a pane does when the reviewer moves on).
type Slot<T> = Rc<RefCell<Option<Result<T, NavError>>>>;

fn pending<T: Unpin + 'static>(asking: Asking<T>, cx: &mut TestAppContext) -> (Slot<T>, Task<()>) {
    let slot = Rc::new(RefCell::new(None));
    let task = cx.foreground_executor().spawn({
        let slot = slot.clone();
        async move {
            let answer = asking.await;
            *slot.borrow_mut() = Some(answer);
        }
    });
    (slot, task)
}

fn resolved<T: 'static>(slot: &Slot<T>, cx: &mut TestAppContext) -> Result<T, NavError> {
    wait_until(cx, "the ask to resolve", |_| slot.borrow().is_some());
    slot.take().expect("the answer")
}

fn hover(
    models: &Models,
    place: Place,
    cx: &mut TestAppContext,
) -> Asking<Answered<Option<String>>> {
    models
        .navigation
        .update(cx, |navigation, cx| navigation.hover(place, cx))
}

/// The models of a window whose Keeper the test plays: `settings` are what the Keeper holds
/// when the Window reaches it, `listed` the answer to the first question for the list. Every
/// model has taken its answer, and no Navigator is connected yet.
fn window(
    settings: Vec<Setting>,
    listed: &[Repository],
    cx: &mut TestAppContext,
) -> (Models, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let wanted = settings.clone();
    let mut keeper = KeeperEnd::attach_staged(hello(), &link, cx, move |keeper| {
        keeper.set_settings(settings);
    });
    keeper.list(listed, cx);
    wait_until(cx, "the list and the settings", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.repositories() == listed)
            && models
                .settings
                .read_with(cx, |list, _| list.all() == wanted)
    });
    (models, keeper)
}

/// One repository with its main worktree, on screen.
fn one_workspace() -> Vec<Repository> {
    vec![repository('a', vec![workspace('1', "main")])]
}

/// The next request of the Window that is this kind of request, the others left
/// unanswered.
fn request_of(
    keeper: &mut KeeperEnd,
    cx: &mut TestAppContext,
    wanted: impl Fn(&Request) -> bool,
) -> (RequestId, Request) {
    loop {
        let (asked, request) = keeper.request(cx);
        if wanted(&request) {
            return (asked, request);
        }
    }
}

#[gpui_kit::test]
fn connected_the_model_tells_the_workspaces_the_one_on_screen_and_the_answers_on_repositories(
    cx: &mut TestAppContext,
) {
    let mut gone = workspace('2', "removed");
    gone.head = None;
    let listed = vec![
        repository('a', vec![workspace('1', "main")]),
        repository('b', vec![gone]),
    ];
    let (models, _keeper) = window(vec![allowed('a', true), allowed('b', false)], &listed, cx);
    let mut navigator = Navigator::connect(&models, cx);

    let (roots, on_screen, consents) = navigator.first_telling(cx);
    assert_eq!(
        roots,
        vec![
            WorkspaceRoot {
                id: id('1'),
                repository: repo('a'),
                folder: Some(PathBuf::from("/gits/main")),
            },
            WorkspaceRoot {
                id: id('2'),
                repository: repo('b'),
                folder: None,
            },
        ],
        "a worktree that is gone has no folder"
    );
    assert_eq!(on_screen, Some(id('1')));
    assert_eq!(consents, vec![consent('a', true), consent('b', false)]);
}

#[gpui_kit::test]
fn connected_with_nothing_listed_the_model_still_tells_the_navigator_so(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &[], cx);
    let mut navigator = Navigator::connect(&models, cx);

    assert_eq!(navigator.first_telling(cx), (Vec::new(), None, Vec::new()));
}

#[gpui_kit::test]
fn another_workspace_coming_on_screen_is_told_and_a_list_that_keeps_it_there_does_not_tell_it_again(
    cx: &mut TestAppContext,
) {
    let listed = vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )];
    let (models, mut keeper) = window(Vec::new(), &listed, cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));
    navigator.expect(Ask::OnScreen(Some(id('2'))), cx);

    // A workspace comes and the one on screen stays: the list is told whole and the
    // workspace on screen is not, so the next ask is the barrier.
    let after = vec![repository(
        'a',
        vec![
            workspace('1', "main"),
            workspace('2', "feature"),
            workspace('3', "third"),
        ],
    )];
    keeper.tell(Fact::Workspaces(after));
    navigator.expect(
        Ask::Workspaces(
            ["main", "feature", "third"]
                .iter()
                .zip(['1', '2', '3'])
                .map(|(branch, digit)| WorkspaceRoot {
                    id: id(digit),
                    repository: repo('a'),
                    folder: Some(PathBuf::from(format!("/gits/{branch}"))),
                })
                .collect(),
        ),
        cx,
    );
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("rust"));
    navigator.expect(
        Ask::Download {
            language: "rust".to_owned(),
        },
        cx,
    );
}

#[gpui_kit::test]
fn a_new_list_of_workspaces_is_told_whole_and_a_workspace_that_left_is_told_by_its_absence(
    cx: &mut TestAppContext,
) {
    let listed = vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )];
    let (models, mut keeper) = window(Vec::new(), &listed, cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));
    navigator.expect(Ask::OnScreen(Some(id('2'))), cx);

    let after = vec![repository('a', vec![workspace('1', "main")])];
    keeper.tell(Fact::Workspaces(after));
    navigator.expect(
        Ask::Workspaces(vec![WorkspaceRoot {
            id: id('1'),
            repository: repo('a'),
            folder: Some(PathBuf::from("/gits/main")),
        }]),
        cx,
    );
    // The workspace on screen left the list; the first of it took its place.
    navigator.expect(Ask::OnScreen(Some(id('1'))), cx);
}

#[gpui_kit::test]
fn an_answer_on_a_repository_is_told_whole_and_another_setting_is_not(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(vec![allowed('a', true)], &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    keeper.tell(Fact::SettingChanged(Setting::FlowForm {
        workspace: id('1'),
        form: FlowForm::LiveDiff,
    }));
    keeper.tell(Fact::SettingChanged(allowed('b', false)));
    // The setting that is no answer on a repository was heard first: had it told anything,
    // that would come before this.
    navigator.expect(
        Ask::Consents(vec![consent('a', true), consent('b', false)]),
        cx,
    );
    keeper.tell(Fact::SettingChanged(allowed('a', false)));
    navigator.expect(
        Ask::Consents(vec![consent('a', false), consent('b', false)]),
        cx,
    );
}

#[gpui_kit::test]
fn the_changed_files_are_told_each_time_the_keeper_answers_with_those_that_left_the_list(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    let told = |paths: &[&str]| Ask::FilesChanged {
        workspace: id('1'),
        paths: paths.iter().map(PathBuf::from).collect(),
    };

    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &["a.rs", "b.rs"]))));
    navigator.expect(told(&["a.rs", "b.rs"]), cx);

    // While the answer is awaited the model shows the last list again, which is no news:
    // the barrier comes first.
    keeper.tell(Fact::ChangesChanged { workspace: id('1') });
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    models
        .navigation
        .update(cx, |navigation, _| navigation.update("rust"));
    navigator.expect(
        Ask::Update {
            language: "rust".to_owned(),
        },
        cx,
    );
    // `a.rs` was put back: it left the list, and the Navigator hears that it changed.
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &["b.rs"]))));
    navigator.expect(told(&["a.rs", "b.rs"]), cx);

    // The same list again is told: `b.rs` may have changed again with its path in it.
    models.changes.update(cx, |changes, cx| changes.refresh(cx));
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &["b.rs"]))));
    navigator.expect(told(&["b.rs"]), cx);

    // A refused question leaves the model showing the last list again, which is no news:
    // the barrier comes first.
    models.changes.update(cx, |changes, cx| changes.refresh(cx));
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the repository is busy".to_owned(),
        }),
    );
    wait_until(cx, "the refusal to be taken", |cx| {
        models.changes.read_with(cx, |changes, _| {
            !changes.is_loading() && changes.refusal().is_some()
        })
    });
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("go"));
    navigator.expect(
        Ask::Download {
            language: "go".to_owned(),
        },
        cx,
    );

    // Nothing changed and nothing is listed: no paths, nothing told. The barrier follows.
    models.changes.update(cx, |changes, cx| changes.refresh(cx));
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &[]))));
    navigator.expect(told(&["b.rs"]), cx);
    models.changes.update(cx, |changes, cx| changes.refresh(cx));
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &[]))));
    wait_until(cx, "the answer to be taken", |cx| {
        models
            .changes
            .read_with(cx, |changes, _| !changes.is_loading())
    });
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("python"));
    navigator.expect(
        Ask::Download {
            language: "python".to_owned(),
        },
        cx,
    );
}

/// The changes of workspace 1 asked for again and answered as `list`, and waited for until
/// the model has taken the answer.
fn answer_again(models: &Models, keeper: &mut KeeperEnd, list: Changes, cx: &mut TestAppContext) {
    models.changes.update(cx, |changes, cx| changes.refresh(cx));
    let (asked, _) = request_of(keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(list)));
    wait_until(cx, "the answer to be taken", |cx| {
        models
            .changes
            .read_with(cx, |changes, _| !changes.is_loading())
    });
}

#[gpui_kit::test]
fn a_connect_tells_no_changed_files_though_the_keeper_answered_before_it_and_holds_that_answer_as_the_last(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), &one_workspace(), cx);
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &["a.rs"]))));
    wait_until(cx, "the answer to be taken", |cx| {
        models
            .changes
            .read_with(cx, |changes, _| changes.changes().is_some())
    });

    // A server starts from the files on disk, so a Navigator that connects now has no stale
    // state to be told of: the three first tellings come, and the Download after them is the
    // barrier for a `FilesChanged` that was on its way.
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("go"));
    navigator.expect(
        Ask::Download {
            language: "go".to_owned(),
        },
        cx,
    );

    // But the list the model held is the last answer: `a.rs` was put back since, and the
    // server that started from the disk may have read it as it was a moment before.
    answer_again(&models, &mut keeper, changes('1', &[]), cx);
    models
        .navigation
        .update(cx, |navigation, _| navigation.update("go"));
    navigator.expect(
        Ask::FilesChanged {
            workspace: id('1'),
            paths: vec![PathBuf::from("a.rs")],
        },
        cx,
    );
    navigator.expect(
        Ask::Update {
            language: "go".to_owned(),
        },
        cx,
    );
}

#[gpui_kit::test]
fn the_old_path_of_a_rename_the_keeper_answered_before_a_connect_is_held_as_the_last_answer_too(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut renamed = changes('1', &[]);
    renamed.files.push(ChangedFile {
        path: PathBuf::from("new.rs"),
        status: FileStatus::Renamed {
            from: PathBuf::from("old.rs"),
        },
    });
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(renamed)));
    wait_until(cx, "the answer to be taken", |cx| {
        models
            .changes
            .read_with(cx, |changes, _| changes.changes().is_some())
    });
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    answer_again(&models, &mut keeper, changes('1', &[]), cx);
    models
        .navigation
        .update(cx, |navigation, _| navigation.update("go"));
    navigator.expect(
        Ask::FilesChanged {
            workspace: id('1'),
            paths: vec![PathBuf::from("new.rs"), PathBuf::from("old.rs")],
        },
        cx,
    );
    navigator.expect(
        Ask::Update {
            language: "go".to_owned(),
        },
        cx,
    );
}

#[gpui_kit::test]
fn a_list_the_model_still_shows_after_a_refusal_is_held_as_the_last_answer_at_a_connect(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), &one_workspace(), cx);
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(changes('1', &["a.rs"]))));
    // The answer is waited for: a new question drops one the model has not taken yet, and
    // the model would then hold no list when the refusal comes.
    wait_until(cx, "the answer to be taken", |cx| {
        models.changes.read_with(cx, |changes, _| {
            changes.changes().is_some() && !changes.is_loading()
        })
    });
    models.changes.update(cx, |changes, cx| changes.refresh(cx));
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the repository is busy".to_owned(),
        }),
    );
    wait_until(cx, "the refusal to be taken", |cx| {
        models.changes.read_with(cx, |changes, _| {
            !changes.is_loading() && changes.refusal().is_some()
        })
    });
    // The model still shows the list it held, which a connect takes as the last answer of the
    // Keeper: a refusal brought no newer one.
    models.changes.read_with(cx, |changes, _| {
        let shown = changes.changes().expect("the list is still shown");
        assert_eq!(
            shown
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("a.rs")]
        );
    });
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    // The list was seeded, so `a.rs`, put back since, is told changed when the Keeper
    // answers an empty list; the update after it is the barrier.
    answer_again(&models, &mut keeper, changes('1', &[]), cx);
    models
        .navigation
        .update(cx, |navigation, _| navigation.update("go"));
    navigator.expect(
        Ask::FilesChanged {
            workspace: id('1'),
            paths: vec![PathBuf::from("a.rs")],
        },
        cx,
    );
    navigator.expect(
        Ask::Update {
            language: "go".to_owned(),
        },
        cx,
    );
}

#[gpui_kit::test]
fn a_renamed_file_is_told_changed_at_both_of_its_paths(cx: &mut TestAppContext) {
    let (models, mut keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    // The old path is gone from the disk, and a server still holds its symbols.
    let mut renamed = changes('1', &["kept.rs"]);
    renamed.files.push(ChangedFile {
        path: PathBuf::from("new.rs"),
        status: FileStatus::Renamed {
            from: PathBuf::from("old.rs"),
        },
    });
    let (asked, _) = request_of(&mut keeper, cx, |r| *r == get_changes('1'));
    keeper.reply(asked, Ok(Reply::Changes(renamed)));
    navigator.expect(
        Ask::FilesChanged {
            workspace: id('1'),
            paths: ["kept.rs", "new.rs", "old.rs"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        },
        cx,
    );
}

#[gpui_kit::test]
fn a_hover_is_asked_with_a_fresh_id_and_resolves_with_the_answer_of_that_id_only(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    let first = hover(&models, place("src/a.rs", "alpha"), cx);
    let second = hover(&models, place("src/b.rs", "beta"), cx);
    let (first_id, second_id) = (first.id(), second.id());
    assert_ne!(first_id, second_id);
    assert_eq!(
        navigator.ask(cx),
        Ask::Hover {
            id: first_id,
            place: place("src/a.rs", "alpha")
        }
    );
    assert_eq!(
        navigator.ask(cx),
        Ask::Hover {
            id: second_id,
            place: place("src/b.rs", "beta")
        }
    );
    let (first, _first_task) = pending(first, cx);
    let (second, _second_task) = pending(second, cx);

    // An answer of an id nobody asked with resolves neither; the answers of the two that
    // were asked come crossed, and each resolves its own.
    navigator.tell(
        Told::Hover {
            id: AskId(second_id.0 + 100),
            answer: hovered("nobody's", false),
        },
        cx,
    );
    assert!(first.borrow().is_none() && second.borrow().is_none());
    navigator.tell(
        Told::Hover {
            id: second_id,
            answer: hovered("beta: fn", true),
        },
        cx,
    );
    navigator.tell(
        Told::Hover {
            id: first_id,
            answer: hovered("alpha: fn", false),
        },
        cx,
    );
    assert_eq!(
        resolved(&first, cx),
        Ok(hovered("alpha: fn", false)),
        "the answer of the first ask"
    );
    assert_eq!(
        resolved(&second, cx),
        Ok(hovered("beta: fn", true)),
        "the answer of the second ask, whose mark that it may be wrong comes with it"
    );
}

#[gpui_kit::test]
fn an_answer_of_another_kind_than_the_ask_takes_is_no_answer_to_it(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    let asking = hover(&models, place("src/a.rs", "alpha"), cx);
    let asked = asking.id();
    navigator.ask(cx);
    let (slot, _task) = pending(asking, cx);
    navigator.tell(
        Told::Definition {
            id: asked,
            answer: Answered {
                value: vec![location("/w/src/a.rs", 1)],
                guessed: false,
            },
        },
        cx,
    );
    assert!(slot.borrow().is_none());
    navigator.tell(
        Told::Hover {
            id: asked,
            answer: hovered("alpha", false),
        },
        cx,
    );
    assert_eq!(resolved(&slot, cx), Ok(hovered("alpha", false)));
}

#[gpui_kit::test]
fn definitions_references_and_opened_files_resolve_with_what_the_navigator_told(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    let definition = models.navigation.update(cx, |navigation, cx| {
        navigation.definition(place("src/a.rs", "alpha"), cx)
    });
    let references = models.navigation.update(cx, |navigation, cx| {
        navigation.references(place("src/a.rs", "alpha"), cx)
    });
    let opened = models.navigation.update(cx, |navigation, cx| {
        navigation.open(location("/w/src/a.rs", 7), cx)
    });
    let refused = models.navigation.update(cx, |navigation, cx| {
        navigation.open(location("/w/image.png", 1), cx)
    });
    let ids = [definition.id(), references.id(), opened.id(), refused.id()];
    assert_eq!(
        navigator.ask(cx),
        Ask::Definition {
            id: ids[0],
            place: place("src/a.rs", "alpha")
        }
    );
    assert_eq!(
        navigator.ask(cx),
        Ask::References {
            id: ids[1],
            place: place("src/a.rs", "alpha")
        }
    );
    assert_eq!(
        navigator.ask(cx),
        Ask::Open {
            id: ids[2],
            location: location("/w/src/a.rs", 7)
        }
    );
    assert_eq!(
        navigator.ask(cx),
        Ask::Open {
            id: ids[3],
            location: location("/w/image.png", 1)
        }
    );
    let (definition, _a) = pending(definition, cx);
    let (references, _b) = pending(references, cx);
    let (opened, _c) = pending(opened, cx);
    let (refused, _d) = pending(refused, cx);

    let file = OpenedFile {
        path: PathBuf::from("/w/src/a.rs"),
        kind: FileKind::Worktree {
            workspace: id('1'),
            relative: PathBuf::from("src/a.rs"),
        },
        text: "fn alpha() {}\n".to_owned(),
    };
    navigator.tell(
        Told::Opened {
            id: ids[3],
            file: Err("not valid text".to_owned()),
        },
        cx,
    );
    navigator.tell(
        Told::Opened {
            id: ids[2],
            file: Ok(file.clone()),
        },
        cx,
    );
    // References come before definitions in what is told, whichever way they were asked.
    let places = vec![location("/w/src/a.rs", 1), location("/w/src/b.rs", 9)];
    navigator.tell(
        Told::References {
            id: ids[1],
            answer: Answered {
                value: places.clone(),
                guessed: true,
            },
        },
        cx,
    );
    navigator.tell(
        Told::Definition {
            id: ids[0],
            answer: Answered {
                value: vec![places[0].clone()],
                guessed: false,
            },
        },
        cx,
    );
    assert_eq!(
        resolved(&definition, cx),
        Ok(Answered {
            value: vec![places[0].clone()],
            guessed: false
        })
    );
    assert_eq!(
        resolved(&references, cx),
        Ok(Answered {
            value: places,
            guessed: true
        })
    );
    assert_eq!(resolved(&opened, cx), Ok(Ok(file)));
    assert_eq!(
        resolved(&refused, cx),
        Ok(Err("not valid text".to_owned())),
        "the Navigator's reason reaches the caller as it was told"
    );
}

#[gpui_kit::test]
fn an_ask_dropped_before_its_answer_cancels_it_and_one_answered_does_not(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    // A pane that moves on drops the task that holds its ask.
    let moved_on = hover(&models, place("src/a.rs", "alpha"), cx);
    let moved_on_id = moved_on.id();
    let answered = hover(&models, place("src/b.rs", "beta"), cx);
    let answered_id = answered.id();
    navigator.ask(cx);
    navigator.ask(cx);
    let (moved_on, moved_on_task) = pending(moved_on, cx);
    let (answered, answered_task) = pending(answered, cx);

    navigator.tell(
        Told::Hover {
            id: answered_id,
            answer: hovered("beta", false),
        },
        cx,
    );
    assert_eq!(resolved(&answered, cx), Ok(hovered("beta", false)));
    drop(answered_task);
    drop(moved_on_task);
    cx.run_until_parked();
    assert!(moved_on.borrow().is_none());
    // The barrier follows both drops: a cancel of the answered ask would come before it.
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("rust"));
    assert_eq!(
        navigator.ask(cx),
        Ask::Cancel { id: moved_on_id },
        "the ask the reviewer moved on from"
    );
    navigator.expect(
        Ask::Download {
            language: "rust".to_owned(),
        },
        cx,
    );

    // An answer that comes for an ask cancelled is read and goes nowhere.
    navigator.tell(
        Told::Hover {
            id: moved_on_id,
            answer: hovered("late", false),
        },
        cx,
    );
}

#[gpui_kit::test]
fn an_ask_that_is_dropped_unpolled_still_cancels(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    let asking = hover(&models, place("src/a.rs", "alpha"), cx);
    let asked = asking.id();
    navigator.ask(cx);
    drop(asking);
    assert_eq!(navigator.ask(cx), Ask::Cancel { id: asked });
}

#[gpui_kit::test]
fn the_servers_and_the_languages_the_navigator_tells_replace_the_last_and_notify(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let navigator = Navigator::connect(&models, cx);
    let notified = Rc::new(RefCell::new(0_usize));
    let _watching = cx.update(|cx| {
        let notified = notified.clone();
        cx.observe(&models.navigation, move |_, _| *notified.borrow_mut() += 1)
    });
    let read = |cx: &mut TestAppContext| {
        models.navigation.read_with(cx, |navigation, _| {
            (
                navigation.languages().to_vec(),
                navigation.servers().to_vec(),
            )
        })
    };
    assert_eq!(read(cx), (Vec::new(), Vec::new()));

    let languages = vec![language("rust", "Rust", &["rs"])];
    navigator.tell(Told::Languages(languages.clone()), cx);
    assert_eq!(read(cx), (languages.clone(), Vec::new()));
    let seen = *notified.borrow();
    assert!(seen > 0, "a change of the languages notifies");

    let starting = vec![server('1', "rust", None, State::Starting)];
    navigator.tell(Told::Servers(starting.clone()), cx);
    assert_eq!(read(cx), (languages.clone(), starting));
    let after_first = *notified.borrow();
    assert!(after_first > seen, "a list of servers notifies");

    let ready = vec![
        server('1', "rust", None, State::Ready),
        server('1', "rust", Some(COMMIT), State::Starting),
    ];
    navigator.tell(Told::Servers(ready.clone()), cx);
    assert_eq!(
        read(cx),
        (languages, ready),
        "the new list replaces the old"
    );
    assert!(*notified.borrow() > after_first);
}

#[gpui_kit::test]
fn the_state_of_a_file_is_the_server_of_its_language_for_the_side_asked(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let navigator = Navigator::connect(&models, cx);
    navigator.tell(
        Told::Languages(vec![
            language("rust", "Rust", &["rs"]),
            language("typescript", "TypeScript", &["ts", "tsx"]),
        ]),
        cx,
    );
    let on_disk = server('1', "rust", None, State::Ready);
    let old = server('1', "rust", Some(COMMIT), State::Starting);
    let script = server('1', "typescript", None, State::Missing { offer: None });
    let elsewhere = server('2', "rust", None, State::Starting);
    navigator.tell(
        Told::Servers(vec![
            elsewhere.clone(),
            old.clone(),
            on_disk.clone(),
            script.clone(),
        ]),
        cx,
    );

    let state = |workspace: char, path: &str, old_side: Option<&str>, cx: &mut TestAppContext| {
        models.navigation.read_with(cx, |navigation, _| {
            navigation
                .state_for(&id(workspace), std::path::Path::new(path), old_side)
                .cloned()
        })
    };
    assert_eq!(state('1', "src/a.rs", None, cx), Some(on_disk));
    assert_eq!(state('1', "src/a.rs", Some(COMMIT), cx), Some(old));
    assert_eq!(state('1', "web/app.tsx", None, cx), Some(script));
    assert_eq!(state('2', "src/a.rs", None, cx), Some(elsewhere));
    assert_eq!(
        state('1', "src/a.rs", Some("another version"), cx),
        None,
        "no server of that old side"
    );
    assert_eq!(state('1', "web/app.ts", Some(COMMIT), cx), None);
    assert_eq!(state('3', "src/a.rs", None, cx), None, "no workspace");
    assert_eq!(state('1', "notes.txt", None, cx), None, "no language");
    assert_eq!(state('1', "Makefile", None, cx), None, "no extension");
    models.navigation.read_with(cx, |navigation, _| {
        assert_eq!(
            navigation
                .language_of(std::path::Path::new("lib/x.tsx"))
                .map(|language| language.name.as_str()),
            Some("typescript")
        );
        assert!(navigation.language_of(std::path::Path::new("RS")).is_none());
        assert!(
            navigation
                .language_of(std::path::Path::new("a.RS"))
                .is_none(),
            "extensions are compared as the registry compares them, exactly"
        );
    });
}

#[gpui_kit::test]
fn an_export_the_navigator_wants_is_asked_of_the_keeper_and_how_it_went_is_told_back(
    cx: &mut TestAppContext,
) {
    let (models, mut keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    let wanted = |k: u32| ExportAsk {
        workspace: id('1'),
        version: COMMIT.to_owned(),
        into: PathBuf::from(format!("/cache/navigator/1-1/old/{k}")),
    };
    let export = |k: u32| Request::ExportVersion {
        workspace: id('1'),
        version: COMMIT.to_owned(),
        into: PathBuf::from(format!("/cache/navigator/1-1/old/{k}")),
    };

    navigator.tell(Told::ExportWanted(wanted(1)), cx);
    let (asked, request) = request_of(&mut keeper, cx, |r| {
        matches!(r, Request::ExportVersion { .. })
    });
    assert_eq!(request, export(1));
    keeper.reply(asked, Ok(Reply::Done));
    navigator.expect(
        Ask::Exported {
            asked: wanted(1),
            failed: None,
        },
        cx,
    );

    let refusal = Refusal::Failed {
        reason: "the folder exists".to_owned(),
    };
    navigator.tell(Told::ExportWanted(wanted(2)), cx);
    let (asked, request) = request_of(&mut keeper, cx, |r| {
        matches!(r, Request::ExportVersion { .. })
    });
    assert_eq!(request, export(2));
    keeper.reply(asked, Err(refusal.clone()));
    navigator.expect(
        Ask::Exported {
            asked: wanted(2),
            failed: Some(refusal.to_string()),
        },
        cx,
    );
    assert!(refusal.to_string().contains("the folder exists"));

    // A Keeper that answers something else than was asked is told as a failure too.
    navigator.tell(Told::ExportWanted(wanted(3)), cx);
    let (asked, _) = request_of(&mut keeper, cx, |r| {
        matches!(r, Request::ExportVersion { .. })
    });
    keeper.reply(asked, Ok(Reply::Workspaces(Vec::new())));
    navigator.expect(
        Ask::Exported {
            asked: wanted(3),
            failed: Some(AskError::Unexpected.to_string()),
        },
        cx,
    );
}

#[gpui_kit::test]
fn an_export_wanted_while_the_keeper_is_lost_is_told_back_as_a_failure(cx: &mut TestAppContext) {
    let (models, keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    keeper.close();
    wait_until(cx, "the link to be lost", |cx| {
        models
            .link
            .read_with(cx, |link, _| matches!(link.status(), LinkStatus::Lost(_)))
    });

    let wanted = ExportAsk {
        workspace: id('1'),
        version: COMMIT.to_owned(),
        into: PathBuf::from("/cache/navigator/1-1/old/1"),
    };
    navigator.tell(Told::ExportWanted(wanted.clone()), cx);
    navigator.expect(
        Ask::Exported {
            asked: wanted,
            failed: Some(AskError::NotLinked.to_string()),
        },
        cx,
    );
}

#[gpui_kit::test]
fn with_no_navigator_connected_every_ask_resolves_to_not_connected_at_once(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    models.navigation.read_with(cx, |navigation, _| {
        assert!(!navigation.is_connected());
        assert!(navigation.servers().is_empty() && navigation.languages().is_empty());
    });

    let asking = hover(&models, place("src/a.rs", "alpha"), cx);
    let first = asking.id();
    let (slot, _task) = pending(asking, cx);
    assert_eq!(resolved(&slot, cx), Err(NavError::NotConnected));
    let second = models.navigation.update(cx, |navigation, cx| {
        navigation.definition(place("src/a.rs", "alpha"), cx)
    });
    assert_ne!(first, second.id());
    let (slot, _task) = pending(second, cx);
    assert_eq!(resolved(&slot, cx), Err(NavError::NotConnected));
    let references = models.navigation.update(cx, |navigation, cx| {
        navigation.references(place("src/a.rs", "alpha"), cx)
    });
    let (slot, _task) = pending(references, cx);
    assert_eq!(resolved(&slot, cx), Err(NavError::NotConnected));
    let opened = models.navigation.update(cx, |navigation, cx| {
        navigation.open(location("/w/src/a.rs", 1), cx)
    });
    let (slot, _task) = pending(opened, cx);
    assert_eq!(resolved(&slot, cx), Err(NavError::NotConnected));

    // What the user does meanwhile is lost, not stored: nothing waits for a connect.
    models.navigation.update(cx, |navigation, _| {
        navigation.download("rust");
        navigation.update("rust");
        navigation.old_side_shown(&id('1'), Some(COMMIT.to_owned()));
    });
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("python"));
    navigator.expect(
        Ask::Download {
            language: "python".to_owned(),
        },
        cx,
    );
    models
        .navigation
        .read_with(cx, |navigation, _| assert!(navigation.is_connected()));
}

#[gpui_kit::test]
fn the_user_s_wishes_reach_the_navigator_as_the_asks_that_name_them(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);

    models.navigation.update(cx, |navigation, _| {
        navigation.download("rust");
        navigation.update("python");
        navigation.old_side_shown(&id('1'), Some(COMMIT.to_owned()));
        navigation.old_side_shown(&id('1'), None);
    });
    navigator.expect(
        Ask::Download {
            language: "rust".to_owned(),
        },
        cx,
    );
    navigator.expect(
        Ask::Update {
            language: "python".to_owned(),
        },
        cx,
    );
    navigator.expect(
        Ask::OldSide {
            workspace: id('1'),
            version: Some(COMMIT.to_owned()),
        },
        cx,
    );
    navigator.expect(
        Ask::OldSide {
            workspace: id('1'),
            version: None,
        },
        cx,
    );
}

#[gpui_kit::test]
fn the_answer_on_a_repository_is_sent_as_its_setting_and_read_back_once_told(
    cx: &mut TestAppContext,
) {
    // A setting that is no answer on a repository is held by the Keeper, so that `window`
    // returns only once the answer to the first question for the settings was taken: an
    // answer that landed after the test's own would take the user's answer off the list.
    let (models, keeper) = window(
        vec![Setting::FlowForm {
            workspace: id('1'),
            form: FlowForm::LiveDiff,
        }],
        &one_workspace(),
        cx,
    );
    let mut navigator = Navigator::connect(&models, cx);
    let (_, _, consents) = navigator.first_telling(cx);
    assert_eq!(consents, Vec::new(), "no repository was answered yet");
    let read = |letter: char, cx: &mut TestAppContext| {
        models
            .navigation
            .read_with(cx, |navigation, cx| navigation.consent(&repo(letter), cx))
    };
    assert_eq!(read('a', cx), None, "the question waits");

    models.navigation.update(cx, |navigation, cx| {
        navigation.answer_consent(repo('a'), true, cx);
    });
    wait_until(cx, "the Keeper to hear the answer", |_| {
        keeper.settings_set() == vec![allowed('a', true)]
    });
    assert_eq!(read('a', cx), Some(true));
    assert_eq!(read('b', cx), None, "another repository still waits");
    navigator.expect(Ask::Consents(vec![consent('a', true)]), cx);

    models.navigation.update(cx, |navigation, cx| {
        navigation.answer_consent(repo('a'), false, cx);
    });
    wait_until(cx, "the Keeper to hear the second answer", |_| {
        keeper.settings_set() == vec![allowed('a', true), allowed('a', false)]
    });
    assert_eq!(read('a', cx), Some(false));
    navigator.expect(Ask::Consents(vec![consent('a', false)]), cx);
}

#[gpui_kit::test]
fn a_second_connect_replaces_the_first_and_what_waited_on_it_is_gone(cx: &mut TestAppContext) {
    let (models, _keeper) = window(vec![allowed('a', true)], &one_workspace(), cx);
    let mut first = Navigator::connect(&models, cx);
    first.first_telling(cx);
    let asking = hover(&models, place("src/a.rs", "alpha"), cx);
    let asked = asking.id();
    first.ask(cx);
    let (waiting, task) = pending(asking, cx);
    navigator_servers(&first, cx);

    let mut second = Navigator::connect(&models, cx);
    assert_eq!(resolved(&waiting, cx), Err(NavError::Gone));
    assert_eq!(
        second.first_telling(cx),
        (
            vec![WorkspaceRoot {
                id: id('1'),
                repository: repo('a'),
                folder: Some(PathBuf::from("/gits/main")),
            }],
            Some(id('1')),
            vec![consent('a', true)]
        ),
        "the new Navigator is told everything, as the first was"
    );
    models.navigation.read_with(cx, |navigation, _| {
        assert!(
            navigation.servers().is_empty(),
            "what the first told is no news of the second"
        );
    });

    // The ask that was waiting is dropped: it asks the new Navigator nothing, and the old
    // one hears nothing more. The barrier is an ask of the new one; the old port is read
    // to its end, which comes once every sender of it is gone.
    drop(task);
    models
        .navigation
        .update(cx, |navigation, _| navigation.download("rust"));
    second.expect(
        Ask::Download {
            language: "rust".to_owned(),
        },
        cx,
    );
    first.ends_with_nothing_more(cx);

    // The model reads nothing more from the first Navigator: its channel is let go.
    wait_until(
        cx,
        "the model to let the first Navigator's channel go",
        |_| first.told.is_closed(),
    );
    assert!(
        first
            .told
            .try_send(Told::Hover {
                id: asked,
                answer: hovered("late", false),
            })
            .is_err()
    );
}

/// The first Navigator tells a server, so that a replacement has something to forget.
fn navigator_servers(navigator: &Navigator, cx: &mut TestAppContext) {
    navigator.tell(
        Told::Servers(vec![server('1', "rust", None, State::Ready)]),
        cx,
    );
}

#[gpui_kit::test]
fn a_navigator_that_ends_resolves_what_waits_to_gone_and_leaves_the_model_unconnected(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    navigator.tell(Told::Languages(vec![language("rust", "Rust", &["rs"])]), cx);
    navigator_servers(&navigator, cx);
    let asking = hover(&models, place("src/a.rs", "alpha"), cx);
    navigator.ask(cx);
    let (waiting, _task) = pending(asking, cx);

    navigator.end(cx);

    assert_eq!(resolved(&waiting, cx), Err(NavError::Gone));
    models.navigation.read_with(cx, |navigation, _| {
        assert!(!navigation.is_connected());
        assert!(
            navigation.servers().is_empty() && navigation.languages().is_empty(),
            "what a Navigator that ended told is no longer true"
        );
    });
    let after = hover(&models, place("src/a.rs", "alpha"), cx);
    let (slot, _task) = pending(after, cx);
    assert_eq!(resolved(&slot, cx), Err(NavError::NotConnected));
}

#[gpui_kit::test]
fn whether_a_navigator_is_connected_notifies_when_it_changes_at_a_connect_and_at_an_end(
    cx: &mut TestAppContext,
) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    // No server and no language is ever told, so nothing else gives the observers a reason
    // to look: only the change of the answer to `is_connected` does.
    let seen = Rc::new(RefCell::new(Vec::<bool>::new()));
    let _watching = cx.update(|cx| {
        let seen = seen.clone();
        cx.observe(&models.navigation, move |navigation, cx| {
            seen.borrow_mut().push(navigation.read(cx).is_connected());
        })
    });
    cx.run_until_parked();
    let before = seen.borrow().len();

    let mut navigator = Navigator::connect(&models, cx);
    navigator.first_telling(cx);
    assert!(
        seen.borrow().len() > before && seen.borrow().last() == Some(&true),
        "a connect notifies, and the model says it is connected: {:?}",
        seen.borrow()
    );

    let connected = seen.borrow().len();
    navigator.end(cx);
    models
        .navigation
        .read_with(cx, |navigation, _| assert!(!navigation.is_connected()));
    assert!(
        seen.borrow().len() > connected && seen.borrow().last() == Some(&false),
        "an end notifies, and the model says it is not connected: {:?}",
        seen.borrow()
    );
}

#[gpui_kit::test]
fn a_navigator_that_no_longer_reads_its_asks_resolves_a_new_ask_to_gone(cx: &mut TestAppContext) {
    let (models, _keeper) = window(Vec::new(), &one_workspace(), cx);
    let (port, end) = port();
    drop(end.asks);
    models
        .navigation
        .update(cx, |navigation, cx| navigation.connect(port, cx));
    cx.run_until_parked();

    // What the model told by itself at the connect went nowhere, and the ask is told so
    // rather than left to wait for an answer that cannot come. `end.told` is open still.
    let asking = hover(&models, place("src/a.rs", "alpha"), cx);
    let (slot, _task) = pending(asking, cx);
    assert_eq!(resolved(&slot, cx), Err(NavError::Gone));
    drop(end.told);
}
