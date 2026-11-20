//! System tests of the rounds of a review, past the first: a second round that the agent
//! answers while it rewrites a commented line, the offer made on attaching a review whose
//! agent does not run, and everything killed as a restart of the machine would. The built
//! Keeper, Dock, hook and reporting programs and the tool server, and the stand-in agent
//! started in a shell or a launcher of the app, read through the Link as a Window reads it
//! and through the stand-in's own log.
//!
//! Each row runs for both CLIs: its body is a function of a [`Dialect`], run as Claude Code
//! by the test that carries the row's name and as pi by the one with `_as_pi` after it. A
//! world as pi has pi installed before its Keeper starts and gives the user's yes after, so
//! that the stand-in typed as `pi` finds the app's extension in the world's pi settings, as
//! pi does.
//!
//! In every worktree here nobody commits: each test notes the commit checked out when it
//! starts and finds it unchanged at its end.

use std::path::PathBuf;

use yaar_link::{
    Anchor, Attachment, Author, ChangedFile, CommentId, DraftOn, Fact, FileStatus, HistoryEntry,
    Lines, Notice, Placed, Program, Review, ReviewId, SentBy, Session, SessionState, Side,
    TerminalId, TerminalSize, ThreadId, ThreadState, WorkspaceId,
};
use yaar_system_tests::standin::{Did, PI_SESSIONS, Script, Step};
use yaar_system_tests::{
    Client, DEADLINE, Dialect, Repo, World, descendants_of, is_alive, wait_until,
};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

fn path(text: &str) -> PathBuf {
    PathBuf::from(text)
}

/// A script of the steps in `words`, each the way `Script::parse` reads it.
fn script(words: &[&str]) -> Script {
    Script::parse(&words.join("\n"))
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

/// Opens the folder of `repo` as a workspace.
fn open(client: &mut Client, repo: &Repo) -> WorkspaceId {
    client.open_folder(repo.path()).expect("open the folder").1
}

/// The id of the `n`th thing of a kind: 32 digits, as the Window draws them.
fn id(kind: u128, n: u128) -> String {
    format!("{:032x}", (kind << 64) + n)
}

/// Starts the `n`th thread of `review` on line `line` of `file`, as the files on disk hold
/// it, where the text of that line is `code`, with the draft `text`. The Window draws the
/// ids of a thread and of its first comment before the first save.
fn start_thread(
    client: &mut Client,
    review: &ReviewId,
    n: u128,
    file: &str,
    line: u32,
    code: &str,
    text: &str,
) -> ThreadId {
    let thread = ThreadId::new(id(2, n));
    let anchor = Anchor {
        tree: None,
        path: path(file),
        side: Side::New,
        lines: Lines {
            first: line,
            last: line,
        },
        text: code.to_owned(),
    };
    client
        .save_draft(
            review,
            &thread,
            &CommentId::new(id(3, n)),
            DraftOn::NewThread { anchor },
            text,
        )
        .expect("save the draft");
    thread
}

/// The session of `workspace` that `wanted` accepts, once there is one: the Keeper's list
/// is asked again until it holds one.
fn session_where(
    client: &mut Client,
    workspace: &WorkspaceId,
    mut wanted: impl FnMut(&Session) -> bool,
) -> Session {
    let mut found = None;
    let came = wait_until(DEADLINE, || {
        found = client
            .agents()
            .into_iter()
            .filter(|agents| agents.workspace == *workspace)
            .flat_map(|agents| agents.sessions)
            .find(|session| wanted(session));
        found.is_some()
    });
    found
        .filter(|_| came)
        .unwrap_or_else(|| panic!("no such session came: {:?}", client.agents()))
}

/// Every session of `workspace`, as the Keeper lists them now.
fn sessions_of(client: &mut Client, workspace: &WorkspaceId) -> Vec<Session> {
    client
        .agents()
        .into_iter()
        .filter(|agents| agents.workspace == *workspace)
        .flat_map(|agents| agents.sessions)
        .collect()
}

/// The review `review` of `workspace` once `wanted` accepts it: the Keeper's reviews are
/// asked again until it does.
fn review_where(
    client: &mut Client,
    workspace: &WorkspaceId,
    review: &ReviewId,
    mut wanted: impl FnMut(&Review) -> bool,
) -> Review {
    let mut found = None;
    let came = wait_until(DEADLINE, || {
        found = client
            .reviews(workspace)
            .reviews
            .into_iter()
            .find(|held| held.id == *review && wanted(held));
        found.is_some()
    });
    found
        .filter(|_| came)
        .unwrap_or_else(|| panic!("no such review came: {:?}", client.reviews(workspace)))
}

/// The review `review` of `workspace` as it is now.
fn review_now(client: &mut Client, workspace: &WorkspaceId, review: &ReviewId) -> Review {
    client
        .reviews(workspace)
        .reviews
        .into_iter()
        .find(|held| held.id == *review)
        .unwrap_or_else(|| panic!("no review {review}"))
}

/// Whether `fact` is a turn's end.
fn is_turn_end(fact: &Fact) -> bool {
    matches!(fact, Fact::TurnEnded { .. })
}

/// What the stand-in of `dialect` was answered at each moment the user's message was taken
/// (Claude Code's `UserPromptSubmit` hook, pi's `input`), one entry for each such moment it
/// reported, in order.
fn prompt_answers(dialect: Dialect, log: &yaar_system_tests::standin::StandinLog) -> Vec<Vec<u8>> {
    log.did
        .iter()
        .filter_map(|did| match did {
            Did::Hook { event, answer, .. } if event == dialect.message_event() => {
                Some(answer.clone())
            }
            _ => None,
        })
        .collect()
}

/// The text an answer of the Keeper to the user's message hands to the model with it: empty
/// for an answer that carries none.
fn carried(dialect: Dialect, answer: &[u8]) -> String {
    if answer.is_empty() {
        return String::new();
    }
    dialect
        .review_in(answer)
        .unwrap_or_else(|| panic!("no text in {:?}", String::from_utf8_lossy(answer)))
}

/// Whether the stand-in of `dialect` reported the end of a tool after step `step` of its
/// script began, and was answered.
fn tool_done_after(
    dialect: Dialect,
    log: &yaar_system_tests::standin::StandinLog,
    step: usize,
) -> bool {
    let mut began = false;
    log.did.iter().any(|did| match did {
        Did::Began { step: at, .. } if *at == step => {
            began = true;
            false
        }
        Did::Hook { event, .. } => began && event == dialect.tool_done_event(),
        _ => false,
    })
}

/// Waits until the stand-in whose log is `name` has ended its program.
fn wait_for_exit(world: &World, name: &str) {
    assert!(
        wait_until(DEADLINE, || world
            .agent_log(name)
            .did
            .iter()
            .any(|did| matches!(did, Did::Exited { .. }))),
        "{name} did not exit: {:?}",
        world.agent_log(name)
    );
}

/// Types into `terminal` the start of the stand-in as `dialect`'s CLI with the script
/// `script` as `name`'s own, and its log as `name`'s: the way a user types the CLI's name in
/// a shell of the app.
fn start_in(
    world: &World,
    client: &mut Client,
    terminal: &TerminalId,
    dialect: Dialect,
    name: &str,
    script: &Script,
) {
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

/// The commit checked out in `repo` and its branch: what nobody changes in a worktree in
/// which nobody commits.
fn checked_out(repo: &Repo) -> (String, String) {
    (
        repo.git(&["rev-parse", "HEAD"]),
        repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]),
    )
}

/// What git says of the worktree and the index of `repo`: nothing is staged and no file of
/// the app is there, so the only lines are the changes the agent wrote itself.
fn worktree_status(repo: &Repo) -> String {
    repo.git(&["status", "--porcelain"])
}

/// The history of `workspace` holds no commit: nobody committed.
fn assert_no_commit_was_made(client: &mut Client, workspace: &WorkspaceId) {
    let history = client.history(workspace).expect("the history");
    assert!(
        history
            .entries
            .iter()
            .all(|entry| !matches!(entry, HistoryEntry::Commit(_))),
        "{history:?}"
    );
}

/// What a stand-in of `dialect` is started with by the resume command of its CLI, as the
/// Keeper's Adapter makes it for the session `cli_session`: Claude Code's `--resume <id>`,
/// pi's `--session-dir <the folder of the session's file> --session <id>`, whose folder is
/// where the stand-in as pi keeps its session files, under the home of the world.
fn resume_arguments(world: &World, dialect: Dialect, cli_session: &str) -> Vec<String> {
    match dialect {
        Dialect::ClaudeCode => vec!["--resume".to_owned(), cli_session.to_owned()],
        Dialect::Pi => vec![
            "--session-dir".to_owned(),
            world
                .home()
                .join(PI_SESSIONS)
                .to_str()
                .expect("a path of text")
                .to_owned(),
            "--session".to_owned(),
            cli_session.to_owned(),
        ],
    }
}

// AGENT-15.
#[test]
fn everything_killed_as_a_restart_of_the_machine_would_leaves_the_review_its_turns_and_its_attachment_as_left_and_resuming_runs_the_cli_with_the_sessions_id()
 {
    everything_killed(Dialect::ClaudeCode);
}

// AGENT-15, as pi.
#[test]
fn everything_killed_as_a_restart_of_the_machine_would_leaves_the_review_its_turns_and_its_attachment_as_left_and_resuming_runs_the_cli_with_the_sessions_id_as_pi()
 {
    everything_killed(Dialect::Pi);
}

fn everything_killed(dialect: Dialect) {
    // One turn that rewrites the line of the first comment, and no exit: the stand-in is
    // still there when everything is killed.
    let one_turn = Script(vec![
        Step::Turn,
        Step::Write(path("notes.txt"), "omega, wholly new\nbeta\n".into()),
        Step::End,
    ]);
    let (mut world, _) = World::with_standin_as(dialect, "default", &one_turn);
    let repo = repository_with(
        &world,
        "notes",
        &[("notes.txt", "alpha\nbeta\n"), ("b.txt", "b\n")],
    );
    let checked_out_at_the_start = checked_out(&repo);
    assert_eq!(worktree_status(&repo), "", "all is committed");
    let mut client = world.client();
    let workspace = open(&mut client, &repo);

    // A session, and a first round with its turn: a comment, attached, and a message that
    // carries it.
    let launcher = client
        .open_launcher(&workspace, dialect.name(), SIZE)
        .expect("open the launcher");
    client.wait_for_prompt(&launcher, 0);
    let session = session_where(&mut client, &workspace, |s| s.state == SessionState::Idle);
    let review = session.review.clone().expect("the session's review");
    start_thread(
        &mut client,
        &review,
        1,
        "notes.txt",
        1,
        "alpha",
        "Why alpha?",
    );
    assert_eq!(client.attach_review(&review, None).expect("attach"), vec![]);
    client.type_line(&launcher, "go");
    let Fact::TurnEnded { turn, state, .. } = client.wait_for_fact(DEADLINE, is_turn_end) else {
        unreachable!()
    };
    let state = state.expect("a state recorded at the turn's end");
    let sent = review_where(
        &mut client,
        &workspace,
        &review,
        |r| matches!(&r.rounds[..], [round] if round.state.is_some() && round.edited),
    );
    assert_eq!(sent.rounds[0].sent_by, SentBy::Delivered);

    // A second thread, a draft, attached to the next message.
    start_thread(&mut client, &review, 2, "b.txt", 1, "b", "And b?");
    assert_eq!(client.attach_review(&review, None).expect("attach"), vec![]);
    // What the Keeper holds settles when the first thread, whose line the agent rewrote, is
    // carried to the state recorded for the second one's anchor, the newest.
    let before = review_where(&mut client, &workspace, &review, |r| {
        r.threads.len() == 2
            && r.threads[1].anchor.tree.is_some()
            && r.threads[0].now.as_ref().is_some_and(|now| {
                Some(&now.tree) == r.threads[1].anchor.tree.as_ref()
                    && now.notice == Some(Notice::Changed)
            })
    });
    assert_eq!(before.attached, Some(Attachment::Review));
    assert_eq!(before.threads[0].state, ThreadState::Sent);
    assert_eq!(before.threads[1].state, ThreadState::Draft);
    let reviews_before = client.reviews(&workspace);
    let history_before = client.history(&workspace).expect("the history");
    let turn_before = client.turn(&turn).expect("the turn");
    assert_eq!(turn_before.state.as_deref(), Some(state.as_str()));
    assert_eq!(turn_before.steps.len(), 1);
    assert!(
        history_before
            .entries
            .iter()
            .any(|entry| matches!(entry, HistoryEntry::Turn { turn: t, .. } if *t == turn)),
        "{history_before:?}"
    );
    let running = session_where(&mut client, &workspace, |s| s.id == session.id);
    assert_eq!(running.attached, Some(Attachment::Review));
    assert!(!running.resumable);
    // The id the CLI gave itself, which the resume command names.
    let cli_session = world.agent_log("default").session;
    assert!(!cli_session.is_empty());

    // Everything is killed, the Keeper and the agent in its terminal with it.
    drop(client);
    world.kill_everything();
    world.agent_script("default", &script(&["turn", "end", "exit"]));
    world.start_keeper();
    let mut client = world.client();

    // The review, its turns and its attachment are as left.
    assert_eq!(client.reviews(&workspace), reviews_before);
    assert_eq!(
        client.history(&workspace).expect("the history"),
        history_before
    );
    assert_eq!(client.turn(&turn).expect("the turn"), turn_before);

    // The agent that ran is listed as not running, with a resume control, and still holds
    // its review and what is attached of it.
    let listed = session_where(&mut client, &workspace, |s| s.id == session.id);
    assert_eq!(listed.state, SessionState::NotRunning);
    assert!(listed.resumable, "{listed:?}");
    assert_eq!(listed.terminal, None);
    assert_eq!(listed.turn, None);
    assert_eq!(listed.review, Some(review.clone()));
    assert_eq!(listed.attached, Some(Attachment::Review));
    assert_eq!(listed.agent, session.agent);
    assert_eq!(listed.title, session.title);
    assert_eq!(sessions_of(&mut client, &workspace).len(), 1);

    // Using the control runs the CLI's own resume command with the session's id.
    let resumed = client
        .open_program(
            &workspace,
            Program::Resume {
                session: session.id.clone(),
            },
            SIZE,
        )
        .expect("open the resume launcher");
    client.wait_for_prompt(&resumed, 0);
    let log = world.agent_log("default");
    assert_eq!(
        log.args,
        resume_arguments(&world, dialect, &cli_session),
        "{log:?}"
    );
    assert_eq!(log.session, cli_session);
    if dialect == Dialect::Pi {
        // It found the extension in pi's settings, as pi does after the yes given before
        // the kill, which the restart kept: no other way gave it one (no `-e`, above).
        assert_eq!(log.extensions, [world.pi_extension()], "{log:?}");
    }
    // The same session runs again, with its review and the attachment, and no other
    // session came.
    let again = session_where(&mut client, &workspace, |s| {
        s.id == session.id && s.state == SessionState::Idle
    });
    assert!(!again.resumable);
    assert_eq!(again.terminal, Some(resumed.clone()));
    assert_eq!(again.review, Some(review.clone()));
    assert_eq!(again.attached, Some(Attachment::Review));
    assert_eq!(sessions_of(&mut client, &workspace).len(), 1);
    // And the attachment goes with its next message.
    client.type_line(&resumed, "go on");
    assert!(
        wait_until(DEADLINE, || !prompt_answers(
            dialect,
            &world.agent_log("default")
        )
        .is_empty()),
        "{:?}",
        world.agent_log("default")
    );
    let answers = prompt_answers(dialect, &world.agent_log("default"));
    assert!(
        carried(dialect, &answers[0]).contains("And b?"),
        "{answers:?}"
    );
    wait_for_exit(&world, "default");

    assert_eq!(checked_out(&repo), checked_out_at_the_start);
    assert_eq!(worktree_status(&repo), " M notes.txt");
    assert_no_commit_was_made(&mut client, &workspace);
    end(world, client);
}

// AGENT-17.
#[test]
fn a_second_round_shows_as_sent_until_the_first_edit_moves_the_thread_with_its_notice_and_leaves_one_thread_addressed_and_one_waiting_and_none_resolved()
 {
    a_second_round(Dialect::ClaudeCode);
}

// AGENT-17, as pi.
#[test]
fn a_second_round_shows_as_sent_until_the_first_edit_moves_the_thread_with_its_notice_and_leaves_one_thread_addressed_and_one_waiting_and_none_resolved_as_pi()
 {
    a_second_round(Dialect::Pi);
}

fn a_second_round(dialect: Dialect) {
    // The second turn holds until the test makes the file, rewrites line 2 of notes.txt and
    // holds again until the test has read the review, so that the reply and the turn's end
    // come after the read. The steps the test waits for, by their place in the script below.
    const HOLD: usize = 4;
    const WRITE: usize = 5;
    const HOLD_AFTER_WRITE: usize = 6;
    // Replies name threads by their number in the review the agent was handed.
    let (world, _) = World::with_standin_as(dialect, "default", &Script(vec![]));
    let gate = world
        .agent_script("default", &Script(vec![]))
        .with_extension("go");
    let rewritten = "alpha\ndelta, replaced entirely\ngamma\n";
    let after_write = gate.with_extension("go-on");
    let steps = Script(vec![
        Step::Turn,
        Step::Reply {
            thread: 1,
            addressed: true,
            body: "Round one is answered.".into(),
        },
        Step::End,
        Step::Turn,
        Step::Hold(gate.clone()),
        Step::Write(path("notes.txt"), rewritten.into()),
        Step::Hold(after_write.clone()),
        Step::Reply {
            thread: 1,
            addressed: true,
            body: "Rewrote it: `delta`.".into(),
        },
        Step::Reply {
            thread: 2,
            addressed: false,
            body: "Why do you want `b` to change?".into(),
        },
        Step::End,
        Step::Exit,
    ]);
    world.agent_script("default", &steps);
    let repo = repository_with(
        &world,
        "rounds",
        &[
            ("a.txt", "a\n"),
            ("b.txt", "b\nsecond\n"),
            ("notes.txt", "alpha\nbeta\ngamma\n"),
        ],
    );
    let checked_out_at_the_start = checked_out(&repo);
    assert_eq!(worktree_status(&repo), "", "all is committed");
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let launcher = client
        .open_launcher(&workspace, dialect.name(), SIZE)
        .expect("open the launcher");
    let prompt = client.wait_for_prompt(&launcher, 0);
    let session = session_where(&mut client, &workspace, |s| s.state == SessionState::Idle);
    let review = session.review.clone().expect("the session's review");

    // Round 1: a thread, the attach, a message, the agent's reply, the turn's end.
    let first = start_thread(&mut client, &review, 1, "a.txt", 1, "a", "What is a?");
    client.attach_review(&review, None).expect("attach");
    client.type_line(&launcher, "first");
    client.wait_for_fact(DEADLINE, is_turn_end);
    let one = review_where(&mut client, &workspace, &review, |r| {
        r.threads[0].state == ThreadState::Addressed
    });
    assert_eq!(one.rounds.len(), 1);
    // A reply is no edit: the round is still shown as sent.
    assert!(!one.rounds[0].edited, "{one:?}");
    assert_eq!(one.threads[0].id, first);

    // Round 2: a thread on line 2 of notes.txt, one on line 1 of b.txt, the attach, a
    // message.
    let t1 = start_thread(&mut client, &review, 2, "notes.txt", 2, "beta", "Why beta?");
    let t2 = start_thread(&mut client, &review, 3, "b.txt", 1, "b", "Rename b.");
    client.attach_review(&review, None).expect("attach");
    client.wait_for_prompt(&launcher, prompt);
    client.type_line(&launcher, "second");
    // The send's second change: its state is recorded, off the agent's way.
    let sent = review_where(&mut client, &workspace, &review, |r| {
        r.rounds.len() == 2 && r.rounds[1].state.is_some()
    });
    let round_two = sent.rounds[1].clone();
    let sent_state = round_two.state.clone().expect("the send's state");
    assert_eq!(round_two.sent_by, SentBy::Delivered);
    // The agent holds, a tool that runs: the round is shown as sent until its first edit.
    // The wait is for the stand-in to reach the hold, which it does after the hook that
    // carried the review was answered, and no edit can come before it.
    assert!(
        wait_until(DEADLINE, || world
            .agent_log("default")
            .did
            .iter()
            .any(|did| matches!(did, Did::Began { step: HOLD, .. }))),
        "{:?}",
        world.agent_log("default")
    );
    assert!(
        !review_now(&mut client, &workspace, &review).rounds[1].edited,
        "the round is shown as edited before the agent edited"
    );
    // The review it carried names both threads, the thread of round 1 not among them.
    let answers = prompt_answers(dialect, &world.agent_log("default"));
    assert_eq!(answers.len(), 2, "{answers:?}");
    let handed = carried(dialect, &answers[1]);
    assert!(handed.contains(&format!("## Thread {t1}")), "{handed}");
    assert!(handed.contains(&format!("## Thread {t2}")), "{handed}");
    assert!(!handed.contains(&format!("## Thread {first}")), "{handed}");

    // The agent makes its edit and answers; the turn ends. The barrier is the report that
    // the write ran, the dialect's `tool_done_event` after the step began, which the stand-in
    // writes down once the Keeper answered it.
    std::fs::write(&gate, "").expect("open the gate");
    assert!(
        wait_until(DEADLINE, || tool_done_after(
            dialect,
            &world.agent_log("default"),
            WRITE
        )),
        "{:?}",
        world.agent_log("default")
    );
    // The stand-in holds right after its write, in the turn that has not ended: the Keeper
    // took the write's report before it was answered, so one read finds the round edited,
    // and a round marked only at the turn's end or later is not.
    assert!(
        wait_until(DEADLINE, || world.agent_log("default").did.iter().any(
            |did| matches!(
                did,
                Did::Began {
                    step: HOLD_AFTER_WRITE,
                    ..
                }
            )
        )),
        "{:?}",
        world.agent_log("default")
    );
    assert!(
        review_now(&mut client, &workspace, &review).rounds[1].edited,
        "the round is not shown as edited after the agent's first edit"
    );
    std::fs::write(&after_write, "").expect("let the agent go on");
    let Fact::TurnEnded { state, .. } = client.wait_for_fact(DEADLINE, is_turn_end) else {
        unreachable!()
    };
    let end_state = state.expect("a state recorded at the turn's end");
    assert_ne!(end_state, sent_state);

    // The thread moved to the line that replaced its own, and says that the code changed;
    // one thread is addressed and one waits for the reviewer; the reviewer resolved none.
    let after = review_where(&mut client, &workspace, &review, |r| {
        r.threads.len() == 3
            && r.threads[1].state == ThreadState::Addressed
            && r.threads[2].state == ThreadState::WaitsForReviewer
            && r.threads[1]
                .now
                .as_ref()
                .is_some_and(|now| now.tree == end_state)
    });
    let (moved, waiting) = (&after.threads[1], &after.threads[2]);
    assert_eq!((moved.id.clone(), waiting.id.clone()), (t1, t2));
    assert_eq!(
        moved.now,
        Some(Placed {
            tree: end_state.clone(),
            path: path("notes.txt"),
            side: Side::New,
            lines: Lines { first: 2, last: 2 },
            notice: Some(Notice::Changed),
        })
    );
    // The thread on the file nobody touched sits where it was written, with no notice. It
    // is carried in the same change as the first.
    assert!(
        waiting
            .now
            .as_ref()
            .is_none_or(|now| now.notice.is_none() && now.lines == Lines { first: 1, last: 1 }),
        "{waiting:?}"
    );
    assert!(moved.addressed && !waiting.addressed);
    assert!(!moved.resolved && !waiting.resolved);
    // Each answer is the agent's comment after the reviewer's, byte for byte.
    let last = |thread: &yaar_link::Thread| {
        let comment = thread.comments.last().expect("a comment");
        assert_eq!(comment.author, Author::Agent);
        comment.text.clone()
    };
    assert_eq!(last(moved), "Rewrote it: `delta`.");
    assert_eq!(last(waiting), "Why do you want `b` to change?");
    assert_eq!(after.rounds.len(), 2);
    assert!(after.rounds[1].edited);

    // What changed since the send is the one file the agent wrote, whether counted to the
    // files on disk or to the state at the turn's end.
    let changed = vec![ChangedFile {
        path: path("notes.txt"),
        status: FileStatus::Modified,
    }];
    assert_eq!(
        client
            .compare(&workspace, &sent_state, None)
            .expect("compare"),
        changed
    );
    assert_eq!(
        client
            .compare(&workspace, &sent_state, Some(&end_state))
            .expect("compare"),
        changed
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("notes.txt")).expect("the file"),
        rewritten
    );

    // Nobody committed.
    assert_eq!(checked_out(&repo), checked_out_at_the_start);
    assert_eq!(worktree_status(&repo), " M notes.txt");
    assert_no_commit_was_made(&mut client, &workspace);
    wait_for_exit(&world, "default");
    end(world, client);
}

// AGENT-19.
#[test]
fn accepting_the_offer_made_on_attaching_starts_the_cli_in_the_workspace_with_the_review_attached_to_its_first_message()
 {
    accepting_the_offer(Dialect::ClaudeCode);
}

// AGENT-19, as pi.
#[test]
fn accepting_the_offer_made_on_attaching_starts_the_cli_in_the_workspace_with_the_review_attached_to_its_first_message_as_pi()
 {
    accepting_the_offer(Dialect::Pi);
}

fn accepting_the_offer(dialect: Dialect) {
    // Both CLIs are installed whichever is accepted, so that a launcher that starts the other
    // one is found by the assertions on the agent of the session, not by a wait that runs out.
    let (world, repo) = World::with_standin_and_pi("default", &script(&["turn", "end", "exit"]));
    let checked_out_at_the_start = checked_out(&repo);
    assert_eq!(worktree_status(&repo), "", "all is committed");
    let mut client = world.client();
    let workspace = open(&mut client, &repo);

    // No agent ran here: the review is of no agent. Attaching a draft of it offers every
    // supported CLI, Claude Code and pi.
    let reviews = client.reviews(&workspace).reviews;
    let [no_agent] = &reviews[..] else {
        panic!("not exactly one review: {reviews:?}");
    };
    assert_eq!(no_agent.agent, None);
    let review = no_agent.id.clone();
    start_thread(
        &mut client,
        &review,
        1,
        "README.md",
        1,
        "# app",
        "Name it better.",
    );
    let offered = client.attach_review(&review, None).expect("attach");
    assert_eq!(
        offered
            .iter()
            .map(|cli| cli.agent.as_str())
            .collect::<Vec<_>>(),
        ["claude", "pi"]
    );

    // Accepting it starts the CLI in the workspace, and its session has the review.
    let launcher = client
        .open_program(
            &workspace,
            Program::AgentFor {
                agent: dialect.name().into(),
                review: review.clone(),
            },
            SIZE,
        )
        .expect("accept the offer");
    client.wait_for_prompt(&launcher, 0);
    let started = session_where(&mut client, &workspace, |s| s.state == SessionState::Idle);
    assert_eq!(
        started.agent,
        dialect.name(),
        "a launcher started the other CLI than the one accepted"
    );
    assert_eq!(started.terminal, Some(launcher.clone()));
    assert_eq!(started.review, Some(review.clone()));
    assert_eq!(started.attached, Some(Attachment::Review));
    // Nothing went to it before its first message: the hook of the session's start was
    // answered before the stand-in printed its prompt, and it answered nothing.
    let log = world.agent_log("default");
    assert!(prompt_answers(dialect, &log).is_empty(), "{log:?}");
    assert!(
        log.did.iter().all(|did| match did {
            Did::Hook { answer, .. } => answer.is_empty(),
            _ => true,
        }),
        "{log:?}"
    );

    // Its first message carries the review, and the comment is sent.
    client.type_line(&launcher, "go");
    assert!(
        wait_until(DEADLINE, || !prompt_answers(
            dialect,
            &world.agent_log("default")
        )
        .is_empty()),
        "{:?}",
        world.agent_log("default")
    );
    let answers = prompt_answers(dialect, &world.agent_log("default"));
    let handed = carried(dialect, &answers[0]);
    assert!(handed.contains("Name it better."), "{handed}");
    assert!(
        handed.contains(&format!("## Thread {}", id(2, 1))),
        "{handed}"
    );
    let sent = review_where(&mut client, &workspace, &review, |r| {
        r.threads[0].comments[0].round == Some(1)
    });
    assert_eq!(sent.rounds.len(), 1);
    assert_eq!(sent.agent, Some(started.id.clone()));
    wait_for_exit(&world, "default");

    // Two sessions that ended leave a review each, while a third runs: attaching the review
    // of the one that ended first offers its CLI, and accepting starts a launcher whose
    // session takes that review, not the one of the session that ended last, which the CLI's
    // next session would take but for the launcher, not the running session's, and nothing
    // goes to the one that runs.
    let other_repo = repository_with(&world, "other", &[("x.txt", "x\n")]);
    let other = open(&mut client, &other_repo);
    let running_shell = client.open_terminal(&other, SIZE).expect("a terminal");
    start_in(
        &world,
        &mut client,
        &running_shell,
        dialect,
        "running",
        &script(&["turn", "end", "exit"]),
    );
    client.wait_for_prompt(&running_shell, 0);
    let running = session_where(&mut client, &other, |s| {
        s.terminal.as_ref() == Some(&running_shell)
    });
    // Each of the other two waits at a gate outside a turn, so that both run at once and
    // each gets a review of its own, and ends when the test opens its gate.
    let mut gated = Vec::new();
    for name in ["first", "second"] {
        let gate = world
            .agent_script(name, &Script(vec![]))
            .with_extension("go");
        let shell = client.open_terminal(&other, SIZE).expect("a terminal");
        start_in(
            &world,
            &mut client,
            &shell,
            dialect,
            name,
            &Script(vec![Step::Hold(gate.clone()), Step::Exit]),
        );
        let session = session_where(&mut client, &other, |s| s.terminal.as_ref() == Some(&shell));
        gated.push((name, gate, session));
    }
    let reviews_of: Vec<_> = gated
        .iter()
        .map(|(_, _, session)| session.review.clone().expect("a review of its own"))
        .collect();
    assert_ne!(reviews_of[0], reviews_of[1]);
    for review in &reviews_of {
        assert_ne!(Some(review), running.review.as_ref());
    }
    // The first ends, then the second: the second is the session that ended last.
    for (name, gate, session) in &gated {
        std::fs::write(gate, "").expect("open the gate");
        wait_for_exit(&world, name);
        session_where(&mut client, &other, |s| {
            s.id == session.id && s.state == SessionState::NotRunning
        });
    }
    let left = reviews_of[0].clone();
    start_thread(&mut client, &left, 2, "x.txt", 1, "x", "Why x?");
    let offered = client.attach_review(&left, None).expect("attach");
    assert_eq!(
        offered
            .iter()
            .map(|cli| cli.agent.as_str())
            .collect::<Vec<_>>(),
        [dialect.name()]
    );
    let second_launcher = client
        .open_program(
            &other,
            Program::AgentFor {
                agent: dialect.name().into(),
                review: left.clone(),
            },
            SIZE,
        )
        .expect("accept the offer");
    client.wait_for_prompt(&second_launcher, 0);
    let launched = session_where(&mut client, &other, |s| {
        s.terminal.as_ref() == Some(&second_launcher)
    });
    assert_eq!(
        launched.agent,
        dialect.name(),
        "a launcher started the other CLI than the one accepted"
    );
    assert_eq!(launched.review, Some(left.clone()));
    assert_eq!(launched.attached, Some(Attachment::Review));
    // The running session and the session that ended last keep their own reviews.
    let held = sessions_of(&mut client, &other);
    let review_of = |session: &Session| {
        held.iter()
            .find(|s| s.id == session.id)
            .unwrap_or_else(|| panic!("{session:?} is not listed in {held:?}"))
            .review
            .clone()
    };
    assert_eq!(review_of(&running), running.review);
    assert_eq!(review_of(&gated[1].2), Some(reviews_of[1].clone()));
    let reviews = client.reviews(&other).reviews;
    assert_eq!(reviews.len(), 3, "{reviews:?}");
    assert_eq!(
        reviews
            .iter()
            .find(|r| r.id == left)
            .and_then(|r| r.agent.clone()),
        Some(launched.id.clone())
    );

    // Its first message carries the review; the message of the session that runs, typed
    // after the attach, carries nothing.
    client.type_line(&second_launcher, "go");
    assert!(
        wait_until(DEADLINE, || !prompt_answers(
            dialect,
            &world.agent_log("default")
        )
        .is_empty()),
        "{:?}",
        world.agent_log("default")
    );
    let answers = prompt_answers(dialect, &world.agent_log("default"));
    assert!(
        carried(dialect, &answers[0]).contains("Why x?"),
        "{answers:?}"
    );
    client.type_line(&running_shell, "hello");
    wait_for_exit(&world, "running");
    let answers = prompt_answers(dialect, &world.agent_log("running"));
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(carried(dialect, &answers[0]), "");
    wait_for_exit(&world, "default");
    assert_eq!(checked_out(&repo), checked_out_at_the_start);
    assert_eq!(worktree_status(&repo), "");
    end(world, client);
}
