//! Each test sets the process environment first, so each needs a process of its own (nextest).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Fact, Hello, Message, OptionKey, Refusal, Reply, Repository, RepositoryId, Request, RequestId,
    Setting, TerminalPlace, ThemeChoice, TurnEndSelects, WorkspaceId, handshake, read_message,
    write_message,
};

mod support;

use support::{Environment, Scratch, git};

const DEADLINE: Duration = Duration::from_secs(20);

impl Scratch {
    fn new() -> Scratch {
        Scratch::in_tmp("app-settings", Environment::Git)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: None,
            home: self.0.join("home"),
            login_shell: None,
        };
        std::thread::spawn(move || run(options))
    }

    fn window(&self) -> Window {
        let mine = Hello::for_this_process("test-window");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Ok(mut stream) = yaar_link::connect(&self.socket())
                && handshake(&mut stream, &mine).is_ok()
            {
                stream
                    .set_read_timeout(Some(DEADLINE))
                    .expect("set the deadline of a read");
                return Window {
                    stream,
                    next_id: 0,
                    facts: Vec::new(),
                    looked_at: 0,
                    replies: Vec::new(),
                };
            }
            assert!(
                Instant::now() < deadline,
                "no Keeper at {:?}",
                self.socket()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn repository(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        git(&folder, &["init", "--quiet", "-b", "main"]);
        git(&folder, &["config", "user.name", "A Test"]);
        git(&folder, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(folder.join("notes.txt"), "notes\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }
}

struct Window {
    stream: UnixStream,
    next_id: u32,
    facts: Vec<Fact>,
    looked_at: usize,
    replies: Vec<(RequestId, Result<Reply, Refusal>, usize)>,
}

impl Window {
    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply, self.facts.len())),
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    fn ask_noting_facts(&mut self, request: Request) -> (Result<Reply, Refusal>, usize) {
        let id = self.send(request);
        loop {
            if let Some(at) = self.replies.iter().position(|(to, ..)| *to == id) {
                let (_, reply, facts) = self.replies.remove(at);
                return (reply, facts);
            }
            self.read("a reply");
        }
    }

    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        self.ask_noting_facts(request).0
    }

    fn setting_changed(&mut self) -> Setting {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if let Fact::SettingChanged(setting) = &self.facts[self.looked_at - 1] {
                    return setting.clone();
                }
            }
            self.read("a setting changed");
        }
    }

    fn told(&self) -> usize {
        self.facts
            .iter()
            .filter(|fact| matches!(fact, Fact::SettingChanged(_)))
            .count()
    }

    fn open(&mut self, folder: &Path) -> WorkspaceId {
        let opened = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        });
        let Ok(Reply::Opened { workspace, .. }) = opened else {
            panic!("not opened: {opened:?}");
        };
        workspace
    }

    fn repositories(&mut self) -> Vec<Repository> {
        match self.ask(Request::ListWorkspaces) {
            Ok(Reply::Workspaces(repositories)) => repositories,
            other => panic!("no repositories: {other:?}"),
        }
    }

    fn repository_of(&mut self, workspace: &WorkspaceId) -> RepositoryId {
        self.repositories()
            .into_iter()
            .find(|repository| repository.workspaces.iter().any(|w| w.id == *workspace))
            .unwrap_or_else(|| panic!("no repository holds {workspace}"))
            .id
    }

    fn settings(&mut self) -> Vec<Setting> {
        match self.ask(Request::GetSettings) {
            Ok(Reply::Settings(settings)) => settings,
            other => panic!("no settings: {other:?}"),
        }
    }

    fn set(&mut self, setting: &Setting) {
        let done = self.ask(Request::SetSetting {
            setting: setting.clone(),
        });
        assert!(matches!(done, Ok(Reply::Done)), "{done:?}");
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn named(name: &str) -> Setting {
    Setting::Theme(ThemeChoice::Named(name.to_owned()))
}

fn every_kind_in_reverse() -> [Setting; 6] {
    [
        Setting::OptionKey(OptionKey::Meta),
        Setting::KeepStates { days: 14 },
        Setting::TerminalPlace(TerminalPlace::Below),
        named("Solarized"),
        Setting::StopAfter { seconds: 90 },
        Setting::TurnEnd(TurnEndSelects::KeepSelection),
    ]
}

#[test]
fn every_setting_of_the_app_is_kept_told_to_every_window_after_the_reply_and_listed() {
    let scratch = Scratch::new();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut second = scratch.window();
    assert_eq!(window.settings(), []);

    for setting in every_kind_in_reverse() {
        let (reply, facts_before) = window.ask_noting_facts(Request::SetSetting {
            setting: setting.clone(),
        });
        assert!(matches!(reply, Ok(Reply::Done)), "{setting:?}: {reply:?}");
        assert_eq!(window.setting_changed(), setting);
        assert_eq!(second.setting_changed(), setting);
        let told_at = window
            .facts
            .iter()
            .rposition(|fact| matches!(fact, Fact::SettingChanged(_)))
            .expect("the fact");
        assert!(
            told_at >= facts_before,
            "the fact of {setting:?} came before the reply it follows"
        );
    }
    let mut listed = every_kind_in_reverse();
    listed.reverse();
    assert_eq!(window.settings(), listed);
    assert_eq!(second.settings(), listed);

    for setting in [
        Setting::KeepStates { days: 7 },
        Setting::TerminalPlace(TerminalPlace::Left),
        Setting::OptionKey(OptionKey::Characters),
    ] {
        second.set(&setting);
        assert_eq!(window.setting_changed(), setting);
        assert_eq!(second.setting_changed(), setting);
    }
    let after = [
        Setting::TurnEnd(TurnEndSelects::KeepSelection),
        Setting::StopAfter { seconds: 90 },
        named("Solarized"),
        Setting::TerminalPlace(TerminalPlace::Left),
        Setting::KeepStates { days: 7 },
        Setting::OptionKey(OptionKey::Characters),
    ];
    assert_eq!(window.settings(), after);
    assert_eq!(second.settings(), after);
    window.stop(keeper);
}

#[test]
fn every_setting_of_the_app_is_listed_by_a_keeper_started_again_on_the_same_data_folder() {
    let scratch = Scratch::new();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    for setting in every_kind_in_reverse() {
        window.set(&setting);
    }
    window.set(&Setting::KeepStates { days: 3 });
    window.set(&named("Nord"));
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert_eq!(
        window.settings(),
        [
            Setting::TurnEnd(TurnEndSelects::KeepSelection),
            Setting::StopAfter { seconds: 90 },
            named("Nord"),
            Setting::TerminalPlace(TerminalPlace::Below),
            Setting::KeepStates { days: 3 },
            Setting::OptionKey(OptionKey::Meta),
        ]
    );
    window.stop(keeper);
}

#[test]
fn the_built_in_themes_are_listed_as_they_were_named_and_kept_as_an_older_build_kept_them() {
    let scratch = Scratch::new();
    let mut keeper = scratch.keeper();
    let mut window = scratch.window();
    let system = Setting::Theme(ThemeChoice::System);
    for (setting, row) in [
        (named("Light"), yaar_store::Theme::Light),
        (named("Dark"), yaar_store::Theme::Dark),
        (system, yaar_store::Theme::System),
    ] {
        window.set(&setting);
        assert_eq!(window.setting_changed(), setting);
        assert_eq!(window.settings(), std::slice::from_ref(&setting));
        window.stop(keeper);

        // The Store holds the built-in theme; a Keeper started again lists it as the Window named
        // it.
        let store = yaar_store::Store::open(&scratch.data().join(yaar_store::FILE_NAME))
            .expect("open the Keeper's database");
        assert_eq!(
            store.settings().expect("settings"),
            [yaar_store::Setting::Theme(row)]
        );
        drop(store);
        keeper = scratch.keeper();
        window = scratch.window();
        assert_eq!(window.settings(), std::slice::from_ref(&setting));
    }
    window.stop(keeper);
}

#[test]
fn a_value_the_store_refuses_is_refused_naming_the_setting_and_nothing_is_kept_or_told() {
    let scratch = Scratch::new();
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut second = scratch.window();
    let kept = Setting::StopAfter { seconds: 90 };
    window.set(&kept);
    assert_eq!(window.setting_changed(), kept);
    assert_eq!(second.setting_changed(), kept);

    for (refused, setting) in [
        (Setting::StopAfter { seconds: 0 }, "stop_after"),
        (Setting::KeepStates { days: 0 }, "keep_states"),
        (named(""), "theme"),
        (named(&"n".repeat(65)), "theme"),
        (named("two\nlines"), "theme"),
    ] {
        let answer = window.ask(Request::SetSetting {
            setting: refused.clone(),
        });
        assert!(
            matches!(&answer, Err(Refusal::Failed { reason }) if reason.contains(setting)),
            "{refused:?}: {answer:?}"
        );
        // The refusal is the barrier for what is kept: a list asked after it holds only what was
        // kept before.
        assert_eq!(
            window.settings(),
            std::slice::from_ref(&kept),
            "{refused:?}"
        );
    }

    // The barrier for what is told: a kept setting is told after the refused ones would have been,
    // so nothing told about them is still on its way.
    let theme = named("Nord");
    window.set(&theme);
    assert_eq!(second.setting_changed(), theme);
    assert_eq!(window.setting_changed(), theme);
    assert_eq!((window.told(), second.told()), (2, 2));
    assert_eq!(window.settings(), [kept, theme]);
    window.stop(keeper);
}

#[test]
fn a_folder_for_worktrees_that_the_store_holds_is_listed_with_its_repository_after_the_answers() {
    let scratch = Scratch::new();
    let (a, b) = (scratch.repository("a"), scratch.repository("b"));
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let first = window.open(&a);
    let other = window.open(&b);
    let (first, other) = (window.repository_of(&first), window.repository_of(&other));
    window.set(&Setting::ServersInRepository {
        repository: first.clone(),
        allowed: true,
    });
    window.set(&Setting::TerminalPlace(TerminalPlace::Left));
    window.stop(keeper);

    let store = yaar_store::Store::open(&scratch.data().join(yaar_store::FILE_NAME))
        .expect("open the Keeper's database");
    for (repository, folder) in [(&other, Some("/trees/é b")), (&first, None)] {
        store
            .set_setting(&yaar_store::Setting::WorktreesFolder {
                repository: yaar_store::RepositoryId::new(repository.as_str()),
                folder: folder.map(PathBuf::from),
            })
            .expect("keep the folder");
    }
    drop(store);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let folder = |repository: &RepositoryId, folder: Option<&str>| Setting::WorktreesFolder {
        repository: repository.clone(),
        folder: folder.map(PathBuf::from),
    };
    assert_eq!(
        window.settings(),
        [
            Setting::TerminalPlace(TerminalPlace::Left),
            Setting::ServersInRepository {
                repository: first.clone(),
                allowed: true,
            },
            folder(&first, None),
            folder(&other, Some("/trees/é b")),
        ]
    );
    window.stop(keeper);
}
