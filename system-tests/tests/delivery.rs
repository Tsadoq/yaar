//! System tests of delivering a review: the built Keeper, the app's hook program and reply
//! tool server or its reporting program for pi's extension, and the stand-in agent as Claude
//! Code and as pi, read through the Link as a Window reads it and through the stand-in's own
//! log, which is the agent's side.
//!
//! Each test is a function of a [`Dialect`]: the test function that carries the row's name
//! runs it as Claude Code, and the one with `_as_pi` after the name runs the same body as
//! pi, in a world whose pi is installed and whose user said yes to the app's extension.
//! What a CLI calls the moment its user's message is taken, and how it hands the model a
//! review, comes from the dialect; the rest of a body is the same.
//!
//! No test here commits in the worktree: the stand-in's scripts edit files and never run
//! git, and each test that edits says what the repository is like when it ends.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use yaar_link::{
    Anchor, Attachment, Author, CommentId, DraftOn, Fact, HistoryEntry, Lines, Review, ReviewId,
    SentBy, Session, SessionState, Side, Step as Taken, TerminalId, TerminalSize, Thread, ThreadId,
    ThreadState, TurnEnd, WorkspaceAgents, WorkspaceId,
};
use yaar_system_tests::standin::{Did, Script, StandinLog, Step};
use yaar_system_tests::{
    Client, DEADLINE, Dialect, Repo, World, descendants_of, is_alive, wait_until,
};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// The name of the reply tool as the instruction of a review delivered to `dialect` names
/// it: the system tests do not depend on the Adapters, so they look for the tool's own name
/// where the Adapter's constant would be.
fn reply_tool(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::ClaudeCode => "mcp__plugin_yaar-review_yaar__reply_to_thread",
        Dialect::Pi => "reply_to_thread",
    }
}

/// A script of the steps in `words`, each the way `Script::parse` reads it.
fn script(words: &[&str]) -> Script {
    Script::parse(&words.join("\n"))
}

/// Opens the folder of `repo` as a workspace.
fn open(client: &mut Client, repo: &Repo) -> WorkspaceId {
    client.open_folder(repo.path()).expect("open the folder").1
}

/// The id of the `n`th thing of a kind: 32 digits, as the Window draws them.
fn id(kind: u128, n: u128) -> String {
    format!("{:032x}", (kind << 64) + n)
}

/// An anchor of one line of a file on disk, which the Keeper puts on a state it records.
fn on_disk(path: &str, line: u32, text: &str) -> Anchor {
    Anchor {
        tree: None,
        path: PathBuf::from(path),
        side: Side::New,
        lines: Lines {
            first: line,
            last: line,
        },
        text: text.to_owned(),
    }
}

/// Writes a comment of the reviewer on a new thread, the `n`th of the test, as a Window
/// saves it, and gives the thread's id.
fn comment_on_a_new_thread(
    client: &mut Client,
    review: &ReviewId,
    n: u128,
    anchor: Anchor,
    text: &str,
) -> ThreadId {
    let thread = ThreadId::new(id(1, n));
    client
        .save_draft(
            review,
            &thread,
            &CommentId::new(id(2, n)),
            DraftOn::NewThread { anchor },
            text,
        )
        .expect("save the draft");
    thread
}

/// The review of `session` in the reviews of `workspace`, as it is now.
fn review_of(client: &mut Client, workspace: &WorkspaceId, session: &Session) -> Review {
    client
        .reviews(workspace)
        .reviews
        .into_iter()
        .find(|review| review.agent.as_ref() == Some(&session.id))
        .unwrap_or_else(|| panic!("no review belongs to {session:?}"))
}

/// The thread `id` of `review`.
fn thread_of<'a>(review: &'a Review, id: &ThreadId) -> &'a Thread {
    review
        .threads
        .iter()
        .find(|thread| thread.id == *id)
        .unwrap_or_else(|| panic!("no thread {id} in {review:?}"))
}

/// Looks at the Keeper until `look` finds something, and gives it: once now, and again at
/// each fact `relevant` accepts, up to the deadline. Nothing sleeps; a fact that came
/// before the look is no loss, since each look asks the Keeper itself.
fn wait_for<T>(
    client: &mut Client,
    mut look: impl FnMut(&mut Client) -> Option<T>,
    relevant: impl Fn(&Fact) -> bool,
) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(found) = look(client) {
            return found;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the Keeper never came to show it");
        client.wait_for_fact(left, &relevant);
    }
}

/// The sessions of `workspace`, once `wanted` accepts them.
fn sessions_when(
    client: &mut Client,
    workspace: &WorkspaceId,
    wanted: impl Fn(&WorkspaceAgents) -> bool,
) -> WorkspaceAgents {
    let of = workspace.clone();
    wait_for(
        client,
        |client| {
            client
                .agents()
                .into_iter()
                .find(|agents| agents.workspace == of && wanted(agents))
        },
        |fact| matches!(fact, Fact::AgentsChanged(agents) if agents.workspace == *workspace),
    )
}

/// The review of `session`, once `wanted` accepts it.
fn review_when(
    client: &mut Client,
    workspace: &WorkspaceId,
    session: &Session,
    wanted: impl Fn(&Review) -> bool,
) -> Review {
    wait_for(
        client,
        |client| Some(review_of(client, workspace, session)).filter(|review| wanted(review)),
        |fact| matches!(fact, Fact::ReviewsChanged { workspace: of } if of == workspace),
    )
}

/// The one session of `agents`.
fn only_session(agents: &WorkspaceAgents) -> &Session {
    let [session] = &agents.sessions[..] else {
        panic!("not exactly one session: {agents:?}");
    };
    session
}

/// Waits until what the stand-in `name` of `world` wrote down satisfies `wanted`, and gives
/// it. The log is read again until it does: what a test waits for is a fact in it.
fn log_when(world: &World, name: &str, wanted: impl Fn(&StandinLog) -> bool) -> StandinLog {
    assert!(
        wait_until(DEADLINE, || wanted(&world.agent_log(name))),
        "the log of {name} never showed it: {:?}",
        world.agent_log(name)
    );
    world.agent_log(name)
}

/// What the report of each taken message of `dialect` (Claude Code's `UserPromptSubmit`,
/// pi's `input`) answered according to the log, with the bytes the stand-in had read when
/// it did, in order.
fn prompts(dialect: Dialect, log: &StandinLog) -> Vec<(&[u8], usize)> {
    log.did
        .iter()
        .filter_map(|did| match did {
            Did::Hook {
                event,
                answer,
                received,
                ..
            } if event == dialect.message_event() => Some((answer.as_slice(), *received)),
            _ => None,
        })
        .collect()
}

/// Whether the stand-in wrote down that it ended.
fn has_exited(log: &StandinLog) -> bool {
    log.did.iter().any(|did| matches!(did, Did::Exited { .. }))
}

/// Starts the stand-in as `dialect`'s CLI in `terminal`, a shell of the app, with the script
/// and the log of the name `name`: the world writes `<name>.script`, and the stand-in writes
/// `<name>.log`.
fn start_in(
    dialect: Dialect,
    world: &World,
    client: &mut Client,
    terminal: &TerminalId,
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

/// The bytes of a file; panics, naming it, when it cannot be read.
fn bytes_of(file: &Path) -> Vec<u8> {
    fs::read(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()))
}

/// What a repository holds that no commit of the test made, as the tests that must show
/// nobody committed read it before they start.
struct Untouched {
    head: String,
    branch: String,
    commits: String,
    index: Vec<u8>,
}

impl Untouched {
    /// Reads the repository, the index first: git refreshes it when it reports on status.
    fn of(repo: &Repo) -> Untouched {
        let index = bytes_of(&repo.path().join(".git/index"));
        Untouched {
            head: repo.git(&["rev-parse", "HEAD"]),
            branch: repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]),
            commits: repo.git(&["rev-list", "--count", "HEAD"]),
            index,
        }
    }

    /// Panics unless the repository still holds exactly this: the same commit on the same
    /// branch, the same number of commits, the same index.
    fn holds_in(&self, repo: &Repo) {
        let index = bytes_of(&repo.path().join(".git/index"));
        assert_eq!(index, self.index, "the index changed");
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), self.head, "HEAD moved");
        assert_eq!(
            repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]),
            self.branch,
            "the branch changed"
        );
        assert_eq!(
            repo.git(&["rev-list", "--count", "HEAD"]),
            self.commits,
            "a commit was made"
        );
    }
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

// UITEST-04.
#[test]
fn a_review_round_goes_from_a_comment_through_the_attach_and_the_users_message_to_the_agents_reply_and_nothing_reaches_the_agent_before_enter()
 {
    a_review_round_goes_from_a_comment_to_the_agents_reply(Dialect::ClaudeCode);
}

// UITEST-04, as pi.
#[test]
fn a_review_round_goes_from_a_comment_through_the_attach_and_the_users_message_to_the_agents_reply_and_nothing_reaches_the_agent_before_enter_as_pi()
 {
    a_review_round_goes_from_a_comment_to_the_agents_reply(Dialect::Pi);
}

fn a_review_round_goes_from_a_comment_to_the_agents_reply(dialect: Dialect) {
    let body = "He said \"yes\", it's `fine`\nsecond line; $(touch nothing)";
    let round = Script(vec![
        Step::Turn,
        Step::Reply {
            thread: 1,
            addressed: true,
            body: body.to_owned(),
        },
        Step::End,
        Step::Exit,
    ]);
    let (world, repo) = World::with_standin_as(dialect, "default", &round);
    // A change nobody committed, so that the index and the status the test compares at the
    // end have something to lose: a recording that wrote the worktree's own index would
    // stage it.
    repo.write("README.md", "# app\nan uncommitted line\n");
    let status = repo.git(&["status", "--porcelain"]);
    assert!(status.contains("README.md"), "{status}");
    let untouched = Untouched::of(&repo);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client
        .open_launcher(&workspace, dialect.name(), SIZE)
        .expect("open the launcher");
    client.wait_for_prompt(&terminal, 0);
    let agents = sessions_when(&mut client, &workspace, |agents| {
        agents
            .sessions
            .iter()
            .any(|session| session.state == SessionState::Idle)
    });
    let session = only_session(&agents).clone();
    assert_eq!(session.agent, dialect.name());
    let review = review_of(&mut client, &workspace, &session);
    assert_eq!(session.review, Some(review.id.clone()));

    // A comment, then the attach: the user's next message is to carry it.
    let thread = comment_on_a_new_thread(
        &mut client,
        &review.id,
        1,
        on_disk("README.md", 1, "# app"),
        "Please give this a better title.",
    );
    assert_eq!(client.attach_review(&review.id, None), Ok(vec![]));
    let attached = review_of(&mut client, &workspace, &session);
    assert_eq!(attached.attached, Some(Attachment::Review));

    // What the view control shows: the instruction, then the review.
    let view = client.attachment(&review.id).expect("the view");
    // The instruction comes first: the view opens with the sentence the instruction of
    // either Adapter opens with, and names the reply tool of this CLI; the review follows.
    assert!(
        view.starts_with("This is a code review of your changes, attached to the user's message."),
        "{view}"
    );
    assert!(view.contains(reply_tool(dialect)), "{view}");
    // The bare name of pi's tool is a part of Claude Code's listed name, so the prefix of
    // Claude Code's tools must be in the one view and not in the other.
    assert_eq!(
        view.contains("mcp__plugin_yaar-review_yaar__"),
        dialect == Dialect::ClaudeCode,
        "{view}"
    );
    let instruction_ends = view
        .find("Only the reviewer resolves a thread.")
        .unwrap_or_else(|| panic!("{view}"));
    assert!(
        view.find(&format!("Thread {thread}")) > Some(instruction_ends),
        "{view}"
    );
    assert!(
        view.contains(&format!("Thread {thread}")) && view.contains("Please give this a better"),
        "{view}"
    );

    // The user types a message and does not press Enter. Once the stand-in has read the
    // keys, which shows that what is typed reaches it, nothing else did: it made no
    // report of the message (the dialect's message_event), and the comment is still a draft.
    client.type_into(&terminal, b"Please look");
    let before_enter = log_when(&world, "default", |log| {
        log.received.ends_with(b"Please look")
    });
    assert_eq!(before_enter.received, b"Please look");
    assert!(
        prompts(dialect, &before_enter).is_empty(),
        "a message was submitted: {:?}",
        before_enter.did
    );
    let waiting = review_of(&mut client, &workspace, &session);
    assert_eq!(waiting.attached, Some(Attachment::Review));
    assert_eq!(waiting.rounds, vec![]);
    assert_eq!(thread_of(&waiting, &thread).comments[0].round, None);

    // Enter: the message goes, and the report's answer carries what the view showed.
    client.type_into(&terminal, b"\r");
    let submitted = log_when(&world, "default", |log| !prompts(dialect, log).is_empty());
    let [(answer, _)] = prompts(dialect, &submitted)[..] else {
        panic!("not exactly one message: {:?}", submitted.did);
    };
    assert_eq!(dialect.review_in(answer), Some(view));

    // The comment is sent, in round 1, delivered with the message.
    let sent = review_when(&mut client, &workspace, &session, |review| {
        review.attached.is_none() && !review.rounds.is_empty()
    });
    assert_eq!(sent.rounds.len(), 1, "{sent:?}");
    assert_eq!(sent.rounds[0].number, 1);
    assert_eq!(sent.rounds[0].sent_by, SentBy::Delivered);
    // The reply may be there already: the agent answers as soon as it has the review.
    let comments = &thread_of(&sent, &thread).comments;
    assert_eq!(comments[0].author, Author::Reviewer);
    assert_eq!(comments[0].round, Some(1));

    // The agent's reply, through the reply tool, is on the thread byte for byte, and the
    // thread is addressed and not resolved.
    let replied = log_when(&world, "default", |log| {
        log.did.iter().any(|did| matches!(did, Did::Tool { .. }))
    });
    let tool = replied
        .did
        .iter()
        .find_map(|did| match did {
            Did::Tool { answer, .. } => Some(answer),
            _ => None,
        })
        .expect("the reply tool was called");
    // Claude Code's tool server answers with an error flag; pi's reporting program prints
    // the text the extension gives the model, or an error, and the stand-in writes down
    // `{"message": ..}` when the program printed nothing it could read.
    match dialect {
        Dialect::ClaudeCode => {
            assert_ne!(tool["isError"], true, "the reply tool answered {tool}");
        }
        Dialect::Pi => assert!(
            tool["text"].as_str().is_some_and(|text| {
                text.starts_with(&format!("recorded reply to {thread} (addressed, "))
            }) && tool["error"].is_null(),
            "the reply tool answered {tool}"
        ),
    }
    let after = review_of(&mut client, &workspace, &session);
    let thread_now = thread_of(&after, &thread);
    let [first, reply] = &thread_now.comments[..] else {
        panic!("not a comment and a reply: {thread_now:?}");
    };
    assert_eq!(first.author, Author::Reviewer);
    assert_eq!(reply.author, Author::Agent);
    assert_eq!(reply.text, body);
    assert_eq!(thread_now.state, ThreadState::Addressed);
    assert!(
        thread_now.addressed && !thread_now.resolved,
        "{thread_now:?}"
    );

    // The stand-in ends, and what it received is what the test typed and nothing more.
    let over = log_when(&world, "default", has_exited);
    assert_eq!(over.received, b"Please look\r");

    // The Keeper records the state of the worktree for the round and for the turn's end on a
    // thread of its own, and keeps the turn's state last. Until the history holds the ended
    // turn with its state and the round holds its own, its git work is not over, and a look
    // at the repository would come before whatever it might do.
    wait_for(
        &mut client,
        |client| {
            let history = client.history(&workspace).ok()?;
            let turn_recorded = history.entries.iter().any(|entry| {
                matches!(
                    entry,
                    HistoryEntry::Turn { session: of, state: Some(_), .. } if *of == session.id
                )
            });
            let round_recorded = review_of(client, &workspace, &session)
                .rounds
                .first()
                .is_some_and(|round| round.state.is_some());
            (turn_recorded && round_recorded).then_some(())
        },
        |fact| matches!(fact, Fact::TurnEnded { .. } | Fact::ReviewsChanged { .. }),
    );

    // Nobody committed, and the app left no file in the worktree.
    untouched.holds_in(&repo);
    assert_eq!(repo.git(&["status", "--porcelain"]), status);
    end(world, client);
}

// AGENT-08.
#[test]
fn a_review_attached_while_the_agent_works_goes_with_the_message_submitted_then_and_is_sent_when_the_agent_takes_it()
 {
    a_review_attached_while_the_agent_works_goes_with_the_message_submitted_then(
        Dialect::ClaudeCode,
    );
}

// AGENT-08, as pi: the line typed during the hold is the `input` of a message queued with
// `steer`.
#[test]
fn a_review_attached_while_the_agent_works_goes_with_the_message_submitted_then_and_is_sent_when_the_agent_takes_it_as_pi()
 {
    a_review_attached_while_the_agent_works_goes_with_the_message_submitted_then(Dialect::Pi);
}

fn a_review_attached_while_the_agent_works_goes_with_the_message_submitted_then(dialect: Dialect) {
    let (world, repo) = World::with_standin_as(dialect, "default", &Script(vec![]));
    repo.write("notes.txt", "alpha\n");
    repo.commit("Add the notes");
    // The turn is held outside the worktree until the test makes the file.
    let gate = repo.path().join("../go");
    let held = script(&[
        "turn",
        &format!("hold {}", gate.display()),
        "edit notes.txt more",
        "end",
        "exit",
    ]);
    world.agent_script("default", &held);
    let untouched = Untouched::of(&repo);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let terminal = client
        .open_launcher(&workspace, dialect.name(), SIZE)
        .expect("open the launcher");
    client.wait_for_prompt(&terminal, 0);
    let agents = sessions_when(&mut client, &workspace, |agents| {
        agents
            .sessions
            .iter()
            .any(|session| session.state == SessionState::Idle)
    });
    let session = only_session(&agents).clone();
    assert_eq!(session.agent, dialect.name());
    let review = review_of(&mut client, &workspace, &session);

    // A message and Enter: the agent works, and holds at the tool that runs.
    client.type_line(&terminal, "start the work");
    sessions_when(&mut client, &workspace, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    // Step 0 is the turn, step 1 the hold.
    log_when(&world, "default", |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::Began { step: 1, .. }))
    });
    assert_eq!(prompts(dialect, &world.agent_log("default")).len(), 1);

    // While it works, a review is attached.
    let thread = comment_on_a_new_thread(
        &mut client,
        &review.id,
        1,
        on_disk("notes.txt", 1, "alpha"),
        "Say more about alpha.",
    );
    assert_eq!(client.attach_review(&review.id, None), Ok(vec![]));
    let view = client.attachment(&review.id).expect("the view");
    let attached = review_of(&mut client, &workspace, &session);
    assert_eq!(attached.attached, Some(Attachment::Review));
    assert_eq!(attached.rounds, vec![]);
    assert_eq!(thread_of(&attached, &thread).comments[0].round, None);

    // And a message is submitted: the report of the queued message runs at Enter (Claude
    // Code's hook, pi's `input` of `steer`), while the hold goes on, and its answer carries
    // the review.
    client.type_line(&terminal, "also this");
    let queued = log_when(&world, "default", |log| prompts(dialect, log).len() == 2);
    // The stand-in is still in its hold: the report was made at Enter, not after the tool.
    assert!(
        !gate.exists()
            && !queued
                .did
                .iter()
                .any(|did| matches!(did, Did::Done { step: 1, .. })),
        "the hold ended before the message was taken: {:?}",
        queued.did
    );
    let (answer, read) = prompts(dialect, &queued)[1];
    let context = dialect
        .review_in(answer)
        .expect("the answer carries the review");
    assert!(
        context.contains(&format!("Thread {thread}")) && context.contains("Say more about alpha."),
        "{context}"
    );
    assert_eq!(context, view);
    assert!(
        queued.received[..read].ends_with(b"also this\r"),
        "the stand-in had read {:?}",
        String::from_utf8_lossy(&queued.received[..read])
    );

    // The comment is sent when the agent reports the message, which is when it takes it for
    // both CLIs, still before the hold ends.
    let sent = review_when(&mut client, &workspace, &session, |review| {
        review.attached.is_none() && !review.rounds.is_empty()
    });
    assert!(
        !gate.exists()
            && !world
                .agent_log("default")
                .did
                .iter()
                .any(|did| matches!(did, Did::Done { step: 1, .. })),
        "the comment was sent only once the hold ended"
    );
    assert_eq!(sent.rounds[0].number, 1);
    assert_eq!(sent.rounds[0].sent_by, SentBy::Delivered);
    assert_eq!(thread_of(&sent, &thread).comments[0].round, Some(1));

    // The hold ends, and the turn with it: the turn that runs when the file is made, which
    // is the one the message joined, ends finished, with the edit its hold kept waiting.
    let running = client
        .agents()
        .into_iter()
        .flat_map(|agents| agents.sessions)
        .find(|candidate| candidate.id == session.id)
        .and_then(|candidate| candidate.turn)
        .expect("the turn still runs");
    fs::write(&gate, "").expect("let the turn go on");
    client.wait_for_fact(
        DEADLINE,
        |fact| matches!(fact, Fact::TurnEnded { turn, .. } if *turn == running),
    );
    let kept = client.turn(&running).expect("the turn is kept");
    assert_eq!(kept.ended, Some(TurnEnd::Finished));
    assert!(
        kept.steps
            .iter()
            .any(|step| matches!(step, Taken::Edit { path, .. } if path == Path::new("notes.txt"))),
        "{:?}",
        kept.steps
    );
    let over = log_when(&world, "default", has_exited);
    assert_eq!(
        prompts(dialect, &over).len(),
        2,
        "no third message: {:?}",
        over.did
    );
    untouched.holds_in(&repo);
    end(world, client);
}

// AGENT-14. The stand-in is started under the dialect's name, which is also the name of its
// script and its log: `start_agent_outside` runs the program of that name.
#[test]
fn an_agent_that_reports_over_the_socket_and_runs_in_no_terminal_is_followed_receives_a_review_and_replies_to_its_threads()
 {
    an_agent_that_runs_in_no_terminal_is_followed_receives_a_review_and_replies(
        Dialect::ClaudeCode,
    );
}

// AGENT-14, as pi: the stand-in as pi runs outside every terminal of the app, with the
// extension its settings list and the reporting program named by its environment.
#[test]
fn an_agent_that_reports_over_the_socket_and_runs_in_no_terminal_is_followed_receives_a_review_and_replies_to_its_threads_as_pi()
 {
    an_agent_that_runs_in_no_terminal_is_followed_receives_a_review_and_replies(Dialect::Pi);
}

fn an_agent_that_runs_in_no_terminal_is_followed_receives_a_review_and_replies(dialect: Dialect) {
    let name = dialect.name();
    let body = "Not sure: `a` or \"b\"?\nTell me which.";
    let outside = Script(vec![
        Step::Turn,
        Step::End,
        Step::Turn,
        Step::Reply {
            thread: 1,
            addressed: false,
            body: body.to_owned(),
        },
        Step::End,
        Step::Exit,
    ]);
    let (world, repo) = World::with_standin_as(dialect, name, &outside);
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    // As pi, `start_agent_outside` lists the extension in pi's settings itself, so the
    // user's yes given in `World::with_standin_as` is not what lets the stand-in report here.
    let mut agent = world.start_agent_outside(dialect, &outside, repo.path());
    agent.wait_for_prompt();

    // Followed: a session of the worktree's workspace that runs in no terminal of the app.
    let agents = sessions_when(&mut client, &workspace, |agents| {
        agents
            .sessions
            .iter()
            .any(|session| session.state == SessionState::Idle)
    });
    let session = only_session(&agents).clone();
    assert_eq!(session.terminal, None);
    assert_eq!(session.agent, name);
    let review = review_of(&mut client, &workspace, &session);

    // A first message: it works, and its turn ends.
    agent.type_line("make a change");
    // The turn is over at once, so the state is waited for in the fact that told it: a look
    // at the sessions could come after it was gone.
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    let Fact::TurnEnded { turn, .. } = client.wait_for_fact(
        DEADLINE,
        |fact| matches!(fact, Fact::TurnEnded { workspace: of, .. } if *of == workspace),
    ) else {
        unreachable!("the wait accepts a turn's end");
    };
    assert_eq!(
        client.turn(&turn).expect("the turn is kept").ended,
        Some(TurnEnd::Finished)
    );
    let first = log_when(&world, name, |log| {
        log.did
            .iter()
            .any(|did| matches!(did, Did::Hook { event, .. } if event == dialect.turn_end_event()))
    });
    assert_eq!(prompts(dialect, &first).len(), 1);
    let turns = client
        .history(&workspace)
        .expect("the history")
        .entries
        .len();
    assert_eq!(turns, 1, "one turn of the agent was followed");

    // A review, attached, goes with its second message.
    agent.wait_for_prompt();
    let thread = comment_on_a_new_thread(
        &mut client,
        &review.id,
        1,
        on_disk("README.md", 1, "# app"),
        "Which title do you want?",
    );
    assert_eq!(client.attach_review(&review.id, None), Ok(vec![]));
    let view = client.attachment(&review.id).expect("the view");
    agent.type_line("now the review");
    let second = log_when(&world, name, |log| prompts(dialect, log).len() == 2);
    assert_eq!(
        dialect.review_in(prompts(dialect, &second)[1].0),
        Some(view)
    );
    let sent = review_when(&mut client, &workspace, &session, |review| {
        review.attached.is_none() && !review.rounds.is_empty()
    });
    assert_eq!(thread_of(&sent, &thread).comments[0].round, Some(1));

    // Its reply, without the addressed mark, leaves the thread waiting for the reviewer.
    log_when(&world, name, |log| {
        log.did.iter().any(|did| matches!(did, Did::Tool { .. }))
    });
    let after = review_of(&mut client, &workspace, &session);
    let thread_now = thread_of(&after, &thread);
    let [_, reply] = &thread_now.comments[..] else {
        panic!("not a comment and a reply: {thread_now:?}");
    };
    assert_eq!((reply.author, reply.text.as_str()), (Author::Agent, body));
    assert_eq!(thread_now.state, ThreadState::WaitsForReviewer);
    assert!(
        !thread_now.addressed && !thread_now.resolved,
        "{thread_now:?}"
    );

    // It is still in no terminal, and the app opened none for it.
    log_when(&world, name, has_exited);
    assert_eq!(client.terminals(), vec![]);
    end(world, client);
}

// DELIVER-04.
#[test]
fn a_review_attached_to_one_of_two_sessions_in_one_worktree_is_received_by_that_session_only() {
    a_review_attached_to_one_of_two_sessions_is_received_by_that_session_only(Dialect::ClaudeCode);
}

// DELIVER-04, as pi: two sessions of pi, each reporting through the reporting program of its
// own shell.
#[test]
fn a_review_attached_to_one_of_two_sessions_in_one_worktree_is_received_by_that_session_only_as_pi()
{
    a_review_attached_to_one_of_two_sessions_is_received_by_that_session_only(Dialect::Pi);
}

fn a_review_attached_to_one_of_two_sessions_is_received_by_that_session_only(dialect: Dialect) {
    let (world, repo) = World::with_standin_as(dialect, "default", &Script(vec![]));
    let mut client = world.client();
    let workspace = open(&mut client, &repo);
    let talk = script(&["turn", "end", "turn", "end", "exit"]);
    let one = client.open_terminal(&workspace, SIZE).expect("a terminal");
    let two = client.open_terminal(&workspace, SIZE).expect("a terminal");

    // Two sessions of one worktree, each in a shell of its own with a script and a log of
    // its own, and each with a review.
    start_in(dialect, &world, &mut client, &one, "a", &talk);
    let at_one = client.wait_for_prompt(&one, 0);
    sessions_when(&mut client, &workspace, |agents| agents.sessions.len() == 1);
    start_in(dialect, &world, &mut client, &two, "b", &talk);
    let at_two = client.wait_for_prompt(&two, 0);
    let both = sessions_when(&mut client, &workspace, |agents| {
        agents.sessions.len() == 2
            && agents
                .sessions
                .iter()
                .all(|session| session.state == SessionState::Idle)
    });
    let session_in = |terminal: &TerminalId| {
        both.sessions
            .iter()
            .find(|session| session.terminal.as_ref() == Some(terminal))
            .expect("a session in the terminal")
            .clone()
    };
    let (a, b) = (session_in(&one), session_in(&two));
    let (review_a, review_b) = (
        review_of(&mut client, &workspace, &a),
        review_of(&mut client, &workspace, &b),
    );
    assert_ne!(review_a.id, review_b.id);

    // A review attached to A.
    let thread = comment_on_a_new_thread(
        &mut client,
        &review_a.id,
        1,
        on_disk("README.md", 1, "# app"),
        "A, please retitle this.",
    );
    assert_eq!(client.attach_review(&review_a.id, None), Ok(vec![]));

    // A message to B and Enter: B's report of the message answers nothing, and A's review is
    // still attached with its draft. That report, in B's log, is the barrier: the Keeper
    // answered it after it decided what went with it.
    client.type_line(&two, "hello B");
    let hello_b = log_when(&world, "b", |log| !prompts(dialect, log).is_empty());
    assert!(
        prompts(dialect, &hello_b)[0].0.is_empty(),
        "B's answer: {:?}",
        String::from_utf8_lossy(prompts(dialect, &hello_b)[0].0)
    );
    let kept = review_of(&mut client, &workspace, &a);
    assert_eq!(kept.attached, Some(Attachment::Review));
    assert_eq!(kept.rounds, vec![]);
    assert_eq!(thread_of(&kept, &thread).comments[0].round, None);
    assert_eq!(review_of(&mut client, &workspace, &b).rounds, vec![]);

    // A message to A: A's answer carries A's review.
    let thread_id = thread.as_str().as_bytes();
    let holds = |log: &StandinLog| {
        log.did.iter().any(|did| {
            matches!(did, Did::Hook { answer, .. }
                if answer.windows(thread_id.len()).any(|window| window == thread_id))
        })
    };
    client.type_line(&one, "hello A");
    let hello_a = log_when(&world, "a", |log| !prompts(dialect, log).is_empty());
    let context = dialect
        .review_in(prompts(dialect, &hello_a)[0].0)
        .expect("A's answer carries a review");
    assert!(
        context.contains(&format!("Thread {thread}")) && context.contains("A, please retitle"),
        "{context}"
    );
    let sent = review_when(&mut client, &workspace, &a, |review| {
        review.attached.is_none() && !review.rounds.is_empty()
    });
    assert_eq!(thread_of(&sent, &thread).comments[0].round, Some(1));
    let b_so_far = world.agent_log("b");
    assert!(
        !holds(&b_so_far),
        "B received A's review: {:?}",
        b_so_far.did
    );

    // Both go on to a second message and end. B's log never held the thread of A's review
    // in any answer.
    client.wait_for_prompt(&one, at_one);
    client.wait_for_prompt(&two, at_two);
    client.type_line(&one, "bye A");
    client.type_line(&two, "bye B");
    for name in ["a", "b"] {
        log_when(&world, name, has_exited);
    }
    let (log_a, log_b) = (world.agent_log("a"), world.agent_log("b"));
    assert!(
        holds(&log_a),
        "A never received the thread: {:?}",
        log_a.did
    );
    assert!(!holds(&log_b), "B received A's review: {:?}", log_b.did);
    let b_answers: Vec<_> = prompts(dialect, &log_b)
        .into_iter()
        .map(|(answer, _)| answer)
        .collect();
    assert!(
        b_answers.iter().all(|answer| answer.is_empty()),
        "B's answers: {b_answers:?}"
    );
    end(world, client);
}
