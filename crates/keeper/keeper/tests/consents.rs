//! Each test sets the process environment first, so each needs a process of its own (cargo
//! nextest).

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    AgentReport, Consent, ConsentAnswer, Fact, Hello, Message, Program, Refusal, Reply, Request,
    RequestId, TerminalId, TerminalSize, WorkspaceId, handshake, read_message, write_message,
};

const DEADLINE: Duration = Duration::from_secs(20);

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 24,
};

const PI: &str = "pi";
const CLAUDE: &str = "claude";

struct Scratch(PathBuf);

impl Scratch {
    /// Sets the process environment; must be a test's first statement.
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-consents-{}-{n}", std::process::id()));
        for folder in ["home", "tmp"] {
            std::fs::create_dir_all(root.join(folder)).expect("make the test's folder");
        }
        let settings = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "A Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
            ("GIT_COMMITTER_NAME", "A Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
            ("SHELL", "/bin/sh"),
        ];
        // SAFETY: the first statement of the test; nextest runs each test in a process of its own,
        // so no other thread exists.
        unsafe {
            for (name, value) in settings {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            std::env::set_var("TMPDIR", root.join("tmp"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
                "ENV",
                "PS1",
                "YAAR_PI_REPORT",
            ] {
                std::env::remove_var(name);
            }
        }
        Scratch(root)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn companions(&self) -> PathBuf {
        self.0.join("companions")
    }

    fn pi_is_installed(&self) -> PathBuf {
        let folder = self.home().join(".pi").join("agent");
        std::fs::create_dir_all(&folder).expect("make pi's folder");
        folder
    }

    fn pi_settings(&self) -> PathBuf {
        self.home().join(".pi").join("agent").join("settings.json")
    }

    fn adapter_folder(&self, agent: &str) -> PathBuf {
        self.data().join("agents").join(agent)
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.data(),
            companions: Some(self.companions()),
            home: self.home(),
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
                return Window { stream, next_id: 0 };
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
        std::fs::write(folder.join("a.txt"), "one\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }

    /// A fake `pi` that prints what a pi would load. A login shell's profile resets the search
    /// path, so the home's `.profile` puts the folder first again.
    fn a_pi_that_says_what_it_would_load(&self) {
        let folder = self.0.join("fake");
        std::fs::create_dir_all(&folder).expect("make the folder of the fake pi");
        let program = folder.join("pi");
        std::fs::write(
            &program,
            "#!/bin/sh\n\
             printf 'PI-REPORT <%s>\\n' \"$YAAR_PI_REPORT\"\n\
             printf 'PI-SETTINGS <%s>\\n' \"$(tr -d ' \\n' < \"$HOME/.pi/agent/settings.json\")\"\n",
        )
        .expect("write the fake pi");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("make the fake pi run");
        std::fs::write(
            self.home().join(".profile"),
            format!("PATH=\"{}:$PATH\"; export PATH\n", folder.display()),
        )
        .expect("write the profile");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(folder: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("start git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn tree(folder: &Path) -> BTreeSet<String> {
    fn walk(base: &Path, folder: &Path, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(folder).expect("read a folder") {
            let path = entry.expect("read an entry").path();
            let name = path
                .strip_prefix(base)
                .expect("under the base")
                .display()
                .to_string();
            if path.is_dir() {
                found.insert(format!("{name}/"));
                walk(base, &path, found);
            } else {
                found.insert(name);
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(folder, folder, &mut found);
    found
}

struct Window {
    stream: UnixStream,
    next_id: u32,
}

impl Window {
    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    fn next(&mut self) -> Message {
        read_message(&mut self.stream).expect("a frame from the Keeper")
    }

    /// Expects the reply as the very next frame: nothing is told between.
    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        match self.next() {
            Message::Reply { to, reply } if to == id => reply,
            other => panic!("not the reply to {id:?}: {other:?}"),
        }
    }

    fn consents_told(&mut self) -> Vec<Consent> {
        match self.next() {
            Message::Fact(Fact::ConsentsChanged(consents)) => consents,
            other => panic!("not the fact that tells the questions: {other:?}"),
        }
    }

    fn consents(&mut self) -> Vec<Consent> {
        match self.ask(Request::ListConsents) {
            Ok(Reply::Consents(consents)) => consents,
            other => panic!("not the list of questions: {other:?}"),
        }
    }

    fn answer(&mut self, agent: &str, answer: ConsentAnswer) -> Result<Vec<Consent>, Refusal> {
        match self.ask(Request::AnswerConsent {
            agent: agent.to_owned(),
            answer,
        }) {
            Ok(Reply::Consents(consents)) => Ok(consents),
            Ok(other) => panic!("not the list of questions: {other:?}"),
            Err(refusal) => Err(refusal),
        }
    }

    fn open(&mut self, folder: &Path) -> WorkspaceId {
        let opened = self.ask(Request::OpenFolder {
            path: folder.to_owned(),
        });
        let Ok(Reply::Opened { workspace, .. }) = opened else {
            panic!("not opened: {opened:?}");
        };
        assert!(matches!(self.next(), Message::Fact(Fact::Workspaces(_))));
        workspace
    }

    /// Skips output of a terminal opened before, which may arrive before the reply.
    fn open_terminal(&mut self, workspace: &WorkspaceId, program: Program) -> TerminalId {
        let id = self.send(Request::OpenTerminal {
            workspace: workspace.clone(),
            program,
            size: SIZE,
        });
        loop {
            match self.next() {
                Message::Reply { to, reply } if to == id => {
                    let Ok(Reply::TerminalOpened { terminal }) = reply else {
                        panic!("no terminal: {reply:?}");
                    };
                    return terminal;
                }
                Message::Output { .. } => {}
                other => panic!("not the reply to {id:?}: {other:?}"),
            }
        }
    }

    fn type_into(&mut self, terminal: &TerminalId, bytes: &[u8]) {
        write_message(
            &mut self.stream,
            &Message::Input {
                terminal: terminal.clone(),
                bytes: bytes.to_vec(),
            },
        )
        .expect("send the bytes");
    }

    fn output_until(&mut self, terminal: &TerminalId, wanted: impl Fn(&str) -> bool) -> String {
        let mut heard = Vec::new();
        while !wanted(&String::from_utf8_lossy(&heard)) {
            match self.next() {
                Message::Output {
                    terminal: from,
                    bytes,
                } if from == *terminal => heard.extend(bytes),
                Message::Output { .. } => {}
                other => panic!(
                    "after {:?}, not output of {terminal}: {other:?}",
                    String::from_utf8_lossy(&heard)
                ),
            }
        }
        String::from_utf8_lossy(&heard).into_owned()
    }

    fn until_ended(&mut self, terminal: &TerminalId) -> String {
        let mut heard = Vec::new();
        loop {
            match self.next() {
                Message::Output {
                    terminal: from,
                    bytes,
                } if from == *terminal => heard.extend(bytes),
                Message::Fact(Fact::TerminalEnded { terminal: from, .. }) if from == *terminal => {
                    return String::from_utf8_lossy(&heard).into_owned();
                }
                Message::Output { .. } => {}
                other => panic!("waiting for the end of {terminal}, got {other:?}"),
            }
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

fn what_the_adapter_says(scratch: &Scratch, agent: &str) -> Option<yaar_agent::Consent> {
    yaar_keeper::adapters(&scratch.home())
        .iter()
        .find(|adapter| adapter.name() == agent)
        .expect("an Adapter of that name")
        .consent(&scratch.adapter_folder(agent))
}

fn of<'a>(consents: &'a [Consent], agent: &str) -> &'a Consent {
    let mut found = consents.iter().filter(|consent| consent.agent == agent);
    let one = found.next().expect("the agent's question is listed");
    assert!(found.next().is_none(), "listed twice: {consents:?}");
    one
}

#[test]
fn a_window_is_told_the_question_of_an_adapter_before_the_reply_to_its_first_request_and_every_window_the_answer()
 {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    let keeper = scratch.keeper();

    let mut first = scratch.window();
    let id = first.send(Request::ListWorkspaces);
    let told = first.consents_told();
    assert_eq!(
        told.len(),
        1,
        "only an Adapter that asks for leave is listed"
    );
    let question = of(&told, PI);
    assert_eq!(question.answer, ConsentAnswer::NotAsked);
    assert_eq!(question.problem, None);
    let adapters_own = what_the_adapter_says(&scratch, PI).expect("pi asks");
    assert_eq!(question.question, adapters_own.question);
    assert!(!question.question.is_empty());
    match first.next() {
        Message::Reply { to, reply: Ok(_) } if to == id => {}
        other => panic!("not the reply to the first request: {other:?}"),
    }

    let mut second = scratch.window();
    let id = second.send(Request::ListWorkspaces);
    assert_eq!(second.consents_told(), told);
    assert!(matches!(second.next(), Message::Reply { to, .. } if to == id));
    let answered = second
        .answer(PI, ConsentAnswer::Yes)
        .expect("the answer is taken");
    assert_eq!(of(&answered, PI).answer, ConsentAnswer::Yes);
    assert_eq!(of(&answered, PI).problem, None);
    assert_eq!(second.consents_told(), answered, "the one who answered");
    assert_eq!(first.consents_told(), answered, "the other Window");

    let mut third = scratch.window();
    assert!(matches!(
        third.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(_))
    ));
    assert_eq!(of(&third.consents(), PI).answer, ConsentAnswer::Yes);

    third.stop(keeper);
}

#[test]
fn a_window_that_connects_after_a_no_is_told_nothing_either_and_the_list_says_no() {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    let keeper = scratch.keeper();

    let mut first = scratch.window();
    let id = first.send(Request::ListWorkspaces);
    assert_eq!(
        of(&first.consents_told(), PI).answer,
        ConsentAnswer::NotAsked
    );
    assert!(matches!(first.next(), Message::Reply { to, .. } if to == id));
    let answered = first.answer(PI, ConsentAnswer::No).expect("taken");
    assert_eq!(of(&answered, PI).answer, ConsentAnswer::No);
    assert_eq!(first.consents_told(), answered);

    let mut later = scratch.window();
    assert!(matches!(
        later.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(_))
    ));
    assert_eq!(of(&later.consents(), PI).answer, ConsentAnswer::No);

    later.stop(keeper);
}

#[test]
fn a_piece_of_a_cli_is_not_told_the_question_before_the_reply_to_its_report() {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    let keeper = scratch.keeper();

    let mut watcher = scratch.window();
    let id = watcher.send(Request::ListWorkspaces);
    assert_eq!(
        of(&watcher.consents_told(), PI).answer,
        ConsentAnswer::NotAsked
    );
    assert!(matches!(watcher.next(), Message::Reply { to, .. } if to == id));

    let mut piece = scratch.window();
    let report = AgentReport {
        agent: PI.to_owned(),
        terminal: None,
        pid: std::process::id(),
        payload: b"not what pi reports".to_vec(),
    };
    assert_eq!(piece.ask(Request::Report { report }), Ok(Reply::Done));

    watcher.stop(keeper);
}

#[test]
fn with_pi_not_installed_no_question_is_listed_or_told_and_a_yes_is_refused_and_writes_nothing() {
    let scratch = Scratch::new();
    // Read before the Keeper starts, so that a write at its start would show.
    let at_the_start = tree(&scratch.home());
    let keeper = scratch.keeper();

    let mut first = scratch.window();
    assert!(matches!(
        first.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(_))
    ));
    assert_eq!(first.consents(), Vec::new());

    let mut second = scratch.window();
    assert!(matches!(
        second.ask(Request::ListWorkspaces),
        Ok(Reply::Workspaces(_))
    ));
    let refused = second.answer(PI, ConsentAnswer::Yes);
    assert!(
        matches!(&refused, Err(Refusal::Failed { reason }) if reason.contains(PI)),
        "{refused:?}"
    );
    assert_eq!(first.consents(), Vec::new());
    assert_eq!(second.consents(), Vec::new());
    assert_eq!(
        tree(&scratch.home()),
        at_the_start,
        "nothing is written under the home"
    );
    assert!(!scratch.pi_settings().exists());

    second.stop(keeper);
}

#[test]
fn an_answer_for_an_adapter_that_asks_no_leave_or_that_does_not_exist_is_refused_and_nothing_is_written_or_told()
 {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    let keeper = scratch.keeper();

    let mut watcher = scratch.window();
    let id = watcher.send(Request::ListWorkspaces);
    assert_eq!(
        of(&watcher.consents_told(), PI).answer,
        ConsentAnswer::NotAsked
    );
    assert!(matches!(watcher.next(), Message::Reply { to, .. } if to == id));
    let at_the_start = tree(&scratch.home());

    let mut user = scratch.window();
    let id = user.send(Request::ListWorkspaces);
    user.consents_told();
    assert!(matches!(user.next(), Message::Reply { to, .. } if to == id));
    for name in [CLAUDE, "otheragent", ""] {
        let refused = user.answer(name, ConsentAnswer::Yes);
        assert!(
            matches!(&refused, Err(Refusal::Failed { reason }) if reason.contains("asks for no leave")),
            "{name:?}: {refused:?}"
        );
    }
    assert_eq!(tree(&scratch.home()), at_the_start);
    assert!(!scratch.pi_settings().exists());
    assert_eq!(of(&watcher.consents(), PI).answer, ConsentAnswer::NotAsked);

    watcher.stop(keeper);
}

#[test]
fn a_settings_file_pi_could_not_read_is_left_as_it_was_and_the_problem_is_told_to_every_window_and_at_every_connect()
 {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    let broken = b"{ \"extensions\": [ // a comment pi does not read\n ] }\n";
    std::fs::write(scratch.pi_settings(), broken).expect("write the settings");
    let keeper = scratch.keeper();

    let mut first = scratch.window();
    let id = first.send(Request::ListWorkspaces);
    first.consents_told();
    assert!(matches!(first.next(), Message::Reply { to, .. } if to == id));

    let answered = first.answer(PI, ConsentAnswer::Yes).expect("taken");
    let problem = of(&answered, PI).problem.clone().expect("a problem");
    assert!(problem.contains("settings.json"), "{problem}");
    assert_ne!(of(&answered, PI).answer, ConsentAnswer::Yes);
    assert_eq!(first.consents_told(), answered);
    assert_eq!(
        std::fs::read(scratch.pi_settings()).expect("read the settings"),
        broken,
        "the file is left byte for byte"
    );

    let mut later = scratch.window();
    let id = later.send(Request::ListWorkspaces);
    assert_eq!(later.consents_told(), answered);
    assert!(matches!(later.next(), Message::Reply { to, .. } if to == id));

    later.stop(keeper);
}

#[test]
fn a_no_after_a_yes_takes_the_entry_out_and_asking_again_leaves_the_question_for_the_next_window() {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    let keeper = scratch.keeper();
    let entry = scratch.adapter_folder(PI).join("extension").join("yaar.ts");

    let mut window = scratch.window();
    let id = window.send(Request::ListWorkspaces);
    window.consents_told();
    assert!(matches!(window.next(), Message::Reply { to, .. } if to == id));

    assert_eq!(
        of(&window.answer(PI, ConsentAnswer::Yes).expect("taken"), PI).answer,
        ConsentAnswer::Yes
    );
    window.consents_told();
    assert!(
        std::fs::read_to_string(scratch.pi_settings())
            .expect("the settings")
            .contains(entry.to_str().expect("text")),
        "the yes wrote the entry"
    );

    assert_eq!(
        of(&window.answer(PI, ConsentAnswer::No).expect("taken"), PI).answer,
        ConsentAnswer::No
    );
    window.consents_told();
    assert!(
        !std::fs::read_to_string(scratch.pi_settings())
            .expect("the settings")
            .contains(entry.to_str().expect("text")),
        "the no took it out"
    );

    assert_eq!(
        of(
            &window.answer(PI, ConsentAnswer::NotAsked).expect("taken"),
            PI
        )
        .answer,
        ConsentAnswer::NotAsked
    );
    window.consents_told();
    let mut later = scratch.window();
    let id = later.send(Request::ListWorkspaces);
    assert_eq!(
        of(&later.consents_told(), PI).answer,
        ConsentAnswer::NotAsked
    );
    assert!(matches!(later.next(), Message::Reply { to, .. } if to == id));

    later.stop(keeper);
}

#[test]
fn every_shell_of_the_app_and_the_launcher_of_pi_name_the_reporting_program() {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    scratch.a_pi_that_says_what_it_would_load();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let program = scratch.companions().join("yaar-pi-report");
    let named = format!("<{}>", program.display());

    let mut window = scratch.window();
    let id = window.send(Request::ListWorkspaces);
    window.consents_told();
    assert!(matches!(window.next(), Message::Reply { to, .. } if to == id));
    let workspace = window.open(&repo);

    for _ in 0..2 {
        let shell = window.open_terminal(&workspace, Program::Shell);
        window.type_into(
            &shell,
            b"printf 'REPORT=%s\\n' \"$YAAR_PI_REPORT\"; printf 'DONE%s\\n' -MARK\r",
        );
        let heard = window.output_until(&shell, |heard| heard.contains("DONE-MARK"));
        assert!(
            heard.contains(&format!("REPORT={}\r", program.display())),
            "{heard:?}"
        );
    }

    let launched = window.open_terminal(
        &workspace,
        Program::Agent {
            agent: PI.to_owned(),
        },
    );
    let heard = window.until_ended(&launched);
    assert!(heard.contains(&format!("PI-REPORT {named}")), "{heard:?}");

    window.stop(keeper);
}

#[test]
fn after_the_users_yes_pi_typed_in_a_shell_and_pi_from_its_launcher_load_the_extension_the_adapter_wrote()
 {
    let scratch = Scratch::new();
    scratch.pi_is_installed();
    scratch.a_pi_that_says_what_it_would_load();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let extension = scratch
        .adapter_folder(PI)
        .join("extension")
        .join(yaar_adapter_pi::EXTENSION_FILE);
    let entry = extension.to_str().expect("a path that is text").to_owned();
    let users = "{\"theme\": \"dark\", \"extensions\": [\"/users/own.ts\"]}\n";
    std::fs::write(scratch.pi_settings(), users).expect("write the settings");

    let mut window = scratch.window();
    let id = window.send(Request::ListWorkspaces);
    window.consents_told();
    assert!(matches!(window.next(), Message::Reply { to, .. } if to == id));
    let workspace = window.open(&repo);

    assert_eq!(
        std::fs::read_to_string(&extension).expect("the extension the Adapter wrote"),
        yaar_adapter_pi::EXTENSION_SOURCE
    );
    let listed = |scratch: &Scratch| -> Vec<String> {
        let text = std::fs::read_to_string(scratch.pi_settings()).expect("pi's settings");
        let json: serde_json::Value = serde_json::from_str(&text).expect("settings are JSON");
        json["extensions"]
            .as_array()
            .expect("a list of extensions")
            .iter()
            .map(|one| one.as_str().expect("a path").to_owned())
            .collect()
    };
    assert_eq!(
        std::fs::read_to_string(scratch.pi_settings()).unwrap(),
        users
    );

    window
        .answer(PI, ConsentAnswer::No)
        .expect("the answer is taken");
    window.consents_told();
    assert_eq!(listed(&scratch), ["/users/own.ts"]);

    window
        .answer(PI, ConsentAnswer::Yes)
        .expect("the answer is taken");
    window.consents_told();
    assert_eq!(listed(&scratch), ["/users/own.ts", entry.as_str()]);
    let text = std::fs::read_to_string(scratch.pi_settings()).expect("pi's settings");
    assert!(text.contains("\"theme\": \"dark\""), "{text}");

    let shell = window.open_terminal(&workspace, Program::Shell);
    window.type_into(&shell, b"pi; printf 'DONE%s\\n' -MARK\r");
    let typed = window.output_until(&shell, |heard| heard.contains("DONE-MARK"));
    let launched = window.open_terminal(
        &workspace,
        Program::Agent {
            agent: PI.to_owned(),
        },
    );
    let launcher = window.until_ended(&launched);

    // A shell prompt may stand in front of the first line the command prints.
    let lines = |heard: &str| -> Vec<String> {
        heard
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter_map(|line| {
                line.find("PI-REPORT <")
                    .or_else(|| line.find("PI-SETTINGS <"))
                    .map(|at| line[at..].to_owned())
            })
            .collect()
    };
    let (typed, launcher) = (lines(&typed), lines(&launcher));
    assert_eq!(typed.len(), 2, "{typed:?}");
    assert_eq!(typed, launcher, "the same variable and the same settings");
    assert_eq!(
        typed[0],
        format!(
            "PI-REPORT <{}>",
            scratch.companions().join("yaar-pi-report").display()
        )
    );
    assert!(
        typed[1].contains(&format!("\"{entry}\"")),
        "pi reads the entry: {typed:?}"
    );

    window.stop(keeper);
}
