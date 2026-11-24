//! System tests of what the Keeper holds for an agent while its Window is away or stuck.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use yaar_link::{
    HistoryEntry, SessionState, Step as Taken, TerminalId, TerminalSize, TurnEnd, WorkspaceId,
};
use yaar_system_tests::standin::{Did, PROMPT, Script};
use yaar_system_tests::{Client, DEADLINE, Dialect, World, descendants_of, is_alive, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

const READS: usize = 100;

/// A bound on a wait, not a measure: two hundred hooks, each a process, on a busy machine.
const TURN_DEADLINE: Duration = Duration::from_secs(60);

const FLOOD_LINES: u32 = 200_000;

const FLOOD_DEADLINE: Duration = Duration::from_secs(120);

/// How much longer than a client that reads everything a stopped client may keep the agent's
/// longest write waiting.
const ALLOWED: Duration = Duration::from_millis(100);

/// How many times the pair of runs is measured before the test fails: the agent's longest write is
/// a time, and a busy machine makes one run's longest longer than another's by itself.
const ATTEMPTS: usize = 3;

fn script(words: &[String]) -> Script {
    Script::parse(&words.join("\n"))
}

struct Started {
    name: String,
}

impl Started {
    fn log(&self, world: &World) -> yaar_system_tests::standin::StandinLog {
        world.agent_log(&self.name)
    }
}

fn start_in(
    dialect: Dialect,
    world: &World,
    client: &mut Client,
    terminal: &TerminalId,
    name: &str,
    script: &Script,
) -> Started {
    let script_file = world.agent_script(name, script);
    let log_file = script_file.with_extension("log");
    client.type_line(
        terminal,
        &format!(
            "STANDIN_SCRIPT='{}' STANDIN_LOG='{}' {}",
            script_file.display(),
            log_file.display(),
            dialect.name()
        ),
    );
    Started { name: name.into() }
}

fn at_prompt_after_a_turn(bytes: &[u8]) -> bool {
    let tail = &bytes[bytes.len().saturating_sub(4 * 1024)..];
    let tail = String::from_utf8_lossy(tail);
    tail.rfind("● Done")
        .is_some_and(|done| tail[done..].contains(PROMPT))
}

fn end(world: World, client: Client) {
    drop(client);
    leave(world);
}

fn leave(mut world: World) {
    let keeper = world.keeper_pid().expect("the Keeper runs");
    let started = descendants_of(keeper);
    drop(world);
    assert!(
        wait_until(DEADLINE, || started.iter().all(|pid| !is_alive(*pid))),
        "still there: {started:?}"
    );
}

#[test]
fn a_client_that_leaves_while_the_agent_works_finds_the_turn_its_state_the_screen_and_every_step_in_order_when_it_comes_back()
 {
    a_client_that_leaves_while_the_agent_works_finds_the_turn_when_it_comes_back(
        Dialect::ClaudeCode,
    );
}

#[test]
fn a_client_that_leaves_while_the_agent_works_finds_the_turn_its_state_the_screen_and_every_step_in_order_when_it_comes_back_as_pi()
 {
    a_client_that_leaves_while_the_agent_works_finds_the_turn_when_it_comes_back(Dialect::Pi);
}

fn a_client_that_leaves_while_the_agent_works_finds_the_turn_when_it_comes_back(dialect: Dialect) {
    // The file that lets the agent go on is made outside the worktree, so making it changes nothing
    // in it.
    let (world, repo) = World::with_standin_as(dialect, "default", &Script(vec![]));
    let go = world.home().with_file_name("go");
    let named: Vec<String> = (1..=READS).map(|k| format!("f{k:03}.txt")).collect();
    for file in &named {
        repo.write(file, &format!("{file}\n"));
    }
    let head = repo.commit("Add the files the agent reads");
    // An order neither by name nor its reverse; 37 and 100 share no factor, so each file is read
    // once.
    let files: Vec<String> = (0..READS).map(|k| named[k * 37 % READS].clone()).collect();
    assert_ne!(files, named, "the reads are in the order of their names");
    let mut steps = vec!["turn".to_owned(), format!("hold {}", go.display())];
    steps.extend(files.iter().map(|file| format!("read {file}")));
    steps.extend(["end".to_owned(), "turn".to_owned()]);
    let end_step = 2 + READS;
    world.agent_script("default", &script(&steps));

    let mut client = world.client();
    let workspace = client.open_folder(repo.path()).expect("open the folder").1;
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    client.type_line(&terminal, dialect.name());
    client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "read them all");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });

    client.vanish();

    repo.write("late.txt", "written while the agent works\n");

    let began = Instant::now();
    fs::write(&go, "").expect("let the agent go on");
    let agent = Started {
        name: "default".into(),
    };
    assert!(
        wait_until(TURN_DEADLINE, || agent.log(&world).did.iter().any(
            |did| matches!(did, Did::Done { step, .. } if *step == end_step)
        )),
        "the agent did not finish its turn: {:?}",
        agent.log(&world).did.last()
    );
    println!(
        "the agent finished its turn of {READS} reads in {:?}",
        began.elapsed()
    );

    let mut later = world.client();
    let shown = |later: &mut Client| {
        later
            .agents()
            .into_iter()
            .find(|agents| agents.workspace == workspace)
            .and_then(|agents| agents.shown)
    };
    assert!(
        wait_until(DEADLINE, || shown(&mut later)
            == Some(SessionState::Unreviewed)),
        "a client that connects later finds {:?}",
        shown(&mut later)
    );
    let listed = later.agents();
    let sessions = &listed
        .iter()
        .find(|agents| agents.workspace == workspace)
        .expect("the workspace's agents")
        .sessions;
    let [session] = &sessions[..] else {
        panic!("not exactly one session: {sessions:?}");
    };
    assert_eq!(session.state, SessionState::Unreviewed);
    let recorded = turns_with_state(&mut later, &workspace);
    let [(turn, session_of_turn, state)] = &recorded[..] else {
        panic!("not exactly one turn with a state in the history: {recorded:?}");
    };
    assert_eq!(*session_of_turn, session.id);
    assert_eq!(session.agent, dialect.name());

    assert_eq!(
        later.compare(&workspace, state, None).expect("compare"),
        vec![]
    );
    let since_head = later
        .compare(&workspace, state, Some(&head))
        .expect("compare with the commit");
    assert_eq!(
        since_head
            .iter()
            .map(|change| change.path.as_path())
            .collect::<Vec<_>>(),
        [std::path::Path::new("late.txt")]
    );

    let kept = later.turn(turn).expect("the turn is kept");
    assert_eq!(kept.ended, Some(TurnEnd::Finished));
    assert_eq!(kept.state.as_deref(), Some(state.as_str()));
    let read: Vec<Taken> = files
        .iter()
        .map(|file| Taken::Read {
            path: PathBuf::from(file),
        })
        .collect();
    let first_difference = kept
        .steps
        .iter()
        .zip(&read)
        .position(|(kept, read)| kept != read);
    assert_eq!(
        (kept.steps.len(), first_difference),
        (READS, None),
        "the steps differ from the reads at {first_difference:?}"
    );

    let last_read = format!("● Read {}", files.last().expect("reads"));
    let cut = later
        .attach_terminal(&terminal, SIZE)
        .expect("attach to the terminal");
    assert!(!cut, "the kept output of the terminal was cut");
    later.wait_for_output(&terminal, DEADLINE, at_prompt_after_a_turn);
    let screen = later.screen(&terminal, SIZE);
    let last: Vec<&str> = screen
        .iter()
        .map(String::as_str)
        .filter(|line| !line.is_empty())
        .rev()
        .take(3)
        .collect();
    assert_eq!(
        last,
        ["standin>", "● Done", last_read.as_str()],
        "the screen: {screen:?}"
    );
    end(world, later);
}

fn turns_with_state(
    client: &mut Client,
    workspace: &WorkspaceId,
) -> Vec<(yaar_link::TurnId, yaar_link::SessionId, String)> {
    let mut found = Vec::new();
    // The Keeper tells the session finished before the state is recorded: the history holds it
    // later.
    wait_until(DEADLINE, || {
        found = client
            .history(workspace)
            .expect("the history")
            .entries
            .into_iter()
            .filter_map(|entry| match entry {
                HistoryEntry::Turn {
                    turn,
                    session,
                    state: Some(state),
                } => Some((turn, session, state)),
                _ => None,
            })
            .collect();
        !found.is_empty()
    });
    found
}

fn longest_write(world: &World, name: &str) -> Duration {
    let log = world.agent_log(name);
    log.did
        .iter()
        .find_map(|did| match did {
            Did::Flooded { longest_us, .. } => Some(Duration::from_micros(*longest_us)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{name} wrote down no flood: {:?}", log.did))
}

fn flood_run(
    dialect: Dialect,
    world: &World,
    workspace: &WorkspaceId,
    name: &str,
    reading: bool,
) -> Duration {
    let mut client = world.client();
    let terminal = client
        .open_terminal(workspace, SIZE)
        .expect("open a terminal");
    let steps = [
        "turn".to_owned(),
        format!("flood {FLOOD_LINES}"),
        "end".to_owned(),
        "turn".to_owned(),
    ];
    let agent = start_in(
        dialect,
        world,
        &mut client,
        &terminal,
        name,
        &script(&steps),
    );
    client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "flood");
    if reading {
        client.wait_for_output(&terminal, FLOOD_DEADLINE, at_prompt_after_a_turn);
    } else {
        assert!(
            wait_until(FLOOD_DEADLINE, || agent
                .log(world)
                .did
                .iter()
                .any(|did| matches!(did, Did::Flooded { .. }))),
            "the stand-in's flood did not end while the client read nothing"
        );
    }
    drop(client);
    // The stand-in reports its turn's end through its CLI's piece of the app, as pi only when pi
    // found the extension. The report may come after the flood, hence the wait.
    assert!(
        wait_until(DEADLINE, || agent.log(world).did.iter().any(
            |did| matches!(did, Did::Hook { event, .. } if event == dialect.turn_end_event())
        )),
        "{name} reported no end of its turn: {:?}",
        agent.log(world).did
    );
    longest_write(world, name)
}

#[test]
fn a_client_that_is_stopped_does_not_hold_the_agent_up_by_more_than_a_hundred_milliseconds() {
    a_stopped_client_does_not_hold_the_agent_up(Dialect::ClaudeCode);
}

#[test]
fn a_client_that_is_stopped_does_not_hold_the_agent_up_by_more_than_a_hundred_milliseconds_as_pi() {
    a_stopped_client_does_not_hold_the_agent_up(Dialect::Pi);
}

fn a_stopped_client_does_not_hold_the_agent_up(dialect: Dialect) {
    let (world, repo) = World::with_standin_as(dialect, "default", &Script(vec![]));
    let mut setup = world.client();
    let workspace = setup.open_folder(repo.path()).expect("open the folder").1;
    drop(setup);

    // One pair that holds is enough; a Keeper that waits for the stopped client fails every one.
    let mut pairs = Vec::new();
    for attempt in 1..=ATTEMPTS {
        let base = flood_run(
            dialect,
            &world,
            &workspace,
            &format!("base-{attempt}"),
            true,
        );
        let stopped = flood_run(
            dialect,
            &world,
            &workspace,
            &format!("stopped-{attempt}"),
            false,
        );
        println!(
            "attempt {attempt}: longest write {base:?} with a client that reads, {stopped:?} with one that is stopped"
        );
        pairs.push((base, stopped));
        if stopped <= base + ALLOWED {
            break;
        }
    }
    let held = pairs
        .iter()
        .any(|(base, stopped)| *stopped <= *base + ALLOWED);
    assert!(
        held,
        "a stopped client held the agent up by more than {ALLOWED:?} every time: \
         (reading, stopped) {pairs:?}"
    );

    leave(world);
}
