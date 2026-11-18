//! Tests of what the system tests stand on: the world of one test, its repositories and
//! the tap on the Link.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use yaar_link::{
    Hello, LinkError, Message, Request, RequestId, RequestKind, handshake, read_message,
    write_message,
};
use yaar_system_tests::{Client, DEADLINE, World, attach};

#[test]
fn the_same_steps_in_two_worlds_give_the_same_commits_on_main() {
    let steps = |world: &World| {
        let repo = world.repository("app");
        assert_eq!(repo.git(&["branch", "--show-current"]), "main");
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), "1");
        repo.write("src/deep/a.txt", "a\n");
        let second = repo.commit("Add a file");
        assert_eq!(repo.git(&["status", "--porcelain"]), "", "all is committed");
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), "2");
        assert_eq!(repo.git(&["show", "HEAD:src/deep/a.txt"]), "a");
        assert_eq!(
            repo.path(),
            world.home().with_file_name("repos").join("app")
        );
        second
    };
    let (one, other) = (World::new(), World::new());
    assert_eq!(steps(&one), steps(&other));
    // Two repositories of one world do not meet.
    assert_ne!(
        one.repository("second").path(),
        one.repository("third").path()
    );
}

#[test]
fn a_program_of_the_world_sees_the_worlds_environment_and_nothing_of_this_process() {
    let world = World::new();
    let printed = world
        .command(Path::new("/usr/bin/env"))
        .output()
        .expect("run env");
    let printed = String::from_utf8_lossy(&printed.stdout).into_owned();
    let value = |name: &str| {
        printed
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(PathBuf::from)
    };
    assert_eq!(value("HOME"), Some(world.home()));
    assert_eq!(value("YAAR_SOCKET"), Some(world.socket()));
    assert_eq!(value("GIT_CONFIG_GLOBAL"), Some(PathBuf::from("/dev/null")));
    assert_eq!(value("GIT_CONFIG_NOSYSTEM"), Some(PathBuf::from("1")));
    assert_eq!(value("PATH"), std::env::var_os("PATH").map(PathBuf::from));
    // Cargo sets this for every test program; the world does not pass it on.
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    assert_eq!(value("CARGO_MANIFEST_DIR"), None);
    // Every place the programs are told lies in the world, whose folder is the one
    // above its home.
    let root = world.home().with_file_name("");
    for name in ["XDG_DATA_HOME", "XDG_RUNTIME_DIR"] {
        let place = value(name).unwrap_or_else(|| panic!("no {name}"));
        assert!(place.starts_with(&root), "{name}={place:?}");
    }
    assert!(world.data_dir().starts_with(&root));
}

fn opens(path: &str) -> Request {
    Request::OpenFolder {
        path: PathBuf::from(path),
    }
}

#[test]
fn a_tap_passes_the_link_on_both_ways_and_keeps_the_requests_it_saw() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let mut tap = world.tap();
    assert_ne!(tap.socket(), world.socket());

    // Through the tap the client meets the Keeper of the world and is answered by it.
    let mut client = Client::attach(&tap.socket());
    assert_eq!(client.keeper().pid, keeper);
    assert_eq!(client.workspaces(), vec![]);
    let nowhere = [
        "/nowhere/first",
        "/nowhere/second",
        "/nowhere/third",
        "/nowhere/fourth",
    ];
    for path in &nowhere[..2] {
        assert!(client.request(opens(path)).is_err());
    }

    // A wait passes the request it returns and every request before it.
    let is_open = |request: &Request| request.kind() == RequestKind::OpenFolder;
    assert_eq!(tap.wait_for_request(DEADLINE, is_open), opens(nowhere[0]));
    assert_eq!(tap.wait_for_request(DEADLINE, is_open), opens(nowhere[1]));
    for path in &nowhere[2..] {
        assert!(client.request(opens(path)).is_err());
    }
    // The list request was passed by the first wait, so it does not answer this one;
    // the third folder is passed with the fourth.
    assert_eq!(
        tap.wait_for_request(DEADLINE, |request| {
            *request == Request::ListWorkspaces || *request == opens(nowhere[3])
        }),
        opens(nowhere[3])
    );

    // The whole list stays, whatever the waits have read.
    let mut all = vec![Request::ListWorkspaces];
    all.extend(nowhere.iter().map(|path| opens(path)));
    assert_eq!(tap.requests(), all);

    // A request that may still be on its way when the wait begins is waited for.
    let asking = std::thread::spawn(move || {
        client.workspaces();
        client
    });
    assert_eq!(
        tap.wait_for_request(DEADLINE, |request| *request == Request::ListWorkspaces),
        Request::ListWorkspaces
    );
    drop(asking.join().expect("the client was answered"));
}

#[test]
fn a_tap_passes_on_a_frame_it_cannot_read_and_serves_the_next_window() {
    let mut world = World::new();
    world.start_keeper();
    let tap = world.tap();

    // A frame of a kind no version knows, after the hello. The tap hands it through,
    // and the Keeper, which cannot read it either, ends the connection: the end reaches
    // this side through the tap.
    let mut odd = UnixStream::connect(tap.socket()).expect("connect to the tap");
    handshake(&mut odd, &Hello::for_this_process("odd-window")).expect("hello");
    odd.write_all(&[3, 0, 0, 0, 99, b'?', b'?'])
        .expect("write the frame");
    odd.set_read_timeout(Some(DEADLINE))
        .expect("set the deadline of the read");
    assert!(matches!(read_message(&mut odd), Err(LinkError::Closed)));
    assert_eq!(tap.requests(), vec![]);

    // The tap is none the worse for it.
    let mut client = Client::attach(&tap.socket());
    assert_eq!(client.workspaces(), vec![]);
    assert_eq!(tap.requests(), vec![Request::ListWorkspaces]);
}

/// A process of the test held still: it runs no further until this is dropped.
struct Held(libc::pid_t);

impl Held {
    /// Stops a process this test started, and returns once the system says it stands
    /// still.
    fn stop(child: u32) -> Held {
        let pid = child as libc::pid_t;
        let mut status = 0;
        // SAFETY: a signal to a child of this process, and a wait for that child that
        // collects nothing: with `WUNTRACED` it returns when the child has stopped.
        let stopped = unsafe {
            libc::kill(pid, libc::SIGSTOP) == 0
                && libc::waitpid(pid, &mut status, libc::WUNTRACED) == pid
                && libc::WIFSTOPPED(status)
        };
        assert!(stopped, "the process {child} was not stopped");
        Held(pid)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: a signal to a child of this process.
        unsafe { libc::kill(self.0, libc::SIGCONT) };
    }
}

#[test]
fn a_tap_lists_a_request_before_it_hands_it_to_the_keeper() {
    let mut world = World::new();
    let keeper = world.start_keeper();
    let mut tap = world.tap();
    let (mut window, who) = attach(&tap.socket());
    assert_eq!(who.pid, keeper);

    // The Keeper stands still and takes nothing, and the request is larger than what
    // a socket holds for it: it cannot have reached the Keeper, let alone be answered.
    // The tap has taken it from the Window, and that is when it is in the list. So a
    // test that reads the list after a reply never finds the list one request short.
    let held = Held::stop(keeper);
    let large = opens(&format!("/nowhere/{}", "x".repeat(8 * 1024 * 1024)));
    let id = RequestId(7);
    let asked = Message::Request {
        id,
        request: large.clone(),
    };
    write_message(&mut window, &asked).expect("send the request");
    assert_eq!(tap.wait_for_request(DEADLINE, |_| true), large);
    assert_eq!(tap.requests(), vec![large.clone()]);

    // Let go, the Keeper gets the request as it was sent and answers it.
    drop(held);
    window
        .set_read_timeout(Some(DEADLINE))
        .expect("set the deadline of the read");
    match read_message(&mut window) {
        Ok(Message::Reply { to, reply: Err(_) }) => assert_eq!(to, id),
        other => panic!("the Keeper did not refuse the folder: {other:?}"),
    }
    assert_eq!(tap.requests(), vec![large]);
}
