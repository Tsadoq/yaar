//! System tests of what the Keeper holds for a Window that comes and goes. A killed client
//! drops its connection with no goodbye and reads nothing more.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use yaar_link::{TerminalId, TerminalSize};
use yaar_system_tests::{Client, DEADLINE, World, descendants_of, is_alive, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

const PARTS: u32 = 50;

const LINES: u32 = 4_000;

/// The output the Keeper keeps for a client that attaches later: a copy of the Dock's
/// `RING_BYTES`, as the system tests do not depend on the Dock's crate.
const RING: usize = 1 << 20;

const SLACK: usize = 4096;

const PART_BYTES: usize = LINES as usize * 19;

const FLOOD_DEADLINE: Duration = Duration::from_secs(180);

/// Names `/bin/sh`: else the interactive shell runs it in a fork that writes a CR of its own on
/// every attach's window change. Writes CRLF itself: on macOS the LF-to-CRLF change doubles the CR
/// when the reader falls behind.
fn flood() -> String {
    format!(
        "\
#!/bin/sh
stty -onlcr
gates=$1
i=1
while [ $i -le {PARTS} ]; do
  while [ ! -e \"$gates/gate-$i\" ]; do sleep 0.01; done
  j=1
  while [ $j -le {LINES} ]; do
    printf 'part %s line %s\\r\\n' \"$i\" \"$j\"
    j=$((j+1))
  done
  i=$((i+1))
done
printf 'flood-%s\\r\\n' done
read line
"
    )
}

/// Kills what a failed test leaves running: the Keeper's descendants and the `known` processes,
/// which a child that ignores the hang-up and the stop would keep alive for good.
/// Dropped before the world, so the Keeper is still there to ask.
struct Leftovers {
    keeper: Option<u32>,
    known: Vec<u32>,
}

impl Leftovers {
    fn of(keeper: u32) -> Leftovers {
        Leftovers {
            keeper: Some(keeper),
            known: Vec::new(),
        }
    }

    /// Called once every process is seen gone: from then on a process id may belong to an
    /// unrelated process, which the drop must not kill.
    fn disarm(&mut self) {
        self.keeper = None;
        self.known.clear();
    }
}

impl Drop for Leftovers {
    fn drop(&mut self) {
        let mut all = self.keeper.map(descendants_of).unwrap_or_default();
        all.append(&mut self.known);
        for pid in all.into_iter().filter(|pid| is_alive(*pid)) {
            // SAFETY: a signal to a process the test started or was told of.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

fn script(world: &World, name: &str, text: &str) -> std::path::PathBuf {
    let path = world.home().join(name);
    std::fs::write(&path, text).expect("write the script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script runnable");
    path
}

/// Looks at the end only: else a megabytes-long run is read again at every frame.
fn newest_holds(bytes: &[u8], text: &str) -> bool {
    let tail = &bytes[bytes.len().saturating_sub(64 * 1024)..];
    String::from_utf8_lossy(tail).contains(text)
}

fn flood_lines(output: &[u8]) -> Vec<(u32, u32)> {
    let text = String::from_utf8_lossy(output);
    let mut pieces: Vec<&str> = text.split('\n').collect();
    pieces.pop();
    let mut lines = Vec::new();
    for piece in pieces.into_iter().skip(1) {
        let piece = piece.trim_end_matches('\r');
        let Some(rest) = piece.strip_prefix("part ") else {
            assert!(
                lines.is_empty() || lines.last() == Some(&(PARTS, LINES)),
                "{piece:?} came in the middle of the flood, after {:?}",
                lines.last()
            );
            continue;
        };
        let (part, line) = rest
            .split_once(" line ")
            .unwrap_or_else(|| panic!("not a line of the flood: {piece:?}"));
        lines.push((
            part.parse().expect("the number of a part"),
            line.parse().expect("the number of a line"),
        ));
    }
    lines
}

fn after(part: u32, line: u32) -> (u32, u32) {
    if line == LINES {
        (part + 1, 1)
    } else {
        (part, line + 1)
    }
}

fn start_flood(
    world: &World,
    client: &mut Client,
    workspace: &yaar_link::WorkspaceId,
    program: &Path,
    name: &str,
) -> (TerminalId, std::path::PathBuf) {
    let gates = world.home().join(name);
    std::fs::create_dir(&gates).expect("make the folder of gates");
    let terminal = client
        .open_terminal(workspace, SIZE)
        .expect("open a terminal");
    client.wait_for_output(&terminal, DEADLINE, |bytes| !bytes.is_empty());
    client.type_line(
        &terminal,
        &format!("{} {}", program.display(), gates.display()),
    );
    (terminal, gates)
}

#[test]
fn the_screen_after_fifty_clients_killed_during_a_flood_is_that_of_a_run_nobody_interrupted() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let mut leftovers = Leftovers::of(keeper);
    let repo = world.repository("app");
    let program = script(&world, "flood.sh", &flood());

    let mut control = world.client();
    let (_, workspace) = control.open_folder(repo.path()).expect("open the folder");
    let (shown, shown_gates) = start_flood(&world, &mut control, &workspace, &program, "gates-a");
    let (uninterrupted, uninterrupted_gates) =
        start_flood(&world, &mut control, &workspace, &program, "gates-b");

    // Reads on a thread of its own, as a Window does: a client that did not read for five seconds
    // is let go.
    let control = {
        let uninterrupted = uninterrupted.clone();
        std::thread::spawn(move || {
            let run = control.wait_for_output(&uninterrupted, FLOOD_DEADLINE, |bytes| {
                newest_holds(bytes, "flood-done")
            });
            let screen = control.screen(&uninterrupted, SIZE);
            (screen, run)
        })
    };

    for part in 1..=PARTS {
        let mut killed = world.client();
        killed
            .attach_terminal(&shown, SIZE)
            .expect("attach to the terminal");
        for gates in [&shown_gates, &uninterrupted_gates] {
            std::fs::write(gates.join(format!("gate-{part}")), "").expect("open the gate");
        }
        let wanted = format!("part {part} line ");
        let heard = killed.wait_for_output(&shown, DEADLINE, |bytes| newest_holds(bytes, &wanted));

        // The client goes before anything is checked: the checks take longer than a part takes to
        // write.
        killed.vanish();

        assert!(
            !newest_holds(&heard, &format!("part {part} line {LINES}")),
            "part {part} was written whole when client {part} heard its first lines"
        );

        let lines = flood_lines(&heard);
        assert!(
            !lines.is_empty(),
            "client {part} heard no line of the flood"
        );
        for pair in lines.windows(2) {
            assert_eq!(
                pair[1],
                after(pair[0].0, pair[0].1),
                "client {part} heard a break in the flood"
            );
        }
    }

    let mut last = world.client();
    let cut = last
        .attach_terminal(&shown, SIZE)
        .expect("attach to the terminal");
    let heard = last.wait_for_output(&shown, DEADLINE, |bytes| newest_holds(bytes, "flood-done"));
    let (uninterrupted_screen, uninterrupted_run) = control.join().expect("the control client");

    assert_eq!(
        last.screen(&shown, SIZE),
        uninterrupted_screen,
        "the screen of the terminal that clients were killed in differs from the other's"
    );
    assert!(
        uninterrupted_screen
            .iter()
            .any(|line| line.contains("flood-done")),
        "the screen holds no end of the flood: {uninterrupted_screen:?}"
    );
    let (heard, run) = (up_to_the_end(&heard), up_to_the_end(&uninterrupted_run));
    assert!(cut, "the Keeper did not say its kept output was cut");
    // Room for one part above the ring: the last client may hear the rest of the part it attached
    // in.
    assert!(
        (RING - SLACK..=RING + PART_BYTES).contains(&heard.len()),
        "the last client heard {} bytes, and the ring holds {RING}, with a part of at most \
         {PART_BYTES} live after it",
        heard.len()
    );
    assert!(
        run.ends_with(heard),
        "what the last client heard is not the end of what the other heard"
    );

    let started = descendants_of(keeper);
    last.stop_everything();
    assert!(
        wait_until(DEADLINE, || started.iter().all(|pid| !is_alive(*pid))),
        "still there: {:?}",
        started
            .iter()
            .filter(|pid| is_alive(**pid))
            .collect::<Vec<_>>()
    );
    leftovers.disarm();
}

fn up_to_the_end(bytes: &[u8]) -> &[u8] {
    let end = b"flood-done\r\n";
    let at = bytes
        .windows(end.len())
        .rposition(|window| window == end)
        .expect("the flood's last line");
    &bytes[..at + end.len()]
}

fn printed_id(output: &[u8], label: &str) -> Option<u32> {
    String::from_utf8_lossy(output).lines().find_map(|line| {
        let (_, digits) = line.trim().rsplit_once(label)?;
        digits.trim().parse().ok()
    })
}

#[test]
fn the_quit_that_stops_everything_leaves_no_process_though_one_ignores_the_hang_up() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let mut leftovers = Leftovers::of(keeper);
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    client.wait_for_output(&terminal, DEADLINE, |bytes| !bytes.is_empty());

    // The child runs in a subshell, so the shell itself does not ignore what it ignores. `$!` ends
    // the line: `!` followed by anything else is history expansion in some shells.
    client.type_line(
        &terminal,
        "(trap '' HUP TERM; while :; do sleep 1; done) & echo child $!",
    );
    client.type_line(&terminal, "echo shell $$");
    let heard = client.wait_for_output(&terminal, DEADLINE, |bytes| {
        printed_id(bytes, "child").is_some() && printed_id(bytes, "shell").is_some()
    });
    let child = printed_id(&heard, "child").expect("the child's id");
    let shell = printed_id(&heard, "shell").expect("the shell's id");
    leftovers.known = vec![shell, child];
    assert!(is_alive(child), "the child that ignores the hang-up runs");

    let everyone = descendants_of(keeper);
    assert!(
        everyone.contains(&shell) && everyone.contains(&child),
        "the Keeper's processes {everyone:?} hold neither the shell {shell} nor the child {child}"
    );

    let stopped = Instant::now();
    client.stop_everything();

    assert!(
        wait_until(DEADLINE, || !is_alive(shell)),
        "the shell did not end at the hang-up"
    );
    assert!(
        is_alive(child),
        "the child that ignores the hang-up was gone when the shell was, after {:?}",
        stopped.elapsed()
    );

    let limit = Duration::from_secs(3);
    let left = limit.saturating_sub(stopped.elapsed());
    assert!(
        wait_until(left, || everyone.iter().all(|pid| !is_alive(*pid))),
        "after {:?}, still there: {:?}",
        stopped.elapsed(),
        everyone
            .iter()
            .filter(|pid| is_alive(**pid))
            .collect::<Vec<_>>()
    );
    assert!(
        wait_until(limit.saturating_sub(stopped.elapsed()), || world
            .keeper_pid()
            .is_none()),
        "the Keeper has not exited {:?} after the stop",
        stopped.elapsed()
    );
    leftovers.disarm();
}
