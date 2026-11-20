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
        companions: None,
        home: place.0.join("home"),
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

#[test]
fn the_keeper_still_leaves_when_its_socket_file_was_removed_under_it() {
    let place = Place::new();
    let keeper = start(&place, Duration::from_millis(300));
    let mut window = connect(&place.socket());
    handshake(&mut window, &Hello::for_this_process("test-window")).expect("hello");

    // Something else removes the file. Nobody can reach the Keeper any more, and it
    // must not stay for ever because of that.
    std::fs::remove_file(place.socket()).expect("remove the socket file");
    drop(window);

    let deadline = Instant::now() + Duration::from_secs(10);
    while !keeper.is_finished() {
        assert!(Instant::now() < deadline, "the Keeper did not leave");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Idle);
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

/// A path for a socket in the folder of `place` that is `length` bytes long, to the byte.
fn socket_of(place: &Place, length: usize) -> PathBuf {
    let folder = place.0.as_os_str().len() + "/".len();
    let path = place.0.join("s".repeat(length - folder));
    assert_eq!(path.as_os_str().len(), length);
    path
}

/// A socket path too long to bind by one byte, that no other test and no other run asks
/// for. The folder of the short place it leads to is removed when the test ends.
struct TooLong {
    wanted: PathBuf,
    short: PathBuf,
}

impl TooLong {
    fn new(place: &Place) -> Self {
        let wanted = socket_of(place, yaar_link::SOCKET_PATH_MAX + 1);
        let short = yaar_link::socket_address(&wanted);
        Self { wanted, short }
    }

    fn folder(&self) -> &Path {
        self.short.parent().expect("the short path has a folder")
    }
}

impl Drop for TooLong {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.folder());
    }
}

fn start_at(socket: &Path, place: &Place) -> JoinHandle<std::io::Result<Exit>> {
    let options = Options {
        socket: socket.to_owned(),
        linger: Duration::from_secs(60),
        app_version: "test-keeper".to_owned(),
        data_dir: place.0.join("data"),
        companions: None,
        home: place.0.join("home"),
    };
    std::thread::spawn(move || run(options))
}

/// Connects as a Window does, by the path the Keeper was asked to listen at, and says
/// hello. Waits for a Keeper to listen there.
fn attach_as_a_window(wanted: &Path) -> (UnixStream, Hello) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match yaar_link::connect(wanted) {
            Ok(mut stream) => {
                if let Ok(keeper) = handshake(&mut stream, &Hello::for_this_process("test-window"))
                {
                    return (stream, keeper);
                }
            }
            Err(e) if Instant::now() > deadline => panic!("no Keeper for {wanted:?}: {e}"),
            Err(_) => {}
        }
        assert!(Instant::now() < deadline, "no Keeper for {wanted:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_socket_path_longer_than_the_system_allows_is_served_at_the_short_one_both_sides_compute() {
    let place = Place::new();

    // What the system allows, to the byte: a Keeper told to listen at a path of that
    // length listens at the path itself, and a Window finds it there.
    let fits = socket_of(&place, yaar_link::SOCKET_PATH_MAX);
    let keeper = start_at(&fits, &place);
    let (mut window, who) = attach_as_a_window(&fits);
    assert_eq!(who.pid, std::process::id());
    assert_eq!(
        window.peer_addr().expect("where it leads").as_pathname(),
        Some(fits.as_path())
    );
    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);

    // One byte more, in the same folder, and the system itself refuses the path: the
    // Keeper cannot be listening there.
    let long = TooLong::new(&place);
    assert_eq!(
        long.wanted.as_os_str().len(),
        yaar_link::SOCKET_PATH_MAX + 1
    );
    assert!(long.short.as_os_str().len() <= yaar_link::SOCKET_PATH_MAX);
    assert!(std::os::unix::net::UnixListener::bind(&long.wanted).is_err());

    let keeper = start_at(&long.wanted, &place);

    // A Window is told the long path, as the Keeper was, and finds the Keeper.
    let (mut window, who) = attach_as_a_window(&long.wanted);
    assert_eq!(who.app_version, "test-keeper");
    assert_eq!(who.pid, std::process::id());

    // Where they met is the short path, computed from the long one alone: the same
    // Keeper answers there to whoever goes straight to it.
    let mut direct = UnixStream::connect(&long.short).expect("the short path is bound");
    let theirs = handshake(&mut direct, &Hello::for_this_process("test-window")).expect("hello");
    assert_eq!(theirs, who);
    // The folder of the short path is the user's alone, and the lock lies beside the
    // socket that is really bound.
    yaar_link::check_private_folder(long.folder()).expect("a folder for the user alone");
    assert!(long.folder().join("keeper.sock.lock").is_file());

    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(keeper.join().expect("join").expect("run"), Exit::Shutdown);
    assert!(!long.short.exists(), "the socket file is removed");
}

#[test]
fn a_second_keeper_asked_for_the_same_long_path_steps_back() {
    let place = Place::new();
    let long = TooLong::new(&place);
    let first = start_at(&long.wanted, &place);
    let (mut window, _) = attach_as_a_window(&long.wanted);

    let second = start_at(&long.wanted, &place);
    assert_eq!(
        second.join().expect("join").expect("run"),
        Exit::AlreadyRunning
    );

    write_message(&mut window, &Message::Shutdown).expect("shutdown");
    assert_eq!(first.join().expect("join").expect("run"), Exit::Shutdown);
}

#[test]
fn the_keeper_does_not_start_in_a_short_folder_that_is_not_the_users_own() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let place = Place::new();
    let long = TooLong::new(&place);
    // Somebody was first at the name anyone can compute, and left the folder open.
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(long.folder())
        .expect("make the folder");
    std::fs::set_permissions(long.folder(), std::fs::Permissions::from_mode(0o755))
        .expect("open it");

    let refused = start_at(&long.wanted, &place)
        .join()
        .expect("join")
        .expect_err("the Keeper must not start there");
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    // Nothing was put there: no socket, no lock.
    let left: Vec<_> = std::fs::read_dir(long.folder())
        .expect("read the folder")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
