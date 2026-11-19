//! Component tests: who is credited with a change, through the calls the Keeper makes, in
//! real repositories the tests make, with the real watcher on them and a real database.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use support::{
    Heard, Scratch, ago, commit_all, git, init, program, touch, wait_until, workspaces, write,
};
use yaar_adapter_claude::ClaudeCode;
use yaar_agent::{Adapter, Event, Phase, Report};
use yaar_system_tests::recorded::{self, HookRecord};
use yaar_workspaces::{Change, CreditedPath, Refusal, WorkspaceError, WorkspaceId, Workspaces};

/// A repository with the files given committed, opened and watched. The scratch folder
/// stays where the test can see it.
struct Watched {
    workspaces: Workspaces,
    workspace: WorkspaceId,
    root: PathBuf,
    heard: Heard,
}

/// A repository with `edited.txt` and `kept.txt` committed, opened and watched.
fn watched(scratch: &Scratch) -> Watched {
    watched_with(scratch, &[("edited.txt", "one\n"), ("kept.txt", "kept\n")])
}

fn watched_with(scratch: &Scratch, files: &[(&str, &str)]) -> Watched {
    let root = scratch.join("repo");
    init(&root);
    for (path, content) in files {
        write(&root, path, content);
    }
    commit_all(&root, "first");
    let workspaces = workspaces(scratch);
    let workspace = workspaces.open_folder(&root).expect("open").workspace;
    let heard = Heard::default();
    workspaces
        .watch(&workspace, heard.told())
        .expect("watch the worktree");
    Watched {
        workspaces,
        workspace,
        root,
        heard,
    }
}

impl Watched {
    fn credits(&self) -> Vec<CreditedPath> {
        self.workspaces.credits(&self.workspace).expect("credits")
    }

    /// The sessions the change of `path` is credited to, none where it is not listed.
    fn by(&self, path: &str) -> Option<Vec<String>> {
        self.credits()
            .into_iter()
            .find(|one| one.path == Path::new(path))
            .map(|one| one.by)
    }

    /// Waits until the watcher has told of a change of files after this call, the way a
    /// test learns that the ledger has noted a write it made before.
    fn wait_for_a_notice_of_files(&self, since: usize) {
        wait_until("the watcher told of the files", || {
            self.heard.files() > since
        });
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_file_changed_by_an_editor_outside_the_app_shows_as_a_change_no_agent_made() {
    let scratch = Scratch::new();
    let w = watched(&scratch);

    // A file an agent's tool named, in the same worktree, to show that the rule tells the
    // two apart.
    write(&w.root, "agent.txt", "from the agent\n");
    w.workspaces
        .agent_wrote(&w.workspace, "agent-session", &w.root.join("agent.txt"));

    // An editor: a program of the user that is no supported agent CLI. First one that
    // writes the file itself, then a child process that writes through its shell.
    let before = w.heard.files();
    std::fs::write(w.root.join("edited.txt"), "two, by hand\n").expect("edit the file");
    w.wait_for_a_notice_of_files(before);
    let before = w.heard.files();
    let status = Command::new("sh")
        .args(["-c", "echo x > other.txt"])
        .current_dir(&w.root)
        .status()
        .expect("start the shell");
    assert!(status.success());
    w.wait_for_a_notice_of_files(before);

    wait_until("credits lists the three files", || {
        ["agent.txt", "edited.txt", "other.txt"]
            .iter()
            .all(|path| w.by(path).is_some())
    });
    let credits = w.credits();
    let edited = credits
        .iter()
        .find(|one| one.path == Path::new("edited.txt"))
        .expect("edited.txt is listed");
    assert_eq!(edited.change, Change::Modified);
    assert!(edited.by.is_empty(), "an editor's change is no agent's");
    let other = credits
        .iter()
        .find(|one| one.path == Path::new("other.txt"))
        .expect("other.txt is listed");
    assert_eq!(other.change, Change::Added);
    assert!(other.by.is_empty(), "a shell's change is no agent's");
    assert_eq!(w.by("agent.txt"), Some(strings(&["agent-session"])));
}

#[test]
fn a_write_an_agents_tool_named_is_credited_whether_the_path_is_absolute_or_relative() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    for (file, named) in [
        ("absolute.txt", w.root.join("absolute.txt")),
        ("relative.txt", PathBuf::from("relative.txt")),
        ("dir/inner.txt", PathBuf::from("./dir/inner.txt")),
    ] {
        write(&w.root, file, "written\n");
        w.workspaces.agent_wrote(&w.workspace, "a", &named);
    }
    assert_eq!(w.by("absolute.txt"), Some(strings(&["a"])));
    assert_eq!(w.by("relative.txt"), Some(strings(&["a"])));
    assert_eq!(w.by("dir/inner.txt"), Some(strings(&["a"])));
}

#[test]
fn a_write_named_outside_the_worktree_is_not_noted() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "x\n");
    write(scratch.path(), "elsewhere/x.txt", "not ours\n");
    for outside in [
        scratch.join("elsewhere/x.txt"),
        PathBuf::from("../elsewhere/x.txt"),
        // A sibling whose name starts like the worktree's own folder.
        scratch.join("repo-two/x.txt"),
    ] {
        w.workspaces.agent_wrote(&w.workspace, "a", &outside);
    }
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn two_sessions_that_named_one_path_are_both_credited_in_the_order_they_wrote() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "x\n");
    for session in ["b", "a", "b"] {
        w.workspaces
            .agent_wrote(&w.workspace, session, Path::new("x.txt"));
    }
    assert_eq!(w.by("x.txt"), Some(strings(&["b", "a"])));
}

#[test]
fn a_write_named_in_a_workspace_that_is_gone_or_unknown_changes_nothing() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "x\n");
    let unknown = WorkspaceId::new("0".repeat(32));
    w.workspaces.agent_wrote(&unknown, "a", Path::new("x.txt"));
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn credits_lists_every_uncommitted_change_ordered_by_path_and_nothing_else() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "kept.txt", "changed\n");
    write(&w.root, "b.txt", "new\n");
    write(&w.root, "a/inner.txt", "new\n");
    let listed: Vec<(PathBuf, Change)> = w
        .credits()
        .into_iter()
        .map(|one| (one.path, one.change))
        .collect();
    assert_eq!(
        listed,
        [
            (PathBuf::from("a/inner.txt"), Change::Added),
            (PathBuf::from("b.txt"), Change::Added),
            (PathBuf::from("kept.txt"), Change::Modified),
        ]
    );
}

#[test]
fn a_path_leaves_the_ledger_when_it_is_committed_and_a_later_change_of_it_is_new() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(&w.root, "x.txt", "by the agent\n");
    w.workspaces
        .agent_wrote(&w.workspace, "a", Path::new("x.txt"));
    assert_eq!(w.by("x.txt"), Some(strings(&["a"])));

    commit_all(&w.root, "the agent's change, committed");
    assert_eq!(w.by("x.txt"), None, "committed: no change to list");

    // The same path edited again, by hand: nothing of the first credit is left.
    write(&w.root, "x.txt", "by hand\n");
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn a_change_seen_while_one_session_runs_a_shell_command_is_credited_to_that_session() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    w.workspaces.command_began(&w.workspace, "a", ago(1));
    let before = w.heard.files();
    write(&w.root, "made-by-the-command.txt", "x\n");
    w.wait_for_a_notice_of_files(before);
    wait_until("the change is credited", || {
        w.by("made-by-the-command.txt") == Some(strings(&["a"]))
    });
}

#[test]
fn a_change_seen_after_a_shell_command_ended_is_no_agents() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    // The command ran, and its end was reported, long before the change.
    w.workspaces.command_began(&w.workspace, "a", ago(2));
    w.workspaces.command_ended(&w.workspace, "a", ago(1));
    let before = w.heard.files();
    write(&w.root, "by-hand.txt", "x\n");
    w.wait_for_a_notice_of_files(before);
    wait_until("the change is listed", || w.by("by-hand.txt").is_some());
    assert_eq!(w.by("by-hand.txt"), Some(Vec::new()));
}

#[test]
fn a_change_seen_while_two_sessions_run_shell_commands_is_no_agents() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    w.workspaces.command_began(&w.workspace, "a", ago(2));
    w.workspaces.command_began(&w.workspace, "b", ago(1));
    let before = w.heard.files();
    write(&w.root, "x.txt", "x\n");
    w.wait_for_a_notice_of_files(before);
    wait_until("the change is listed", || w.by("x.txt").is_some());
    assert_eq!(w.by("x.txt"), Some(Vec::new()));
}

#[test]
fn a_file_written_in_a_folder_made_by_a_command_is_credited_to_its_session() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    w.workspaces.command_began(&w.workspace, "a", ago(1));
    // The folder and the file come at once, as a command makes them: the system may not
    // be watching the folder yet when the file is written.
    let before = w.heard.files();
    let status = Command::new("sh")
        .args(["-c", "mkdir -p out/deep && echo x > out/deep/f.txt"])
        .current_dir(&w.root)
        .status()
        .expect("start the shell");
    assert!(status.success());
    w.wait_for_a_notice_of_files(before);
    wait_until("the file is credited", || {
        w.by("out/deep/f.txt") == Some(strings(&["a"]))
    });
}

#[test]
fn a_folder_with_files_moved_into_the_worktree_is_credited_to_the_session_whose_command_runs() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    write(scratch.path(), "outside/moved/inner/f.txt", "x\n");
    write(scratch.path(), "outside/moved-by-hand/f.txt", "x\n");

    // Nobody's command runs: a folder moved in by hand is no agent's.
    let before = w.heard.files();
    std::fs::rename(
        scratch.join("outside/moved-by-hand"),
        w.root.join("moved-by-hand"),
    )
    .expect("move the folder in");
    w.wait_for_a_notice_of_files(before);
    wait_until("the file is listed", || {
        w.by("moved-by-hand/f.txt").is_some()
    });
    assert_eq!(w.by("moved-by-hand/f.txt"), Some(Vec::new()));

    // A command of a session, begun since, moves one in: what it brings is its change.
    w.workspaces
        .command_began(&w.workspace, "a", Instant::now());
    let before = w.heard.files();
    std::fs::rename(scratch.join("outside/moved"), w.root.join("moved")).expect("move in");
    w.wait_for_a_notice_of_files(before);
    wait_until("the file is credited", || {
        w.by("moved/inner/f.txt") == Some(strings(&["a"]))
    });
    assert_eq!(w.by("moved-by-hand/f.txt"), Some(Vec::new()));
}

#[test]
fn a_file_written_while_the_worktree_was_not_watched_is_no_agents_in_a_folder_a_command_made() {
    let scratch = Scratch::new();
    let w = watched(&scratch);
    // A command of a session makes a folder, and ends.
    w.workspaces
        .command_began(&w.workspace, "a", Instant::now());
    let before = w.heard.files();
    std::fs::create_dir(w.root.join("out")).expect("make the folder");
    w.wait_for_a_notice_of_files(before);
    w.workspaces
        .command_ended(&w.workspace, "a", Instant::now());

    // Nothing watches while the user writes a file in it by hand: nothing saw it written,
    // and the folder's credit is not the file's.
    w.workspaces.unwatch(&w.workspace);
    write(&w.root, "out/by-hand.txt", "x\n");
    assert_eq!(w.by("out/by-hand.txt"), Some(Vec::new()));
    w.workspaces
        .watch(&w.workspace, w.heard.told())
        .expect("watch again");
    assert_eq!(w.by("out/by-hand.txt"), Some(Vec::new()));
}

#[test]
fn a_write_noted_while_git_reads_the_files_is_credited_though_the_list_git_gave_lacks_it() {
    let scratch = Scratch::new();
    let w = watched_with(
        &scratch,
        &[
            ("a.txt", "a\n"),
            ("slow.txt", "slow\n"),
            (".gitattributes", "slow.txt filter=hold\n"),
        ],
    );
    // A filter that makes git wait over `slow.txt`, which it reads after `a.txt`, until
    // the test lets it go.
    let started = scratch.join("started");
    let go = scratch.join("go");
    let filter = scratch.join("bin/hold");
    program(
        &filter,
        &format!(
            "touch '{}'\nn=0\nwhile [ ! -e '{}' ] && [ $n -lt 400 ]; do sleep 0.05; n=$((n+1)); done\ncat",
            started.display(),
            go.display()
        ),
    );
    git(
        &w.root,
        &["config", "filter.hold.clean", &filter.to_string_lossy()],
    );
    // Its time of change is not the list's, so git has to read it, through the filter.
    touch(&w.root, "slow.txt");

    std::thread::scope(|scope| {
        let asking = scope.spawn(|| w.workspaces.credits(&w.workspace));
        wait_until("git is reading the files", || started.exists());
        // `a.txt` is written, and named by the agent's tool, after git looked at it.
        write(&w.root, "a.txt", "a, written while git reads\n");
        w.workspaces
            .agent_wrote(&w.workspace, "agent-session", Path::new("a.txt"));
        std::fs::write(&go, "").expect("let git go on");
        asking.join().expect("credits ran").expect("credits");
    });
    assert_eq!(w.by("a.txt"), Some(strings(&["agent-session"])));
}

#[test]
fn a_workspace_that_is_not_watched_credits_only_what_a_tool_named() {
    let scratch = Scratch::new();
    let root = scratch.join("repo");
    init(&root);
    write(&root, "kept.txt", "kept\n");
    commit_all(&root, "first");
    let workspaces = workspaces(&scratch);
    let workspace = workspaces.open_folder(&root).expect("open").workspace;
    assert!(workspaces.watched().is_empty());

    write(&root, "named.txt", "x\n");
    write(&root, "by-hand.txt", "x\n");
    workspaces.agent_wrote(&workspace, "a", Path::new("named.txt"));
    workspaces.command_began(&workspace, "a", ago(1));
    let by: Vec<(PathBuf, Vec<String>)> = workspaces
        .credits(&workspace)
        .expect("credits")
        .into_iter()
        .map(|one| (one.path, one.by))
        .collect();
    assert_eq!(
        by,
        [
            (PathBuf::from("by-hand.txt"), Vec::new()),
            (PathBuf::from("named.txt"), strings(&["a"])),
        ]
    );
}

#[test]
fn the_credits_of_a_workspace_the_store_does_not_hold_are_refused() {
    let scratch = Scratch::new();
    let workspaces = workspaces(&scratch);
    let unknown = WorkspaceId::new("0".repeat(32));
    match workspaces.credits(&unknown) {
        Err(WorkspaceError::Refused(Refusal::UnknownWorkspace(id))) => assert_eq!(id, unknown),
        other => panic!("not refused: {other:?}"),
    }
}

// ---- Two sessions in one worktree, replayed from what Claude Code reported ----

/// The longest gap of the recording that a replay keeps: a longer one is a command that
/// slept, and the replay does not.
const LONGEST_GAP: Duration = Duration::from_secs(1);

/// The sessions of the recordings, by the names the test gives them: the one that
/// reported from the terminal `term-A`, and the one from `term-B`.
const A: &str = "session-a";
const B: &str = "session-b";

/// What the recordings were made with: the first of them has `README.md` read and then
/// edited, and the second's first line is this one.
const README: &str = "# fixture\n\nA scratch repository for delivery experiments.\n";

/// A recording of two sessions, replayed through the real Adapter into a watched
/// worktree, the way the Keeper hands what the Adapter says to Workspaces.
struct Replay<'a> {
    w: &'a Watched,
    adapter: ClaudeCode,
    records: Vec<HookRecord>,
    fed: Option<Instant>,
}

impl<'a> Replay<'a> {
    /// The recording `name`, every line of it moved into the worktree of `w`.
    fn new(w: &'a Watched, name: &str) -> Replay<'a> {
        Replay {
            w,
            adapter: ClaudeCode::new(),
            records: recorded::hook_events(name)
                .iter()
                .map(|record| record.in_project(&w.root))
                .collect(),
            fed: None,
        }
    }

    fn lines(&self) -> usize {
        self.records.len()
    }

    /// Lets pass the gap the recording has before line `line`, up to a bound, counted from
    /// when the line before was fed: what the test did since is part of it. This is the
    /// input's own timing, not a wait for a fact.
    fn pace(&self, line: usize) {
        if let (Some(fed), Some(before)) = (self.fed, line.checked_sub(1)) {
            let gap = Duration::from_nanos(
                self.records[line]
                    .started_ns
                    .saturating_sub(self.records[before].started_ns),
            )
            .min(LONGEST_GAP);
            if let Some(left) = (fed + gap).checked_duration_since(Instant::now()) {
                std::thread::sleep(left);
            }
        }
    }

    /// Feeds line `line` of the recording to the Adapter, and gives what the Adapter said
    /// to Workspaces: a file written is named, a command begins or ends for the session
    /// that reported it. The test paces the lines itself, with [`Replay::pace`].
    fn give(&mut self, line: usize) {
        let record = &self.records[line];
        // The moment the report reached the Keeper, before the Adapter read it.
        let at = Instant::now();
        let reported = self
            .adapter
            .report(&Report {
                terminal: record.terminal(),
                pid: record.parent,
                payload: &record.input_bytes(),
            })
            .unwrap_or_else(|e| panic!("line {line} is read: {e}"));
        self.fed = Some(Instant::now());
        for one in reported {
            let session = match one.terminal.as_deref() {
                Some("term-A") => A,
                Some("term-B") => B,
                other => {
                    panic!("line {line} reports from a terminal the recording lacks: {other:?}")
                }
            };
            match one.event {
                Event::FileWritten { path, .. } => {
                    self.w
                        .workspaces
                        .agent_wrote(&self.w.workspace, session, &path);
                }
                Event::Command {
                    phase: Phase::Began,
                    ..
                } => self
                    .w
                    .workspaces
                    .command_began(&self.w.workspace, session, at),
                Event::Command {
                    phase: Phase::Ended,
                    ..
                } => self
                    .w
                    .workspaces
                    .command_ended(&self.w.workspace, session, at),
                _ => {}
            }
        }
    }

    /// Does in the worktree what the tool of line `line`, a `PreToolUse` of the write tool
    /// or the edit tool, does before its `PostToolUse`: the write tool writes its content
    /// and the edit tool replaces one text by another. A line that is no such tool's start
    /// does nothing.
    fn the_file_tool_acts(&self, line: usize) {
        let record = &self.records[line];
        if record.event != "PreToolUse" {
            return;
        }
        let input = &record.input["tool_input"];
        let text = |key: &str| input[key].as_str().unwrap_or_default();
        match record.input["tool_name"].as_str() {
            Some("Write") => {
                std::fs::write(text("file_path"), text("content")).expect("write the file");
            }
            Some("Edit") => {
                let file = text("file_path");
                let before = std::fs::read_to_string(file).expect("read the file to edit");
                assert!(
                    before.contains(text("old_string")),
                    "line {line} edits a text that {file} does not hold"
                );
                let after = if input["replace_all"].as_bool().unwrap_or(false) {
                    before.replace(text("old_string"), text("new_string"))
                } else {
                    before.replacen(text("old_string"), text("new_string"), 1)
                };
                std::fs::write(file, after).expect("write the edited file");
            }
            _ => {}
        }
    }

    /// Runs, when line `line` is the `PostToolUse` of a shell command, that command in the
    /// worktree: a command writes when it runs, which is after the hook that told of its
    /// beginning has returned and the shell has started, so the test runs it at the end
    /// of the interval the recording has between the two, not at once. Returns whether it
    /// ran one.
    fn the_command_runs(&self, line: usize) -> bool {
        let record = &self.records[line];
        if record.event != "PostToolUse" || record.input["tool_name"] != "Bash" {
            return false;
        }
        let command = record.input["tool_input"]["command"]
            .as_str()
            .unwrap_or_default();
        let status = Command::new("sh")
            .args(["-c", command])
            .current_dir(&self.w.root)
            .status()
            .expect("start the shell");
        assert!(status.success(), "the command of line {line} succeeds");
        true
    }
}

impl Watched {
    /// Waits until `path` is listed as credited to `session`, among others or alone.
    fn wait_credited_to(&self, path: &str, session: &str) {
        wait_until(&format!("{path} is credited to {session}"), || {
            self.by(path)
                .is_some_and(|by| by.iter().any(|one| one == session))
        });
    }
}

/// What `credits` says, as the path of each change and its sessions.
fn credited(w: &Watched) -> Vec<(String, Vec<String>)> {
    w.credits()
        .into_iter()
        .map(|one| (one.path.to_string_lossy().into_owned(), one.by))
        .collect()
}

fn credit(path: &str, by: &[&str]) -> (String, Vec<String>) {
    (path.to_owned(), strings(by))
}

#[test]
fn two_sessions_in_one_worktree_editing_different_files_have_each_write_credited_to_its_session() {
    let scratch = Scratch::new();
    let w = watched_with(
        &scratch,
        &[("README.md", README), ("notes.txt", "notes\nsecond line\n")],
    );
    let mut replay = Replay::new(&w, "claude/two-sessions-files.jsonl");
    assert_eq!(replay.lines(), 49);

    for line in 0..replay.lines() {
        replay.pace(line);
        // A shell command writes through the shell, which no tool named: the write is seen
        // by the watcher and credited by whose command is open. It falls at the end of the
        // interval between the command's two reports, as a real command's does, and the
        // report of its end is not given before the write is credited, so that the command
        // is still the one that is open.
        if replay.the_command_runs(line) {
            match line {
                16 => w.wait_credited_to("a3.txt", A),
                19 => w.wait_credited_to("b3.txt", B),
                31 => {
                    w.wait_credited_to("shared.log", A);
                    assert_eq!(w.by("shared.log"), Some(strings(&[A])));
                }
                42 => w.wait_credited_to("shared.log", B),
                _ => panic!("line {line} runs a command the test does not wait for"),
            }
        }
        replay.give(line);
        replay.the_file_tool_acts(line);
    }

    // Every change is somebody's, and the one file two sessions wrote with a command each,
    // alone, is both's, in the order they wrote.
    assert_eq!(
        credited(&w),
        [
            credit("README.md", &[A]),
            credit("a1.txt", &[A]),
            credit("a3.txt", &[A]),
            credit("a6.txt", &[A]),
            credit("b1.txt", &[B]),
            credit("b2.txt", &[B]),
            credit("b3.txt", &[B]),
            credit("b6.txt", &[B]),
            credit("notes.txt", &[B]),
            credit("shared.log", &[A, B]),
        ]
    );
}

#[test]
fn two_sessions_running_overlapping_shell_commands_leave_the_files_written_as_changes_no_agent_made()
 {
    let scratch = Scratch::new();
    let w = watched_with(&scratch, &[("README.md", README), ("notes.txt", "notes\n")]);
    let mut replay = Replay::new(&w, "claude/two-sessions-shell-overlap.jsonl");
    assert_eq!(replay.lines(), 16);

    for line in 0..replay.lines() {
        replay.pace(line);
        // Line 6 begins B's command, which runs alone and writes nothing yet, and line 7
        // begins A's, while B's runs. Both commands write after their sleep, which falls
        // before either ends: the test writes what they would, in the order of their
        // files, when the interval the recording has before line 8 has passed, both
        // commands having run since line 7.
        if line == 8 {
            let before = w.heard.files();
            write(&w.root, "overlap_b.txt", "B\n");
            w.wait_for_a_notice_of_files(before);
            let before = w.heard.files();
            write(&w.root, "overlap_a.txt", "A\n");
            w.wait_for_a_notice_of_files(before);
            wait_until("both writes are listed", || {
                w.by("overlap_a.txt").is_some() && w.by("overlap_b.txt").is_some()
            });
        }
        replay.give(line);
    }

    assert_eq!(
        credited(&w),
        [credit("overlap_a.txt", &[]), credit("overlap_b.txt", &[])],
        "written while two commands ran: no agent's"
    );

    // The limit: with exactly one command running, a hand edit is credited to its
    // session, because nothing tells the two apart. A's command begins again, alone, and
    // runs for the interval the recording has before its end: far longer than the settle
    // time, which a command that has not ended does not wait out.
    replay.give(7);
    replay.pace(8);
    write(&w.root, "README.md", &format!("{README}by hand\n"));
    w.wait_credited_to("README.md", A);
    replay.give(9);
    assert_eq!(w.by("README.md"), Some(strings(&[A])));
    assert_eq!(w.by("overlap_a.txt"), Some(Vec::new()));
    assert_eq!(w.by("overlap_b.txt"), Some(Vec::new()));
}
