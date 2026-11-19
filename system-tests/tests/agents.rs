//! System tests of the agent: the built Keeper and Dock, the app's own hook program and
//! plugin, and the stand-in agent started in a shell or a launcher of the app, read through
//! the Link as a Window reads it and through the stand-in's own log.
//!
//! The stand-in runs as Claude Code. Its terminal is the app's, so what a test types
//! reaches it as keys, and what it prints is what the Dock keeps.

use std::collections::HashMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use yaar_link::{
    Fact, SessionState, Step as Taken, TerminalId, TerminalSize, TurnEnd, TurnId, WorkspaceAgents,
    WorkspaceId,
};
use yaar_system_tests::standin::{Did, INTERRUPT_MARK, Script};
use yaar_system_tests::{Client, DEADLINE, Repo, World, descendants_of, is_alive, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// The progress sequence that says no progress, which ends a turn at the terminal.
const NO_PROGRESS: &[u8] = b"\x1b]9;4;0;\x07";

fn path(text: &str) -> PathBuf {
    PathBuf::from(text)
}

/// A script of the steps in `words`, each the way `Script::parse` reads it.
fn script(words: &[&str]) -> Script {
    Script::parse(&words.join("\n"))
}

/// Opens the folder of `repo` as a workspace.
fn open(client: &mut Client, repo: &Repo) -> WorkspaceId {
    client.open_folder(repo.path()).expect("open the folder").1
}

/// A repository of `world` with an identity of its own in its configuration, since git on
/// Linux guesses none, holding `files` in its first commit after the one of `repository`.
fn repository_with(world: &World, name: &str, files: &[(&str, &str)]) -> Repo {
    let repo = world.repository(name);
    repo.git(&["config", "user.name", "A Test"]);
    repo.git(&["config", "user.email", "test@example.invalid"]);
    for (file, content) in files {
        repo.write(file, content);
    }
    repo.commit("Add the files of the test");
    repo
}

/// A stand-in the test started in a shell, with the script and log of its own.
struct Started {
    name: String,
}

impl Started {
    /// What the stand-in wrote down so far.
    fn log(&self, world: &World) -> yaar_system_tests::standin::StandinLog {
        world.agent_log(&self.name)
    }

    /// Waits until the stand-in has ended its program.
    fn wait_for_exit(&self, world: &World) {
        assert!(
            wait_until(DEADLINE, || self
                .log(world)
                .did
                .iter()
                .any(|did| matches!(did, Did::Exited { .. }))),
            "{} did not exit: {:?}",
            self.name,
            self.log(world)
        );
    }
}

/// Types into `terminal` the start of the program `program` ("claude", "otheragent") with the
/// script `script` as `name`'s own, and with `env` among its variables as the shell reads
/// them: the stand-in follows the script and writes down its log as `name`.
fn start_in(
    world: &World,
    client: &mut Client,
    terminal: &TerminalId,
    name: &str,
    program: &str,
    env: &str,
    script: &Script,
) -> Started {
    let script_file = world.agent_script(name, script);
    let log_file = script_file.with_extension("log");
    client.type_line(
        terminal,
        &format!(
            "STANDIN_SCRIPT='{}' STANDIN_LOG='{}' {env} {program}",
            script_file.display(),
            log_file.display()
        ),
    );
    Started { name: name.into() }
}

/// Waits for facts until `done` accepts one, and gives every fact seen, that one last.
/// Facts the client holds already are seen first, so the run starts at the first fact no
/// earlier wait has taken.
fn seen_until(client: &mut Client, mut done: impl FnMut(&Fact) -> bool) -> Vec<Fact> {
    let mut seen = Vec::new();
    client.wait_for_fact(DEADLINE, |fact| {
        seen.push(fact.clone());
        done(fact)
    });
    seen
}

/// Whether `fact` is the sessions of `workspace` with the state `state` shown.
fn shows(fact: &Fact, workspace: &WorkspaceId, state: SessionState) -> bool {
    matches!(fact, Fact::AgentsChanged(agents)
        if agents.workspace == *workspace && agents.shown == Some(state))
}

/// Whether `fact` is a turn's end.
fn is_turn_end(fact: &Fact) -> bool {
    matches!(fact, Fact::TurnEnded { .. })
}

/// The one session of `agents`.
fn only_session(agents: &WorkspaceAgents) -> &yaar_link::Session {
    let [session] = &agents.sessions[..] else {
        panic!("not exactly one session: {agents:?}");
    };
    session
}

/// Ends the test the way it must: the client leaves, the Keeper ends with the world, and
/// every process the Keeper started, the stand-ins among them, is gone before the test is
/// over.
fn end(mut world: World, client: Client) {
    let keeper = world.keeper_pid().expect("the Keeper runs");
    let started = descendants_of(keeper);
    drop(client);
    drop(world);
    assert!(
        wait_until(DEADLINE, || started.iter().all(|pid| !is_alive(*pid))),
        "still there: {started:?}"
    );
}

/// The bytes of a file; panics, naming it, when it cannot be read.
fn bytes_of(file: &Path) -> Vec<u8> {
    fs::read(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()))
}

// AGENT-01.
#[test]
fn a_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_is_seen_as_works_then_finished_with_its_steps_in_order_and_a_state_recorded()
 {
    let turn = script(&[
        "turn",
        "read notes.txt",
        "edit notes.txt more",
        "write new.txt hello",
        "shell echo x > shell.txt",
        "end",
        "turn",
        "exit",
    ]);
    let (world, _) = World::with_standin("default", &turn);
    // The repository of the test, with the file the agent reads and edits.
    let repo = repository_with(&world, "notes", &[("notes.txt", "alpha\n")]);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, "claude");
    let prompt = client.wait_for_prompt(&terminal, 0);
    let branch = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let head = repo.git(&["rev-parse", "HEAD"]);
    let index = repo.path().join(".git/index");
    // Read before any `git status` of the test: git refreshes the index there.
    let index_before = bytes_of(&index);
    assert_eq!(branch, "main");

    client.type_line(&terminal, "make the changes");
    let seen = seen_until(&mut client, is_turn_end);

    // Works, then finished.
    let shown: Vec<_> = seen
        .iter()
        .filter_map(|fact| match fact {
            Fact::AgentsChanged(agents) if agents.workspace == workspace => agents.shown,
            _ => None,
        })
        .collect();
    let works = shown.iter().position(|s| *s == SessionState::Works);
    let finished = shown.iter().position(|s| *s == SessionState::Unreviewed);
    assert!(
        works.is_some() && finished.is_some() && works < finished,
        "the states shown: {shown:?}"
    );
    // The turn's end comes after the fact that shows the session unreviewed.
    let last_shown = seen
        .iter()
        .rposition(|fact| shows(fact, &workspace, SessionState::Unreviewed))
        .expect("the finished state was shown");
    assert!(
        last_shown < seen.len() - 1 && is_turn_end(&seen[seen.len() - 1]),
        "the end of the turn comes after the fact that shows it: {seen:?}"
    );

    // The steps, in order, each with its number.
    let steps: Vec<(u32, Taken)> = seen
        .iter()
        .filter_map(|fact| match fact {
            Fact::Step { number, step, .. } => Some((*number, step.clone())),
            _ => None,
        })
        .collect();
    let notes = fs::read_to_string(repo.path().join("notes.txt")).expect("the edited file");
    let last_line = notes.lines().count() as u32;
    assert_eq!(last_line, 2, "{notes:?}");
    assert_eq!(
        steps,
        vec![
            (
                1,
                Taken::Read {
                    path: path("notes.txt")
                }
            ),
            (
                2,
                Taken::Edit {
                    path: path("notes.txt"),
                    lines: Some(vec![RangeInclusive::new(last_line, last_line)]),
                }
            ),
            (
                3,
                Taken::Write {
                    path: path("new.txt")
                }
            ),
            (
                4,
                Taken::Command {
                    command: "echo x > shell.txt".into()
                }
            ),
        ]
    );

    // A state is recorded: the turn's end names it, and it is what the files hold.
    let Some(Fact::TurnEnded { turn, state, .. }) = seen.last() else {
        panic!("the last fact is the turn's end: {seen:?}");
    };
    let state = state
        .clone()
        .expect("a state was recorded at the turn's end");
    assert_eq!(
        client.compare(&workspace, &state, None).expect("compare"),
        vec![]
    );
    let kept = client.turn(turn).expect("the turn is kept");
    assert_eq!(kept.ended, Some(TurnEnd::Finished));
    assert_eq!(kept.state.as_deref(), Some(state.as_str()));
    assert_eq!(
        kept.steps,
        steps.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("shell.txt")).unwrap(),
        "x\n"
    );

    // The branch and the index are as they were.
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(bytes_of(&index), index_before, "the index changed");

    // Let the stand-in end with its own exit.
    client.wait_for_prompt(&terminal, prompt);
    client.type_line(&terminal, "");
    Started {
        name: "default".into(),
    }
    .wait_for_exit(&world);
    end(world, client);
}

// AGENT-04.
#[test]
fn edits_of_a_program_that_is_no_supported_cli_and_of_a_cli_started_around_the_apps_pieces_are_no_session_turn_step_or_notice_and_show_as_changes_no_agent_made()
 {
    let (world, _) = World::with_standin("default", &script(&["turn", "exit"]));
    let repo = repository_with(&world, "edited", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let elsewhere = repository_with(&world, "elsewhere", &[("x.txt", "x\n")]);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let other = open(&mut client, &elsewhere);

    // A program that is neither Claude Code nor pi, which prints the mark too.
    let first = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let unsupported = start_in(
        &world,
        &mut client,
        &first,
        "unsupported",
        "otheragent",
        "",
        &script(&["turn", "edit a.txt x", "mark", "end", "exit"]),
    );
    let after = client.wait_for_prompt(&first, 0);
    client.type_line(&first, "edit a file");
    // A CLI started around the app's pieces.
    let second = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let around = start_in(
        &world,
        &mut client,
        &second,
        "around",
        "claude",
        "STANDIN_IGNORE_PIECES=1",
        &script(&["turn", "edit b.txt y", "end", "exit"]),
    );
    client.wait_for_prompt(&second, 0);
    client.type_line(&second, "edit another");

    // The barrier. Both are over; the mark of the first has reached the terminal, and so
    // the Dock that tells the Keeper of it.
    unsupported.wait_for_exit(&world);
    around.wait_for_exit(&world);
    client.wait_for_output(&first, DEADLINE, |output| {
        output
            .get(after..)
            .is_some_and(|new| new.windows(NO_PROGRESS.len()).any(|w| w == NO_PROGRESS))
    });
    // Then a supported CLI starts in another workspace and is heard: the Keeper takes its
    // report after every report and mark that came before it.
    let third = client.open_terminal(&other, SIZE).expect("a terminal");
    start_in(
        &world,
        &mut client,
        &third,
        "heard",
        "claude",
        "",
        &script(&["turn", "exit"]),
    );
    let seen = seen_until(&mut client, |fact| {
        matches!(fact, Fact::AgentsChanged(agents)
            if agents.workspace == other && !agents.sessions.is_empty())
    });

    // No session, no turn, no step, no notice.
    for fact in &seen {
        match fact {
            Fact::Step { .. } | Fact::TurnEnded { .. } => panic!("{fact:?} among {seen:?}"),
            Fact::AgentsChanged(agents) if agents.workspace == workspace => {
                panic!("{agents:?} among {seen:?}")
            }
            _ => {}
        }
    }
    assert!(
        client
            .agents()
            .iter()
            .all(|agents| agents.workspace != workspace || agents.sessions.is_empty()),
        "{:?}",
        client.agents()
    );
    // The edits are there and no agent made them.
    let uncommitted = client.uncommitted(&workspace, None).expect("uncommitted");
    let mut edited: Vec<_> = uncommitted
        .iter()
        .map(|credited| (credited.file.path.clone(), credited.by.len()))
        .collect();
    edited.sort();
    assert_eq!(edited, vec![(path("a.txt"), 0), (path("b.txt"), 0)]);
    assert_eq!(
        fs::read_to_string(repo.path().join("a.txt")).unwrap(),
        "a\nx\n"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("b.txt")).unwrap(),
        "b\ny\n"
    );

    // End the one that listens.
    client.type_line(&third, "");
    end(world, client);
}

// AGENT-09.
#[test]
fn the_stand_in_started_from_its_launcher_and_by_its_name_gives_the_same_session_and_each_takes_over_the_review_the_last_left()
 {
    // The launcher's stand-in follows the default script: one turn, then it ends.
    let (world, repo) = World::with_standin("default", &script(&["turn", "end", "exit"]));
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let before = client.reviews(&workspace);
    let [no_agent] = &before.reviews[..] else {
        panic!("not exactly one review: {:?}", before.reviews);
    };
    assert_eq!(no_agent.agent, None);
    let review = no_agent.id.clone();

    // From the launcher.
    let launcher = client
        .open_launcher(&workspace, "claude", SIZE)
        .expect("open the launcher");
    client.wait_for_prompt(&launcher, 0);
    let from_launcher = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state == SessionState::Idle)
    });
    let launched = only_session(&from_launcher).clone();
    // The first session takes over the review of no agent.
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0].id, review);
    assert_eq!(reviews[0].agent, Some(launched.id.clone()));
    assert_eq!(launched.review, Some(review.clone()));

    // It ends: its review stays open.
    client.type_line(&launcher, "hello");
    let ended = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.id == launched.id && s.state == SessionState::NotRunning)
    });
    let left = ended.sessions.iter().find(|s| s.id == launched.id).unwrap();
    assert_eq!(left.review, Some(review.clone()));
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0].id, review);
    assert_eq!(reviews[0].agent, Some(launched.id.clone()));

    // By its name typed in a shell.
    let shell = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let typed = start_in(
        &world,
        &mut client,
        &shell,
        "typed",
        "claude",
        "",
        &script(&["turn", "end", "exit"]),
    );
    client.wait_for_prompt(&shell, 0);
    let from_shell = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state == SessionState::Idle && s.id != launched.id)
    });
    let second = from_shell
        .sessions
        .iter()
        .find(|s| s.state == SessionState::Idle)
        .expect("the typed session")
        .clone();

    // The same session either way: its CLI, its own terminal, idle, a review.
    assert_ne!(second.id, launched.id);
    assert_eq!(second.agent, launched.agent);
    assert_eq!(second.agent, "claude");
    assert_eq!(launched.title, "Claude Code 1");
    assert_eq!(second.title, "Claude Code 2");
    assert_eq!(launched.terminal, Some(launcher.clone()));
    assert_eq!(second.terminal, Some(shell.clone()));
    assert_eq!(launched.state, SessionState::Idle);
    assert_eq!(second.state, SessionState::Idle);

    // The next session of that agent took the review over.
    assert_eq!(second.review, Some(review.clone()));
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0].id, review);
    assert_eq!(reviews[0].agent, Some(second.id.clone()));
    // The earlier session no longer holds it.
    let later = client.agents();
    let all = later.iter().find(|a| a.workspace == workspace).unwrap();
    assert!(
        all.sessions
            .iter()
            .filter(|s| s.id == launched.id)
            .all(|s| s.review.is_none()),
        "{all:?}"
    );

    client.type_line(&shell, "bye");
    typed.wait_for_exit(&world);
    end(world, client);
}

// AGENT-16.
#[test]
fn an_agent_that_asks_for_permission_turns_the_waiting_state_on_and_the_app_writes_nothing_to_its_terminal_and_the_keys_of_the_answer_turn_it_off()
 {
    // The hold after the answer keeps the turn open, so that the state the answer gives
    // can be seen on its own, apart from the state the turn's end gives. It waits for a
    // file beside the repository, which the test makes once it has seen that state.
    let asking = script(&["turn", "ask p.txt x", "hold ../go", "end", "turn", "exit"]);
    let (world, repo) = World::with_standin("default", &asking);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, "claude");
    let prompt = client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "write p");

    // The waiting state turns on, and is what the Keeper says just before the answer.
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::WaitsForPermission)
    });
    let listed = client.agents();
    let shown = listed.iter().find(|a| a.workspace == workspace).unwrap();
    assert_eq!(shown.shown, Some(SessionState::WaitsForPermission));
    assert!(!repo.path().join("p.txt").exists());

    // The answer, as keys. It turns the state off while the turn is still open.
    client.type_into(&terminal, b"y");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    let working = client.agents();
    let working = working.iter().find(|a| a.workspace == workspace).unwrap();
    assert_eq!(working.shown, Some(SessionState::Works), "{working:?}");
    fs::write(repo.path().join("../go"), "").expect("let the turn go on");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Unreviewed)
    });
    assert_eq!(fs::read_to_string(repo.path().join("p.txt")).unwrap(), "x");

    // The app wrote nothing to the terminal: from the dialog to the end of the last
    // turn's line the stand-in received what the test typed and nothing else.
    client.wait_for_prompt(&terminal, prompt);
    client.type_line(&terminal, "done");
    let started = Started {
        name: "default".into(),
    };
    started.wait_for_exit(&world);
    let log = started.log(&world);
    let asked = log
        .did
        .iter()
        .find_map(|did| match did {
            Did::Began { step: 1, received } => Some(*received),
            _ => None,
        })
        .expect("the dialog began");
    assert_eq!(&log.received[asked..], b"ydone\r");
    // And its answer to the permission request is none (FR-028).
    let requests: Vec<_> = log
        .did
        .iter()
        .filter_map(|did| match did {
            Did::Hook { event, answer, .. } if event == "PermissionRequest" => Some(answer),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1, "{:?}", log.did);
    assert!(requests[0].is_empty(), "{:?}", requests[0]);
    end(world, client);
}

// LISTEN-03.
#[test]
fn esc_during_a_turn_makes_the_stand_in_print_the_mark_and_report_nothing_and_the_turn_ends_in_the_app()
 {
    let held = script(&["turn", "hold never", "end"]);
    let (world, repo) = World::with_standin("default", &held);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, "claude");
    client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "work");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    client.wait_for_output(&terminal, DEADLINE, |output| {
        String::from_utf8_lossy(output).contains("Hold never")
    });

    client.type_into(&terminal, b"\x1b");
    // The mark Claude Code prints is in the terminal's output.
    client.wait_for_output(&terminal, DEADLINE, |output| {
        output
            .windows(INTERRUPT_MARK.len())
            .any(|window| window == INTERRUPT_MARK)
    });
    // The turn ends in the app.
    let seen = seen_until(&mut client, is_turn_end);
    assert!(
        seen.iter()
            .any(|fact| shows(fact, &workspace, SessionState::Unreviewed)),
        "{seen:?}"
    );
    let Some(Fact::TurnEnded { turn, .. }) = seen.last() else {
        panic!("{seen:?}");
    };
    assert_eq!(
        client.turn(turn).expect("the turn").ended,
        Some(TurnEnd::Interrupted)
    );

    // The stand-in reported nothing after the interrupt: it ran no hook, and its script is
    // over, which is the barrier for what it might still have done.
    let started = Started {
        name: "default".into(),
    };
    assert!(
        wait_until(DEADLINE, || started
            .log(&world)
            .did
            .iter()
            .any(|did| matches!(did, Did::ScriptDone { .. }))),
        "{:?}",
        started.log(&world)
    );
    let did = started.log(&world).did;
    let at = did
        .iter()
        .position(|did| matches!(did, Did::Interrupted { .. }))
        .expect("the stand-in was interrupted");
    assert!(
        did[at..].iter().all(|did| !matches!(did, Did::Hook { .. })),
        "{did:?}"
    );
    end(world, client);
}

// LISTEN-07.
#[test]
fn an_agent_whose_process_is_killed_shows_as_not_running_while_its_terminal_lives() {
    let idle = Script(vec![]);
    let (world, repo) = World::with_standin("default", &idle);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, "claude");
    let seen = seen_until(&mut client, |fact| {
        shows(fact, &workspace, SessionState::Idle)
    });
    assert!(
        seen.iter().all(|fact| !matches!(fact,
            Fact::AgentsChanged(agents)
                if agents.sessions.iter().any(|s| s.state == SessionState::NotRunning))),
        "{seen:?}"
    );

    // Killed, with no goodbye.
    let pid = Started {
        name: "default".into(),
    }
    .log(&world)
    .pid;
    assert_ne!(pid, 0);
    // SAFETY: a signal to a process of this test.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) }, 0);
    let gone = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::NotRunning)
    });
    assert_eq!(only_session(&gone).state, SessionState::NotRunning);
    // The shell it ran in is still there: the end came from the process, not the terminal.
    let terminals = client.terminals();
    assert!(
        terminals.iter().any(|info| info.id == terminal),
        "{terminals:?}"
    );
    end(world, client);
}

// AGENT-20.
#[test]
fn two_sessions_in_one_worktree_each_have_a_review() {
    let (world, repo) = World::with_standin("default", &Script(vec![]));
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let one = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let two = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let turn = script(&["turn", "end", "exit"]);
    let first = start_in(&world, &mut client, &one, "one", "claude", "", &turn);
    client.wait_for_prompt(&one, 0);
    let second = start_in(&world, &mut client, &two, "two", "claude", "", &turn);
    client.wait_for_prompt(&two, 0);

    let both = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.sessions.len() == 2
            && agents
                .sessions
                .iter()
                .all(|s| s.state == SessionState::Idle)
    });
    let by_terminal: HashMap<_, _> = both
        .sessions
        .iter()
        .map(|s| (s.terminal.clone().expect("a terminal"), s))
        .collect();
    let (a, b) = (by_terminal[&one], by_terminal[&two]);
    assert_ne!(a.id, b.id);

    // A review each, whose agent is its session, and none left for no agent.
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 2, "{reviews:?}");
    let review_of = |session: &yaar_link::Session| {
        reviews
            .iter()
            .find(|review| review.agent.as_ref() == Some(&session.id))
            .unwrap_or_else(|| panic!("no review of {session:?} in {reviews:?}"))
            .id
            .clone()
    };
    let (review_a, review_b) = (review_of(a), review_of(b));
    assert_ne!(review_a, review_b);
    assert_eq!(a.review, Some(review_a.clone()));
    assert_eq!(b.review, Some(review_b.clone()));

    // Each takes a turn of its own, which is its session's alone.
    client.type_line(&one, "first");
    client.type_line(&two, "second");
    let mut ended: HashMap<_, TurnId> = HashMap::new();
    while ended.len() < 2 {
        let fact = client.wait_for_fact(DEADLINE, is_turn_end);
        let Fact::TurnEnded { session, turn, .. } = fact else {
            unreachable!()
        };
        ended.insert(session, turn);
    }
    assert_eq!(ended.keys().collect::<Vec<_>>().len(), 2);
    assert!(ended.contains_key(&a.id) && ended.contains_key(&b.id));
    assert_ne!(ended[&a.id], ended[&b.id]);

    // Both end, and each review stays with its session.
    first.wait_for_exit(&world);
    second.wait_for_exit(&world);
    let later = client.reviews(&workspace).reviews;
    assert_eq!(later.len(), 2, "{later:?}");
    for (session, review) in [(a, &review_a), (b, &review_b)] {
        let held = later.iter().find(|r| r.id == *review).expect("a review");
        assert_eq!(held.agent.as_ref(), Some(&session.id));
    }

    // Every answer of the hook programs of both is empty: the app decided nothing, and
    // each ran its hooks, so that the emptiness is not for want of any.
    for started in [&first, &second] {
        let log = started.log(&world);
        let hooks: Vec<_> = log
            .did
            .iter()
            .filter_map(|did| match did {
                Did::Hook {
                    event,
                    answer,
                    killed,
                    ..
                } => Some((event.clone(), answer.clone(), *killed)),
                _ => None,
            })
            .collect();
        let events: Vec<_> = hooks.iter().map(|(event, ..)| event.as_str()).collect();
        assert_eq!(
            events,
            ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"]
        );
        assert!(
            hooks
                .iter()
                .all(|(_, answer, killed)| answer.is_empty() && !killed),
            "{hooks:?}"
        );
    }
    end(world, client);
}

// Not a row: the environment of a shell of the app.
#[test]
fn a_shell_of_the_app_differs_from_the_users_environment_only_by_the_apps_entries() {
    let mut world = World::new();
    world.agents_on_path();
    world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let printed = world.home().with_file_name("env.txt");
    client.type_line(
        &terminal,
        &format!("env > '{}'; printf 'env-%s\\n' done", printed.display()),
    );
    // The prompt of the shell may stand in front of the marker on its line, and the typed
    // line holds `env-%s`, never the marker.
    client.wait_for_output(&terminal, DEADLINE, |output| {
        String::from_utf8_lossy(output)
            .lines()
            .any(|line| line.contains("env-done"))
    });

    let in_shell = environment(&fs::read(&printed).expect("the shell wrote its environment"));
    // What the world gives every program it starts: the user's own environment.
    let users: HashMap<String, String> = world
        .command(Path::new("/bin/sh"))
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned())))
        .collect();

    // Nothing of the user's is missing or changed but the search path.
    for (name, value) in &users {
        if name != "PATH" {
            assert_eq!(in_shell.get(name), Some(value), "{name}");
        }
    }
    // What the app and the terminal add, and what the shell itself keeps.
    let agents = world.agents_dir().expect("the agents folder");
    let plugin = world.data_dir().join("agents/claude/plugin");
    assert_eq!(
        in_shell.get("CLAUDE_CODE_PLUGIN_DIRS"),
        Some(&plugin.to_str().unwrap().to_owned())
    );
    assert_eq!(
        in_shell.get("YAAR_TERMINAL"),
        Some(&terminal.as_str().to_owned())
    );
    for name in ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"] {
        assert!(in_shell.contains_key(name), "{name} is not in {in_shell:?}");
    }
    let path = in_shell.get("PATH").expect("a search path");
    let folders: Vec<&str> = path.split(':').collect();
    assert_eq!(Path::new(folders[0]), agents, "{path}");
    // The folders are those a login shell of the world gets without the app: whatever the
    // system's profile adds to the user's own is not the app's, but a folder the app puts
    // on the path is. The folder of the agents is first because the world's profile puts it
    // there: the app adds no folder to the path in this slice, which the equality shows.
    let login = world
        .command(Path::new("/bin/sh"))
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
        .expect("a login shell of the world");
    let baseline = String::from_utf8(login.stdout).expect("a search path");
    let expected: std::collections::BTreeSet<&str> = baseline.split(':').collect();
    let found: std::collections::BTreeSet<&str> = folders.iter().copied().collect();
    assert_eq!(
        found, expected,
        "the shell's path is {path}, a login shell's {baseline}"
    );
    let allowed = [
        // What the world's own Keeper was told, and what macOS's libc gives every program.
        "YAAR_KEEPER_LINGER_MS",
        "__CF_USER_TEXT_ENCODING",
        "CLAUDE_CODE_PLUGIN_DIRS",
        "YAAR_TERMINAL",
        "TERM",
        "COLORTERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "PATH",
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
    ];
    // What the system's profile adds to the environment of a login shell is not the app's
    // either: on Ubuntu it adds XDG_DATA_DIRS. A login shell of the world, run with no
    // app in between, shows which names those are and what it gives them: a name the app
    // set to another value than the profile does is still the app's.
    let profile = world
        .command(Path::new("/bin/sh"))
        .args(["-l", "-c", "env"])
        .output()
        .expect("a login shell of the world");
    let by_profile = environment(&profile.stdout);
    let extra: Vec<_> = in_shell
        .keys()
        .filter(|name| {
            !users.contains_key(*name)
                && by_profile.get(*name) != in_shell.get(*name)
                && !allowed.contains(&name.as_str())
        })
        .collect();
    assert!(extra.is_empty(), "variables nothing explains: {extra:?}");
    // No editor connection, and nothing of another terminal. The world's Keeper starts with
    // none of these, so this shows only that none is here; DOCK-02 shows that the Dock
    // leaves them out when the Keeper has them.
    for name in [
        "CLAUDE_CODE_SSE_PORT",
        "ENABLE_IDE_INTEGRATION",
        "VSCODE_INJECTION",
        "VSCODE_IPC_HOOK_CLI",
        "VSCODE_GIT_IPC_HANDLE",
        "ZED_TERM",
    ] {
        assert!(!in_shell.contains_key(name), "{name}");
    }
    end(world, client);
}

/// The variables of the output of `env`: a name and its value to a line.
fn environment(printed: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(printed)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}
