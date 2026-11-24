//! Each test sets the process environment first, so each needs a process of its own (nextest).

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{
    AgentReport, Fact, Hello, Message, Program, Refusal, Reply, Request, RequestId,
    SOCKET_VARIABLE, SessionState, Step, TERMINAL_VARIABLE, TerminalId, WorkspaceAgents,
    WorkspaceId, handshake, read_message, write_message,
};
use yaar_store::{EndedBy, StepRecord, Store, TurnEndRecord, TurnId};
use yaar_system_tests::program;
use yaar_system_tests::recorded::{self, HookRecord};

mod support;

use support::{Environment, Process, SIZE, Scratch, git, is_state};

const DEADLINE: Duration = Duration::from_secs(20);

const HOOK_DEADLINE: Duration = Duration::from_secs(2);

const TOOLS: &str = "claude/turn-tools.jsonl";
const DIALOGS: &str = "claude/permission-dialogs.jsonl";

const PRINT_THE_MARK: &[u8] = b"printf '\\033]9;4;0;\\007'; printf 'after-%s\\n' mark\r";
const AFTER_THE_MARK: &str = "after-mark";

const FIRST_FORM: &str = "\\033]9;4;0;\\007";
const LAST_FORM: &str = "\\033]9;4;0\\033\\\\";

const PRINT_THE_NEXT: &[u8] = b"printf 'then-%s\\n' mark\r";
const THEN: &str = "then-mark";

impl Scratch {
    fn new() -> Scratch {
        Scratch::in_tmp("agents", Environment::Full)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }

    fn database(&self) -> PathBuf {
        self.0.join("data").join(yaar_store::FILE_NAME)
    }

    /// A copy of the built hook program, made at the first call: cargo puts the program in place
    /// again for every test process, so under parallel tests it is briefly missing.
    fn companions(&self) -> PathBuf {
        let folder = self.0.join("companions");
        let copy = folder.join(yaar_adapter_claude::HOOK_PROGRAM);
        if copy.is_file() {
            return folder;
        }
        std::fs::create_dir_all(&folder).expect("make the folder");
        let deadline = Instant::now() + DEADLINE;
        loop {
            let built = std::panic::catch_unwind(|| program("yaar-claude-hook"));
            if let Ok(built) = built
                && std::fs::copy(built, &copy).is_ok()
            {
                return folder;
            }
            assert!(Instant::now() < deadline, "the hook program is never there");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn keeper(&self) -> JoinHandle<std::io::Result<Exit>> {
        self.keeper_told(Some(self.companions()))
    }

    fn keeper_told(&self, companions: Option<PathBuf>) -> JoinHandle<std::io::Result<Exit>> {
        let options = Options {
            socket: self.socket(),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: self.0.join("data"),
            companions,
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
                return Window::new(stream);
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
        std::fs::write(folder.join("a.txt"), "one\ntwo\nthree\n").expect("write a file");
        git(&folder, &["add", "--all"]);
        git(&folder, &["commit", "--quiet", "-m", "first"]);
        folder
    }

    fn store(&self) -> Store {
        Store::open(&self.database()).expect("open the Keeper's database")
    }

    /// Must succeed and print nothing, whatever became of the report; `terminal` is none for a CLI
    /// outside the app.
    fn hook(&self, terminal: Option<&TerminalId>, payload: &[u8]) -> Duration {
        let mut command = Command::new(self.companions().join(yaar_adapter_claude::HOOK_PROGRAM));
        command
            .env(SOCKET_VARIABLE, self.socket())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        match terminal {
            Some(terminal) => command.env(TERMINAL_VARIABLE, terminal.as_str()),
            None => command.env_remove(TERMINAL_VARIABLE),
        };
        let began = Instant::now();
        let mut child = command.spawn().expect("start the hook program");
        child
            .stdin
            .take()
            .expect("its standard input")
            .write_all(payload)
            .expect("hand it the hook's input");
        let output = child.wait_with_output().expect("wait for the hook program");
        let took = began.elapsed();
        assert!(output.status.success(), "the hook program failed");
        assert!(output.stdout.is_empty(), "the hook program printed");
        took
    }
}

struct Window {
    stream: UnixStream,
    next_id: u32,
    facts: Vec<Fact>,
    looked_at: usize,
    replies: Vec<(RequestId, Result<Reply, Refusal>)>,
    output: Vec<(TerminalId, Vec<u8>)>,
}

impl Window {
    fn new(stream: UnixStream) -> Window {
        Window {
            stream,
            next_id: 0,
            facts: Vec::new(),
            looked_at: 0,
            replies: Vec::new(),
            output: Vec::new(),
        }
    }

    fn send(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_message(&mut self.stream, &Message::Request { id, request }).expect("send");
        id
    }

    fn read(&mut self, waiting_for: &str) {
        match read_message(&mut self.stream) {
            Ok(Message::Fact(fact)) => self.facts.push(fact),
            Ok(Message::Reply { to, reply }) => self.replies.push((to, reply)),
            Ok(Message::Output { terminal, bytes }) => {
                match self.output.iter_mut().find(|(from, _)| *from == terminal) {
                    Some((_, heard)) => heard.extend(bytes),
                    None => self.output.push((terminal, bytes)),
                }
            }
            Ok(other) => panic!("waiting for {waiting_for}, a frame nobody sends: {other:?}"),
            Err(e) => panic!(
                "waiting for {waiting_for}: no frame from the Keeper ({e}); the facts so far: {:#?}",
                self.facts
            ),
        }
    }

    fn ask(&mut self, request: Request) -> Result<Reply, Refusal> {
        let id = self.send(request);
        loop {
            if let Some(at) = self.replies.iter().position(|(to, _)| *to == id) {
                return self.replies.remove(at).1;
            }
            self.read("a reply");
        }
    }

    fn until<T>(&mut self, what: &str, mut wanted: impl FnMut(&Fact) -> Option<T>) -> T {
        loop {
            while self.looked_at < self.facts.len() {
                self.looked_at += 1;
                if let Some(found) = wanted(&self.facts[self.looked_at - 1]) {
                    return found;
                }
            }
            self.read(what);
        }
    }

    fn agents_until(
        &mut self,
        workspace: &WorkspaceId,
        what: &str,
        wanted: impl Fn(&WorkspaceAgents) -> bool,
    ) -> WorkspaceAgents {
        self.until(what, |fact| match fact {
            Fact::AgentsChanged(agents) if agents.workspace == *workspace && wanted(agents) => {
                Some(agents.clone())
            }
            _ => None,
        })
    }

    fn output_until(&mut self, terminal: &TerminalId, text: &str) {
        loop {
            let heard = self.output.iter().find(|(from, _)| from == terminal);
            if heard.is_some_and(|(_, bytes)| String::from_utf8_lossy(bytes).contains(text)) {
                return;
            }
            self.read(text);
        }
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

    fn open_terminal(&mut self, workspace: &WorkspaceId) -> TerminalId {
        let opened = self.ask(Request::OpenTerminal {
            workspace: workspace.clone(),
            program: Program::Shell,
            size: SIZE,
        });
        let Ok(Reply::TerminalOpened { terminal }) = opened else {
            panic!("no terminal: {opened:?}");
        };
        self.type_into(&terminal, b"printf 'ready-%s\\n' now\r");
        self.output_until(&terminal, "ready-now");
        terminal
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

    fn agents(&mut self) -> Vec<WorkspaceAgents> {
        match self.ask(Request::ListAgents) {
            Ok(Reply::Agents(agents)) => agents,
            other => panic!("no list of agents: {other:?}"),
        }
    }

    fn stop(mut self, keeper: JoinHandle<std::io::Result<Exit>>) {
        write_message(&mut self.stream, &Message::Shutdown).expect("ask for the stop");
        assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    }
}

struct Setup {
    scratch: Scratch,
    keeper: JoinHandle<std::io::Result<Exit>>,
    window: Window,
    repo: PathBuf,
    workspace: WorkspaceId,
    terminal: TerminalId,
}

impl Setup {
    fn new() -> Setup {
        let scratch = Scratch::new();
        let repo = scratch.repository("repo");
        Setup::on(scratch, repo)
    }

    fn on(scratch: Scratch, repo: PathBuf) -> Setup {
        let keeper = scratch.keeper();
        let mut window = scratch.window();
        let workspace = window.open(&repo);
        let terminal = window.open_terminal(&workspace);
        Setup {
            scratch,
            keeper,
            window,
            repo,
            workspace,
            terminal,
        }
    }

    fn hook(&self, record: &HookRecord) -> Duration {
        let record = record.in_project(&self.repo);
        self.scratch
            .hook(Some(&self.terminal), &record.input_bytes())
    }

    fn hook_in(&self, terminal: &TerminalId, record: &HookRecord) -> Duration {
        let record = record.in_project(&self.repo);
        self.scratch.hook(Some(terminal), &record.input_bytes())
    }

    fn hooks(&self, records: &[HookRecord], lines: &[usize]) {
        for line in lines {
            self.hook(&records[*line]);
        }
    }

    fn agents_until(
        &mut self,
        what: &str,
        wanted: impl Fn(&WorkspaceAgents) -> bool,
    ) -> WorkspaceAgents {
        self.window.agents_until(&self.workspace, what, wanted)
    }

    fn print_the_mark(&mut self) {
        self.print_the_mark_in(PRINT_THE_MARK);
    }

    fn print_the_mark_form(&mut self, form: &str) {
        let line = format!("printf '{form}'; printf 'after-%s\\n' mark\r");
        self.print_the_mark_in(line.as_bytes());
    }

    fn print_the_mark_in(&mut self, line: &[u8]) {
        self.window.type_into(&self.terminal, line);
        self.window.output_until(&self.terminal, AFTER_THE_MARK);
        self.window.type_into(&self.terminal, PRINT_THE_NEXT);
        self.window.output_until(&self.terminal, THEN);
    }

    fn finish(self) -> (Store, Scratch) {
        self.window.stop(self.keeper);
        let store = self.scratch.store();
        (store, self.scratch)
    }
}

fn assert_the_plugin_runs(data: &Path, hook: &Path) {
    let plugin = data
        .join("agents")
        .join(yaar_adapter_claude::NAME)
        .join("plugin");
    let read = |file: &str| -> serde_json::Value {
        let text = std::fs::read_to_string(plugin.join(file))
            .unwrap_or_else(|e| panic!("the plugin has no {file}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file} is not JSON: {e}"))
    };
    assert_eq!(
        read(".claude-plugin/plugin.json")["name"],
        yaar_adapter_claude::PLUGIN
    );
    let hooks = read("hooks/hooks.json");
    for event in yaar_adapter_claude::HOOK_EVENTS {
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["command"],
            format!("exec '{}'", hook.display()),
            "the hook {event}"
        );
    }
}

#[test]
fn the_plugin_of_claude_code_is_written_at_every_start_to_run_the_hook_program_where_told() {
    let scratch = Scratch::new();
    let hook = scratch.companions().join(yaar_adapter_claude::HOOK_PROGRAM);
    let data = scratch.0.join("data");
    let plugin = data
        .join("agents")
        .join(yaar_adapter_claude::NAME)
        .join("plugin");

    let keeper = scratch.keeper();
    let window = scratch.window();
    assert_the_plugin_runs(&data, &hook);
    window.stop(keeper);

    // A start writes it again, whether it is gone or was changed.
    std::fs::remove_dir_all(&plugin).expect("remove the plugin");
    let keeper = scratch.keeper();
    let window = scratch.window();
    assert_the_plugin_runs(&data, &hook);
    window.stop(keeper);
    std::fs::write(plugin.join("hooks/hooks.json"), "{}").expect("change a file");
    let keeper = scratch.keeper();
    let window = scratch.window();
    assert_the_plugin_runs(&data, &hook);
    window.stop(keeper);
}

#[test]
fn with_no_folder_told_the_pieces_are_looked_for_beside_the_keepers_own_program() {
    let scratch = Scratch::new();
    let beside = std::env::current_exe()
        .expect("this program's path")
        .parent()
        .expect("its folder")
        .join(yaar_adapter_claude::HOOK_PROGRAM);
    let keeper = scratch.keeper_told(None);
    let window = scratch.window();
    assert_the_plugin_runs(&scratch.0.join("data"), &beside);
    window.stop(keeper);
}

#[test]
fn nothing_waits_for_the_recording_of_a_turns_state() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    // A state is recorded through the repository's filters: a slow clean filter makes the recording
    // slow, so what must not wait for it is shown to run during it.
    const RECORDING: Duration = Duration::from_secs(3);
    git(&repo, &["config", "filter.slow.clean", "sleep 3; cat"]);
    std::fs::write(repo.join(".gitattributes"), "slow.txt filter=slow\n").expect("write it");
    git(&repo, &["add", ".gitattributes"]);
    git(&repo, &["commit", "--quiet", "-m", "attributes"]);
    let mut setup = Setup::on(scratch, repo);
    let records = recorded::hook_events(TOOLS);

    setup.hooks(&records, &[1, 2]);
    setup.agents_until("the turn works", is_state(SessionState::Works));
    std::fs::write(setup.repo.join("slow.txt"), "slow\n").expect("write a file");
    let stopped = Instant::now();
    setup.hook(&records[32]);
    setup.agents_until("the turn is unreviewed", is_state(SessionState::Unreviewed));

    let took = setup.hook(&records[33]);
    assert!(
        took < HOOK_DEADLINE,
        "the hook waited {took:?}: a report is answered before anything it leads to"
    );
    setup.agents_until("the next turn works", is_state(SessionState::Works));
    assert!(
        stopped.elapsed() < RECORDING,
        "the next turn worked only {:?} after the stop: it waited for the recording",
        stopped.elapsed()
    );
    let listed = Instant::now();
    assert!(!setup.window.agents().is_empty());
    assert!(listed.elapsed() < HOOK_DEADLINE, "the list waited");
    assert!(
        setup
            .window
            .facts
            .iter()
            .all(|fact| !matches!(fact, Fact::TurnEnded { .. })),
        "the turn's end was told before the recording could have ended"
    );

    let state = setup
        .window
        .until("the end of the first turn", |fact| match fact {
            Fact::TurnEnded { state, .. } => Some(state.clone()),
            _ => None,
        });
    assert!(state.is_some());
    assert!(
        stopped.elapsed() >= RECORDING,
        "the recording took {:?}: the filter was not run",
        stopped.elapsed()
    );
    let _ = setup.finish();
}

#[test]
fn a_stop_followed_at_once_by_the_mark_ends_the_turn_finished_and_the_mark_changes_nothing() {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1, 2]);
    let working = setup.agents_until("the turn works", is_state(SessionState::Works));
    let turn = working.sessions[0].turn.clone().expect("the open turn");

    // The hook has returned: its report is on the queue, and the mark that follows is behind it.
    setup.hook(&records[32]);
    setup.print_the_mark();
    // Another turn begins after the mark: when it is told, the mark was worked out.
    setup.hook(&records[33]);
    let next = setup.agents_until("the next turn works", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state == SessionState::Works && s.turn.as_ref() != Some(&turn))
    });
    let second = next.sessions[0].turn.clone().expect("the second turn");

    let (store, _scratch) = setup.finish();
    let (first, _) = store
        .turn(&TurnId::new(turn.as_str()))
        .expect("read")
        .expect("the first turn");
    assert_eq!(first.ended, Some(TurnEndRecord::Finished));
    let (second, _) = store
        .turn(&TurnId::new(second.as_str()))
        .expect("read")
        .expect("the second turn");
    assert_eq!(
        second.ended, None,
        "the mark ended the turn that came after"
    );
}

#[test]
fn the_mark_alone_during_a_turn_ends_it_interrupted() {
    for form in [FIRST_FORM, LAST_FORM] {
        a_mark_alone_ends_the_turn_interrupted(form);
    }
}

fn a_mark_alone_ends_the_turn_interrupted(form: &str) {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1, 2]);
    let working = setup.agents_until("the turn works", is_state(SessionState::Works));
    let turn = working.sessions[0].turn.clone().expect("the open turn");

    setup.print_the_mark_form(form);
    let ended = setup.agents_until("the turn is unreviewed", is_state(SessionState::Unreviewed));
    assert_eq!(ended.shown, Some(SessionState::Unreviewed));
    assert_eq!(ended.sessions[0].turn, None, "no turn is open any more");
    setup
        .window
        .until("the end of the turn", |fact| match fact {
            Fact::TurnEnded { turn: ended, .. } if *ended == turn => Some(()),
            _ => None,
        });

    let (store, _scratch) = setup.finish();
    let (kept, _) = store
        .turn(&TurnId::new(turn.as_str()))
        .expect("read")
        .expect("the turn");
    assert_eq!(kept.ended, Some(TurnEndRecord::Interrupted));
}

#[test]
fn the_steps_of_a_turn_are_told_in_order_with_paths_in_the_worktree_and_are_kept() {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1, 2, 3, 4, 9, 10, 12, 13, 15, 16, 32]);
    let ended = setup
        .window
        .until("the end of the turn", |fact| match fact {
            Fact::TurnEnded { turn, state, .. } => Some((turn.clone(), state.clone())),
            _ => None,
        });
    assert!(ended.1.is_some(), "a state is recorded");
    let steps: Vec<(u32, Step)> = setup
        .window
        .facts
        .iter()
        .filter_map(|fact| match fact {
            Fact::Step {
                turn, number, step, ..
            } if *turn == ended.0 => Some((*number, step.clone())),
            _ => None,
        })
        .collect();

    let numbers: Vec<u32> = steps.iter().map(|(number, _)| *number).collect();
    assert_eq!(numbers, [1, 2, 3, 4]);
    let command = records[15].input["tool_input"]["command"]
        .as_str()
        .expect("the shell command");
    let path_of = |line: usize| {
        let file = records[line].input["tool_input"]["file_path"]
            .as_str()
            .expect("the file of the tool");
        Path::new(file)
            .strip_prefix(recorded::PROJECT)
            .expect("a file of the project")
            .to_owned()
    };
    assert_eq!(steps[0].1, Step::Read { path: path_of(4) });
    let Step::Edit { path, lines } = &steps[1].1 else {
        panic!("not an edit: {:?}", steps[1].1);
    };
    assert_eq!(*path, path_of(10));
    assert!(
        lines.as_ref().is_some_and(|lines| !lines.is_empty()),
        "the edit names the lines it changed"
    );
    assert_eq!(steps[2].1, Step::Write { path: path_of(13) });
    assert_eq!(
        steps[3].1,
        Step::Command {
            command: command.to_owned()
        }
    );

    let (store, _scratch) = setup.finish();
    let (_, kept) = store
        .turn(&TurnId::new(ended.0.as_str()))
        .expect("read")
        .expect("the turn");
    assert_eq!(kept.len(), 4);
    assert_eq!(kept[0], StepRecord::Read(path_of(4)));
    assert_eq!(kept[2], StepRecord::Write(path_of(13)));
    assert_eq!(kept[3], StepRecord::Command(command.to_owned()));
    let StepRecord::Edit(path, kept_lines) = &kept[1] else {
        panic!("not an edit: {:?}", kept[1]);
    };
    assert_eq!((path, kept_lines), (&path_of(10), lines));
}

#[test]
fn an_unreadable_payload_is_answered_done_and_what_follows_it_is_still_followed() {
    let mut setup = Setup::new();
    let garbled = AgentReport {
        agent: "claude".to_owned(),
        terminal: Some(setup.terminal.clone()),
        pid: std::process::id(),
        payload: b"this is not what Claude Code hands a hook".to_vec(),
    };
    let mut piece = setup.scratch.window();
    assert_eq!(
        piece.ask(Request::Report { report: garbled }),
        Ok(Reply::Done)
    );

    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1]);
    let idle = setup.agents_until("the session is idle", is_state(SessionState::Idle));
    assert_eq!(idle.sessions.len(), 1);
    let _ = setup.finish();
}

#[test]
fn a_report_for_an_agent_the_keeper_does_not_have_is_refused() {
    let scratch = Scratch::new();
    let repo = scratch.repository("repo");
    let keeper = scratch.keeper();
    let mut window = scratch.window();
    window.open(&repo);
    let strange = AgentReport {
        agent: "no-such-agent".to_owned(),
        terminal: None,
        pid: std::process::id(),
        payload: Vec::new(),
    };
    let refused = scratch.window().ask(Request::Report { report: strange });
    assert!(
        matches!(refused, Err(Refusal::Failed { .. })),
        "answered {refused:?}"
    );
    window.stop(keeper);
}

#[test]
fn a_report_from_outside_the_apps_terminals_is_answered_and_followed_by_its_folder() {
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    let outside = dialogs[0].in_project(&setup.repo);
    setup.scratch.hook(None, &outside.input_bytes());
    let told = setup.agents_until("the session of the CLI outside the app", |agents| {
        !agents.sessions.is_empty()
    });
    assert_eq!(told.sessions.len(), 1);
    assert_eq!(told.sessions[0].workspace, setup.workspace);
    assert_eq!(told.sessions[0].terminal, None, "it runs in no terminal");
    assert!(
        told.sessions[0].review.is_some(),
        "it holds a review as any session does"
    );

    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1]);
    let running = setup.terminal.clone();
    setup.agents_until("the session of the app's terminal", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.terminal.as_ref() == Some(&running))
    });
    let listed = setup.window.agents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sessions.len(), 2, "{listed:?}");
    let mut terminals: Vec<_> = listed[0].sessions.iter().map(|s| &s.terminal).collect();
    terminals.sort();
    assert_eq!(terminals, [&None, &Some(setup.terminal.clone())]);
    let _ = setup.finish();
}

#[test]
fn a_report_from_outside_the_apps_terminals_whose_folder_no_workspace_holds_is_followed_no_further()
{
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    let elsewhere = setup.scratch.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("make the folder");
    let outside = dialogs[0].in_project(&elsewhere);
    setup.scratch.hook(None, &outside.input_bytes());
    setup.scratch.hook(
        Some(&TerminalId::new("not-a-terminal")),
        &outside.input_bytes(),
    );
    // Reports are worked out in the order they came, so when this one is told the first two were
    // passed over.
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1]);
    setup.agents_until("the session is idle", is_state(SessionState::Idle));

    let listed = setup.window.agents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sessions.len(), 1, "only the session of the app");
    assert_eq!(listed[0].sessions[0].terminal, Some(setup.terminal.clone()));
    let _ = setup.finish();
}

#[test]
fn a_session_started_in_a_terminal_of_one_workspace_in_the_folder_of_another_belongs_to_the_other()
{
    let mut setup = Setup::new();
    let other_repo = setup.scratch.repository("other");
    let other = setup.window.open(&other_repo);
    let records = recorded::hook_events(TOOLS);

    let record = records[1].in_project(&other_repo);
    setup
        .scratch
        .hook(Some(&setup.terminal), &record.input_bytes());
    let told = setup
        .window
        .agents_until(&other, "the session in the other workspace", |agents| {
            !agents.sessions.is_empty()
        });
    assert_eq!(told.sessions[0].workspace, other);
    assert_eq!(told.sessions[0].terminal, Some(setup.terminal.clone()));

    let listed = setup.window.agents();
    assert!(
        listed.iter().all(|agents| agents.workspace == other),
        "the workspace of the terminal has no session: {listed:?}"
    );
    let _ = setup.finish();
}

#[test]
fn a_session_in_a_folder_inside_a_worktree_belongs_to_the_deepest_workspace_that_holds_it() {
    let mut setup = Setup::new();
    let inner_path = setup.repo.join("inner");
    git(
        &setup.repo,
        &["worktree", "add", "--quiet", "-b", "inner", "inner"],
    );
    let inner = setup.window.open(&inner_path);
    let records = recorded::hook_events(TOOLS);

    let below = inner_path.join("sub");
    std::fs::create_dir(&below).expect("make the folder");
    let record = records[1].in_project(&below);
    setup
        .scratch
        .hook(Some(&setup.terminal), &record.input_bytes());
    setup
        .window
        .agents_until(&inner, "the session in the inner workspace", |agents| {
            !agents.sessions.is_empty()
        });
    let listed = setup.window.agents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].workspace, inner);
    let _ = setup.finish();
}

#[test]
fn a_session_in_a_folder_that_holds_worktrees_but_is_in_none_belongs_to_the_workspace_of_its_terminal()
 {
    let mut setup = Setup::new();
    let inner_path = setup.repo.join("inner");
    git(
        &setup.repo,
        &["worktree", "add", "--quiet", "-b", "inner", "inner"],
    );
    setup.window.open(&inner_path);
    let records = recorded::hook_events(TOOLS);

    let record = records[1].in_project(&setup.scratch.0);
    setup
        .scratch
        .hook(Some(&setup.terminal), &record.input_bytes());
    let told = setup.window.agents_until(
        &setup.workspace,
        "the session in its terminal's workspace",
        |agents| !agents.sessions.is_empty(),
    );
    assert_eq!(told.sessions[0].terminal, Some(setup.terminal.clone()));
    let listed = setup.window.agents();
    assert_eq!(
        listed.len(),
        1,
        "no session in the inner workspace: {listed:?}"
    );
    assert_eq!(listed[0].workspace, setup.workspace);
    let _ = setup.finish();
}

#[test]
fn a_report_from_a_terminal_the_keeper_does_not_hold_is_followed_by_its_folder() {
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    let records = recorded::hook_events(TOOLS);

    setup.hook_in(&TerminalId::new("not-a-terminal"), &dialogs[0]);
    let first = setup.agents_until("the session of the terminal that is not held", |agents| {
        !agents.sessions.is_empty()
    });
    assert_eq!(first.sessions.len(), 1);
    assert_eq!(first.sessions[0].workspace, setup.workspace);
    assert_eq!(first.sessions[0].terminal, None, "none the app knows");

    let ended = setup.window.open_terminal(&setup.workspace);
    setup.window.type_into(&ended, b"exit\r");
    setup
        .window
        .until("the end of the terminal", |fact| match fact {
            Fact::TerminalEnded { terminal, .. } if *terminal == ended => Some(()),
            _ => None,
        });
    setup.hook_in(&ended, &dialogs[0]);

    // A report from the terminal that runs, after both: when it is told, both were taken.
    setup.hooks(&records, &[1]);
    let running = setup.terminal.clone();
    let told = setup.agents_until("the session of the terminal that runs", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.terminal.as_ref() == Some(&running))
    });
    assert_eq!(told.sessions.len(), 2, "{told:?}");
    let held: Vec<_> = told
        .sessions
        .iter()
        .filter(|s| s.terminal.is_some())
        .collect();
    assert_eq!(held.len(), 1, "the other two reports were one session");
    assert_eq!(setup.window.agents()[0].sessions.len(), 2);
    let _ = setup.finish();
}

#[test]
fn the_next_session_in_a_terminal_takes_over_the_review_of_the_one_that_ended() {
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    setup.hooks(&dialogs, &[0]);
    let first = setup.agents_until("the first session", is_state(SessionState::Idle));
    let review = first.sessions[0].review.clone().expect("it has a review");
    assert_eq!(first.sessions[0].title, "Claude Code 1");
    setup.hooks(&dialogs, &[13]);
    let ended = setup.agents_until(
        "the first session ended",
        is_state(SessionState::NotRunning),
    );
    assert_eq!(ended.shown, Some(SessionState::NotRunning));
    assert_eq!(ended.sessions[0].terminal, None, "it runs in no terminal");
    assert_eq!(
        ended.sessions[0].review,
        Some(review.clone()),
        "its review stays"
    );

    setup.hooks(&dialogs, &[14]);
    let second = setup.agents_until("the second session", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state == SessionState::Idle && s.title == "Claude Code 2")
    });
    assert_eq!(
        second.sessions.len(),
        1,
        "the first holds no review any more"
    );
    assert_eq!(second.sessions[0].review, Some(review));
    assert_eq!(second.shown, Some(SessionState::Idle));
    let _ = setup.finish();
}

#[test]
fn the_next_session_in_a_terminal_takes_the_review_of_the_one_that_ended_there_not_of_the_one_that_ended_last()
 {
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    let records = recorded::hook_events(TOOLS);
    let first_terminal = setup.terminal.clone();
    let second_terminal = setup.window.open_terminal(&setup.workspace);

    setup.hook_in(&first_terminal, &dialogs[0]);
    let one = setup.agents_until("the first session", is_state(SessionState::Idle));
    let first_review = one.sessions[0].review.clone().expect("a review");
    setup.hook_in(&second_terminal, &records[1]);
    let both = setup.agents_until("the second session", |agents| agents.sessions.len() == 2);
    let second_review = both.sessions[1].review.clone().expect("a review");
    assert_ne!(first_review, second_review);

    setup.hook_in(&first_terminal, &dialogs[13]);
    setup.agents_until(
        "the first session ended",
        is_state(SessionState::NotRunning),
    );
    setup.window.type_into(&second_terminal, b"exit\r");
    setup.agents_until("both ended", |agents| {
        agents
            .sessions
            .iter()
            .all(|s| s.state == SessionState::NotRunning)
    });

    setup.hook_in(&first_terminal, &dialogs[14]);
    let third = setup.agents_until("the third session", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state == SessionState::Idle && s.title == "Claude Code 3")
    });
    let new = third
        .sessions
        .iter()
        .find(|s| s.title == "Claude Code 3")
        .expect("the third session");
    assert_eq!(new.review, Some(first_review));
    assert!(
        third
            .sessions
            .iter()
            .any(|s| s.review == Some(second_review.clone())),
        "the review of the session that ended last stays with it"
    );
    let _ = setup.finish();
}

#[test]
fn a_session_its_cli_ends_is_kept_as_ended_by_the_cli() {
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    setup.hooks(&dialogs, &[0]);
    let started = setup.agents_until("the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    setup.hooks(&dialogs, &[13]);
    setup.agents_until("the session ended", is_state(SessionState::NotRunning));

    let (store, _scratch) = setup.finish();
    let kept = store
        .session(&yaar_store::SessionId::new(session.as_str()))
        .expect("read")
        .expect("the session");
    assert!(!kept.running);
    assert_eq!(kept.ended_by, Some(EndedBy::Cli));
}

#[test]
fn a_session_whose_terminal_ends_is_not_running_and_is_kept_as_gone_with_it() {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1, 2]);
    let working = setup.agents_until("the turn works", is_state(SessionState::Works));
    let session = working.sessions[0].id.clone();
    let turn = working.sessions[0].turn.clone().expect("the open turn");

    setup.window.type_into(&setup.terminal, b"exit\r");
    let gone = setup.agents_until(
        "the session is not running",
        is_state(SessionState::NotRunning),
    );
    assert_eq!(gone.sessions[0].terminal, None);
    assert_eq!(gone.sessions[0].turn, None, "its turn ended with it");

    let (store, _scratch) = setup.finish();
    let kept = store
        .session(&yaar_store::SessionId::new(session.as_str()))
        .expect("read")
        .expect("the session");
    assert_eq!(kept.ended_by, Some(EndedBy::ProcessGone));
    let (turn, _) = store
        .turn(&TurnId::new(turn.as_str()))
        .expect("read")
        .expect("the turn");
    assert_eq!(turn.ended, Some(TurnEndRecord::Interrupted));
}

#[test]
fn a_session_whose_process_is_gone_is_not_running_within_the_watch() {
    // The Keeper looks for gone processes every 250 ms: 2 s is eight watches, far under the 20 s
    // deadline that would let a slower watch pass.
    const WITHIN_THE_WATCH: Duration = Duration::from_secs(2);
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    let mut cli = Process(
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .spawn()
            .expect("start a process"),
    );
    let report = AgentReport {
        agent: "claude".to_owned(),
        terminal: Some(setup.terminal.clone()),
        pid: cli.0.id(),
        payload: records[1].in_project(&setup.repo).input_bytes(),
    };
    let mut piece = setup.scratch.window();
    assert_eq!(piece.ask(Request::Report { report }), Ok(Reply::Done));
    let started = setup.agents_until("the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    assert!(
        setup
            .window
            .facts
            .iter()
            .all(|fact| !matches!(fact, Fact::AgentsChanged(a) if a.shown == Some(SessionState::NotRunning))),
        "the session did not run for a moment only"
    );

    cli.end();
    let gone = Instant::now();
    setup.agents_until(
        "the session is not running",
        is_state(SessionState::NotRunning),
    );
    assert!(
        gone.elapsed() < WITHIN_THE_WATCH,
        "the session was not running {:?} after its process ended",
        gone.elapsed()
    );

    let (store, _scratch) = setup.finish();
    let kept = store
        .session(&yaar_store::SessionId::new(session.as_str()))
        .expect("read")
        .expect("the session");
    assert_eq!(kept.ended_by, Some(EndedBy::ProcessGone));
}

#[test]
fn marking_the_review_reviewed_turns_the_unreviewed_session_idle() {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1, 2, 32]);
    let finished = setup.agents_until("the turn is unreviewed", is_state(SessionState::Unreviewed));
    let review = finished.sessions[0].review.clone().expect("a review");
    setup.window.until("the end of the turn", |fact| {
        matches!(fact, Fact::TurnEnded { .. }).then_some(())
    });

    assert_eq!(
        setup.window.ask(Request::MarkReviewed { review }),
        Ok(Reply::Done)
    );
    let idle = setup.agents_until("the session is idle", is_state(SessionState::Idle));
    assert_eq!(idle.shown, Some(SessionState::Idle));
    let _ = setup.finish();
}

#[test]
fn a_keeper_started_on_a_store_with_a_running_session_marks_it_ended_by_keeper_stopped() {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1]);
    let started = setup.agents_until("the session", is_state(SessionState::Idle));
    let session = started.sessions[0].id.clone();
    let scratch = setup.scratch;
    setup.window.stop(setup.keeper);
    let before = scratch
        .store()
        .session(&yaar_store::SessionId::new(session.as_str()))
        .expect("read")
        .expect("the session");
    assert!(before.running);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let listed = window.agents();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sessions.len(), 1);
    assert_eq!(listed[0].sessions[0].id, session);
    assert_eq!(listed[0].sessions[0].state, SessionState::NotRunning);
    assert_eq!(listed[0].sessions[0].terminal, None);
    window.stop(keeper);
    let after = scratch
        .store()
        .session(&yaar_store::SessionId::new(session.as_str()))
        .expect("read")
        .expect("the session");
    assert!(!after.running);
    assert_eq!(after.ended_by, Some(EndedBy::KeeperStopped));
}

#[test]
fn a_session_its_cli_resumes_after_a_restart_keeps_its_id_and_its_review() {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1]);
    let started = setup.agents_until("the session", is_state(SessionState::Idle));
    let (session, review) = (
        started.sessions[0].id.clone(),
        started.sessions[0].review.clone().expect("a review"),
    );
    let (scratch, repo, workspace) = (setup.scratch, setup.repo, setup.workspace);
    setup.window.stop(setup.keeper);

    let keeper = scratch.keeper();
    let mut window = scratch.window();
    let terminal = window.open_terminal(&workspace);
    let record = records[1].in_project(&repo);
    scratch.hook(Some(&terminal), &record.input_bytes());
    let resumed = window.agents_until(
        &workspace,
        "the session runs again",
        is_state(SessionState::Idle),
    );
    assert_eq!(resumed.sessions.len(), 1);
    assert_eq!(resumed.sessions[0].id, session);
    assert_eq!(resumed.sessions[0].review, Some(review));
    assert_eq!(resumed.sessions[0].terminal, Some(terminal));
    window.stop(keeper);
}

#[test]
fn a_session_that_ended_with_an_unreviewed_turn_is_shown_unreviewed_with_its_review_when_its_cli_resumes_it()
 {
    let mut setup = Setup::new();
    let records = recorded::hook_events(TOOLS);
    setup.hooks(&records, &[1, 2, 32]);
    let finished = setup.agents_until("the turn is unreviewed", is_state(SessionState::Unreviewed));
    let (session, review) = (
        finished.sessions[0].id.clone(),
        finished.sessions[0].review.clone().expect("a review"),
    );
    setup.window.until("the end of the turn", |fact| {
        matches!(fact, Fact::TurnEnded { .. }).then_some(())
    });
    setup.hook(&records[35]);
    let ended = setup.agents_until("the session ended", is_state(SessionState::NotRunning));
    assert_eq!(ended.sessions[0].review, Some(review.clone()));

    setup.hook(&records[1]);
    let resumed = setup.agents_until("the session runs again", |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state != SessionState::NotRunning)
    });
    assert_eq!(resumed.shown, Some(SessionState::Unreviewed));
    assert_eq!(resumed.sessions.len(), 1);
    assert_eq!(resumed.sessions[0].id, session);
    assert_eq!(resumed.sessions[0].state, SessionState::Unreviewed);
    assert_eq!(resumed.sessions[0].review, Some(review.clone()));
    assert_eq!(
        setup.window.agents()[0].shown,
        Some(SessionState::Unreviewed)
    );

    assert_eq!(
        setup.window.ask(Request::MarkReviewed { review }),
        Ok(Reply::Done)
    );
    let idle = setup.agents_until("the session is idle", is_state(SessionState::Idle));
    assert_eq!(idle.shown, Some(SessionState::Idle));
    let _ = setup.finish();
}

#[test]
fn a_bash_command_denied_at_its_dialog_leaves_no_command_window_open() {
    let mut setup = Setup::new();
    let dialogs = recorded::hook_events(DIALOGS);
    setup
        .window
        .ask(Request::ShowWorkspace {
            workspace: Some(setup.workspace.clone()),
        })
        .expect("the workspace is on screen");
    setup.hooks(&dialogs, &[0, 10, 11, 12]);
    setup.agents_until("the dialog", is_state(SessionState::WaitsForPermission));
    setup.print_the_mark();
    setup.agents_until("the turn is over", is_state(SessionState::Unreviewed));

    // The file below is made by hand once the command's window could have closed, reckoned in the
    // ledger's settle time: the clock the rule is written in, not a fact to wait for.
    std::thread::sleep(2 * yaar_workspaces::SETTLE);
    // The watcher's fact for the file comes after what the Keeper told so far.
    setup.window.ask(Request::ListAgents).expect("the list");
    setup.window.looked_at = setup.window.facts.len();
    std::fs::write(setup.repo.join("by-hand.txt"), "typed\n").expect("write a file");
    // The change is judged once the watcher has seen it: its fact says so.
    let workspace = setup.workspace.clone();
    setup
        .window
        .until("the watcher saw the file", |fact| match fact {
            Fact::ChangesChanged { workspace: seen } if *seen == workspace => Some(()),
            _ => None,
        });
    let deadline = Instant::now() + DEADLINE;
    let credited = loop {
        let listed = setup.window.ask(Request::GetUncommitted {
            workspace: setup.workspace.clone(),
            review: None,
        });
        let Ok(Reply::Uncommitted(files)) = listed else {
            panic!("no uncommitted changes: {listed:?}");
        };
        if let Some(file) = files
            .into_iter()
            .find(|f| f.file.path == Path::new("by-hand.txt"))
        {
            break file;
        }
        assert!(Instant::now() < deadline, "the file was never listed");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        credited.by.is_empty(),
        "a hand-made file was credited to {:?}",
        credited.by
    );
    let _ = setup.finish();
}
