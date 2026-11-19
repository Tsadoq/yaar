//! System tests of what the Keeper holds for a Window that comes and goes: the screen of
//! a terminal through many clients killed in the middle of a flood, and the quit that
//! stops everything.
//!
//! The client is the test's own. "Killed" is what the Keeper sees of a killed Window: the
//! connection is dropped with no goodbye and nothing more is read from it.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use yaar_link::{TerminalId, TerminalSize};
use yaar_system_tests::{Client, DEADLINE, World, descendants_of, is_alive, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// How many parts of the flood, and so how many clients are killed.
const PARTS: u32 = 50;

/// How many lines one part writes.
const LINES: u32 = 4_000;

/// What the Keeper keeps of a terminal's output for a client that attaches later: the
/// ring of the Dock, `RING_BYTES` in its crate, which the system tests do not depend on.
const RING: usize = 1 << 20;

/// How far under the ring what the last client heard may fall: the bytes after the end of
/// the flood that are cut away from it.
const SLACK: usize = 4096;

/// How long the control client waits for the whole flood to be over: 200,000 lines through
/// a shell's loop and a terminal, twice at once, on a machine that builds other things. A
/// bound on that one wait, which spans the fifty rounds of the test, and not a measure:
/// the flood is over as soon as the shell has written it.
const FLOOD_DEADLINE: Duration = Duration::from_secs(180);

/// The program of the flood, a script for a POSIX shell: for each part, wait for its gate,
/// then write its lines; after the last part, say so and wait where it is, so that the
/// shell does not print its prompt, which comes at its own time, onto the screen that is
/// compared. A folder of gates is its one argument.
///
/// It names `/bin/sh`. Without that line the terminal's interactive shell runs the script
/// in a fork of itself, and that fork writes a carriage return of its own when it is told
/// the window changed, which every attach does: a byte in the run of one terminal that the
/// other's does not hold. With it the script is a new shell that writes nothing of its own.
///
/// It turns the terminal's change of a line feed into a carriage return and a line feed
/// off and writes both itself. A terminal that makes the change has been seen to write the
/// carriage return twice for a line, on macOS, when whoever reads the terminal falls
/// behind: a byte in the run of one terminal that the other's does not hold, which no
/// Keeper put there. Written as they are, the bytes of the two runs are the same whatever
/// the pace of the readers.
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

/// Ends what a test leaves running when it fails: whatever descends from the Keeper, and
/// the processes the test was told of, which are not its descendants once the Keeper is
/// gone. A child that ignores the hang-up and the stop would otherwise run on for good.
/// Dropped before the world, so that the Keeper is still there to ask.
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

    /// Called once the test has seen every process gone: a process id may be given to an
    /// unrelated process from then on, and the drop must not look for one.
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

/// A script of the world's own folder, runnable.
fn script(world: &World, name: &str, text: &str) -> std::path::PathBuf {
    let path = world.home().join(name);
    std::fs::write(&path, text).expect("write the script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script runnable");
    path
}

/// Whether the newest output of a terminal holds `text`. Looks at the end of what came
/// only: a wait for something that is written last would otherwise read all of a
/// megabytes-long run again at every frame.
fn newest_holds(bytes: &[u8], text: &str) -> bool {
    let tail = &bytes[bytes.len().saturating_sub(64 * 1024)..];
    String::from_utf8_lossy(tail).contains(text)
}

/// The (part, line) of each line of `output` that is one of the flood's, in order. The first
/// and the last piece of the run may be cut by where it started and stopped, and are left
/// out; what the shell printed before the flood is not a line of it.
fn flood_lines(output: &[u8]) -> Vec<(u32, u32)> {
    let text = String::from_utf8_lossy(output);
    let mut pieces: Vec<&str> = text.split('\n').collect();
    pieces.pop();
    let mut lines = Vec::new();
    for piece in pieces.into_iter().skip(1) {
        let piece = piece.trim_end_matches('\r');
        let Some(rest) = piece.strip_prefix("part ") else {
            // Before the flood and after it the shell says its own things; in the middle of
            // it, nothing but the flood is written.
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

/// The line that must follow `(part, line)` in an unbroken run of the flood.
fn after(part: u32, line: u32) -> (u32, u32) {
    if line == LINES {
        (part + 1, 1)
    } else {
        (part, line + 1)
    }
}

/// Opens a terminal of `workspace` for `client` and starts the flood in it, with its own
/// folder of gates, which is made and returned.
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

    // The control client stays attached the whole time. It reads on a thread of its own,
    // as a Window does: one that did not read for five seconds would be let go.
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

        // No goodbye, nothing more read. The client goes at once, before anything is
        // checked: what it heard is a copy, and the checks below take longer than a part
        // takes to be written, so that after them the part could be over.
        killed.vanish();

        // The part has only begun: the client had heard a frame that holds its first
        // lines, which is less than a part, so that the kill fell in the middle of the
        // flood and of a part, with output on its way.
        assert!(
            !newest_holds(&heard, &format!("part {part} line {LINES}")),
            "part {part} was written whole when client {part} heard its first lines"
        );

        // What reached it, the kept output and what came live after it, is one unbroken
        // run: nothing lost and nothing twice where the two meet.
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

    // A last client attaches when the flood is over.
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
    // The bytes the last client heard are the newest of the run that nobody interrupted,
    // to the byte, up to the end of the flood: what the Keeper kept lost and doubled
    // nothing where its memory ends and the live output begins.
    let (heard, run) = (up_to_the_end(&heard), up_to_the_end(&uninterrupted_run));
    // The flood is several times what the Keeper keeps, so its kept output was cut, and
    // is as long as the ring the Dock keeps for each terminal: `RING_BYTES`, 1 MiB. A
    // Keeper that kept less would not give the last client a whole ring.
    assert!(cut, "the Keeper did not say its kept output was cut");
    assert!(
        (RING - SLACK..=RING).contains(&heard.len()),
        "the last client heard {} bytes, and the ring holds {RING}",
        heard.len()
    );
    assert!(
        run.ends_with(heard),
        "what the last client heard is not the end of what the other heard"
    );

    // The scripts wait for a line the test never types: the programs are the test's to end.
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

/// What `bytes` holds up to the end of the line that says the flood is done.
fn up_to_the_end(bytes: &[u8]) -> &[u8] {
    let end = b"flood-done\r\n";
    let at = bytes
        .windows(end.len())
        .rposition(|window| window == end)
        .expect("the flood's last line");
    &bytes[..at + end.len()]
}

/// The id a shell printed after `label`, as in `child 123`.
fn printed_id(output: &[u8], label: &str) -> Option<u32> {
    String::from_utf8_lossy(output).lines().find_map(|line| {
        let digits = line.trim().strip_prefix(label)?.trim();
        digits.parse().ok()
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

    // A child, in a subshell so that the shell itself does not ignore what it ignores,
    // and the shell's own id. What is typed holds `$!` and `$$`, the answer their numbers;
    // a `!` followed by anything but the end of the line is history expansion in some
    // shells.
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

    // Every process the Keeper started, the shell, the child and what they run.
    let everyone = descendants_of(keeper);
    assert!(
        everyone.contains(&shell) && everyone.contains(&child),
        "the Keeper's processes {everyone:?} hold neither the shell {shell} nor the child {child}"
    );

    let stopped = Instant::now();
    client.stop_everything();

    // The hang-up reaches the shell, which ends, and the child ignores it: it is still
    // there when the shell is gone, and falls only to the kill that comes after the
    // terminate signal, which it ignores too.
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
