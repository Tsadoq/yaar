//! Each test cleans the process environment first, so each needs a process of its own (nextest).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    Fact, FlowForm, Hello, Message, Refusal, Reply, Repository, RepositoryId, Request, RequestId,
    Setting, WorkspaceId, handshake, read_message, write_message,
};

mod support;

use support::{Environment, Scratch, git};

const DEADLINE: Duration = Duration::from_secs(20);

impl Scratch {
    fn new() -> Scratch {
        Scratch::in_tmp("repository-settings", Environment::Git)
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

    /// Waits until the Keeper serves this Window: a fact told before that is lost to it, and the
    /// first reply proves it.
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
                let mut window = Window {
                    stream,
                    next_id: 0,
                    facts: Vec::new(),
                    looked_at: 0,
                    replies: Vec::new(),
                };
                window
                    .ask(Request::ListClis)
                    .expect("the Keeper serves the Window");
                return window;
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

/// Keeps everything it is sent, in order; what has been looked at is not looked at again.
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

    /// The reply comes with the number of facts this Window had been told when it came.
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

fn answer(repository: &RepositoryId, allowed: bool) -> Setting {
    Setting::ServersInRepository {
        repository: repository.clone(),
        allowed,
    }
}

#[test]
fn an_answer_on_a_repository_is_kept_told_to_every_window_after_the_reply_and_listed() {
    let scratch = Scratch::new();
    let repo = scratch.repository("a");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let workspace = window.open(&repo);
    let repository = window.repository_of(&workspace);
    let mut second = scratch.window();
    assert_eq!(window.settings(), []);

    let yes = answer(&repository, true);
    let (reply, facts_before) = window.ask_noting_facts(Request::SetSetting {
        setting: yes.clone(),
    });
    assert!(matches!(reply, Ok(Reply::Done)), "{reply:?}");
    assert_eq!(window.setting_changed(), yes);
    assert_eq!(second.setting_changed(), yes);
    let told_at = window
        .facts
        .iter()
        .position(|fact| matches!(fact, Fact::SettingChanged(_)))
        .expect("the fact");
    assert!(
        told_at >= facts_before,
        "the fact came before the reply it follows"
    );
    assert_eq!(window.settings(), std::slice::from_ref(&yes));
    assert_eq!(second.settings(), [yes]);

    let no = answer(&repository, false);
    second.set(&no);
    assert_eq!(window.setting_changed(), no);
    assert_eq!(second.setting_changed(), no);
    assert_eq!(window.settings(), std::slice::from_ref(&no));
    assert_eq!(second.settings(), [no]);
    window.stop(keeper);
}

#[test]
fn the_answers_are_listed_by_a_keeper_started_again_on_the_same_data_folder() {
    let scratch = Scratch::new();
    let (a, b) = (scratch.repository("a"), scratch.repository("b"));
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let first = window.open(&a);
    let other = window.open(&b);
    let (first, other) = (window.repository_of(&first), window.repository_of(&other));
    assert_ne!(first, other);
    // Set in the opposite order to the one the repositories were opened in: the last change is to
    // the first one's answer.
    window.set(&answer(&other, false));
    window.set(&answer(&first, false));
    window.set(&answer(&first, true));
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert_eq!(
        window.settings(),
        [answer(&first, true), answer(&other, false)]
    );
    window.stop(keeper);
}

#[test]
fn an_answer_on_a_repository_the_keeper_does_not_know_is_refused_naming_it_and_tells_nobody() {
    let scratch = Scratch::new();
    let repo = scratch.repository("a");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let mut second = scratch.window();
    let workspace = window.open(&repo);
    let repository = window.repository_of(&workspace);

    let unknown = RepositoryId::new("0123456789abcdef0123456789abcdef");
    let refused = window.ask(Request::SetSetting {
        setting: answer(&unknown, true),
    });
    assert!(
        matches!(&refused, Err(Refusal::Failed { reason }) if reason.contains(unknown.as_str())),
        "{refused:?}"
    );
    // The refusal is the barrier for what is kept: a list asked after it holds nothing.
    assert_eq!(window.settings(), []);

    let kept = answer(&repository, true);
    window.set(&kept);
    assert_eq!(second.setting_changed(), kept);
    assert_eq!(window.setting_changed(), kept);
    assert_eq!((window.told(), second.told()), (1, 1));
    assert_eq!(window.settings(), [kept]);
    window.stop(keeper);
}

#[test]
fn one_answer_serves_every_worktree_of_a_repository_and_not_another_repository() {
    let scratch = Scratch::new();
    let main = scratch.repository("a");
    let linked = scratch.0.join("a-linked");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "second",
            linked.to_str().expect("a path of text"),
        ],
    );
    let apart = scratch.repository("b");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let in_main = window.open(&main);
    let in_linked = window.open(&linked);
    let in_apart = window.open(&apart);
    assert_ne!(in_main, in_linked);

    let repository = window.repository_of(&in_main);
    assert_eq!(window.repository_of(&in_linked), repository);
    let apart = window.repository_of(&in_apart);
    assert_ne!(apart, repository);

    let mut second = scratch.window();
    let no = answer(&repository, false);
    window.set(&no);
    assert_eq!(second.setting_changed(), no);
    assert_eq!(window.settings(), std::slice::from_ref(&no));
    window.stop(keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    assert_eq!(window.settings(), std::slice::from_ref(&no));
    let form = Setting::FlowForm {
        workspace: in_linked,
        form: FlowForm::LiveDiff,
    };
    window.set(&form);
    window.set(&answer(&apart, true));
    assert_eq!(window.settings(), [form, no, answer(&apart, true)]);
    window.stop(keeper);
}
