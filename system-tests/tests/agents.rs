//! System tests of the agent with a stand-in agent: Claude Code, and pi in the `_as_pi` function.

use std::collections::HashMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use yaar_link::{
    Anchor, Attachment, CommentId, DraftOn, Fact, Lines, ReviewId, SessionState, Side,
    Step as Taken, TerminalId, TerminalSize, ThreadId, TurnEnd, TurnId, WorkspaceAgents,
    WorkspaceId,
};
use yaar_system_tests::standin::{Did, INTERRUPT_MARK, Script};
use yaar_system_tests::{
    Client, DEADLINE, Dialect, Repo, World, descendants_of, is_alive, wait_until,
};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

const NO_PROGRESS: &[u8] = b"\x1b]9;4;0;\x07";

fn path(text: &str) -> PathBuf {
    PathBuf::from(text)
}

fn script(words: &[&str]) -> Script {
    Script::parse(&words.join("\n"))
}

fn open(client: &mut Client, repo: &Repo) -> WorkspaceId {
    client.open_folder(repo.path()).expect("open the folder").1
}

/// Sets an identity in the repository's configuration, since git on Linux guesses none.
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum PiHere {
    Absent,
    /// Installed and not answered: pi's settings list nothing of the app's.
    Asked,
    Allowed,
}

fn world_with(script: &Script, pi: PiHere) -> (World, Repo) {
    let mut world = World::new();
    world.agents_on_path();
    world.agent_script("default", script);
    let repo = world.repository("app");
    repo.git(&["config", "user.name", "A Test"]);
    repo.git(&["config", "user.email", "test@example.invalid"]);
    if pi != PiHere::Absent {
        world.install_pi();
    }
    world.start_keeper();
    if pi == PiHere::Allowed {
        world.allow_pi_extension();
    }
    (world, repo)
}

fn world_for(dialect: Dialect, script: &Script) -> (World, Repo) {
    let pi = match dialect {
        Dialect::ClaudeCode => PiHere::Absent,
        Dialect::Pi => PiHere::Allowed,
    };
    world_with(script, pi)
}

fn title_of(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::ClaudeCode => "Claude Code",
        Dialect::Pi => "pi",
    }
}

struct Started {
    name: String,
}

impl Started {
    fn log(&self, world: &World) -> yaar_system_tests::standin::StandinLog {
        world.agent_log(&self.name)
    }

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

/// Facts the client holds already are seen first.
fn seen_until(client: &mut Client, mut done: impl FnMut(&Fact) -> bool) -> Vec<Fact> {
    let mut seen = Vec::new();
    client.wait_for_fact(DEADLINE, |fact| {
        seen.push(fact.clone());
        done(fact)
    });
    seen
}

fn shows(fact: &Fact, workspace: &WorkspaceId, state: SessionState) -> bool {
    matches!(fact, Fact::AgentsChanged(agents)
        if agents.workspace == *workspace && agents.shown == Some(state))
}

fn is_turn_end(fact: &Fact) -> bool {
    matches!(fact, Fact::TurnEnded { .. })
}

fn only_session(agents: &WorkspaceAgents) -> &yaar_link::Session {
    let [session] = &agents.sessions[..] else {
        panic!("not exactly one session: {agents:?}");
    };
    session
}

fn id(kind: u128, n: u128) -> String {
    format!("{:032x}", (kind << 64) + n)
}

fn the_review_of_no_agent(client: &mut Client, workspace: &WorkspaceId) -> ReviewId {
    let reviews = client.reviews(workspace);
    let [review] = &reviews.reviews[..] else {
        panic!("not exactly one review: {:?}", reviews.reviews);
    };
    assert_eq!(review.agent, None);
    assert_eq!(review.threads, vec![]);
    review.id.clone()
}

fn start_a_thread_on_line_one(
    client: &mut Client,
    review: &ReviewId,
    file: &str,
    line: &str,
    text: &str,
) -> ThreadId {
    let thread = ThreadId::new(id(2, 1));
    let anchor = Anchor {
        tree: None,
        path: path(file),
        side: Side::New,
        lines: Lines { first: 1, last: 1 },
        text: line.to_owned(),
    };
    client
        .save_draft(
            review,
            &thread,
            &CommentId::new(id(3, 1)),
            DraftOn::NewThread { anchor },
            text,
        )
        .expect("save the draft");
    thread
}

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

fn bytes_of(file: &Path) -> Vec<u8> {
    fs::read(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()))
}

#[test]
fn a_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_is_seen_as_works_then_finished_with_its_steps_in_order_and_a_state_recorded()
 {
    a_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_is_seen(Dialect::ClaudeCode);
}

#[test]
fn a_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_is_seen_as_works_then_finished_with_its_steps_in_order_and_a_state_recorded_as_pi()
 {
    a_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_is_seen(Dialect::Pi);
}

fn a_turn_with_a_read_an_edit_a_new_file_and_a_shell_write_is_seen(dialect: Dialect) {
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
    let (world, _) = world_for(dialect, &turn);
    let repo = repository_with(&world, "notes", &[("notes.txt", "alpha\n")]);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, dialect.name());
    let prompt = client.wait_for_prompt(&terminal, 0);
    let branch = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let head = repo.git(&["rev-parse", "HEAD"]);
    let index = repo.path().join(".git/index");
    // Read before any `git status` of the test: git refreshes the index there.
    let index_before = bytes_of(&index);
    assert_eq!(branch, "main");

    client.type_line(&terminal, "make the changes");
    let seen = seen_until(&mut client, is_turn_end);

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
    let last_shown = seen
        .iter()
        .rposition(|fact| shows(fact, &workspace, SessionState::Unreviewed))
        .expect("the finished state was shown");
    assert!(
        last_shown < seen.len() - 1 && is_turn_end(&seen[seen.len() - 1]),
        "the end of the turn comes after the fact that shows it: {seen:?}"
    );

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

    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(bytes_of(&index), index_before, "the index changed");

    client.wait_for_prompt(&terminal, prompt);
    client.type_line(&terminal, "");
    Started {
        name: "default".into(),
    }
    .wait_for_exit(&world);
    end(world, client);
}

#[test]
fn edits_of_a_program_that_is_no_supported_cli_and_of_a_cli_started_around_the_apps_pieces_are_no_session_turn_step_or_notice_and_show_as_changes_no_agent_made()
 {
    edits_of_programs_the_app_does_not_follow_are_no_session(Dialect::ClaudeCode);
}

#[test]
fn edits_of_a_program_that_is_no_supported_cli_and_of_a_cli_started_around_the_apps_pieces_are_no_session_turn_step_or_notice_and_show_as_changes_no_agent_made_as_pi()
 {
    edits_of_programs_the_app_does_not_follow_are_no_session(Dialect::Pi);
}

fn edits_of_programs_the_app_does_not_follow_are_no_session(dialect: Dialect) {
    // As pi, the CLI started around the app's pieces is a pi the user has not said yes to; the yes
    // (`allow_pi_extension`) comes after, so the supported CLI started last loads the extension.
    let pi = match dialect {
        Dialect::ClaudeCode => PiHere::Absent,
        Dialect::Pi => PiHere::Asked,
    };
    let (world, _) = world_with(&script(&["turn", "exit"]), pi);
    let repo = repository_with(&world, "edited", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let elsewhere = repository_with(&world, "elsewhere", &[("x.txt", "x\n")]);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let other = open(&mut client, &elsewhere);
    let review = the_review_of_no_agent(&mut client, &workspace);
    start_a_thread_on_line_one(&mut client, &review, "a.txt", "a", "Why a?");
    client
        .attach_review(&review, None)
        .expect("attach the review");

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
    let second = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let around = start_in(
        &world,
        &mut client,
        &second,
        "around",
        dialect.name(),
        match dialect {
            Dialect::ClaudeCode => "STANDIN_IGNORE_PIECES=1",
            Dialect::Pi => "",
        },
        &script(&["turn", "edit b.txt y", "end", "exit"]),
    );
    client.wait_for_prompt(&second, 0);
    client.type_line(&second, "edit another");

    // Barrier: both are over, so the first's mark has reached the Dock and the Keeper.
    unsupported.wait_for_exit(&world);
    around.wait_for_exit(&world);
    client.wait_for_output(&first, DEADLINE, |output| {
        output
            .get(after..)
            .is_some_and(|new| new.windows(NO_PROGRESS.len()).any(|w| w == NO_PROGRESS))
    });
    if dialect == Dialect::Pi {
        let log = around.log(&world);
        assert_eq!(log.extensions, Vec::<PathBuf>::new(), "{log:?}");
        assert!(!world.pi_settings().exists());
        world.allow_pi_extension();
    }
    let third = client.open_terminal(&other, SIZE).expect("a terminal");
    start_in(
        &world,
        &mut client,
        &third,
        "heard",
        dialect.name(),
        "",
        &script(&["turn", "exit"]),
    );
    let seen = seen_until(&mut client, |fact| {
        matches!(fact, Fact::AgentsChanged(agents)
            if agents.workspace == other && !agents.sessions.is_empty())
    });

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
    for started in [&unsupported, &around] {
        let log = started.log(&world);
        assert!(
            log.did.iter().all(|did| !matches!(did, Did::Hook { .. })),
            "{log:?}"
        );
    }
    let reviews = client.reviews(&workspace).reviews;
    let [waiting] = &reviews[..] else {
        panic!("not exactly one review: {reviews:?}");
    };
    assert_eq!(waiting.agent, None);
    assert_eq!(waiting.attached, Some(Attachment::Review));
    assert_eq!(waiting.rounds, vec![]);
    assert_eq!(waiting.threads.len(), 1, "{waiting:?}");
    assert_eq!(waiting.threads[0].comments[0].round, None);
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

    client.type_line(&third, "");
    end(world, client);
}

#[test]
fn the_stand_in_started_from_its_launcher_and_by_its_name_gives_the_same_session_and_each_takes_over_the_review_the_last_left()
 {
    sessions_of_a_cli_take_over_the_review_of_no_agent_in_turn(Dialect::ClaudeCode);
}

#[test]
fn the_stand_in_started_from_its_launcher_and_by_its_name_gives_the_same_session_and_each_takes_over_the_review_the_last_left_as_pi()
 {
    sessions_of_a_cli_take_over_the_review_of_no_agent_in_turn(Dialect::Pi);
}

fn sessions_of_a_cli_take_over_the_review_of_no_agent_in_turn(dialect: Dialect) {
    let (world, repo) = world_with(&script(&["turn", "end", "exit"]), PiHere::Allowed);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let before = client.reviews(&workspace);
    let [no_agent] = &before.reviews[..] else {
        panic!("not exactly one review: {:?}", before.reviews);
    };
    assert_eq!(no_agent.agent, None);
    let review = no_agent.id.clone();

    let launcher = client
        .open_launcher(&workspace, dialect.name(), SIZE)
        .expect("open the launcher");
    client.wait_for_prompt(&launcher, 0);
    let from_launcher = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.state == SessionState::Idle)
    });
    let launched = only_session(&from_launcher).clone();
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0].id, review);
    assert_eq!(reviews[0].agent, Some(launched.id.clone()));
    assert_eq!(launched.review, Some(review.clone()));

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

    let shell = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let typed = start_in(
        &world,
        &mut client,
        &shell,
        "typed",
        dialect.name(),
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

    assert_ne!(second.id, launched.id);
    assert_eq!(second.agent, launched.agent);
    assert_eq!(second.agent, dialect.name());
    assert_eq!(launched.title, format!("{} 1", title_of(dialect)));
    assert_eq!(second.title, format!("{} 2", title_of(dialect)));
    assert_eq!(launched.terminal, Some(launcher.clone()));
    assert_eq!(second.terminal, Some(shell.clone()));
    assert_eq!(launched.state, SessionState::Idle);
    assert_eq!(second.state, SessionState::Idle);

    assert_eq!(second.review, Some(review.clone()));
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0].id, review);
    assert_eq!(reviews[0].agent, Some(second.id.clone()));
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
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.id == second.id && s.state == SessionState::NotRunning)
    });

    let other = dialect.other();
    let other_shell = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let started = start_in(
        &world,
        &mut client,
        &other_shell,
        "other",
        other.name(),
        "",
        &script(&["turn", "end", "exit"]),
    );
    client.wait_for_prompt(&other_shell, 0);
    let with_other = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.agent == other.name() && s.state == SessionState::Idle)
    });
    let theirs = with_other
        .sessions
        .iter()
        .find(|s| s.agent == other.name())
        .expect("the other agent's session")
        .clone();
    assert_eq!(theirs.title, format!("{} 1", title_of(other)));
    let their_review = theirs.review.clone().expect("a review of its own");
    assert_ne!(their_review, review);
    let reviews = client.reviews(&workspace).reviews;
    assert_eq!(reviews.len(), 2, "{reviews:?}");
    let held = reviews.iter().find(|r| r.id == review).expect("the review");
    assert_eq!(held.agent, Some(second.id.clone()), "{reviews:?}");
    let own = reviews
        .iter()
        .find(|r| r.id == their_review)
        .expect("its review");
    assert_eq!(own.agent, Some(theirs.id.clone()), "{reviews:?}");
    let later = client.agents();
    let all = later.iter().find(|a| a.workspace == workspace).unwrap();
    let held_by = |id: &yaar_link::SessionId| all.sessions.iter().find(|s| s.id == *id).unwrap();
    assert_eq!(held_by(&second.id).review, Some(review.clone()), "{all:?}");

    client.type_line(&other_shell, "bye");
    started.wait_for_exit(&world);
    end(world, client);
}

// Claude Code only: pi's stand-in refuses a script with an `ask`.
#[test]
fn an_agent_that_asks_for_permission_turns_the_waiting_state_on_and_the_app_writes_nothing_to_its_terminal_and_the_keys_of_the_answer_turn_it_off()
 {
    let asking = script(&["turn", "ask p.txt x", "hold ../go", "end", "turn", "exit"]);
    let (world, repo) = World::with_standin("default", &asking);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, "claude");
    let prompt = client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "write p");

    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::WaitsForPermission)
    });
    let listed = client.agents();
    let shown = listed.iter().find(|a| a.workspace == workspace).unwrap();
    assert_eq!(shown.shown, Some(SessionState::WaitsForPermission));
    assert!(!repo.path().join("p.txt").exists());

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

#[test]
fn esc_during_a_turn_makes_the_stand_in_print_the_mark_and_report_nothing_and_the_turn_ends_in_the_app()
 {
    esc_during_a_turn_ends_the_turn_in_the_app(Dialect::ClaudeCode);
}

#[test]
fn esc_during_a_turn_makes_the_stand_in_print_the_mark_and_report_nothing_and_the_turn_ends_in_the_app_as_pi()
 {
    esc_during_a_turn_ends_the_turn_in_the_app(Dialect::Pi);
}

fn esc_during_a_turn_ends_the_turn_in_the_app(dialect: Dialect) {
    // The prompt of the `turn` after the interrupted one is the barrier: the Esc's work is done.
    let held = script(&["turn", "hold never", "end", "turn"]);
    let (world, repo) = world_for(dialect, &held);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, dialect.name());
    let prompt = client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "work");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    client.wait_for_output(&terminal, DEADLINE, |output| {
        String::from_utf8_lossy(output).contains("Hold never")
    });

    client.type_into(&terminal, b"\x1b");
    let has_mark = |output: &[u8]| {
        output
            .windows(INTERRUPT_MARK.len())
            .any(|window| window == INTERRUPT_MARK)
    };
    if dialect == Dialect::ClaudeCode {
        client.wait_for_output(&terminal, DEADLINE, has_mark);
    }
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

    client.wait_for_prompt(&terminal, prompt);
    let output = client.wait_for_output(&terminal, DEADLINE, |_| true);
    assert_eq!(has_mark(&output), dialect == Dialect::ClaudeCode);
    let screen = client.screen(&terminal, SIZE);
    assert_eq!(
        screen.iter().any(|line| line.contains("Interrupted")),
        dialect == Dialect::ClaudeCode,
        "{screen:?}"
    );

    let started = Started {
        name: "default".into(),
    };
    let did = started.log(&world).did;
    let at = did
        .iter()
        .position(|did| matches!(did, Did::Interrupted { .. }))
        .expect("the stand-in was interrupted");
    assert!(
        did[at..].iter().all(|did| !matches!(did, Did::Hook { .. })),
        "{did:?}"
    );
    let moments: Vec<_> = did[..at]
        .iter()
        .filter_map(|did| match did {
            Did::Hook { event, .. } => Some(event.as_str()),
            _ => None,
        })
        .collect();
    let ends = moments
        .iter()
        .filter(|event| **event == dialect.turn_end_event())
        .count();
    assert_eq!(
        ends,
        usize::from(dialect == Dialect::Pi),
        "reported before the interrupt: {moments:?}"
    );
    end(world, client);
}

#[test]
fn an_agent_whose_process_is_killed_shows_as_not_running_while_its_terminal_lives() {
    a_killed_agent_shows_as_not_running(Dialect::ClaudeCode);
}

#[test]
fn an_agent_whose_process_is_killed_shows_as_not_running_while_its_terminal_lives_as_pi() {
    a_killed_agent_shows_as_not_running(Dialect::Pi);
}

fn a_killed_agent_shows_as_not_running(dialect: Dialect) {
    let (world, repo) = world_for(dialect, &Script(vec![]));
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let bystander_terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let doomed = start_in(
        &world,
        &mut client,
        &terminal,
        "doomed",
        dialect.name(),
        "",
        &script(&["turn"]),
    );
    client.wait_for_prompt(&terminal, 0);
    let bystander = start_in(
        &world,
        &mut client,
        &bystander_terminal,
        "bystander",
        dialect.name(),
        "",
        &script(&["turn"]),
    );
    client.wait_for_prompt(&bystander_terminal, 0);
    let seen = seen_until(&mut client, |fact| {
        matches!(fact, Fact::AgentsChanged(agents)
            if agents.workspace == workspace
                && agents.sessions.len() == 2
                && agents.sessions.iter().all(|s| s.state == SessionState::Idle))
    });
    assert!(
        seen.iter().all(|fact| !matches!(fact,
            Fact::AgentsChanged(agents)
                if agents.sessions.iter().any(|s| s.state == SessionState::NotRunning))),
        "{seen:?}"
    );
    let Some(Fact::AgentsChanged(agents)) = seen.last() else {
        panic!("the wait ended on no change of the agents: {seen:?}");
    };
    let session_in = |terminal: &TerminalId| {
        agents
            .sessions
            .iter()
            .find(|s| s.terminal.as_ref() == Some(terminal))
            .expect("a session in the terminal")
            .id
            .clone()
    };
    let (doomed_session, other_session) = (session_in(&terminal), session_in(&bystander_terminal));
    let not_running = |agents: &WorkspaceAgents, session: &yaar_link::SessionId| {
        agents.workspace == workspace
            && agents
                .sessions
                .iter()
                .any(|s| s.id == *session && s.state == SessionState::NotRunning)
    };

    // The Keeper finds processes gone in one pass over all it follows. The bystander is killed
    // first: the pass that finds it gone also looked at the session under test, which must not be
    // found gone before its own kill.
    kill(bystander.log(&world).pid);
    let seen = seen_until(
        &mut client,
        |fact| matches!(fact, Fact::AgentsChanged(agents) if not_running(agents, &other_session)),
    );
    assert!(
        seen.iter().all(|fact| !matches!(fact,
            Fact::AgentsChanged(agents) if not_running(agents, &doomed_session))),
        "the session was found gone before it was killed: {seen:?}"
    );

    kill(doomed.log(&world).pid);
    let gone = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        not_running(agents, &doomed_session)
    });
    assert!(
        gone.sessions
            .iter()
            .any(|s| s.id == doomed_session && s.state == SessionState::NotRunning),
        "{gone:?}"
    );
    let terminals = client.terminals();
    assert!(
        terminals.iter().any(|info| info.id == terminal),
        "{terminals:?}"
    );
    end(world, client);
}

fn kill(pid: u32) {
    assert_ne!(pid, 0);
    // SAFETY: a signal to a process of this test.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) }, 0);
}

#[test]
fn an_agent_that_works_is_never_shown_as_not_running_while_the_keeper_finds_another_agents_process_gone()
 {
    a_working_agent_is_not_found_gone_with_another(Dialect::ClaudeCode);
}

#[test]
fn an_agent_that_works_is_never_shown_as_not_running_while_the_keeper_finds_another_agents_process_gone_as_pi()
 {
    a_working_agent_is_not_found_gone_with_another(Dialect::Pi);
}

fn a_working_agent_is_not_found_gone_with_another(dialect: Dialect) {
    let (world, repo) = world_for(dialect, &Script(vec![]));
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let one = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let two = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let holding = script(&["turn", "hold ../go", "end", "turn"]);
    start_in(
        &world,
        &mut client,
        &one,
        "one",
        dialect.name(),
        "",
        &holding,
    );
    client.wait_for_prompt(&one, 0);
    let idle = start_in(
        &world,
        &mut client,
        &two,
        "two",
        dialect.name(),
        "",
        &script(&["turn"]),
    );
    client.wait_for_prompt(&two, 0);
    let both = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.sessions.len() == 2
            && agents
                .sessions
                .iter()
                .all(|s| s.state == SessionState::Idle)
    });
    let session_in = |terminal: &TerminalId| {
        both.sessions
            .iter()
            .find(|s| s.terminal.as_ref() == Some(terminal))
            .expect("a session in the terminal")
            .id
            .clone()
    };
    let (working, doomed) = (session_in(&one), session_in(&two));
    client.type_line(&one, "work");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents
            .sessions
            .iter()
            .any(|s| s.id == working && s.state == SessionState::Works)
    });

    // The idle agent is killed. The Keeper's pass over its processes looks at the working agent
    // too, so the idle session found not running shows the pass was made without the working one
    // gone.
    kill(idle.log(&world).pid);
    let not_running = |agents: &WorkspaceAgents, session: &yaar_link::SessionId| {
        agents.workspace == workspace
            && agents
                .sessions
                .iter()
                .any(|s| s.id == *session && s.state == SessionState::NotRunning)
    };
    let seen = seen_until(
        &mut client,
        |fact| matches!(fact, Fact::AgentsChanged(agents) if not_running(agents, &doomed)),
    );
    assert!(
        seen.iter().all(|fact| !matches!(fact,
            Fact::AgentsChanged(agents) if not_running(agents, &working))),
        "the working agent was found gone: {seen:?}"
    );

    fs::write(repo.path().join("../go"), "").expect("let the turn go on");
    let seen = seen_until(&mut client, |fact| {
        matches!(fact, Fact::AgentsChanged(agents)
            if agents.workspace == workspace
                && agents
                    .sessions
                    .iter()
                    .any(|s| s.id == working && s.state == SessionState::Unreviewed))
    });
    assert!(
        seen.iter().all(|fact| !matches!(fact,
            Fact::AgentsChanged(agents) if not_running(agents, &working))),
        "the working agent was found gone: {seen:?}"
    );
    end(world, client);
}

#[test]
fn two_sessions_in_one_worktree_each_have_a_review() {
    two_sessions_in_one_worktree_each_have_a_review_of(Dialect::ClaudeCode);
}

#[test]
fn two_sessions_in_one_worktree_each_have_a_review_as_pi() {
    two_sessions_in_one_worktree_each_have_a_review_of(Dialect::Pi);
}

fn two_sessions_in_one_worktree_each_have_a_review_of(dialect: Dialect) {
    let (world, repo) = world_for(dialect, &Script(vec![]));
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let one = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let two = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let turn = script(&["turn", "end", "exit"]);
    let first = start_in(&world, &mut client, &one, "one", dialect.name(), "", &turn);
    client.wait_for_prompt(&one, 0);
    let second = start_in(&world, &mut client, &two, "two", dialect.name(), "", &turn);
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

    first.wait_for_exit(&world);
    second.wait_for_exit(&world);
    let later = client.reviews(&workspace).reviews;
    assert_eq!(later.len(), 2, "{later:?}");
    for (session, review) in [(a, &review_a), (b, &review_b)] {
        let held = later.iter().find(|r| r.id == *review).expect("a review");
        assert_eq!(held.agent.as_ref(), Some(&session.id));
    }

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
        let (start, run) = match dialect {
            Dialect::ClaudeCode => ("SessionStart", None),
            Dialect::Pi => ("session_start", Some("agent_start")),
        };
        let mut expected = vec![start, dialect.message_event()];
        expected.extend(run);
        expected.extend([dialect.turn_end_event(), dialect.session_end_event()]);
        assert_eq!(events, expected);
        assert!(
            hooks
                .iter()
                .all(|(_, answer, killed)| answer.is_empty() && !killed),
            "{hooks:?}"
        );
    }
    end(world, client);
}

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
    // The shell's prompt may stand in front of the marker; the typed line holds `env-%s`, not it.
    client.wait_for_output(&terminal, DEADLINE, |output| {
        String::from_utf8_lossy(output)
            .lines()
            .any(|line| line.contains("env-done"))
    });

    let in_shell = environment(&fs::read(&printed).expect("the shell wrote its environment"));
    let users: HashMap<String, String> = world
        .command(Path::new("/bin/sh"))
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned())))
        .collect();

    for (name, value) in &users {
        if name != "PATH" {
            assert_eq!(in_shell.get(name), Some(value), "{name}");
        }
    }
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
    let reporter = in_shell
        .get("YAAR_PI_REPORT")
        .expect("pi's reporting program");
    assert!(Path::new(reporter).is_absolute(), "{reporter}");
    assert_eq!(
        Path::new(reporter).file_name(),
        Some(std::ffi::OsStr::new("yaar-pi-report")),
        "{reporter}"
    );
    for name in ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"] {
        assert!(in_shell.contains_key(name), "{name} is not in {in_shell:?}");
    }
    let path = in_shell.get("PATH").expect("a search path");
    let folders: Vec<&str> = path.split(':').collect();
    assert_eq!(Path::new(folders[0]), agents, "{path}");
    // The shell's folders are those a login shell of the world gets without the app: what a system
    // profile adds is not the app's, and the equality shows the app adds no folder.
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
        "YAAR_PI_REPORT",
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
    // A system profile adds variables (Ubuntu: XDG_DATA_DIRS) that are not the app's.
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

fn environment(printed: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(printed)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}
