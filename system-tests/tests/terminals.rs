//! System tests of the terminals of the Keeper through the scripted client, and of the
//! stand-ins the world holds for them.

use std::time::Duration;

use yaar_link::TerminalSize;
use yaar_system_tests::{DEADLINE, World, descendants_of, is_alive, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

/// Waits until every process in `pids` is gone, which a test that started them has to do
/// before it ends.
fn wait_until_gone(pids: &[u32]) {
    assert!(
        wait_until(DEADLINE, || pids.iter().all(|pid| !is_alive(*pid))),
        "still there: {:?}",
        pids.iter()
            .filter(|pid| is_alive(**pid))
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_shell_of_a_terminal_in_a_world_is_the_one_of_the_world() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");

    // What the typed line holds is `$SHELL` and `$HOME`: the answer is what they are.
    client.type_into(&terminal, b"echo shell-is-$SHELL home-is-$HOME\r");
    let heard = client.wait_for_output(&terminal, DEADLINE, |bytes| {
        String::from_utf8_lossy(bytes).contains("shell-is-/bin/sh home-is-/")
    });
    assert!(
        String::from_utf8_lossy(&heard).contains(&format!(
            "shell-is-/bin/sh home-is-{}",
            world.home().display()
        )),
        "{}",
        String::from_utf8_lossy(&heard)
    );

    let started = descendants_of(keeper);
    drop(client);
    drop(world);
    wait_until_gone(&started);
}

#[test]
fn the_key_recorder_writes_down_every_byte_a_program_receives_as_it_was_typed() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    client.wait_for_output(&terminal, DEADLINE, |bytes| !bytes.is_empty());

    // The recorder is started as a user starts a program. Its file is made once the
    // terminal is raw, so every byte typed after that is recorded as typed.
    let keys = world.home().join("keys");
    let recorder = world.key_recorder();
    client.type_into(
        &terminal,
        format!("{} {}\r", recorder.display(), keys.display()).as_bytes(),
    );
    assert!(
        wait_until(DEADLINE, || keys.exists()),
        "the recorder did not start"
    );

    // Every value a byte can have, among them what a cooked terminal would turn into a
    // signal, a line end or an end of input.
    let every_byte: Vec<u8> = (0..=255).collect();
    for part in every_byte.chunks(37) {
        client.type_into(&terminal, part);
    }
    assert!(
        wait_until(DEADLINE, || std::fs::read(&keys)
            .is_ok_and(|got| got == every_byte)),
        "the recorder holds {:?}",
        std::fs::read(&keys)
    );
    let started = descendants_of(keeper);
    drop(client);
    drop(world);
    wait_until_gone(&started);
}

#[test]
fn waiting_for_output_that_does_not_come_fails_with_what_did_come() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let repo = world.repository("app");
    let mut client = world.client();
    let (_, workspace) = client.open_folder(repo.path()).expect("open the folder");
    let terminal = client
        .open_terminal(&workspace, SIZE)
        .expect("open a terminal");
    let started = descendants_of(keeper);

    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.wait_for_output(&terminal, Duration::from_millis(500), |bytes| {
            String::from_utf8_lossy(bytes).contains("never printed")
        });
    }))
    .expect_err("the wait accepted output that never came");
    let said = failed
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failed.downcast_ref::<&str>().copied())
        .expect("the wait failed with a message");
    assert!(said.contains("was not what was wanted within"), "{said}");

    // The terminal's program is the world's to end, and this test waits until it is.
    drop(client);
    drop(world);
    wait_until_gone(&started);
}
