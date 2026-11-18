//! Component test: the Keeper through the surface a Window uses, on a real socket.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yaar_keeper::{Exit, Options, run};
use yaar_link::{Hello, Message, PROTOCOL_VERSION, handshake, write_message};

/// A socket path short enough for every system, in a folder of this test's own.
struct Place(PathBuf);

impl Place {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(PathBuf::from(format!(
            "/tmp/yaar-keeper-{}-{n}",
            std::process::id()
        )))
    }
    fn socket(&self) -> PathBuf {
        self.0.join("k.sock")
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn start(place: &Place, linger: Duration) -> JoinHandle<std::io::Result<Exit>> {
    let options = Options {
        socket: place.socket(),
        linger,
        app_version: "test-keeper".to_owned(),
        data_dir: place.0.join("data"),
    };
    std::thread::spawn(move || run(options))
}

fn connect(socket: &Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return stream,
            Err(e) if Instant::now() > deadline => panic!("no Keeper at {socket:?}: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

#[test]
fn a_window_learns_who_the_keeper_is_and_can_stop_it() {
    let place = Place::new();
    let keeper = start(&place, Duration::from_secs(60));

    let mut window = connect(&place.socket());
    let theirs = handshake(&mut window, &Hello::for_this_process("test-window")).expect("hello");
    assert_eq!(theirs.protocol, PROTOCOL_VERSION);
    assert_eq!(theirs.app_version, "test-keeper");
    assert_eq!(theirs.pid, std::process::id());

    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    assert!(!place.socket().exists(), "the socket file is removed");
}

#[test]
fn a_window_of_another_protocol_version_is_still_answered_and_obeyed() {
    let place = Place::new();
    let keeper = start(&place, Duration::from_secs(60));

    let mut window = connect(&place.socket());
    let mut mine = Hello::for_this_process("window-from-the-future");
    mine.protocol = PROTOCOL_VERSION + 1;
    let theirs = handshake(&mut window, &mine).expect("hello");
    assert!(!mine.speaks_with(&theirs));

    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
}

#[test]
fn the_keeper_leaves_when_no_window_needs_it_and_stays_while_one_does() {
    let place = Place::new();
    let linger = Duration::from_millis(300);
    let keeper = start(&place, linger);

    let mut window = connect(&place.socket());
    handshake(&mut window, &Hello::for_this_process("test-window")).expect("hello");
    std::thread::sleep(linger * 3);
    assert!(!keeper.is_finished(), "an attached Window keeps the Keeper");

    let closed = Instant::now();
    drop(window);
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Idle);
    assert!(closed.elapsed() >= linger, "it lingers before it leaves");
}

#[test]
fn a_second_keeper_on_the_same_socket_steps_back() {
    let place = Place::new();
    let first = start(&place, Duration::from_secs(60));
    let mut window = connect(&place.socket());
    handshake(&mut window, &Hello::for_this_process("test-window")).expect("hello");

    let second = start(&place, Duration::from_secs(60));
    assert_eq!(
        second.join().expect("join").expect("run"),
        Exit::AlreadyRunning
    );

    // The first one is untouched by the attempt.
    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(first.join().expect("join").expect("run"), Exit::Shutdown);
}

#[test]
fn a_socket_file_left_by_a_dead_keeper_does_not_block_the_next_one() {
    let place = Place::new();
    std::fs::create_dir_all(&place.0).expect("folder");
    // A bound and dropped listener leaves its file behind, as a killed Keeper does.
    drop(std::os::unix::net::UnixListener::bind(place.socket()).expect("bind"));
    assert!(place.socket().exists());

    let keeper = start(&place, Duration::from_secs(60));
    let mut window = connect_live(&place.socket());
    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
}

/// Connects and says hello, retrying while the stale file is still in the way.
fn connect_live(socket: &Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(mut stream) = UnixStream::connect(socket)
            && handshake(&mut stream, &Hello::for_this_process("test-window")).is_ok()
        {
            return stream;
        }
        assert!(Instant::now() < deadline, "no live Keeper at {socket:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
