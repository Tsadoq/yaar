//! Tests of what the system tests of the agents stand on: the stand-in on a world's search
//! path, the Keeper run under a wrapper, the tap's view of the stop-everything frame, and
//! the client's reading of a terminal as lines and of the stand-in's prompt.
//!
//! The Keeper's side of the agent requests is not built when these tests are written, so
//! the client's calls for it are tried only as far as the request they send; the calls
//! themselves are used, and so tried, by the packages that wire them.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::Duration;

use yaar_link::{Program, Request, ReviewId, TerminalId, TerminalSize, TurnId, WorkspaceId};
use yaar_system_tests::standin::{Did, Script, Step};
use yaar_system_tests::{
    Client, DEADLINE, World, descendants_of, is_alive, program, wait_until, without_agent_clis,
};

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// The lines of what a terminal's program wrote, as the bytes came.
fn lines_of(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .split("\r\n")
        .map(str::to_owned)
        .collect()
}

/// A world with its stand-ins on the search path, a Keeper, and a shell of the app open
/// in a workspace.
fn world_with_a_shell() -> (World, Client, TerminalId) {
    let mut world = World::new();
    world.agents_on_path();
    world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    (world, client, terminal)
}

/// The process that started `pid`, read where the system keeps it.
fn parent_of(pid: u32) -> u32 {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read the stat");
        // The name may hold spaces and brackets: the fields start after the last bracket.
        let after_name = stat.rsplit_once(')').expect("the name ends in a bracket").1;
        after_name
            .split_whitespace()
            .nth(1)
            .and_then(|ppid| ppid.parse().ok())
            .expect("the parent's id")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let printed = std::process::Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .expect("run ps");
        String::from_utf8_lossy(&printed.stdout)
            .trim()
            .parse()
            .expect("the parent's id")
    }
}

#[test]
fn a_shell_of_the_app_finds_the_worlds_claude_before_any_other() {
    let (world, mut client, terminal) = world_with_a_shell();
    client.type_line(&terminal, "command -v claude; printf 'path-%s\\n' done");
    let heard = client.wait_for_output(&terminal, DEADLINE, |bytes| {
        lines_of(bytes).iter().any(|line| line == "path-done")
    });
    let lines = lines_of(&heard);
    let marker = lines
        .iter()
        .position(|line| line == "path-done")
        .expect("the marker is there");
    let agents = world
        .agents_dir()
        .expect("the world has a folder of agents");
    assert_eq!(
        lines[marker - 1],
        agents.join("claude").display().to_string(),
        "{lines:?}"
    );
}

#[test]
fn the_agents_folder_holds_the_stand_in_as_claude_pi_and_otheragent() {
    let mut world = World::new();
    assert_eq!(world.agents_dir(), None);
    world.agents_on_path();
    let agents = world
        .agents_dir()
        .expect("the world has a folder of agents");
    let standin = program("standin-agent")
        .canonicalize()
        .expect("the stand-in is built");
    for name in ["claude", "pi", "otheragent"] {
        let found = agents.join(name);
        assert_eq!(
            found.canonicalize().expect("the name leads to a program"),
            standin,
            "{name}"
        );
        assert_ne!(
            std::fs::metadata(&found)
                .expect("the stand-in")
                .permissions()
                .mode()
                & 0o111,
            0,
            "{name} runs"
        );
    }
    // A second call leaves the same folder.
    world.agents_on_path();
    assert_eq!(world.agents_dir(), Some(agents));
}

#[test]
fn a_world_with_no_stand_in_finds_no_agent_cli_of_the_machine() {
    let world = World::new();
    // Where the machine has a `claude` or a `pi` of the user's, this is what keeps a
    // test from starting it; where it has none, it holds trivially.
    let found = world
        .command(Path::new("/bin/sh"))
        .args(["-c", "command -v claude; command -v pi"])
        .output()
        .expect("run a shell");
    assert_eq!(String::from_utf8_lossy(&found.stdout), "");
}

#[test]
fn the_search_path_of_a_world_loses_the_folders_that_hold_claude_or_pi_and_nothing_else() {
    let world = World::new();
    let folder = |name: &str, files: &[(&str, u32)]| {
        let folder = world.home().join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        for (file, mode) in files {
            let path = folder.join(file);
            std::fs::write(&path, "#!/bin/sh\n").expect("write the file");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(*mode))
                .expect("set the mode");
        }
        folder
    };
    let with_claude = folder("with-claude", &[("claude", 0o755), ("git", 0o755)]);
    let with_pi = folder("with-pi", &[("pi", 0o755)]);
    let with_both = folder("with-both", &[("claude", 0o755), ("pi", 0o755)]);
    let with_others = folder("with-others", &[("claudette", 0o755), ("git", 0o755)]);
    // A file of that name that nothing can run is no agent CLI.
    let not_runnable = folder("not-runnable", &[("claude", 0o644)]);
    let empty = folder("empty", &[]);

    let search = std::env::join_paths([
        &with_claude,
        &with_others,
        &with_pi,
        &not_runnable,
        &with_both,
        &empty,
    ])
    .expect("join the folders");
    let kept: Vec<PathBuf> = std::env::split_paths(&without_agent_clis(&search)).collect();
    assert_eq!(kept, [with_others, not_runnable, empty]);
}

#[test]
fn a_program_of_a_world_with_stand_ins_gets_the_agents_folder_first_and_the_names_of_the_default_script_and_log()
 {
    let mut world = World::new();
    let printed = |world: &World| {
        let printed = world
            .command(Path::new("/usr/bin/env"))
            .output()
            .expect("run env");
        String::from_utf8_lossy(&printed.stdout).into_owned()
    };
    let value = |printed: &str, name: &str| {
        printed
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
    };

    let before = printed(&world);
    assert_eq!(value(&before, "STANDIN_SCRIPT"), None);
    assert_eq!(value(&before, "STANDIN_LOG"), None);

    world.agents_on_path();
    let after = printed(&world);
    let root = world.home().with_file_name("");
    let place = |file: &str| root.join(file).display().to_string();
    assert_eq!(
        value(&after, "STANDIN_SCRIPT"),
        Some(place("default.script"))
    );
    assert_eq!(value(&after, "STANDIN_LOG"), Some(place("default.log")));
    let search = value(&after, "PATH").expect("a search path");
    let first = std::env::split_paths(&search).next().expect("a folder");
    assert_eq!(Some(first), world.agents_dir());
    // What follows is the machine's own search path, less the folders of agent CLIs.
    let machine = std::env::var_os("PATH").expect("the machine has a search path");
    let rest: Vec<_> = std::env::split_paths(&search).skip(1).collect();
    assert_eq!(
        rest,
        std::env::split_paths(&without_agent_clis(&machine)).collect::<Vec<_>>()
    );
}

#[test]
fn a_keeper_run_by_a_wrapper_is_told_apart_from_it_and_both_are_gone_after_a_kill() {
    let mut world = World::new();
    // The shell runs the Keeper, the last argument, as its child, and waits for it.
    let wrapper = ["/bin/sh", "-c", "\"$0\" & wait"].map(OsString::from);
    let pid = world.start_keeper_under(&wrapper);
    let tracer = parent_of(pid);
    assert_ne!(tracer, pid);
    assert!(is_alive(pid) && is_alive(tracer));
    assert_eq!(world.keeper_pid(), Some(pid), "the Keeper's own id");
    assert_eq!(world.client().keeper().pid, pid, "the process that answers");

    world.kill_keeper();
    assert!(!is_alive(pid), "the Keeper is still there");
    assert!(!is_alive(tracer), "the wrapper is still there");
    assert_eq!(world.keeper_pid(), None);

    // The world starts another on the same folders.
    let again = world.start_keeper();
    assert_ne!(again, pid);
    assert_eq!(world.keeper_pid(), Some(again));
}

#[test]
fn a_world_that_ends_while_its_keeper_runs_under_a_wrapper_ends_both() {
    let mut world = World::new();
    let wrapper = ["/bin/sh", "-c", "\"$0\" & wait"].map(OsString::from);
    let pid = world.start_keeper_under(&wrapper);
    let tracer = parent_of(pid);
    drop(world);
    assert!(
        wait_until(DEADLINE, || !is_alive(pid) && !is_alive(tracer)),
        "the Keeper is {}, the wrapper is {}",
        if is_alive(pid) { "there" } else { "gone" },
        if is_alive(tracer) { "there" } else { "gone" },
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_keeper_under_strace_is_found_by_its_own_id_and_ends_with_its_tracer() {
    let mut world = World::new();
    let trace = world.home().join("trace");
    let wrapper: Vec<OsString> = vec![
        "strace".into(),
        "-f".into(),
        "-qq".into(),
        "-o".into(),
        trace.clone().into_os_string(),
    ];
    let pid = world.start_keeper_under(&wrapper);
    let comm = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let tracer = parent_of(pid);
    assert_eq!(comm(pid), "yaar-keeper");
    assert_eq!(comm(tracer), "strace");

    world.kill_keeper();
    assert!(!is_alive(pid), "the Keeper is still there");
    assert!(!is_alive(tracer), "the tracer is still there");
    assert!(trace.is_file(), "the tracer wrote its file");
}

#[test]
fn the_tap_sees_the_stop_everything_frame_and_passes_it_on() {
    let mut world = World::new();
    let pid = world.start_keeper();
    let tap = world.tap();
    let mut client = Client::attach(&tap.socket());
    // Everything sent before this reply has passed the tap, so a stop that was sent would
    // be there.
    client.workspaces();
    assert!(!tap.wait_for_stop(Duration::from_millis(100)));

    client.stop_everything();
    assert!(tap.wait_for_stop(DEADLINE));
    // The frame went on to the Keeper, which stops, and it is no request.
    assert!(
        wait_until(DEADLINE, || world.keeper_pid().is_none()),
        "the Keeper {pid} still runs"
    );
    assert_eq!(tap.requests(), [Request::ListWorkspaces]);
}

#[test]
fn a_vanished_client_leaves_the_keeper_serving_the_next_one() {
    let mut world = World::new();
    world.start_keeper();
    let mut client = world.client();
    // A request on its way, and a reply that is never read.
    client.send(Request::ListWorkspaces);
    client.vanish();
    assert!(world.client().workspaces().is_empty());
}

#[test]
fn the_new_calls_of_the_client_send_the_requests_the_link_names() {
    let mut world = World::new();
    world.start_keeper();
    let tap = world.tap();
    let mut client = Client::attach(&tap.socket());
    let workspace = WorkspaceId::new("w".repeat(32));
    let review = ReviewId::new("r".repeat(32));
    let terminal = TerminalId::new("t".repeat(32));
    let turn = TurnId::new("u".repeat(32));

    // What the Keeper answers is the business of the packages that wire it: here only
    // what was asked. A call that refuses to go on with a refusal panics, and is let to.
    let _ = client.open_launcher(&workspace, "claude", SIZE);
    let _ = client.history(&workspace);
    let _ = client.uncommitted(&workspace, Some(&review));
    let _ = client.uncommitted(&workspace, None);
    let _ = client.compare(&workspace, "from", Some("to"));
    let _ = client.compare(&workspace, "from", None);
    let _ = client.turn(&turn);
    let _ = client.show_workspace(Some(&workspace));
    let _ = client.show_workspace(None);
    let _ = client.attach_terminal(&terminal, SIZE);
    let _ = catch_unwind(AssertUnwindSafe(|| client.agents()));
    let _ = catch_unwind(AssertUnwindSafe(|| client.terminals()));
    // The reply to the last request has come, so every request before it has passed the
    // tap.
    client.workspaces();

    assert_eq!(
        tap.requests(),
        [
            Request::OpenTerminal {
                workspace: workspace.clone(),
                program: Program::Agent {
                    agent: "claude".to_owned()
                },
                size: SIZE,
            },
            Request::GetHistory {
                workspace: workspace.clone()
            },
            Request::GetUncommitted {
                workspace: workspace.clone(),
                review: Some(review),
            },
            Request::GetUncommitted {
                workspace: workspace.clone(),
                review: None,
            },
            Request::CompareVersions {
                workspace: workspace.clone(),
                from: "from".to_owned(),
                to: Some("to".to_owned()),
            },
            Request::CompareVersions {
                workspace: workspace.clone(),
                from: "from".to_owned(),
                to: None,
            },
            Request::GetTurn { turn },
            Request::ShowWorkspace {
                workspace: Some(workspace)
            },
            Request::ShowWorkspace { workspace: None },
            Request::AttachTerminal {
                terminal,
                size: SIZE
            },
            Request::ListAgents,
            Request::ListTerminals,
            Request::ListWorkspaces,
        ]
    );
}

/// The screen a shell of the app draws after `command`, once it printed `marker`.
fn screen_after(command: &str, marker: &str, size: TerminalSize) -> Vec<String> {
    let mut world = World::new();
    world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, size)
        .expect("open a terminal");
    // The marker is built so that the line typed does not hold it.
    client.type_line(
        &terminal,
        &format!("{command}; printf 'screen-%s\\n' {marker}"),
    );
    let done = format!("screen-{marker}");
    client.wait_for_output(&terminal, DEADLINE, |bytes| lines_of(bytes).contains(&done));
    client.screen(&terminal, size)
}

fn holds_in_order(screen: &[String], wanted: &[&str]) -> bool {
    screen
        .windows(wanted.len())
        .any(|window| window.iter().map(String::as_str).eq(wanted.iter().copied()))
}

#[test]
fn the_screen_is_the_lines_the_emulator_draws_with_their_trailing_blanks_dropped() {
    let size = TerminalSize {
        columns: 40,
        rows: 12,
    };
    // A carriage return overwrites, and a wide character takes two cells of which the
    // second is no character.
    let screen = screen_after(
        "printf 'one\\ntwo   \\nab\\rXY\\n\\346\\227\\245x\\n'",
        "done",
        size,
    );
    assert_eq!(screen.len(), usize::from(size.rows), "{screen:?}");
    assert!(
        holds_in_order(&screen, &["one", "two", "XY", "日x", "screen-done"]),
        "{screen:?}"
    );
    assert!(
        screen.iter().all(|line| line == line.trim_end()),
        "{screen:?}"
    );
}

#[test]
fn a_cleared_screen_shows_nothing_of_what_was_there_before() {
    let screen = screen_after(
        "printf '\\033[2J\\033[Hhello\\n'",
        "cleared",
        TerminalSize {
            columns: 40,
            rows: 12,
        },
    );
    assert_eq!(&screen[..2], ["hello", "screen-cleared"], "{screen:?}");
    assert!(
        screen.iter().all(|line| !line.contains("printf")),
        "the typed line is gone: {screen:?}"
    );
}

#[test]
fn lines_that_scrolled_off_the_screen_are_not_on_it() {
    let screen = screen_after(
        "i=1; while [ $i -le 30 ]; do echo n$i; i=$((i + 1)); done",
        "scrolled",
        TerminalSize {
            columns: 40,
            rows: 10,
        },
    );
    assert_eq!(screen.len(), 10, "{screen:?}");
    assert!(screen.contains(&"n30".to_owned()), "{screen:?}");
    assert!(!screen.contains(&"n1".to_owned()), "{screen:?}");
}

#[test]
fn the_screen_of_a_terminal_that_wrote_nothing_is_blank() {
    let mut world = World::new();
    world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    let size = TerminalSize {
        columns: 20,
        rows: 4,
    };
    // Nothing was read from the Keeper yet: the client holds no output of the terminal.
    assert_eq!(client.screen(&terminal, size), ["", "", "", ""]);
}

#[test]
fn the_wait_for_a_prompt_looks_only_after_the_byte_it_is_given_and_says_where_the_prompt_ended() {
    let (_world, mut client, terminal) = world_with_a_shell();
    // The prompt is printed by a command whose typed line does not hold it.
    let prompt = "printf 'standin%s ' '>'";

    client.type_line(&terminal, prompt);
    let first = client.wait_for_prompt(&terminal, 0);
    let run = client.wait_for_output(&terminal, DEADLINE, |bytes| bytes.len() >= first);
    assert!(
        String::from_utf8_lossy(&run[..first]).ends_with("standin> "),
        "{:?}",
        String::from_utf8_lossy(&run[..first])
    );

    // The first prompt is behind `first`: a second wait for one starts after it, so it
    // is not answered by that one, and what came between is what was printed meanwhile.
    client.type_line(&terminal, &format!("printf 'mid-%s\\n' marker; {prompt}"));
    let second = client.wait_for_prompt(&terminal, first);
    assert!(second > first);
    let run = client.wait_for_output(&terminal, DEADLINE, |bytes| bytes.len() >= second);
    let between = String::from_utf8_lossy(&run[first..second]).into_owned();
    assert!(between.contains("mid-marker"), "{between:?}");
    assert!(between.ends_with("standin> "), "{between:?}");
}

#[test]
fn a_typed_line_reaches_the_program_with_the_enter_key_after_it() {
    let mut world = World::new();
    world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    client.type_into(&terminal, b"stty raw -echo; cat > keys.txt\r");
    assert!(
        wait_until(DEADLINE, || repo.path().join("keys.txt").exists()),
        "the program did not start"
    );
    client.type_line(&terminal, "a line");
    client.type_line(&terminal, "");
    assert!(
        wait_until(DEADLINE, || {
            std::fs::read(repo.path().join("keys.txt")).is_ok_and(|bytes| bytes == b"a line\r\r")
        }),
        "{:?}",
        std::fs::read(repo.path().join("keys.txt"))
    );
}

#[test]
fn a_world_with_a_stand_in_has_a_repository_a_keeper_and_a_script_the_stand_in_follows() {
    // Waits for the stand-in itself and for the text of a script, which another package
    // builds: this is the first test that runs the program.
    let script = Script(vec![Step::Turn, Step::Exit]);
    let (mut world, repo) = World::with_standin("default", &script);
    let keeper = world.keeper_pid().expect("the Keeper runs");
    assert_eq!(repo.git(&["config", "--local", "user.name"]), "A Test");
    assert_eq!(
        repo.git(&["config", "--local", "user.email"]),
        "test@example.invalid"
    );
    assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), "1");
    let file = world.home().with_file_name("default.script");
    assert_eq!(
        std::fs::read_to_string(&file).expect("the script is written"),
        script.text()
    );
    assert_eq!(Script::parse(&script.text()), script);

    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    client.type_line(&terminal, "claude");
    let prompt = client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "hello");
    // The stand-in ran with the default script and log, which the shell got from the
    // world.
    assert!(
        wait_until(DEADLINE, || world
            .agent_log("default")
            .did
            .iter()
            .any(|did| matches!(did, Did::Exited { status: 0 }))),
        "{:?}",
        world.agent_log("default")
    );
    let log = world.agent_log("default");
    assert_ne!(log.pid, 0);
    assert!(log.args.is_empty(), "{:?}", log.args);
    assert!(prompt > 0);
    assert!(
        String::from_utf8_lossy(&log.received).contains("hello"),
        "what reached it: {:?}",
        log.received
    );

    let started = descendants_of(keeper);
    drop(client);
    drop(world);
    assert!(
        wait_until(DEADLINE, || started.iter().all(|pid| !is_alive(*pid))),
        "still there: {started:?}"
    );
}

#[test]
fn every_program_of_the_app_is_built_for_a_test_that_asks_for_it() {
    // The hook program of Claude Code among them: a test of the Keeper runs it as Claude
    // Code would, and finds it beside the Keeper.
    for name in [
        "yaar-keeper",
        "yaar-review",
        "yaar-claude-hook",
        "standin-agent",
    ] {
        assert!(program(name).is_file(), "{name} is not built");
    }
}
