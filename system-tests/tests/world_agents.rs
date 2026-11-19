//! Tests of what the system tests of the agents stand on: the stand-in on a world's search
//! path, the Keeper run under a wrapper, the tap's view of the stop-everything frame, and
//! the client's reading of a terminal as lines and of the stand-in's prompt.
//!
//! The Keeper's side of the agent requests is not built when these tests are written, so
//! the client's calls for it are tried against the real Keeper only as far as the request
//! they send, and as far as the reply they read against a peer of the test's own that
//! speaks the Link; the calls are also used, and so tried, by the packages that wire them.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use yaar_link::{
    ChangedFile, CreditedFile, Fact, FileStatus, Hello, History, Message, Program, Refusal, Reply,
    Request, ReviewId, SessionState, TerminalId, TerminalInfo, TerminalSize, Turn, TurnId,
    WorkspaceAgents, WorkspaceId, handshake, read_message, write_message,
};
use yaar_system_tests::standin::{Did, Script, Step};
use yaar_system_tests::{
    Client, DEADLINE, Place, World, descendants_of, is_alive, program, wait_until,
    without_agent_clis,
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

/// The programs of the app that tests start, by the names Cargo builds them under.
const PROGRAMS: [&str; 4] = [
    "yaar-keeper",
    "yaar-review",
    "yaar-claude-hook",
    "standin-agent",
];

#[test]
fn every_program_a_test_starts_is_built_from_the_tree_and_the_companions_lie_beside_the_keeper() {
    for name in PROGRAMS {
        let path = program(name);
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(name));
        let mode = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{name} is not there: {e}"))
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "{name} runs");
    }
    // The Keeper looks for the hook program beside itself.
    assert_eq!(
        program("yaar-keeper").parent(),
        program("yaar-claude-hook").parent()
    );

    // Nothing is left for a build to do: a program that was built earlier and has gone
    // stale would show up here as one that Cargo builds again.
    let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    build.args([
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "-p",
        "yaar-keeper",
        "-p",
        "yaar-window",
        "-p",
        "yaar-companion-claude",
        "-p",
        "yaar-system-tests",
    ]);
    for name in PROGRAMS {
        build.args(["--bin", name]);
    }
    if std::env::current_exe().is_ok_and(|exe| exe.components().any(|c| c.as_os_str() == "release"))
    {
        build.arg("--release");
    }
    let built = build.output().expect("run cargo");
    assert!(built.status.success(), "building the programs failed");
    let stale: Vec<String> = String::from_utf8_lossy(&built.stdout)
        .lines()
        .filter(|line| line.contains(r#""reason":"compiler-artifact""#))
        .filter(|line| line.contains(r#""kind":["bin"]"#))
        .filter(|line| line.contains(r#""fresh":false"#))
        .map(str::to_owned)
        .collect();
    assert!(stale.is_empty(), "built again: {stale:?}");
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
fn a_login_shell_of_a_world_with_no_stand_in_has_the_machines_search_path_less_its_agent_clis() {
    let world = World::new();
    // Every terminal of the app is a login shell, and the system's profile may add folders
    // to the search path that the process had no part in: macOS's puts `/usr/local/bin`
    // and the rest of `/etc/paths` in, in an order of its own. Where the machine has a
    // `claude` or a `pi` in one of those, this is what keeps a test from starting it.
    let printed = world
        .command(Path::new("/bin/sh"))
        .args([
            "-l",
            "-c",
            "command -v claude; command -v pi; printf 'path-%s\\n' \"$PATH\"",
        ])
        .output()
        .expect("run a login shell");
    let printed = String::from_utf8_lossy(&printed.stdout);
    let machine = std::env::var_os("PATH").expect("the machine has a search path");
    let path = printed
        .strip_prefix("path-")
        .and_then(|rest| rest.strip_suffix('\n'))
        .unwrap_or_else(|| {
            panic!("a login shell found an agent CLI or printed nothing: {printed}")
        });
    // The machine's folders are all still there, whichever the order the system chose.
    let found: Vec<PathBuf> = std::env::split_paths(path).collect();
    for folder in std::env::split_paths(&without_agent_clis(&machine)) {
        assert!(
            found.contains(&folder),
            "{} is lost from {found:?}",
            folder.display()
        );
    }
}

/// The search path `profile` of a world leaves a shell that started with `start` as its
/// path and no more than that: what the profile alone makes of it, whichever system's
/// profile ran before it.
fn path_after_the_profile(world: &World, start: &OsString) -> Vec<PathBuf> {
    let printed = world
        .command(Path::new("/bin/sh"))
        .env("PATH", start)
        .args(["-c", ". \"$HOME/.profile\"; printf %s \"$PATH\""])
        .output()
        .expect("run a shell");
    assert!(printed.status.success());
    std::env::split_paths(&OsString::from(
        String::from_utf8_lossy(&printed.stdout).into_owned(),
    ))
    .collect()
}

#[test]
fn the_profile_of_a_world_keeps_the_folders_a_shell_started_with_and_drops_the_agent_clis() {
    let mut world = World::new();
    let folder = |name: &str, files: &[&str]| {
        let folder = world.home().join(name);
        std::fs::create_dir_all(&folder).expect("make the folder");
        for file in files {
            let path = folder.join(file);
            std::fs::write(&path, "#!/bin/sh\n").expect("write the file");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("set the mode");
        }
        folder
    };
    // The app may put a folder first; a name with a blank and a star must stay one folder.
    let put_first = folder("put first", &[]);
    let odd = folder("odd *", &["git"]);
    // A folder the star would match, were the name globbed, is not in the search path.
    folder("odd x", &[]);
    let with_claude = folder("with-claude", &["claude", "git"]);
    let with_pi = folder("with-pi", &["pi"]);
    let machine = std::env::var_os("PATH").expect("the machine has a search path");
    let machine: Vec<PathBuf> = std::env::split_paths(&without_agent_clis(&machine)).collect();

    // Twice a folder, an empty entry, folders of agent CLIs among the others.
    let start = std::env::join_paths(
        [
            &put_first,
            &with_claude,
            &odd,
            &PathBuf::new(),
            &with_pi,
            &put_first,
        ]
        .into_iter()
        .chain(&machine),
    )
    .expect("join the folders");
    let path = path_after_the_profile(&world, &start);
    assert_eq!(
        path,
        [put_first.clone(), odd.clone()]
            .into_iter()
            .chain(machine.iter().cloned())
            .fold(Vec::new(), |mut kept, next| {
                if !kept.contains(&next) {
                    kept.push(next);
                }
                kept
            }),
        "the folders once each, in the order they came, the agent CLIs' folders lost"
    );

    // The folders the machine's own path holds come back where a system's profile
    // dropped them, after what the shell had.
    let sparse = std::env::join_paths([&odd]).expect("join the folders");
    let path = path_after_the_profile(&world, &sparse);
    assert_eq!(path.first(), Some(&odd));
    for folder in &machine {
        assert!(path.contains(folder), "{} is lost", folder.display());
    }

    // With the stand-ins: their folder first, and the folder the shell started with
    // next. The shell's own options and variables are not left behind.
    world.agents_on_path();
    let agents = world.agents_dir().expect("a folder of stand-ins");
    let path = path_after_the_profile(&world, &start);
    assert_eq!(&path[..2], [agents.clone(), put_first.clone()]);
    assert_eq!(path.iter().filter(|f| **f == agents).count(), 1);
    let printed = world
        .command(Path::new("/bin/sh"))
        .env("PATH", &start)
        .args([
            "-c",
            ". \"$HOME/.profile\"; printf '%s|%s|%s' \"$folder\" \"$new\" \"$-\"",
        ])
        .output()
        .expect("run a shell");
    let printed = String::from_utf8_lossy(&printed.stdout);
    let mut parts = printed.split('|');
    assert_eq!(
        (parts.next(), parts.next()),
        (Some(""), Some("")),
        "{printed}"
    );
    assert!(!parts.next().unwrap_or_default().contains('f'), "{printed}");
}

#[cfg(target_os = "macos")]
#[test]
fn a_login_shell_of_a_world_keeps_a_folder_the_program_that_started_it_put_first() {
    // macOS's system profile only adds to what a shell was started with, so what the
    // app puts first on the path of a terminal's shell is still there after the login.
    let world = World::new();
    let put_first = world.home().join("put-first");
    std::fs::create_dir_all(&put_first).expect("make the folder");
    let machine = std::env::var_os("PATH").expect("the machine has a search path");
    let start = std::env::join_paths(
        std::iter::once(put_first.clone()).chain(std::env::split_paths(&machine)),
    )
    .expect("join the folders");
    let printed = world
        .command(Path::new("/bin/sh"))
        .env("PATH", &start)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
        .expect("run a login shell");
    let found: Vec<PathBuf> = std::env::split_paths(&OsString::from(
        String::from_utf8_lossy(&printed.stdout).into_owned(),
    ))
    .collect();
    assert!(found.contains(&put_first), "{found:?}");
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

#[test]
fn a_keeper_that_ended_under_a_wrapper_that_runs_on_is_not_the_worlds_keeper_any_more() {
    let mut world = World::new();
    // The wrapper stays when the Keeper has gone, as a tracer may with other processes.
    let wrapper = ["/bin/sh", "-c", "\"$0\" & wait; exec sleep 3600"].map(OsString::from);
    let pid = world.start_keeper_under(&wrapper);
    let tracer = parent_of(pid);
    // SAFETY: a plain signal to the Keeper of this world.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    assert!(
        wait_until(DEADLINE, || world.keeper_pid().is_none()),
        "the Keeper {pid} is gone, and the world still has it"
    );
    // The world holds nothing of it, so it may start another.
    let again = world.start_keeper_under(&wrapper);
    assert_ne!(again, pid);
    assert!(!is_alive(tracer), "the first wrapper is still there");

    let tracer = parent_of(again);
    drop(world);
    assert!(
        wait_until(DEADLINE, || !is_alive(again) && !is_alive(tracer)),
        "the Keeper or its wrapper is left"
    );
}

#[test]
fn a_wrapper_that_ends_before_its_keeper_leaves_the_keeper_the_worlds_to_end() {
    let mut world = World::new();
    // The wrapper ends once the test makes a file, leaving the Keeper running.
    let wrapper = [
        "/bin/sh",
        "-c",
        "\"$0\" & while [ ! -e \"$HOME/go\" ]; do :; done",
    ]
    .map(OsString::from);
    let pid = world.start_keeper_under(&wrapper);
    let tracer = parent_of(pid);
    std::fs::write(world.home().join("go"), "").expect("let the wrapper end");
    // A Keeper whose parent has ended is handed on to another process.
    assert!(
        wait_until(DEADLINE, || parent_of(pid) != tracer),
        "the wrapper {tracer} did not end"
    );
    assert_eq!(world.keeper_pid(), Some(pid), "the Keeper still runs");

    drop(world);
    assert!(
        wait_until(DEADLINE, || !is_alive(pid)),
        "the Keeper {pid} is left"
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
    // While it runs: the stand-in is among what the Keeper started, a shell and the program
    // that shell started.
    let started = descendants_of(keeper);
    let running = world.agent_log("default").pid;
    assert!(started.contains(&running), "{running} not in {started:?}");
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

    drop(client);
    drop(world);
    assert!(
        wait_until(DEADLINE, || started.iter().all(|pid| !is_alive(*pid))),
        "still there: {started:?}"
    );
}

/// What a scripted Keeper does about one request: sends `before`, then the reply, then
/// `after`.
struct Answer {
    before: Vec<Message>,
    reply: Result<Reply, Refusal>,
    after: Vec<Message>,
}

impl Answer {
    fn to(reply: Result<Reply, Refusal>) -> Self {
        Answer {
            before: Vec::new(),
            reply,
            after: Vec::new(),
        }
    }
}

/// A peer at `place`'s socket that speaks the Link as a Keeper does, for a test of the
/// client alone, where the Keeper of the app does not answer what the test needs: it
/// answers the requests it gets, in order, as `answers` say, and then waits for the client
/// to go. Gives what it was asked. It is no Keeper of the app and no test of one.
fn scripted_keeper(place: &Place, answers: Vec<Answer>) -> std::thread::JoinHandle<Vec<Request>> {
    let listener = UnixListener::bind(place.socket()).expect("listen");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("a client");
        handshake(&mut stream, &Hello::for_this_process("scripted-keeper")).expect("hello");
        let mut asked = Vec::new();
        for answer in answers {
            let Ok(Message::Request { id, request }) = read_message(&mut stream) else {
                break;
            };
            asked.push(request);
            for message in answer.before {
                write_message(&mut stream, &message).expect("send");
            }
            let reply = Message::Reply {
                to: id,
                reply: answer.reply,
            };
            write_message(&mut stream, &reply).expect("send the reply");
            for message in answer.after {
                write_message(&mut stream, &message).expect("send");
            }
        }
        // Until the client goes, so that what was sent last is read.
        while read_message(&mut stream).is_ok() {}
        asked
    })
}

fn output(terminal: &TerminalId, bytes: &str) -> Message {
    Message::Output {
        terminal: terminal.clone(),
        bytes: bytes.as_bytes().to_vec(),
    }
}

#[test]
fn what_a_terminal_wrote_before_the_attach_was_refused_is_kept() {
    let place = Place::new();
    let terminal = TerminalId::new("t".repeat(32));
    let keeper = scripted_keeper(
        &place,
        vec![Answer {
            before: vec![output(&terminal, "early")],
            ..Answer::to(Err(Refusal::UnknownTerminal {
                terminal: terminal.clone(),
            }))
        }],
    );
    let mut client = Client::attach(&place.socket());

    // No kept output follows a refusal, so what came before it is all there is.
    assert_eq!(
        client.attach_terminal(&terminal, SIZE),
        Err(Refusal::UnknownTerminal {
            terminal: terminal.clone()
        })
    );
    let heard = client.wait_for_output(&terminal, DEADLINE, |bytes| bytes == b"early");
    assert_eq!(heard, b"early");
    drop(client);
    keeper.join().expect("the scripted Keeper");
}

#[test]
fn an_attach_replaces_what_the_client_held_of_a_terminal_by_the_kept_output_and_keeps_other_terminals()
 {
    let place = Place::new();
    let terminal = TerminalId::new("t".repeat(32));
    let other = TerminalId::new("u".repeat(32));
    let keeper = scripted_keeper(
        &place,
        vec![
            // The terminal's first bytes reach the client that opened it.
            Answer {
                before: vec![output(&terminal, "old")],
                ..Answer::to(Ok(Reply::Workspaces(Vec::new())))
            },
            // The Keeper then sends the kept output from its start, `old` among it, and
            // what another terminal wrote meanwhile comes before the reply.
            Answer {
                before: vec![output(&terminal, "early"), output(&other, "other")],
                after: vec![output(&terminal, "old+ring")],
                ..Answer::to(Ok(Reply::Attached { cut: true }))
            },
        ],
    );
    let mut client = Client::attach(&place.socket());
    client.workspaces();
    assert_eq!(client.attach_terminal(&terminal, SIZE), Ok(true));

    let heard = client.wait_for_output(&terminal, DEADLINE, |bytes| bytes == b"old+ring");
    assert_eq!(heard, b"old+ring", "the first bytes are held once");
    let other_heard = client.wait_for_output(&other, DEADLINE, |bytes| bytes == b"other");
    assert_eq!(other_heard, b"other");
    drop(client);
    keeper.join().expect("the scripted Keeper");
}

#[test]
fn the_new_calls_of_the_client_give_what_the_reply_holds_and_the_refusal_when_it_refuses() {
    let place = Place::new();
    let workspace = WorkspaceId::new("w".repeat(32));
    let elsewhere = WorkspaceId::new("x".repeat(32));
    let terminal = TerminalId::new("t".repeat(32));
    let history = History {
        workspace: workspace.clone(),
        start: Some("abc".to_owned()),
        entries: Vec::new(),
    };
    let credited = CreditedFile {
        file: ChangedFile {
            path: "a.txt".into(),
            status: FileStatus::Added,
        },
        by: Vec::new(),
    };
    let compared = vec![ChangedFile {
        path: "b.txt".into(),
        status: FileStatus::Modified,
    }];
    let turn = Turn {
        id: TurnId::new("u".repeat(32)),
        session: yaar_link::SessionId::new("s".repeat(32)),
        number: 3,
        ended: None,
        state: None,
        steps: Vec::new(),
    };
    let info = TerminalInfo {
        id: terminal.clone(),
        workspace: workspace.clone(),
        program: Program::Shell,
    };
    let agents = WorkspaceAgents {
        workspace: workspace.clone(),
        shown: Some(SessionState::Works),
        sessions: Vec::new(),
    };
    let refused = Refusal::UnknownWorkspace {
        workspace: workspace.clone(),
    };
    let keeper = scripted_keeper(
        &place,
        vec![
            Answer::to(Ok(Reply::TerminalOpened {
                terminal: terminal.clone(),
            })),
            Answer::to(Ok(Reply::History(history.clone()))),
            Answer::to(Err(refused.clone())),
            Answer::to(Ok(Reply::Uncommitted(vec![credited.clone()]))),
            Answer::to(Ok(Reply::Compared(compared.clone()))),
            Answer::to(Ok(Reply::Turn(turn.clone()))),
            Answer::to(Ok(Reply::Done)),
            Answer::to(Ok(Reply::Terminals(vec![info.clone()]))),
            Answer::to(Ok(Reply::Agents(vec![agents.clone()]))),
            // Two facts about agents come and one is the one waited for.
            Answer {
                after: vec![
                    Message::Fact(Fact::AgentsChanged(WorkspaceAgents {
                        workspace: elsewhere,
                        shown: Some(SessionState::Works),
                        sessions: Vec::new(),
                    })),
                    Message::Fact(Fact::AgentsChanged(WorkspaceAgents {
                        shown: Some(SessionState::Idle),
                        ..agents.clone()
                    })),
                    Message::Fact(Fact::AgentsChanged(agents.clone())),
                ],
                ..Answer::to(Ok(Reply::Done))
            },
        ],
    );
    let mut client = Client::attach(&place.socket());

    assert_eq!(
        client.open_launcher(&workspace, "claude", SIZE),
        Ok(terminal.clone())
    );
    // A terminal that was opened has no output yet, and a screen of blank lines.
    assert_eq!(client.screen(&terminal, SIZE).len(), usize::from(SIZE.rows));
    assert_eq!(client.history(&workspace), Ok(history));
    assert_eq!(client.history(&workspace), Err(refused));
    assert_eq!(client.uncommitted(&workspace, None), Ok(vec![credited]));
    assert_eq!(client.compare(&workspace, "a", None), Ok(compared));
    assert_eq!(client.turn(&turn.id), Ok(turn));
    assert_eq!(client.show_workspace(Some(&workspace)), Ok(()));
    assert_eq!(client.terminals(), [info]);
    assert_eq!(client.agents(), std::slice::from_ref(&agents));

    // The wait skips the facts of another workspace and those `wanted` does not accept.
    assert_eq!(client.show_workspace(None), Ok(()));
    let waited = client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Works)
    });
    assert_eq!(waited, agents);
    drop(client);
    assert_eq!(keeper.join().expect("the scripted Keeper").len(), 10);
}
