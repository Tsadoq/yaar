//! System tests of the walking skeleton: the two built programs and the Link between
//! them, with no window on any screen.

use std::process::{Command, Stdio};
use std::time::Duration;

use yaar_link::{Message, PROTOCOL_VERSION, write_message};
use yaar_system_tests::{Place, attach, is_alive, program, wait_until};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn both_programs_say_their_version() {
    for (name, expected) in [
        ("yaar-review", format!("yaar-review {VERSION}\n")),
        ("yaar-keeper", format!("yaar-keeper {VERSION}\n")),
    ] {
        let out = Command::new(program(name))
            .arg("--version")
            .output()
            .expect("run the program");
        assert!(out.status.success(), "{name} --version failed");
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    }
}

#[test]
fn the_keeper_answers_a_scripted_window_and_stops_when_asked() {
    let place = Place::new();
    let mut keeper = place
        .command(&program("yaar-keeper"))
        .arg("--socket")
        .arg(place.socket())
        .stdin(Stdio::null())
        .spawn()
        .expect("start the Keeper");

    let (mut window, who) = attach(&place.socket());
    assert_eq!(who.protocol, PROTOCOL_VERSION);
    assert_eq!(who.app_version, VERSION);
    assert_eq!(who.pid, keeper.id());

    write_message(&mut window, &Message::Shutdown).expect("ask for the stop");
    assert!(
        wait_until(Duration::from_secs(5), || keeper
            .try_wait()
            .expect("wait")
            .is_some()),
        "the Keeper did not stop"
    );
    assert!(keeper.wait().expect("wait").success());
    assert!(!place.socket().exists(), "the socket file is removed");
}

#[test]
fn the_window_program_starts_a_keeper_that_outlives_it() {
    let place = Place::new();
    // The Window program, told to find or start the Keeper as the window would and leave.
    let out = place
        .command(&program("yaar-review"))
        .arg("--keeper-status")
        .env("YAAR_SOCKET", place.socket())
        .env("YAAR_KEEPER", program("yaar-keeper"))
        .env("YAAR_KEEPER_LINGER_MS", "20000")
        .output()
        .expect("run the Window program");
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{said}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let field = |name: &str| -> String {
        said.split_whitespace()
            .find_map(|word| word.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in: {said}"))
            .to_owned()
    };
    assert_eq!(field("version"), VERSION);
    assert_eq!(field("started"), "true");
    let pid: u32 = field("pid").parse().expect("pid");

    // The Window program is gone. Its Keeper still runs, in a session of its own.
    assert!(is_alive(pid), "the Keeper ended with the Window program");
    assert_eq!(
        field("session"),
        pid.to_string(),
        "the Keeper leads its own session"
    );
    // SAFETY: `getsid` only reads.
    let my_session = unsafe { libc::getsid(0) };
    assert_ne!(my_session as u32, pid);

    // A second Window finds the same Keeper and starts none.
    let again = place
        .command(&program("yaar-review"))
        .arg("--keeper-status")
        .env("YAAR_SOCKET", place.socket())
        .env("YAAR_KEEPER", program("yaar-keeper"))
        .output()
        .expect("run the Window program again");
    let said_again = String::from_utf8_lossy(&again.stdout).into_owned();
    assert!(said_again.contains(&format!("pid={pid} ")), "{said_again}");
    assert!(said_again.contains("started=false"), "{said_again}");

    let (mut window, who) = attach(&place.socket());
    assert_eq!(who.pid, pid);
    write_message(&mut window, &Message::Shutdown).expect("ask for the stop");
    assert!(
        wait_until(Duration::from_secs(5), || !is_alive(pid)),
        "the Keeper did not stop"
    );
}
